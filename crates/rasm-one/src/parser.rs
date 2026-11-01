//! Parses source lines into atoms and symbols.

use std::collections::HashMap;

use crate::atom::{Atom, AtomKind, DataItem};
use crate::error::{Diagnostic, Related};
use crate::expr::{BinaryOp, Expr, SymId, UnaryOp};
use crate::isa::encode::Instruction;
use crate::isa::table::{rows_for, InsnRow};
use crate::isa::{IndexReg, Mode, Operand};
use crate::lexer::{LexError, Lexer, Punct, Tok};
use crate::source::{FileId, Span};
use crate::types::{Cpu, MemKind, SectionKind, Size};

/// A symbol being collected.
#[derive(Debug, Clone)]
pub(crate) struct SymbolEntry {
    pub name: String,
    /// Where the label is defined; `None` if it has only been referenced.
    pub defined_at: Option<Span>,
}

/// A section being collected.
#[derive(Debug, Clone)]
pub(crate) struct SectionEntry {
    pub name: String,
    pub kind: SectionKind,
    pub mem: MemKind,
    pub atoms: Vec<Atom>,
}

/// Output of parsing.
#[derive(Debug, Default)]
pub(crate) struct Program {
    pub sections: Vec<SectionEntry>,
    pub symbols: Vec<SymbolEntry>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Name of the section used before any `section` directive.
const DEFAULT_SECTION: &str = "CODE";

/// A register operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reg {
    Data(u8),
    Addr(u8),
    Pc,
}

fn parse_reg(name: &str) -> Option<Reg> {
    let lower = name.to_ascii_lowercase();
    if lower == "sp" {
        return Some(Reg::Addr(7));
    }
    if lower == "pc" {
        return Some(Reg::Pc);
    }
    match lower.as_bytes() {
        [b'd', n @ b'0'..=b'7'] => Some(Reg::Data(n - b'0')),
        [b'a', n @ b'0'..=b'7'] => Some(Reg::Addr(n - b'0')),
        _ => None,
    }
}

fn is_directive(name: &str) -> bool {
    matches!(name, "dc" | "ds" | "blk" | "section")
}

/// Parses whole files into a [`Program`].
pub(crate) struct Parser {
    cpu: Cpu,
    program: Program,
    symbol_ids: HashMap<String, SymId>,
    sections_by_key: HashMap<(String, SectionKind), usize>,
    current: Option<usize>,
}

impl Parser {
    pub fn new(cpu: Cpu) -> Self {
        Self {
            cpu,
            program: Program::default(),
            symbol_ids: HashMap::new(),
            sections_by_key: HashMap::new(),
            current: None,
        }
    }

    pub fn finish(self) -> Program {
        self.program
    }

    pub fn parse_file(&mut self, file: FileId, source: &[u8]) {
        for (index, raw) in source.split(|&b| b == b'\n').enumerate() {
            let text = raw.strip_suffix(b"\r").unwrap_or(raw);
            let line = LineParser {
                parser: self,
                lexer: Lexer::new(text),
                file,
                line: index as u32 + 1,
            };
            if let Err(diagnostic) = line.parse() {
                self.program.diagnostics.push(diagnostic);
            }
        }
    }

    fn symbol(&mut self, name: &str) -> SymId {
        if let Some(&id) = self.symbol_ids.get(name) {
            return id;
        }
        let id = self.program.symbols.len() as SymId;
        self.program.symbols.push(SymbolEntry {
            name: name.to_string(),
            defined_at: None,
        });
        self.symbol_ids.insert(name.to_string(), id);
        id
    }

    /// The current section, creating the default one if there is none yet.
    fn section(&mut self) -> usize {
        match self.current {
            Some(section) => section,
            None => {
                let section = self.open_section(DEFAULT_SECTION, SectionKind::Code, MemKind::Any);
                self.current = Some(section);
                section
            }
        }
    }

    /// Finds or creates a section. Name and kind identify it, so one name can
    /// be used for both code and data. A later memory requirement replaces an
    /// earlier one.
    fn open_section(&mut self, name: &str, kind: SectionKind, mem: MemKind) -> usize {
        let key = (name.to_string(), kind);
        if let Some(&section) = self.sections_by_key.get(&key) {
            if mem != MemKind::Any {
                self.program.sections[section].mem = mem;
            }
            return section;
        }
        let section = self.program.sections.len();
        self.program.sections.push(SectionEntry {
            name: name.to_string(),
            kind,
            mem,
            atoms: Vec::new(),
        });
        self.sections_by_key.insert(key, section);
        section
    }

    fn push_atom(&mut self, kind: AtomKind, span: Span) {
        let section = self.section();
        self.program.sections[section]
            .atoms
            .push(Atom { kind, span });
    }
}

struct LineParser<'p, 'a> {
    parser: &'p mut Parser,
    lexer: Lexer<'a>,
    file: FileId,
    line: u32,
}

impl LineParser<'_, '_> {
    fn span(&self, start: usize) -> Span {
        Span {
            file: self.file,
            line: self.line,
            col: start as u32 + 1,
            len: self.lexer.pos().saturating_sub(start).max(1) as u32,
        }
    }

    fn error_at(&self, code: &'static str, message: impl Into<String>, start: usize) -> Diagnostic {
        Diagnostic::error(code, message, self.span(start))
    }

    fn lex_error(&self, error: LexError) -> Diagnostic {
        Diagnostic::error(
            "syntax",
            error.message,
            Span {
                file: self.file,
                line: self.line,
                col: error.pos as u32 + 1,
                len: 1,
            },
        )
    }

    fn peek(&self) -> Result<Tok<'_>, Diagnostic> {
        self.lexer.peek().map_err(|e| self.lex_error(e))
    }

    fn peek_is(&self, punct: Punct) -> Result<bool, Diagnostic> {
        Ok(self.peek()? == Tok::Punct(punct))
    }

    fn eat(&mut self, punct: Punct) -> Result<bool, Diagnostic> {
        if self.peek_is(punct)? {
            self.advance()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn advance(&mut self) -> Result<(), Diagnostic> {
        self.lexer
            .next_tok()
            .map(|_| ())
            .map_err(|e| self.lex_error(e))
    }

    /// Reads an identifier if one is next, returned as an owned string so the
    /// parser stays free to mutate itself.
    fn eat_ident(&mut self) -> Result<Option<String>, Diagnostic> {
        if let Tok::Ident(name) = self.peek()? {
            let name = name.to_string();
            self.advance()?;
            return Ok(Some(name));
        }
        Ok(None)
    }

    fn expect(&mut self, punct: Punct, what: &str) -> Result<(), Diagnostic> {
        if self.eat(punct)? {
            return Ok(());
        }
        Err(self.error_at("syntax", format!("{what} expected"), self.lexer.pos()))
    }

    fn parse(mut self) -> Result<(), Diagnostic> {
        if self.lexer.at_blank() {
            self.lexer.skip_blanks();
        } else {
            match self.peek() {
                Ok(Tok::Punct(Punct::Star)) => return Ok(()),
                Ok(Tok::End) => return Ok(()),
                _ => {}
            }
            return self.parse_after_column_zero();
        }
        // An indented word is a label only if a colon follows it.
        let start = self.lexer.pos();
        if let Tok::Ident(_) = self.peek()? {
            let saved = self.lexer.pos();
            if let Some(name) = self.eat_ident()? {
                if self.peek_is(Punct::Colon)? {
                    self.eat_colons()?;
                    let span = self.span(start);
                    return self.parse_statement(Some((name, span)));
                }
            }
            self.lexer.set_pos(saved);
        }
        self.parse_statement(None)
    }

    fn eat_colons(&mut self) -> Result<(), Diagnostic> {
        self.expect(Punct::Colon, "colon")?;
        self.eat(Punct::Colon)?;
        Ok(())
    }

    fn parse_after_column_zero(&mut self) -> Result<(), Diagnostic> {
        if self.peek_is(Punct::Dot)? {
            return Err(self.error_at(
                "unsupported",
                "local labels are not supported yet",
                self.lexer.pos(),
            ));
        }
        let start = self.lexer.pos();
        let Some(name) = self.eat_ident()? else {
            return Err(self.error_at("syntax", "label expected", start));
        };
        let span = self.span(start);
        if self.peek_is(Punct::Colon)? {
            self.eat_colons()?;
        } else {
            let lower = name.to_ascii_lowercase();
            if rows_for(&lower).is_some() || is_directive(&lower) {
                self.parser.program.diagnostics.push(Diagnostic::warning(
                    "label-is-mnemonic",
                    format!("label {name} has the name of a mnemonic; indent the instruction"),
                    span,
                ));
            }
        }
        if !self.lexer.at_blank() && self.peek()? != Tok::End {
            return Err(self.error_at("syntax", "unexpected text after label", self.lexer.pos()));
        }
        self.parse_statement(Some((name, span)))
    }

    fn define_label(&mut self, name: &str, span: Span) -> Result<(), Diagnostic> {
        if parse_reg(name).is_some() {
            return Err(Diagnostic::error(
                "reserved-name",
                format!("{name} is a register and cannot be a label"),
                span,
            ));
        }
        let id = self.parser.symbol(name);
        let entry = &mut self.parser.program.symbols[id as usize];
        if let Some(first) = entry.defined_at {
            let mut diagnostic =
                Diagnostic::error("redefined-label", format!("label {name} redefined"), span);
            diagnostic.related.push(Related {
                message: "first defined here".to_string(),
                span: first,
            });
            return Err(diagnostic);
        }
        entry.defined_at = Some(span);
        self.parser.push_atom(AtomKind::Label(id), span);
        Ok(())
    }

    /// Parses what follows the label, if any, and defines the label. A label
    /// on an instruction's line takes the instruction's aligned address,
    /// whereas a label on any other line keeps the address it finds.
    fn parse_statement(&mut self, label: Option<(String, Span)>) -> Result<(), Diagnostic> {
        // Symbols are listed in order of first mention, and a line's label
        // comes before anything its operands refer to.
        if let Some((name, _)) = &label {
            self.parser.symbol(name);
        }
        self.lexer.skip_blanks();
        let start = self.lexer.pos();
        let Some(name) = self.eat_ident()? else {
            if let Some((label, span)) = label {
                self.define_label(&label, span)?;
            }
            return match self.peek()? {
                Tok::End => Ok(()),
                _ => Err(self.error_at("syntax", "mnemonic expected", start)),
            };
        };
        let size = self.parse_size_suffix()?;
        let span = self.span(start);
        let lower = name.to_ascii_lowercase();

        if let Some(rows) = rows_for(&lower) {
            self.lexer.skip_blanks();
            let instruction = self.parse_instruction(&lower, rows, size, span);
            self.parser.push_atom(AtomKind::AlignInstruction, span);
            if let Some((label, label_span)) = label {
                self.define_label(&label, label_span)?;
            }
            let instruction = instruction?;
            self.warn_if_bss(span);
            self.parser
                .push_atom(AtomKind::Instruction(instruction), span);
            return Ok(());
        }

        if let Some((label, label_span)) = label {
            self.define_label(&label, label_span)?;
        }
        self.lexer.skip_blanks();
        match lower.as_str() {
            "dc" => self.parse_dc(size, span),
            "ds" | "blk" => self.parse_space(&lower, size, span),
            "section" => self.parse_section(size, span),
            _ => Err(Diagnostic::error(
                "unknown-mnemonic",
                format!("unknown mnemonic {name}"),
                span,
            )),
        }
    }

    /// Reads `.b`, `.w`, `.l` or `.s`, if present.
    fn parse_size_suffix(&mut self) -> Result<Option<Size>, Diagnostic> {
        if !self.peek_is(Punct::Dot)? {
            return Ok(None);
        }
        let start = self.lexer.pos();
        self.advance()?;
        let Some(name) = self.eat_ident()? else {
            return Err(self.error_at("bad-size", "size expected after dot", start));
        };
        Size::from_suffix(&name)
            .map(Some)
            .ok_or_else(|| self.error_at("bad-size", format!("illegal size .{name}"), start))
    }

    fn warn_if_bss(&mut self, span: Span) {
        let section = self.parser.section();
        if self.parser.program.sections[section].kind == SectionKind::Bss {
            self.parser.program.diagnostics.push(Diagnostic::warning(
                "data-in-bss",
                "initialised data in a BSS section",
                span,
            ));
        }
    }

    fn parse_instruction(
        &mut self,
        mnemonic: &str,
        rows: &'static [InsnRow],
        size: Option<Size>,
        span: Span,
    ) -> Result<Instruction, Diagnostic> {
        let unsized_insn = rows.iter().all(|r| r.default_size.is_none());
        if unsized_insn {
            if size.is_some() {
                self.parser.program.diagnostics.push(Diagnostic::warning(
                    "size-ignored",
                    format!("{mnemonic} has no size; the suffix is ignored"),
                    span,
                ));
            }
            // Text after an operandless mnemonic is a comment.
            if rows.iter().all(|r| r.operands.is_empty()) {
                return self.select_row(mnemonic, rows, None, Vec::new(), span);
            }
        }
        if let Some(size) = size {
            if !rows.iter().any(|r| r.sizes.contains(&size)) {
                return Err(Diagnostic::error(
                    "bad-size",
                    format!("{mnemonic} cannot have this size"),
                    span,
                ));
            }
        }
        let operands = self.parse_operands()?;
        self.select_row(mnemonic, rows, size, operands, span)
    }

    fn parse_operands(&mut self) -> Result<Vec<Operand>, Diagnostic> {
        let mut operands = Vec::new();
        if self.peek()? == Tok::End {
            return Ok(operands);
        }
        loop {
            operands.push(self.parse_operand()?);
            if !self.eat(Punct::Comma)? {
                break;
            }
        }
        if self.peek()? != Tok::End {
            return Err(self.error_at("syntax", "unexpected text in operand", self.lexer.pos()));
        }
        Ok(operands)
    }

    fn select_row(
        &self,
        mnemonic: &str,
        rows: &'static [InsnRow],
        size: Option<Size>,
        operands: Vec<Operand>,
        span: Span,
    ) -> Result<Instruction, Diagnostic> {
        let fits = |row: &InsnRow| {
            row.operands.len() == operands.len()
                && row
                    .operands
                    .iter()
                    .zip(&operands)
                    .all(|(set, op)| set.intersects(op.mode.class()))
                && row
                    .default_size
                    .map_or(true, |default| row.sizes.contains(&size.unwrap_or(default)))
        };
        let mut matching = rows.iter().filter(|row| fits(row)).peekable();
        if matching.peek().is_none() {
            return Err(Diagnostic::error(
                "bad-operands",
                format!("invalid operands or size for {mnemonic}"),
                span,
            ));
        }
        let Some(row) = matching.find(|row| row.cpus.contains(self.parser.cpu)) else {
            return Err(Diagnostic::error(
                "unsupported-cpu",
                format!("{mnemonic} is not available on the selected CPU"),
                span,
            ));
        };
        Ok(Instruction {
            row,
            size: size.or(row.default_size),
            operands,
        })
    }

    fn parse_operand(&mut self) -> Result<Operand, Diagnostic> {
        let start = self.lexer.pos();
        let mode = self.parse_mode()?;
        Ok(Operand {
            mode,
            span: self.span(start),
        })
    }

    fn parse_mode(&mut self) -> Result<Mode, Diagnostic> {
        if self.eat(Punct::Hash)? {
            return Ok(Mode::Immediate(self.parse_expr()?));
        }
        if let Tok::Ident(name) = self.peek()? {
            match parse_reg(name) {
                Some(Reg::Data(n)) => {
                    self.advance()?;
                    return Ok(Mode::DataReg(n));
                }
                Some(Reg::Addr(n)) => {
                    self.advance()?;
                    return Ok(Mode::AddrReg(n));
                }
                _ => {}
            }
        }
        if let Some(mode) = self.try_predecrement()? {
            return Ok(mode);
        }
        if let Some(mode) = self.try_parenthesised()? {
            return Ok(mode);
        }
        let expr = self.parse_expr()?;
        if self.eat(Punct::LParen)? {
            return self.parse_base_and_index(expr);
        }
        match self.parse_size_suffix()? {
            Some(Size::Word) => Ok(Mode::AbsWord(expr)),
            Some(Size::Long) | None => Ok(Mode::AbsLong(expr)),
            Some(_) => Err(self.error_at(
                "bad-size",
                "an absolute address is .w or .l",
                self.lexer.pos(),
            )),
        }
    }

    /// Recognises `-(An)`. Anything else, such as `-(4)(a0)`, is left for
    /// the expression parser.
    fn try_predecrement(&mut self) -> Result<Option<Mode>, Diagnostic> {
        let saved = self.lexer.pos();
        if self.eat(Punct::Minus)? && self.eat(Punct::LParen)? {
            if let Some(name) = self.eat_ident()? {
                if let Some(Reg::Addr(n)) = parse_reg(&name) {
                    if self.eat(Punct::RParen)? {
                        return Ok(Some(Mode::PreDec(n)));
                    }
                }
            }
        }
        self.lexer.set_pos(saved);
        Ok(None)
    }

    /// Recognises `(An)`, `(An)+`, `(d,An)`, `(An,Xn)`, `(d,An,Xn)` and the
    /// PC forms. Returns `None` for a parenthesised expression such as
    /// `(4+2)*3(a0)`, leaving the position unchanged.
    fn try_parenthesised(&mut self) -> Result<Option<Mode>, Diagnostic> {
        let saved = self.lexer.pos();
        if !self.eat(Punct::LParen)? {
            return Ok(None);
        }
        // A leading base register has no displacement, so `(An)` is plain
        // indirect rather than the zero displacement this call reports.
        if self.next_is_base_register()? {
            let mode = self.parse_base_and_index(Expr::Num(0))?;
            if let Mode::Disp(_, n) = &mode {
                let n = *n;
                if self.eat(Punct::Plus)? {
                    return Ok(Some(Mode::PostInc(n)));
                }
                return Ok(Some(Mode::Indirect(n)));
            }
            return Ok(Some(mode));
        }
        let parsed = self.parse_expr();
        if let Ok(expr) = parsed {
            if self.eat(Punct::Comma)? {
                return self.parse_base_and_index(expr).map(Some);
            }
        }
        self.lexer.set_pos(saved);
        Ok(None)
    }

    fn next_is_base_register(&self) -> Result<bool, Diagnostic> {
        Ok(match self.peek()? {
            Tok::Ident(name) => matches!(parse_reg(name), Some(Reg::Addr(_) | Reg::Pc)),
            _ => false,
        })
    }

    /// Parses `An)`, `An,Xn)`, `PC)` or `PC,Xn)` after the displacement.
    fn parse_base_and_index(&mut self, disp: Expr) -> Result<Mode, Diagnostic> {
        let start = self.lexer.pos();
        let base = self
            .eat_ident()?
            .as_deref()
            .and_then(parse_reg)
            .filter(|reg| !matches!(reg, Reg::Data(_)));
        let Some(base) = base else {
            return Err(self.error_at("syntax", "address register or PC expected", start));
        };
        let index = if self.eat(Punct::Comma)? {
            Some(self.parse_index()?)
        } else {
            None
        };
        self.expect(Punct::RParen, ")")?;
        Ok(match (base, index) {
            (Reg::Addr(n), None) => Mode::Disp(disp, n),
            (Reg::Addr(n), Some(index)) => Mode::Index(disp, n, index),
            (_, None) => Mode::PcDisp(disp),
            (_, Some(index)) => Mode::PcIndex(disp, index),
        })
    }

    fn parse_index(&mut self) -> Result<IndexReg, Diagnostic> {
        let start = self.lexer.pos();
        let reg = match self.eat_ident()?.as_deref().and_then(parse_reg) {
            Some(Reg::Data(n)) => n,
            Some(Reg::Addr(n)) => n + 8,
            _ => return Err(self.error_at("syntax", "index register expected", start)),
        };
        let long = match self.parse_size_suffix()? {
            None | Some(Size::Word) => false,
            Some(Size::Long) => true,
            Some(_) => return Err(self.error_at("bad-size", "an index is .w or .l", start)),
        };
        Ok(IndexReg { reg, long })
    }

    fn parse_dc(&mut self, size: Option<Size>, span: Span) -> Result<(), Diagnostic> {
        let size = self.data_size(size, span)?;
        let mut items = Vec::new();
        loop {
            items.push(self.parse_data_item(size)?);
            if !self.eat(Punct::Comma)? {
                break;
            }
            // Unlike instruction operands, a dc list may have blanks after commas.
            self.lexer.skip_blanks();
        }
        if self.peek()? != Tok::End {
            return Err(self.error_at("syntax", "unexpected text in data", self.lexer.pos()));
        }
        self.warn_if_bss(span);
        self.parser.push_atom(AtomKind::Data { size, items }, span);
        Ok(())
    }

    fn parse_data_item(&mut self, size: Size) -> Result<DataItem, Diagnostic> {
        let start = self.lexer.pos();
        if size == Size::Byte {
            if let Tok::Str(bytes) = self.peek()? {
                let saved = self.lexer.pos();
                self.advance()?;
                if matches!(self.peek()?, Tok::End | Tok::Punct(Punct::Comma)) {
                    return Ok(DataItem::Bytes(bytes));
                }
                // The string starts a larger expression.
                self.lexer.set_pos(saved);
            }
        }
        let expr = self.parse_expr()?;
        Ok(DataItem::Expr(expr, self.span(start)))
    }

    fn parse_space(
        &mut self,
        directive: &str,
        size: Option<Size>,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let size = self.data_size(size, span)?;
        let count = self.parse_expr()?;
        let fill = if directive == "blk" && self.eat(Punct::Comma)? {
            Some(self.parse_expr()?)
        } else {
            None
        };
        if self.peek()? != Tok::End {
            return Err(self.error_at("syntax", "unexpected text after count", self.lexer.pos()));
        }
        if fill.is_some() {
            self.warn_if_bss(span);
        }
        self.parser
            .push_atom(AtomKind::Space { size, count, fill }, span);
        Ok(())
    }

    /// Size of a data directive: `.b`, `.w` or `.l`, defaulting to `.w`.
    fn data_size(&self, size: Option<Size>, span: Span) -> Result<Size, Diagnostic> {
        match size {
            None => Ok(Size::Word),
            Some(Size::Short) => Err(Diagnostic::error("bad-size", "illegal data size", span)),
            Some(size) => Ok(size),
        }
    }

    fn parse_section(&mut self, size: Option<Size>, span: Span) -> Result<(), Diagnostic> {
        if size.is_some() {
            return Err(Diagnostic::error("bad-size", "section takes no size", span));
        }
        let start = self.lexer.pos();
        let name = match self.peek()? {
            Tok::Ident(name) => name.to_string(),
            Tok::Str(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            _ => return Err(self.error_at("syntax", "section name expected", start)),
        };
        self.advance()?;
        let (mut kind, mut mem) = (SectionKind::Code, MemKind::Any);
        if self.eat(Punct::Comma)? {
            let start = self.lexer.pos();
            let word = self.eat_ident()?.unwrap_or_default().to_ascii_lowercase();
            (kind, mem) = parse_section_type(&word)
                .ok_or_else(|| self.error_at("bad-section", "illegal section type", start))?;
            if self.eat(Punct::Comma)? {
                let start = self.lexer.pos();
                mem = match self
                    .eat_ident()?
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "chip" => MemKind::Chip,
                    "fast" => MemKind::Fast,
                    _ => return Err(self.error_at("bad-section", "chip or fast expected", start)),
                };
            }
        }
        if self.peek()? != Tok::End {
            return Err(self.error_at("syntax", "unexpected text in section", self.lexer.pos()));
        }
        let section = self.parser.open_section(&name, kind, mem);
        self.parser.current = Some(section);
        Ok(())
    }

    fn parse_expr(&mut self) -> Result<Expr, Diagnostic> {
        self.parse_binary(0)
    }

    /// Precedence climbing, tightest level last. The order is vasm's: shifts
    /// and bit operators bind tighter than `*`, and `*` tighter than `+`.
    fn parse_binary(&mut self, level: usize) -> Result<Expr, Diagnostic> {
        if level == LEVELS {
            return self.parse_unary();
        }
        let mut left = self.parse_binary(level + 1)?;
        while let Tok::Punct(punct) = self.peek()? {
            let Some(op) = binary_op(punct, level) else {
                break;
            };
            self.advance()?;
            let right = self.parse_binary(level + 1)?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, Diagnostic> {
        let start = self.lexer.pos();
        let tok = self.peek()?;
        let op = match tok {
            Tok::Punct(Punct::Minus) => Some(UnaryOp::Neg),
            Tok::Punct(Punct::Tilde) => Some(UnaryOp::Not),
            Tok::Punct(Punct::Bang) => Some(UnaryOp::LogicalNot),
            _ => None,
        };
        if let Some(op) = op {
            self.advance()?;
            return Ok(Expr::Unary(op, Box::new(self.parse_unary()?)));
        }
        if self.eat(Punct::Plus)? {
            return self.parse_unary();
        }
        match self.peek()? {
            Tok::Num(n) => {
                self.advance()?;
                Ok(Expr::Num(n))
            }
            Tok::Str(bytes) => {
                self.advance()?;
                if bytes.len() > 8 {
                    return Err(self.error_at("syntax", "string too long for a value", start));
                }
                Ok(Expr::Num(
                    bytes.iter().fold(0, |v: i64, &b| v << 8 | i64::from(b)),
                ))
            }
            Tok::Ident(name) => {
                let name = name.to_string();
                self.advance()?;
                let span = self.span(start);
                if parse_reg(&name).is_some() {
                    return Err(Diagnostic::error(
                        "register-in-expression",
                        format!("register {name} cannot be used in an expression"),
                        span,
                    ));
                }
                Ok(Expr::Sym(self.parser.symbol(&name), span))
            }
            Tok::Punct(Punct::Star) => {
                self.advance()?;
                Ok(Expr::Pc)
            }
            Tok::Punct(Punct::Percent) => {
                self.advance()?;
                match self.lexer.number(2).map_err(|e| self.lex_error(e))? {
                    Tok::Num(n) => Ok(Expr::Num(n)),
                    _ => Err(self.error_at("syntax", "binary number expected", start)),
                }
            }
            Tok::Punct(Punct::LParen) => {
                self.advance()?;
                let expr = self.parse_expr()?;
                self.expect(Punct::RParen, ")")?;
                Ok(expr)
            }
            _ => Err(self.error_at("syntax", "number or identifier expected", start)),
        }
    }
}

fn parse_section_type(word: &str) -> Option<(SectionKind, MemKind)> {
    let (base, mem) = match word.rsplit_once('_') {
        Some((base, "c")) => (base, MemKind::Chip),
        Some((base, "f")) => (base, MemKind::Fast),
        Some((base, "p")) => (base, MemKind::Any),
        Some(_) => return None,
        None => (word, MemKind::Any),
    };
    let kind = match base {
        "code" => SectionKind::Code,
        "data" => SectionKind::Data,
        "bss" => SectionKind::Bss,
        _ => return None,
    };
    Some((kind, mem))
}

/// Number of binary operator levels in [`binary_op`].
const LEVELS: usize = 10;

fn binary_op(punct: Punct, level: usize) -> Option<BinaryOp> {
    Some(match (level, punct) {
        (0, Punct::PipePipe) => BinaryOp::LogicalOr,
        (1, Punct::AmpAmp) => BinaryOp::LogicalAnd,
        (2, Punct::Eq | Punct::EqEq) => BinaryOp::Eq,
        (2, Punct::BangEq | Punct::LtGt) => BinaryOp::Ne,
        (3, Punct::Lt) => BinaryOp::Lt,
        (3, Punct::Le) => BinaryOp::Le,
        (3, Punct::Gt) => BinaryOp::Gt,
        (3, Punct::Ge) => BinaryOp::Ge,
        (4, Punct::Plus) => BinaryOp::Add,
        (4, Punct::Minus) => BinaryOp::Sub,
        (5, Punct::Star) => BinaryOp::Mul,
        (5, Punct::Slash) => BinaryOp::Div,
        (5, Punct::Percent) => BinaryOp::Rem,
        (6, Punct::Pipe | Punct::Bang) => BinaryOp::Or,
        (7, Punct::Caret | Punct::Tilde) => BinaryOp::Xor,
        (8, Punct::Amp) => BinaryOp::And,
        (9, Punct::Shl) => BinaryOp::Shl,
        (9, Punct::Shr) => BinaryOp::Shr,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_section_type_suffixes() {
        assert_eq!(
            parse_section_type("data_c"),
            Some((SectionKind::Data, MemKind::Chip))
        );
        assert_eq!(parse_section_type("foo"), None);
        assert_eq!(parse_section_type("code_x"), None);
    }

    #[test]
    fn test_parse_reg_names() {
        assert_eq!(parse_reg("SP"), Some(Reg::Addr(7)));
        assert_eq!(parse_reg("d7"), Some(Reg::Data(7)));
        assert_eq!(parse_reg("d8"), None);
    }
}
