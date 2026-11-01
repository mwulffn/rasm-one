//! Parses source lines into atoms and symbols.

use std::collections::HashMap;
use std::rc::Rc;

use crate::atom::{Atom, AtomKind, DataItem};
use crate::error::FileError;
use crate::error::{Diagnostic, Related};
use crate::expr::{eval, Base, BinaryOp, Expr, SymId, UnaryOp, Value};
use crate::isa::encode::Instruction;
use crate::isa::table::{rows_for, InsnRow};
use crate::isa::{IndexReg, Mode, Operand};
use crate::lexer::{LexError, Lexer, Punct, Tok};
use crate::source::{FileId, FileProvider, Span};
use crate::types::{Cpu, MemKind, SectionKind, Size};
use crate::{Expansion, ExpansionKind};

/// A symbol being collected.
#[derive(Debug, Clone)]
pub(crate) struct SymbolEntry {
    pub name: String,
    /// Where it is defined; `None` if it has only been referenced.
    pub defined_at: Option<Span>,
    /// True if defined by `set`, which may define it again.
    pub redefinable: bool,
    /// True if declared by `xref`.
    pub external: bool,
    /// Where `xdef` or `::` exported it.
    pub exported_at: Option<Span>,
}

/// A section being collected.
#[derive(Debug, Clone)]
pub(crate) struct SectionEntry {
    pub name: String,
    pub kind: SectionKind,
    pub mem: MemKind,
    /// Fixed address given by `org`.
    pub origin: Option<i64>,
}

/// Output of parsing.
#[derive(Debug, Default)]
pub(crate) struct Program {
    pub sections: Vec<SectionEntry>,
    /// All atoms, in source order.
    pub atoms: Vec<Atom>,
    /// Names of the source files; a [`FileId`] indexes this list.
    pub files: Vec<String>,
    /// Includes and macro expansions.
    pub expansions: Vec<Expansion>,
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
    Sr,
    Ccr,
    Usp,
}

fn parse_reg(name: &str) -> Option<Reg> {
    let lower = name.to_ascii_lowercase();
    if lower == "sp" {
        return Some(Reg::Addr(7));
    }
    match lower.as_str() {
        "pc" => return Some(Reg::Pc),
        "sr" => return Some(Reg::Sr),
        "ccr" => return Some(Reg::Ccr),
        "usp" => return Some(Reg::Usp),
        _ => {}
    }
    match lower.as_bytes() {
        [b'd', n @ b'0'..=b'7'] => Some(Reg::Data(n - b'0')),
        [b'a', n @ b'0'..=b'7'] => Some(Reg::Addr(n - b'0')),
        _ => None,
    }
}

fn is_directive(name: &str) -> bool {
    matches!(
        name,
        "dc" | "ds"
            | "blk"
            | "dcb"
            | "section"
            | "even"
            | "odd"
            | "cnop"
            | "align"
            | "org"
            | "rorg"
            | "equ"
            | "set"
            | "reg"
            | "equr"
            | "rs"
            | "rsreset"
            | "rsset"
            | "fail"
            | "end"
            | "include"
            | "incbin"
            | "incdir"
            | "xref"
            | "xdef"
            | "public"
            | "macro"
            | "endm"
            | "mexit"
            | "rept"
            | "endr"
            | "else"
            | "elif"
            | "elseif"
            | "endc"
            | "endif"
    ) || is_if(name)
        || section_shortcut(name).is_some()
}

/// Maps `code`, `data_c` and so on to a section name, kind and memory type.
fn section_shortcut(name: &str) -> Option<(&'static str, SectionKind, MemKind)> {
    Some(match name {
        "code" => ("CODE", SectionKind::Code, MemKind::Any),
        "code_c" => ("CODE_C", SectionKind::Code, MemKind::Chip),
        "code_f" => ("CODE_F", SectionKind::Code, MemKind::Fast),
        "data" => ("DATA", SectionKind::Data, MemKind::Any),
        "data_c" => ("DATA_C", SectionKind::Data, MemKind::Chip),
        "data_f" => ("DATA_F", SectionKind::Data, MemKind::Fast),
        "bss" => ("BSS", SectionKind::Bss, MemKind::Any),
        "bss_c" => ("BSS_C", SectionKind::Bss, MemKind::Chip),
        "bss_f" => ("BSS_F", SectionKind::Bss, MemKind::Fast),
        _ => return None,
    })
}

/// Macro calls nested deeper than this are an error. Calls recurse in the
/// parser, so this also protects the stack, which is small under WASM.
const MAX_MACRO_DEPTH: usize = 200;

/// Name of the symbol holding the argument count during a macro.
const NARG: &str = "NARG";

/// Name of the symbol holding the iteration number during a `rept`.
const REPTN: &str = "REPTN";

/// True for symbols the assembler maintains itself, which aren't exported.
pub(crate) fn is_internal_symbol(name: &str) -> bool {
    matches!(name, NARG | REPTN | RS_COUNTER)
}

fn is_local_label(name: &str) -> bool {
    name.starts_with('.') || name.ends_with('$')
}

fn is_if(word: &str) -> bool {
    matches!(
        word,
        "if" | "ifne"
            | "ifeq"
            | "ifgt"
            | "ifge"
            | "iflt"
            | "ifle"
            | "ifd"
            | "ifnd"
            | "ifc"
            | "ifnc"
            | "ifb"
            | "ifnb"
    )
}

/// A source line to parse.
#[derive(Debug, Clone, Copy)]
struct SrcLine<'a> {
    number: u32,
    text: &'a [u8],
}

/// A macro body: line numbers and text from where it was defined.
#[derive(Debug)]
struct MacroDef {
    file: FileId,
    lines: Vec<(u32, Vec<u8>)>,
}

/// A conditional block that is open.
#[derive(Debug, Clone, Copy)]
struct Conditional {
    span: Span,
    /// False if the whole block is inside an inactive one.
    parent_active: bool,
    /// True once a branch has been chosen.
    taken: bool,
    active: bool,
    seen_else: bool,
}

/// Work for the line loop to do once a line has been parsed. Doing it there,
/// rather than while parsing the line, keeps the stack small when expansions
/// nest, since WASM stacks are small and line parsing has large frames.
#[derive(Debug)]
enum PendingWork {
    /// A macro definition, whose body follows.
    DefineMacro {
        name: String,
        span: Span,
    },
    /// A `rept`, whose body follows.
    Repeat {
        count: i64,
        span: Span,
    },
    CallMacro {
        name: String,
        size: &'static str,
        args: Vec<Vec<u8>>,
        span: Span,
    },
    Include {
        path: String,
        span: Span,
    },
}

/// Finds the first word that could be a directive, skipping any label. Returns
/// it in lower case with the position after it.
fn directive_word(text: &[u8]) -> Option<(String, usize)> {
    let mut pos = 0;
    let is_word = |c: &u8| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'$');
    if !matches!(text.first(), Some(b' ' | b'\t')) {
        while text.get(pos).is_some_and(is_word) {
            pos += 1;
        }
        while text.get(pos) == Some(&b':') {
            pos += 1;
        }
    } else {
        // An indented label needs a colon, so look past one if it is there.
        let start = text
            .iter()
            .take_while(|c| matches!(c, b' ' | b'\t'))
            .count();
        let end = start + text[start..].iter().take_while(|c| is_word(c)).count();
        if text.get(end) == Some(&b':') {
            pos = end;
            while text.get(pos) == Some(&b':') {
                pos += 1;
            }
        }
    }
    while matches!(text.get(pos), Some(b' ' | b'\t')) {
        pos += 1;
    }
    let start = pos;
    while text
        .get(pos)
        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
    {
        pos += 1;
    }
    if start == pos {
        return None;
    }
    let word = String::from_utf8_lossy(&text[start..pos]).to_ascii_lowercase();
    Some((word, pos))
}

/// Finds the line ending a block, as an index into `lines`. Only `rept` nests.
fn find_block_end(lines: &[SrcLine], opener: Option<&str>, closer: &str) -> Option<usize> {
    let mut depth = 0;
    for (index, line) in lines.iter().enumerate() {
        let Some((word, _)) = directive_word(line.text) else {
            continue;
        };
        if Some(word.as_str()) == opener {
            depth += 1;
        } else if word == closer {
            if depth == 0 {
                return Some(index);
            }
            depth -= 1;
        }
    }
    None
}

/// Splits macro call arguments. Commas inside quotes, parentheses or `<>` don't
/// separate, and `<>` around an argument is removed. A blank outside all of
/// those ends the list, except straight after a comma.
fn split_macro_args(text: &[u8]) -> Vec<Vec<u8>> {
    let mut args = Vec::new();
    let mut pos = 0;
    while matches!(text.get(pos), Some(b' ' | b'\t')) {
        pos += 1;
    }
    if matches!(text.get(pos), None | Some(b';')) {
        return args;
    }
    loop {
        let mut arg = Vec::new();
        if text.get(pos) == Some(&b'<') {
            let inner = pos + 1;
            let length = text[inner..].iter().position(|&c| c == b'>');
            if let Some(length) = length {
                arg.extend_from_slice(&text[inner..inner + length]);
                pos = inner + length + 1;
            }
        }
        let mut quote = None;
        let mut depth = 0;
        while let Some(&c) = text.get(pos) {
            match quote {
                Some(open) if c == open => quote = None,
                Some(_) => {}
                None => match c {
                    b'"' | b'\'' => quote = Some(c),
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    b',' if depth <= 0 => break,
                    b' ' | b'\t' | b';' if depth <= 0 => break,
                    _ => {}
                },
            }
            arg.push(c);
            pos += 1;
        }
        args.push(arg);
        if text.get(pos) != Some(&b',') {
            return args;
        }
        pos += 1;
        while matches!(text.get(pos), Some(b' ' | b'\t')) {
            pos += 1;
        }
    }
}

/// Values that macro parameters expand to.
struct MacroCall<'a> {
    size: &'a str,
    args: &'a [Vec<u8>],
    id: u32,
}

/// Name of the symbol holding the `rs` counter.
const RS_COUNTER: &str = "__RS";

/// Parses whole files into a [`Program`].
pub(crate) struct Parser<'f> {
    cpu: Cpu,
    provider: &'f dyn FileProvider,
    /// Directories to search for files, from options and `incdir`.
    include_dirs: Vec<String>,
    /// Files being parsed, outermost first, to catch include cycles.
    file_stack: Vec<FileId>,
    current_expansion: Option<u32>,
    /// Diagnostics before this index have their expansion chains.
    chained_diagnostics: usize,
    program: Program,
    symbol_ids: HashMap<String, SymId>,
    sections_by_key: HashMap<(String, SectionKind), usize>,
    current: Option<usize>,
    /// Register names defined by `equr`.
    register_aliases: HashMap<String, Reg>,
    /// Values of symbols already known while parsing, for directives whose
    /// operand has to be constant at that point.
    constants: HashMap<SymId, i64>,
    rs_counter: i64,
    /// Set by `end`.
    ended: bool,
    /// Set by `mexit` until the macro has been left.
    macro_exit: bool,
    macros: HashMap<String, Rc<MacroDef>>,
    /// Number for the next macro call, for `\\@`.
    macro_calls: u32,
    macro_depth: usize,
    /// Argument count of the macro being expanded.
    narg: Option<i64>,
    /// Iteration of the `rept` being expanded.
    reptn: Option<i64>,
    conditionals: Vec<Conditional>,
    pending_work: Option<PendingWork>,
    /// The last global label, which local labels belong to.
    label_scope: String,
}

impl<'f> Parser<'f> {
    pub fn new(cpu: Cpu, provider: &'f dyn FileProvider, include_dirs: &[String]) -> Self {
        Self {
            cpu,
            provider,
            include_dirs: include_dirs.to_vec(),
            file_stack: Vec::new(),
            current_expansion: None,
            chained_diagnostics: 0,
            program: Program::default(),
            symbol_ids: HashMap::new(),
            sections_by_key: HashMap::new(),
            current: None,
            register_aliases: HashMap::new(),
            constants: HashMap::new(),
            rs_counter: 0,
            ended: false,
            macro_exit: false,
            macros: HashMap::new(),
            macro_calls: 1,
            macro_depth: 0,
            narg: None,
            reptn: None,
            conditionals: Vec::new(),
            pending_work: None,
            label_scope: String::new(),
        }
    }

    pub fn finish(self) -> Program {
        self.program
    }

    /// Parses the main source file.
    pub fn parse_main(&mut self, path: &str) -> Result<(), FileError> {
        let source = self.provider.read(path, None)?;
        let file = self.file_id(path);
        self.parse_file(file, &source);
        if !self.ended {
            for conditional in std::mem::take(&mut self.conditionals) {
                self.program.diagnostics.push(Diagnostic::error(
                    "unterminated-block",
                    "conditional block has no endc",
                    conditional.span,
                ));
            }
        }
        Ok(())
    }

    fn file_id(&mut self, path: &str) -> FileId {
        let files = &mut self.program.files;
        match files.iter().position(|name| name == path) {
            Some(index) => index as FileId,
            None => {
                files.push(path.to_string());
                files.len() as FileId - 1
            }
        }
    }

    /// Reads the first of the places a file might be: as given, beside the
    /// main source, then in each include directory. As in vasm, the directory
    /// of an included file is not searched.
    fn find_file(&self, path: &str, from: Option<&str>) -> Result<(String, Vec<u8>), FileError> {
        let path = path.replace('\\', "/");
        let mut candidates = vec![path.clone()];
        let main = self.program.files.first();
        if let Some((directory, _)) = main.and_then(|file| file.rsplit_once('/')) {
            candidates.push(format!("{directory}/{path}"));
        }
        for directory in &self.include_dirs {
            candidates.push(format!("{}/{path}", directory.trim_end_matches('/')));
        }
        for candidate in candidates {
            let candidate = normalize_path(&candidate);
            match self.provider.read(&candidate, from) {
                Ok(contents) => return Ok((candidate, contents)),
                Err(FileError::NotFound(_)) => {}
                Err(other) => return Err(other),
            }
        }
        Err(FileError::NotFound(path))
    }

    /// The name of the file now being parsed.
    fn current_file(&self) -> Option<&str> {
        let file = *self.file_stack.last()?;
        self.program.files.get(file as usize).map(String::as_str)
    }

    fn include(&mut self, path: &str, site: Span) -> Result<(), Diagnostic> {
        let from = self.current_file().map(str::to_string);
        let (resolved, source) = self
            .find_file(path, from.as_deref())
            .map_err(|error| Diagnostic::error("file-read", error.to_string(), site))?;
        let file = self.file_id(&resolved);
        if self.file_stack.contains(&file) {
            return Err(Diagnostic::error(
                "include-cycle",
                format!("{resolved} includes itself"),
                site,
            ));
        }
        self.program.expansions.push(Expansion {
            kind: ExpansionKind::Include,
            site,
            parent: self.current_expansion,
        });
        let outer = self.current_expansion;
        self.current_expansion = Some(self.program.expansions.len() as u32 - 1);
        self.parse_file(file, &source);
        self.current_expansion = outer;
        Ok(())
    }

    fn parse_file(&mut self, file: FileId, source: &[u8]) {
        self.file_stack.push(file);
        let lines: Vec<SrcLine> = source
            .split(|&b| b == b'\n')
            .enumerate()
            .map(|(index, raw)| SrcLine {
                number: index as u32 + 1,
                text: raw.strip_suffix(b"\r").unwrap_or(raw),
            })
            .collect();
        self.parse_lines(file, &lines);
        self.file_stack.pop();
    }

    fn parse_lines(&mut self, file: FileId, lines: &[SrcLine]) {
        let mut next = 0;
        while let Some(line) = lines.get(next) {
            if self.ended || self.macro_exit {
                break;
            }
            next += 1;
            let result = if self.is_skipping() {
                self.parse_skipped_line(file, *line)
            } else {
                LineParser {
                    parser: self,
                    lexer: Lexer::new(line.text),
                    file,
                    line: line.number,
                }
                .parse()
            };
            if let Err(diagnostic) = result {
                self.program.diagnostics.push(diagnostic);
            }
            self.chain_new_diagnostics();
            if let Some(work) = self.pending_work.take() {
                next += self.run_pending(work, file, &lines[next..]);
                self.chain_new_diagnostics();
            }
        }
    }

    fn chain_new_diagnostics(&mut self) {
        // Lines of a nested expansion have already had their whole chain added.
        add_expansion_chain(
            &mut self.program.diagnostics[self.chained_diagnostics..],
            self.current_expansion,
            &self.program.expansions,
        );
        self.chained_diagnostics = self.program.diagnostics.len();
    }

    /// Does work left by the last line. Returns how many following lines were
    /// used as its body.
    fn run_pending(&mut self, work: PendingWork, file: FileId, lines: &[SrcLine]) -> usize {
        let (opener, closer, span) = match &work {
            PendingWork::DefineMacro { span, .. } => (None, "endm", *span),
            PendingWork::Repeat { span, .. } => (Some("rept"), "endr", *span),
            PendingWork::CallMacro {
                name,
                size,
                args,
                span,
            } => {
                let call = MacroCall {
                    size,
                    args,
                    id: self.macro_calls,
                };
                self.macro_calls += 1;
                let result = match self.macros.get(name).cloned() {
                    Some(definition) => self.expand_macro(&definition, &call, *span),
                    None => Ok(()),
                };
                self.report(result);
                return 0;
            }
            PendingWork::Include { path, span } => {
                let result = self.include(path, *span);
                self.report(result);
                return 0;
            }
        };
        let Some(end) = find_block_end(lines, opener, closer) else {
            self.program.diagnostics.push(Diagnostic::error(
                "unterminated-block",
                format!("block has no {closer}"),
                span,
            ));
            return lines.len();
        };
        let body = &lines[..end];
        match work {
            PendingWork::DefineMacro { name, .. } => {
                let lines = body
                    .iter()
                    .map(|line| (line.number, line.text.to_vec()))
                    .collect();
                let definition = Rc::new(MacroDef { file, lines });
                self.macros.insert(name, definition);
            }
            PendingWork::Repeat { count, span } => self.run_repeat(count, span, file, body),
            PendingWork::CallMacro { .. } | PendingWork::Include { .. } => {}
        }
        end + 1
    }

    fn report(&mut self, result: Result<(), Diagnostic>) {
        if let Err(diagnostic) = result {
            self.program.diagnostics.push(diagnostic);
        }
    }

    fn run_repeat(&mut self, count: i64, site: Span, file: FileId, body: &[SrcLine]) {
        let outer_reptn = self.reptn;
        for iteration in 0..count {
            if self.ended || self.macro_exit {
                break;
            }
            self.reptn = Some(iteration);
            self.set_special_symbol(REPTN, iteration, site);
            self.run_expansion(ExpansionKind::Repeat, site, file, body);
        }
        self.reptn = outer_reptn;
        if let Some(value) = outer_reptn {
            self.set_special_symbol(REPTN, value, site);
        }
    }

    /// Gives a symbol maintained by the assembler a new value.
    fn set_special_symbol(&mut self, name: &str, value: i64, span: Span) {
        let id = self.symbol(name);
        let entry = &mut self.program.symbols[id as usize];
        entry.defined_at = Some(span);
        entry.redefinable = true;
        self.constants.insert(id, value);
        self.push_atom(AtomKind::Equate(id, Expr::Num(value)), span);
    }

    fn run_expansion(&mut self, kind: ExpansionKind, site: Span, file: FileId, lines: &[SrcLine]) {
        self.program.expansions.push(Expansion {
            kind,
            site,
            parent: self.current_expansion,
        });
        let outer = self.current_expansion;
        self.current_expansion = Some(self.program.expansions.len() as u32 - 1);
        self.parse_lines(file, lines);
        self.current_expansion = outer;
    }

    fn expand_macro(
        &mut self,
        definition: &MacroDef,
        call: &MacroCall,
        site: Span,
    ) -> Result<(), Diagnostic> {
        if self.macro_depth >= MAX_MACRO_DEPTH {
            return Err(Diagnostic::error(
                "macro-depth",
                format!("macro calls nested more than {MAX_MACRO_DEPTH} levels deep"),
                site,
            ));
        }
        let text: Vec<(u32, Vec<u8>)> = definition
            .lines
            .iter()
            .map(|(number, line)| (*number, self.substitute(line, call)))
            .collect();
        let lines: Vec<SrcLine> = text
            .iter()
            .map(|(number, text)| SrcLine {
                number: *number,
                text,
            })
            .collect();

        let outer_narg = self.narg;
        let narg = call.args.len() as i64;
        self.narg = Some(narg);
        self.set_special_symbol(NARG, narg, site);
        let conditional_depth = self.conditionals.len();
        self.macro_depth += 1;
        self.run_expansion(ExpansionKind::Macro, site, definition.file, &lines);
        self.macro_depth -= 1;
        // Leaving with `mexit` also leaves any conditionals it was inside.
        if self.macro_exit {
            self.conditionals.truncate(conditional_depth);
            self.macro_exit = false;
        }
        self.narg = outer_narg;
        if let Some(value) = outer_narg {
            self.set_special_symbol(NARG, value, site);
        }
        Ok(())
    }

    /// Replaces macro parameters in a line. Other backslashes stay, so they
    /// are harmless in comments and an error anywhere else.
    fn substitute(&self, line: &[u8], call: &MacroCall) -> Vec<u8> {
        let mut out = Vec::with_capacity(line.len());
        let mut pos = 0;
        while let Some(&c) = line.get(pos) {
            pos += 1;
            if c != b'\\' {
                out.push(c);
                continue;
            }
            match line.get(pos) {
                Some(d) if d.is_ascii_digit() => {
                    let digits = line[pos..]
                        .iter()
                        .take_while(|c| c.is_ascii_digit())
                        .count();
                    let text = String::from_utf8_lossy(&line[pos..pos + digits]);
                    pos += digits;
                    match text.parse::<usize>() {
                        Ok(0) => out.extend_from_slice(call.size.as_bytes()),
                        Ok(n) => {
                            if let Some(arg) = call.args.get(n - 1) {
                                out.extend_from_slice(arg);
                            }
                        }
                        Err(_) => {}
                    }
                }
                Some(b'@') => {
                    pos += 1;
                    out.extend_from_slice(format!("_{:06}", call.id).as_bytes());
                }
                Some(b'#') => {
                    pos += 1;
                    out.extend_from_slice(call.args.len().to_string().as_bytes());
                }
                Some(b'<') => {
                    let rest = &line[pos + 1..];
                    let value = rest.iter().position(|&c| c == b'>').and_then(|end| {
                        let name = String::from_utf8_lossy(&rest[..end]);
                        let id = self.symbol_ids.get(self.symbol_key(&name).as_str())?;
                        Some((end, *self.constants.get(id)?))
                    });
                    match value {
                        Some((end, value)) => {
                            pos += end + 2;
                            out.extend_from_slice(value.to_string().as_bytes());
                        }
                        None => out.push(c),
                    }
                }
                _ => out.push(c),
            }
        }
        out
    }

    fn is_skipping(&self) -> bool {
        self.conditionals.last().is_some_and(|c| !c.active)
    }

    /// Handles a line in an inactive block: only block structure matters.
    fn parse_skipped_line(&mut self, file: FileId, line: SrcLine) -> Result<(), Diagnostic> {
        let Some((word, end)) = directive_word(line.text) else {
            return Ok(());
        };
        if is_if(&word) {
            let span = Span {
                file,
                line: line.number,
                col: 1,
                len: 1,
            };
            self.conditionals.push(Conditional {
                span,
                parent_active: false,
                taken: true,
                active: false,
                seen_else: false,
            });
            return Ok(());
        }
        match word.as_str() {
            "endc" | "endif" => {
                self.conditionals.pop();
                Ok(())
            }
            "else" => {
                if let Some(top) = self.conditionals.last_mut() {
                    if top.seen_else && top.parent_active {
                        return Err(Diagnostic::error(
                            "syntax",
                            "second else in a block",
                            Span {
                                file,
                                line: line.number,
                                col: 1,
                                len: 1,
                            },
                        ));
                    }
                    top.seen_else = true;
                    if top.parent_active && !top.taken {
                        top.taken = true;
                        top.active = true;
                    }
                }
                Ok(())
            }
            "elif" | "elseif" => {
                let eligible = self
                    .conditionals
                    .last()
                    .is_some_and(|top| top.parent_active && !top.taken);
                if !eligible {
                    return Ok(());
                }
                let mut lexer = Lexer::new(line.text);
                lexer.set_pos(end);
                lexer.skip_blanks();
                let mut parser = LineParser {
                    parser: self,
                    lexer,
                    file,
                    line: line.number,
                };
                let value = parser.parse_constant()?;
                parser.expect_end()?;
                if let Some(top) = self.conditionals.last_mut() {
                    if value != 0 {
                        top.taken = true;
                        top.active = true;
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Looks up a register by name, including names defined by `equr`.
    fn reg(&self, name: &str) -> Option<Reg> {
        self.register_aliases
            .get(name)
            .copied()
            .or_else(|| parse_reg(name))
    }

    /// The key a symbol is stored under: local labels are per global label.
    fn symbol_key(&self, name: &str) -> String {
        if is_local_label(name) {
            format!("{}\0{name}", self.label_scope)
        } else {
            name.to_string()
        }
    }

    fn symbol(&mut self, name: &str) -> SymId {
        let key = self.symbol_key(name);
        if let Some(&id) = self.symbol_ids.get(&key) {
            return id;
        }
        let id = self.program.symbols.len() as SymId;
        self.program.symbols.push(SymbolEntry {
            name: name.to_string(),
            defined_at: None,
            redefinable: false,
            external: false,
            exported_at: None,
        });
        self.symbol_ids.insert(key, id);
        id
    }

    /// The current section, creating the default one if there is none yet.
    fn section(&mut self) -> usize {
        if let Some(section) = self.current {
            return section;
        }
        let key = (DEFAULT_SECTION.to_string(), SectionKind::Code);
        let section = self.add_section(DEFAULT_SECTION, SectionKind::Code, MemKind::Any, None);
        self.sections_by_key.insert(key, section);
        self.current = Some(section);
        section
    }

    fn add_section(
        &mut self,
        name: &str,
        kind: SectionKind,
        mem: MemKind,
        origin: Option<i64>,
    ) -> usize {
        self.program.sections.push(SectionEntry {
            name: name.to_string(),
            kind,
            mem,
            origin,
        });
        self.program.sections.len() - 1
    }

    /// Finds or creates a section and makes it current. Name and kind identify
    /// it, so one name can be used for both code and data. A section without a
    /// memory type can gain one, but one with a type can't change it.
    fn open_section(
        &mut self,
        name: &str,
        kind: SectionKind,
        mem: MemKind,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let key = (name.to_string(), kind);
        let section = match self.sections_by_key.get(&key) {
            Some(&section) => {
                let existing = &mut self.program.sections[section];
                if existing.mem == MemKind::Any {
                    existing.mem = mem;
                } else if existing.mem != mem {
                    return Err(Diagnostic::error(
                        "section-attributes",
                        format!("section {name} already has a different memory type"),
                        span,
                    ));
                }
                section
            }
            None => {
                let section = self.add_section(name, kind, mem, None);
                self.sections_by_key.insert(key, section);
                section
            }
        };
        self.current = Some(section);
        Ok(())
    }

    fn push_atom(&mut self, kind: AtomKind, span: Span) {
        let section = self.section();
        self.program.atoms.push(Atom {
            kind,
            span,
            section,
            expansion: self.current_expansion,
        });
    }

    /// Evaluates an operand that must be constant by this point in the source.
    fn constant(&self, expr: &Expr, span: Span) -> Result<i64, Diagnostic> {
        // A `*` evaluates to a section offset, which is not constant.
        let pc = Value {
            base: Base::Section(usize::MAX),
            offset: 0,
        };
        let lookup = |id: SymId| self.constants.get(&id).map(|&v| Value::abs(v));
        match eval(expr, pc, &lookup) {
            Ok(Value {
                base: Base::Absolute,
                offset,
            }) => Ok(offset),
            _ => Err(Diagnostic::error(
                "not-constant",
                "expression must be constant here",
                span,
            )),
        }
    }
}

/// Removes `.` and `dir/..` from a path, since providers that aren't
/// filesystems can't be expected to understand them.
fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "." => {}
            ".." if parts
                .last()
                .is_some_and(|last| !last.is_empty() && *last != "..") =>
            {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    parts.join("/")
}

/// Adds where each diagnostic's code was included or expanded from.
pub(crate) fn add_expansion_chain(
    diagnostics: &mut [Diagnostic],
    expansion: Option<u32>,
    expansions: &[Expansion],
) {
    for diagnostic in diagnostics {
        let mut current = expansion;
        while let Some(entry) = current.and_then(|id| expansions.get(id as usize)) {
            diagnostic.related.push(Related {
                message: entry.kind.description().to_string(),
                span: entry.site,
            });
            current = entry.parent;
        }
    }
}

struct LineParser<'p, 'a, 'f> {
    parser: &'p mut Parser<'f>,
    lexer: Lexer<'a>,
    file: FileId,
    line: u32,
}

impl LineParser<'_, '_, '_> {
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
        if let Some(name) = self.eat_label_name()? {
            if self.peek_is(Punct::Colon)? {
                let span = self.span(start);
                self.eat_colons(&name, span)?;
                return self.parse_statement(Some((name, span)));
            }
        }
        self.lexer.set_pos(start);
        self.parse_statement(None)
    }

    /// Reads a label name, which may be a `.local` one.
    fn eat_label_name(&mut self) -> Result<Option<String>, Diagnostic> {
        if self.peek_is(Punct::Dot)? {
            self.advance()?;
            return Ok(self.eat_ident()?.map(|name| format!(".{name}")));
        }
        self.eat_ident()
    }

    /// Reads the colons after a label. A second one exports it.
    fn eat_colons(&mut self, label: &str, span: Span) -> Result<(), Diagnostic> {
        self.expect(Punct::Colon, "colon")?;
        if self.eat(Punct::Colon)? {
            self.export_symbol(label, span)?;
        }
        Ok(())
    }

    fn export_symbol(&mut self, name: &str, span: Span) -> Result<(), Diagnostic> {
        if is_local_label(name) {
            return Err(Diagnostic::error(
                "local-export",
                format!("local label {name} cannot be exported"),
                span,
            ));
        }
        let id = self.parser.symbol(name);
        self.parser.program.symbols[id as usize].exported_at = Some(span);
        Ok(())
    }

    /// `xref` and `xdef` take a list of names.
    fn parse_symbol_list(&mut self, directive: &str, span: Span) -> Result<(), Diagnostic> {
        loop {
            let start = self.lexer.pos();
            let Some(name) = self.eat_label_name()? else {
                return Err(self.error_at("syntax", "symbol name expected", start));
            };
            let name_span = self.span(start);
            if directive == "xref" {
                self.declare_external(&name, name_span)?;
            } else {
                self.export_symbol(&name, name_span)?;
            }
            if !self.eat(Punct::Comma)? {
                break;
            }
        }
        self.expect_end().map_err(|_| {
            Diagnostic::error(
                "syntax",
                format!("unexpected text after {directive} list"),
                span,
            )
        })
    }

    fn declare_external(&mut self, name: &str, span: Span) -> Result<(), Diagnostic> {
        if is_local_label(name) {
            return Err(Diagnostic::error(
                "local-export",
                format!("local label {name} cannot be external"),
                span,
            ));
        }
        let id = self.parser.symbol(name);
        let entry = &mut self.parser.program.symbols[id as usize];
        if entry.defined_at.is_some() {
            return Err(Diagnostic::error(
                "external-defined",
                format!("{name} is already defined and cannot be external"),
                span,
            ));
        }
        entry.external = true;
        Ok(())
    }

    fn parse_after_column_zero(&mut self) -> Result<(), Diagnostic> {
        let start = self.lexer.pos();
        let Some(name) = self.eat_label_name()? else {
            return Err(self.error_at("syntax", "label expected", start));
        };
        let span = self.span(start);
        if self.peek_is(Punct::Colon)? {
            self.eat_colons(&name, span)?;
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
        let at_equals = self.peek_is(Punct::Eq)?;
        if !self.lexer.at_blank() && !at_equals && self.peek()? != Tok::End {
            return Err(self.error_at("syntax", "unexpected text after label", self.lexer.pos()));
        }
        self.parse_statement(Some((name, span)))
    }

    fn define_label(&mut self, name: &str, span: Span) -> Result<(), Diagnostic> {
        if !is_local_label(name) {
            self.parser.label_scope = name.to_string();
        }
        let id = self.claim_symbol(name, span, false)?;
        self.parser.push_atom(AtomKind::Label(id), span);
        Ok(())
    }

    /// Marks a symbol as defined, failing if it can't be defined (again).
    fn claim_symbol(
        &mut self,
        name: &str,
        span: Span,
        redefinable: bool,
    ) -> Result<SymId, Diagnostic> {
        if self.parser.reg(name).is_some() {
            return Err(Diagnostic::error(
                "reserved-name",
                format!("{name} is a register and cannot be a symbol"),
                span,
            ));
        }
        let id = self.parser.symbol(name);
        let entry = &mut self.parser.program.symbols[id as usize];
        if entry.external {
            return Err(Diagnostic::error(
                "external-defined",
                format!("{name} is declared external and cannot be defined here"),
                span,
            ));
        }
        if let Some(first) = entry.defined_at {
            if !(redefinable && entry.redefinable) {
                let mut diagnostic =
                    Diagnostic::error("redefined-symbol", format!("symbol {name} redefined"), span);
                diagnostic.related.push(Related {
                    message: "first defined here".to_string(),
                    span: first,
                });
                return Err(diagnostic);
            }
        }
        entry.defined_at = Some(span);
        entry.redefinable = redefinable;
        Ok(id)
    }

    /// Defines a symbol as `expr`, recording its value if it's already known.
    fn define_equate(
        &mut self,
        name: &str,
        expr: Expr,
        redefinable: bool,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let id = self.claim_symbol(name, span, redefinable)?;
        match self.parser.constant(&expr, span) {
            Ok(value) => self.parser.constants.insert(id, value),
            Err(_) => self.parser.constants.remove(&id),
        };
        self.parser.push_atom(AtomKind::Equate(id, expr), span);
        Ok(())
    }

    /// Parses the expression of `equ`, `=` or `set` and defines the label.
    fn parse_equate(
        &mut self,
        label: Option<(String, Span)>,
        redefinable: bool,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let (name, label_span) = self.required_label(label, span)?;
        let expr = self.parse_expr()?;
        self.expect_end()?;
        self.define_equate(&name, expr, redefinable, label_span)
    }

    fn required_label(
        &self,
        label: Option<(String, Span)>,
        span: Span,
    ) -> Result<(String, Span), Diagnostic> {
        label.ok_or_else(|| Diagnostic::error("syntax", "a label is required here", span))
    }

    fn expect_end(&self) -> Result<(), Diagnostic> {
        if self.peek()? == Tok::End {
            return Ok(());
        }
        Err(self.error_at("syntax", "unexpected text in operand", self.lexer.pos()))
    }

    fn no_size(&self, size: Option<Size>, span: Span) -> Result<(), Diagnostic> {
        match size {
            None => Ok(()),
            Some(_) => Err(Diagnostic::error(
                "bad-size",
                "this directive takes no size",
                span,
            )),
        }
    }

    fn parse_constant(&mut self) -> Result<i64, Diagnostic> {
        let start = self.lexer.pos();
        let expr = self.parse_expr()?;
        self.parser.constant(&expr, self.span(start))
    }

    /// Parses the operands of the alignment directives into an atom.
    fn parse_alignment(&mut self, directive: &str, span: Span) -> Result<AtomKind, Diagnostic> {
        let (alignment, offset, nops) = match directive {
            "even" => (2, 0, false),
            "odd" => (2, 1, false),
            "align" => {
                let bits = self.parse_constant()?;
                self.expect_end()?;
                if !(0..=31).contains(&bits) {
                    return Err(Diagnostic::error(
                        "out-of-range",
                        "align takes a number of bits from 0 to 31",
                        span,
                    ));
                }
                (1 << bits, 0, false)
            }
            _ => {
                let offset = self.parse_constant()?;
                self.expect(Punct::Comma, ",")?;
                let alignment = self.parse_constant()?;
                self.expect_end()?;
                if alignment < 0 || offset < 0 {
                    return Err(Diagnostic::error(
                        "out-of-range",
                        "cnop takes non-negative values",
                        span,
                    ));
                }
                (alignment, offset, true)
            }
        };
        Ok(AtomKind::Align {
            alignment,
            offset,
            nops,
        })
    }

    fn parse_macro_definition(
        &mut self,
        label: Option<(String, Span)>,
        span: Span,
    ) -> Result<(), Diagnostic> {
        if self.parser.macro_depth > 0 {
            return Err(Diagnostic::error(
                "nested-macro",
                "a macro cannot be defined inside a macro",
                span,
            ));
        }
        let name = match label {
            Some((name, _)) => name,
            None => self
                .eat_ident()?
                .ok_or_else(|| Diagnostic::error("syntax", "macro name expected", span))?,
        };
        let lower = name.to_ascii_lowercase();
        if rows_for(&lower).is_some() || is_directive(&lower) {
            self.parser.program.diagnostics.push(Diagnostic::warning(
                "macro-is-mnemonic",
                format!("{name} is a mnemonic or directive, which takes priority over the macro"),
                span,
            ));
        } else if self.parser.macros.contains_key(&name) {
            self.parser.program.diagnostics.push(Diagnostic::warning(
                "macro-redefined",
                format!("macro {name} redefined"),
                span,
            ));
        }
        self.parser.pending_work = Some(PendingWork::DefineMacro { name, span });
        Ok(())
    }

    fn parse_macro_call(
        &mut self,
        name: &str,
        size: Option<Size>,
        span: Span,
    ) -> Result<(), Diagnostic> {
        // Without a suffix, \\0 expands to the default size.
        let size = match size {
            Some(Size::Byte) => "b",
            Some(Size::Long) => "l",
            Some(Size::Short) => "s",
            Some(Size::Word) | None => "w",
        };
        self.parser.pending_work = Some(PendingWork::CallMacro {
            name: name.to_string(),
            size,
            args: split_macro_args(self.lexer.rest()),
            span,
        });
        Ok(())
    }

    fn parse_if(&mut self, word: &str, span: Span) -> Result<(), Diagnostic> {
        let condition = match word {
            "ifd" | "ifnd" => {
                let start = self.lexer.pos();
                let Some(name) = self.eat_label_name()? else {
                    return Err(self.error_at("syntax", "symbol expected", start));
                };
                self.expect_end()?;
                let id = self.parser.symbol(&name);
                let defined = self.parser.program.symbols[id as usize]
                    .defined_at
                    .is_some();
                defined == (word == "ifd")
            }
            "ifc" | "ifnc" => {
                let first = self.parse_compare_operand()?;
                self.expect(Punct::Comma, ",")?;
                let second = self.parse_compare_operand()?;
                self.expect_end()?;
                (first == second) == (word == "ifc")
            }
            "ifb" | "ifnb" => (self.peek()? == Tok::End) == (word == "ifb"),
            _ => {
                let value = self.parse_constant()?;
                self.expect_end()?;
                match word {
                    "ifeq" => value == 0,
                    "ifgt" => value > 0,
                    "ifge" => value >= 0,
                    "iflt" => value < 0,
                    "ifle" => value <= 0,
                    _ => value != 0,
                }
            }
        };
        self.parser.conditionals.push(Conditional {
            span,
            parent_active: true,
            taken: condition,
            active: condition,
            seen_else: false,
        });
        Ok(())
    }

    /// Reads a string, or bare text up to a comma or blank.
    fn parse_compare_operand(&mut self) -> Result<Vec<u8>, Diagnostic> {
        if let Tok::Str(bytes) = self.peek()? {
            self.advance()?;
            return Ok(bytes);
        }
        let rest = self.lexer.rest();
        let length = rest
            .iter()
            .position(|c| matches!(c, b',' | b' ' | b'\t' | b';'))
            .unwrap_or(rest.len());
        self.lexer.set_pos(self.lexer.pos() + length);
        Ok(rest[..length].to_vec())
    }

    /// Handles `else`, `elif` and `endc` on a line in an active block.
    fn parse_block_change(&mut self, word: &str, span: Span) -> Result<(), Diagnostic> {
        let Some(top) = self.parser.conditionals.last_mut() else {
            return Err(Diagnostic::error(
                "unexpected-end",
                format!("{word} without if"),
                span,
            ));
        };
        match word {
            "endc" | "endif" => {
                self.parser.conditionals.pop();
            }
            "else" if top.seen_else => {
                return Err(Diagnostic::error("syntax", "second else in a block", span));
            }
            _ => {
                top.active = false;
                top.seen_else = word == "else";
            }
        }
        Ok(())
    }

    /// Reads a file name, quoted or not.
    fn parse_path(&mut self, span: Span) -> Result<String, Diagnostic> {
        let bytes = match self.peek()? {
            Tok::Str(bytes) => {
                self.advance()?;
                bytes
            }
            _ => self.lexer.word().to_vec(),
        };
        if bytes.is_empty() {
            return Err(Diagnostic::error("syntax", "file name expected", span));
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// `incbin` inserts a file, optionally from an offset and for a length. A
    /// length that is not positive means the rest of the file, and a longer
    /// one stops at the end of the file.
    fn parse_incbin(&mut self, span: Span) -> Result<(), Diagnostic> {
        let path = self.parse_path(span)?;
        let mut offset = 0;
        let mut length = 0;
        if self.eat(Punct::Comma)? {
            offset = self.parse_constant()?;
            if self.eat(Punct::Comma)? {
                length = self.parse_constant()?;
            }
        }
        self.expect_end()?;
        let from = self.parser.current_file().map(str::to_string);
        let (_, contents) = self
            .parser
            .find_file(&path, from.as_deref())
            .map_err(|error| Diagnostic::error("file-read", error.to_string(), span))?;
        let start = usize::try_from(offset)
            .ok()
            .filter(|&start| start <= contents.len())
            .ok_or_else(|| Diagnostic::error("bad-offset", "incbin offset out of range", span))?;
        let available = contents.len() - start;
        let count = match usize::try_from(length) {
            Ok(length) if length > 0 => length.min(available),
            _ => available,
        };
        self.warn_if_bss(span);
        let bytes = contents[start..start + count].to_vec();
        self.parser.push_atom(
            AtomKind::Data {
                size: Size::Byte,
                items: vec![DataItem::Bytes(bytes)],
            },
            span,
        );
        Ok(())
    }

    /// `org` starts a new section at a fixed address.
    fn parse_org(&mut self, span: Span) -> Result<(), Diagnostic> {
        let address = self.parse_constant()?;
        self.expect_end()?;
        if !(0..=0xFFFF_FFFF).contains(&address) {
            return Err(Diagnostic::error(
                "out-of-range",
                "org address out of range",
                span,
            ));
        }
        let name = format!("org{address:x}");
        let section =
            self.parser
                .add_section(&name, SectionKind::Code, MemKind::Any, Some(address));
        self.parser.current = Some(section);
        Ok(())
    }

    /// `reg` gives a symbol the mask of a register list.
    fn parse_reg_directive(
        &mut self,
        label: Option<(String, Span)>,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let (name, label_span) = self.required_label(label, span)?;
        let start = self.lexer.pos();
        let mask = match self.parse_mode()? {
            Mode::DataReg(n) => 1 << n,
            Mode::AddrReg(n) => 1 << (n + 8),
            Mode::RegList(mask) => mask,
            _ => return Err(self.error_at("syntax", "register list expected", start)),
        };
        self.expect_end()?;
        self.define_equate(&name, Expr::Num(i64::from(mask)), false, label_span)
    }

    /// `equr` gives a data or address register another name.
    fn parse_equr(&mut self, label: Option<(String, Span)>, span: Span) -> Result<(), Diagnostic> {
        let (name, label_span) = self.required_label(label, span)?;
        let start = self.lexer.pos();
        let reg = self.eat_ident()?.and_then(|word| self.parser.reg(&word));
        let reg = reg
            .filter(|reg| matches!(reg, Reg::Data(_) | Reg::Addr(_)))
            .ok_or_else(|| self.error_at("syntax", "data or address register expected", start))?;
        self.expect_end()?;
        self.claim_symbol(&name, label_span, false)?;
        self.parser.register_aliases.insert(name, reg);
        Ok(())
    }

    /// `rs` gives its label the counter's value and advances the counter.
    fn parse_rs(
        &mut self,
        label: Option<(String, Span)>,
        size: Option<Size>,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let size = self.data_size(size, span)?;
        let count = self.parse_constant()?;
        self.expect_end()?;
        let value = self.parser.rs_counter;
        if let Some((name, label_span)) = label {
            self.define_equate(&name, Expr::Num(value), false, label_span)?;
        }
        self.set_rs_counter(
            value.wrapping_add(count.wrapping_mul(i64::from(size.bytes()))),
            span,
        )
    }

    fn set_rs_counter(&mut self, value: i64, span: Span) -> Result<(), Diagnostic> {
        self.parser.rs_counter = value;
        self.define_equate(RS_COUNTER, Expr::Num(value), true, span)
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
        if label.is_some() && self.eat(Punct::Eq)? {
            self.lexer.skip_blanks();
            let span = self.span(start);
            return self.parse_equate(label, false, span);
        }
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

        self.lexer.skip_blanks();
        // These use the label as the symbol they define.
        match lower.as_str() {
            "equ" => {
                self.no_size(size, span)?;
                return self.parse_equate(label, false, span);
            }
            "set" => {
                self.no_size(size, span)?;
                return self.parse_equate(label, true, span);
            }
            "reg" => {
                self.no_size(size, span)?;
                return self.parse_reg_directive(label, span);
            }
            "equr" => {
                self.no_size(size, span)?;
                return self.parse_equr(label, span);
            }
            "rs" => return self.parse_rs(label, size, span),
            "macro" => {
                self.no_size(size, span)?;
                return self.parse_macro_definition(label, span);
            }
            _ => {}
        }
        // These move the location first, so a label takes the new location.
        match lower.as_str() {
            "even" | "odd" | "align" | "cnop" => {
                self.no_size(size, span)?;
                let mut atom = self.parse_alignment(&lower, span)?;
                // vasm pads with zeros if a label shares the line with cnop.
                if let (AtomKind::Align { nops, .. }, true) = (&mut atom, label.is_some()) {
                    *nops = false;
                }
                self.parser.push_atom(atom, span);
                return self.define_optional_label(label);
            }
            "org" => {
                self.no_size(size, span)?;
                self.parse_org(span)?;
                return self.define_optional_label(label);
            }
            _ => {}
        }
        self.define_optional_label(label)?;
        if is_if(&lower) {
            self.no_size(size, span)?;
            return self.parse_if(&lower, span);
        }
        match lower.as_str() {
            "xref" | "xdef" | "public" => {
                self.no_size(size, span)?;
                let directive = if lower == "xref" { "xref" } else { "xdef" };
                return self.parse_symbol_list(directive, span);
            }
            "else" | "elif" | "elseif" | "endc" | "endif" => {
                self.no_size(size, span)?;
                return self.parse_block_change(&lower, span);
            }
            "rept" => {
                self.no_size(size, span)?;
                let count = self.parse_constant()?;
                self.expect_end()?;
                self.parser.pending_work = Some(PendingWork::Repeat { count, span });
                return Ok(());
            }
            "endr" | "endm" => {
                return Err(Diagnostic::error(
                    "unexpected-end",
                    format!("{lower} without a block to end"),
                    span,
                ));
            }
            "mexit" => {
                // As in vasm, a `rept` inside the macro can't be left this way.
                let innermost = self.parser.current_expansion;
                let entry =
                    innermost.and_then(|id| self.parser.program.expansions.get(id as usize));
                if entry.map(|entry| entry.kind) != Some(ExpansionKind::Macro) {
                    return Err(Diagnostic::error(
                        "unexpected-end",
                        "mexit outside a macro",
                        span,
                    ));
                }
                self.parser.macro_exit = true;
                return Ok(());
            }
            "include" => {
                self.no_size(size, span)?;
                let path = self.parse_path(span)?;
                self.expect_end()?;
                self.parser.pending_work = Some(PendingWork::Include { path, span });
                return Ok(());
            }
            "incbin" => {
                self.no_size(size, span)?;
                return self.parse_incbin(span);
            }
            "incdir" => {
                self.no_size(size, span)?;
                let path = self.parse_path(span)?;
                self.expect_end()?;
                self.parser.include_dirs.push(path);
                return Ok(());
            }
            _ => {}
        }
        if lower == "rorg" {
            self.no_size(size, span)?;
            let expr = self.parse_expr()?;
            self.expect_end()?;
            self.parser.push_atom(AtomKind::Rorg(expr), span);
            return Ok(());
        }
        if let Some((section, kind, mem)) = section_shortcut(&lower) {
            self.no_size(size, span)?;
            return self.parser.open_section(section, kind, mem, span);
        }
        match lower.as_str() {
            "dc" => self.parse_dc(size, span),
            "ds" | "blk" | "dcb" => self.parse_space(&lower, size, span),
            "section" => self.parse_section(size, span),
            "rsreset" => {
                self.no_size(size, span)?;
                self.set_rs_counter(0, span)
            }
            "rsset" => {
                self.no_size(size, span)?;
                let value = self.parse_constant()?;
                self.expect_end()?;
                self.set_rs_counter(value, span)
            }
            "fail" => {
                let text = String::from_utf8_lossy(self.lexer.rest()).into_owned();
                Err(Diagnostic::error(
                    "fail",
                    format!("fail: {}", text.trim()),
                    span,
                ))
            }
            "end" => {
                self.parser.ended = true;
                Ok(())
            }
            _ if self.parser.macros.contains_key(&name) => self.parse_macro_call(&name, size, span),
            _ => Err(Diagnostic::error(
                "unknown-mnemonic",
                format!("unknown mnemonic {name}"),
                span,
            )),
        }
    }

    fn define_optional_label(&mut self, label: Option<(String, Span)>) -> Result<(), Diagnostic> {
        match label {
            Some((label, span)) => self.define_label(&label, span),
            None => Ok(()),
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
            let operands = self.parse_operands()?;
            return self.select_row(mnemonic, rows, None, operands, span);
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
            match self.parser.reg(name) {
                Some(Reg::Data(n)) => return self.parse_after_register(n, Mode::DataReg(n)),
                Some(Reg::Addr(n)) => return self.parse_after_register(n + 8, Mode::AddrReg(n)),
                Some(Reg::Sr) => {
                    self.advance()?;
                    return Ok(Mode::Sr);
                }
                Some(Reg::Ccr) => {
                    self.advance()?;
                    return Ok(Mode::Ccr);
                }
                Some(Reg::Usp) => {
                    self.advance()?;
                    return Ok(Mode::Usp);
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

    /// Having peeked a register numbered 0-15, returns `single` unless a
    /// register list follows.
    fn parse_after_register(&mut self, first: u8, single: Mode) -> Result<Mode, Diagnostic> {
        self.advance()?;
        if !self.peek_is(Punct::Minus)? && !self.peek_is(Punct::Slash)? {
            return Ok(single);
        }
        let mut mask: u16 = 0;
        let mut low = first;
        loop {
            let mut high = low;
            if self.eat(Punct::Minus)? {
                high = self.parse_list_register()?;
            }
            // A range may run in either direction and across register kinds.
            for n in low.min(high)..=low.max(high) {
                mask |= 1 << n;
            }
            if !self.eat(Punct::Slash)? {
                return Ok(Mode::RegList(mask));
            }
            low = self.parse_list_register()?;
        }
    }

    fn parse_list_register(&mut self) -> Result<u8, Diagnostic> {
        let start = self.lexer.pos();
        match self
            .eat_ident()?
            .as_deref()
            .and_then(|word| self.parser.reg(word))
        {
            Some(Reg::Data(n)) => Ok(n),
            Some(Reg::Addr(n)) => Ok(n + 8),
            _ => Err(self.error_at("syntax", "register expected in list", start)),
        }
    }

    /// Recognises `-(An)`. Anything else, such as `-(4)(a0)`, is left for
    /// the expression parser.
    fn try_predecrement(&mut self) -> Result<Option<Mode>, Diagnostic> {
        let saved = self.lexer.pos();
        if self.eat(Punct::Minus)? && self.eat(Punct::LParen)? {
            if let Some(name) = self.eat_ident()? {
                if let Some(Reg::Addr(n)) = self.parser.reg(&name) {
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
            Tok::Ident(name) => matches!(self.parser.reg(name), Some(Reg::Addr(_) | Reg::Pc)),
            _ => false,
        })
    }

    /// Parses `An)`, `An,Xn)`, `PC)` or `PC,Xn)` after the displacement.
    fn parse_base_and_index(&mut self, disp: Expr) -> Result<Mode, Diagnostic> {
        let start = self.lexer.pos();
        let base = self
            .eat_ident()?
            .as_deref()
            .and_then(|word| self.parser.reg(word))
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
        let reg = match self
            .eat_ident()?
            .as_deref()
            .and_then(|word| self.parser.reg(word))
        {
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
        let fill = if directive != "ds" && self.eat(Punct::Comma)? {
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
        self.parser.open_section(&name, kind, mem, span)
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
                if self.parser.reg(&name).is_some() {
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
            Tok::Punct(Punct::Dot) => {
                let Some(name) = self.eat_label_name()? else {
                    return Err(self.error_at("syntax", "local label expected", start));
                };
                let span = self.span(start);
                Ok(Expr::Sym(self.parser.symbol(&name), span))
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
    fn test_normalize_path_resolves_dots() {
        assert_eq!(normalize_path("sub/../d.bin"), "d.bin");
        assert_eq!(normalize_path("./a/./b"), "a/b");
        assert_eq!(normalize_path("../d.bin"), "../d.bin");
        assert_eq!(normalize_path("../../d.bin"), "../../d.bin");
        assert_eq!(normalize_path("/abs/../d.bin"), "/d.bin");
    }

    #[test]
    fn test_directive_word_skips_labels() {
        let word = |text: &[u8]| directive_word(text).map(|(word, _)| word);
        assert_eq!(word(b" ENDC ;x").as_deref(), Some("endc"));
        assert_eq!(word(b"lab: endm").as_deref(), Some("endm"));
        assert_eq!(word(b"lab endm").as_deref(), Some("endm"));
        assert_eq!(word(b" lab: endm").as_deref(), Some("endm"));
        assert_eq!(word(b"endm"), None);
        assert_eq!(word(b""), None);
    }

    #[test]
    fn test_split_macro_args_protects_nested_commas() {
        let args = split_macro_args(b" 1, (a0,d1.w),<2,3>,'x,y' comment");
        let expected: [&[u8]; 4] = [b"1", b"(a0,d1.w)", b"2,3", b"'x,y'"];
        assert_eq!(args, expected);
        assert!(split_macro_args(b" ;c").is_empty());
        assert_eq!(split_macro_args(b",5").len(), 2);
    }

    #[test]
    fn test_parse_reg_names() {
        assert_eq!(parse_reg("SP"), Some(Reg::Addr(7)));
        assert_eq!(parse_reg("d7"), Some(Reg::Data(7)));
        assert_eq!(parse_reg("d8"), None);
    }
}
