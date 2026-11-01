//! Assigns addresses to symbols and generates section contents.

use crate::atom::{Atom, AtomKind, DataItem};
use crate::error::Diagnostic;
use crate::expr::{eval, Value};
use crate::isa::encode::{encode, EmitContext};
use crate::parser::Program;
use crate::types::Size;
use crate::{LineEntry, Reloc, Section, Symbol};

/// Forward references in `ds` counts settle quickly; this bound only catches
/// sources that can never settle.
const MAX_PASSES: usize = 32;

/// Largest permitted `ds`/`blk` size in bytes.
const MAX_SPACE: i64 = 0x7FFF_FFFF;

/// Symbol values for one layout pass.
type SymbolValues = Vec<Option<Value>>;

/// Result of [`build`].
pub(crate) struct Built {
    pub sections: Vec<Section>,
    pub symbols: Vec<Symbol>,
    pub line_table: Vec<LineEntry>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Lays out a parsed program and generates its bytes.
pub(crate) fn build(program: Program) -> Built {
    let mut diagnostics = program.diagnostics;
    let mut values: SymbolValues = vec![None; program.symbols.len()];
    let mut settled = false;
    for _ in 0..MAX_PASSES {
        let next = layout_pass(&program.sections, &values);
        settled = next == values;
        values = next;
        if settled {
            break;
        }
    }
    if !settled {
        diagnostics.push(Diagnostic {
            severity: crate::Severity::Error,
            code: "layout-not-settled",
            message: "symbol addresses depend on themselves and never settle".to_string(),
            span: None,
            related: Vec::new(),
        });
    }

    let lookup = |id: u32| values[id as usize];
    let name_of = |id: u32| program.symbols[id as usize].name.clone();
    let mut line_table = Vec::new();
    let mut sections = Vec::with_capacity(program.sections.len());
    for (index, entry) in program.sections.iter().enumerate() {
        let mut data = Vec::new();
        let mut relocs: Vec<Reloc> = Vec::new();
        for atom in &entry.atoms {
            let start = data.len() as u32;
            let mut ctx = EmitContext {
                section: index,
                instruction_start: 0,
                lookup: &lookup,
                name_of: &name_of,
                data: &mut data,
                relocs: &mut relocs,
                diagnostics: &mut diagnostics,
            };
            emit_atom(atom, &mut ctx);
            let len = data.len() as u32 - start;
            if len > 0 {
                line_table.push(LineEntry {
                    section: index,
                    offset: start,
                    len,
                    span: atom.span,
                    expansion: None,
                });
            }
        }
        sections.push(Section {
            name: entry.name.clone(),
            kind: entry.kind,
            mem: entry.mem,
            data,
            relocs,
        });
    }

    let symbols = program
        .symbols
        .iter()
        .zip(&values)
        .filter_map(|(entry, value)| {
            let value = (*value)?;
            Some(Symbol {
                name: entry.name.clone(),
                section: value.section,
                value: value.offset,
                span: entry.defined_at?,
            })
        })
        .collect();

    // Undefined symbols are reported where they are used, during emission.
    Built {
        sections,
        symbols,
        line_table,
        diagnostics,
    }
}

/// Computes symbol values using `previous` for forward references, which are
/// taken as zero until known.
fn layout_pass(sections: &[crate::parser::SectionEntry], previous: &SymbolValues) -> SymbolValues {
    let mut values = vec![None; previous.len()];
    for (index, section) in sections.iter().enumerate() {
        let mut offset: i64 = 0;
        for atom in &section.atoms {
            if let AtomKind::Label(id) = atom.kind {
                values[id as usize] = Some(Value {
                    section: Some(index),
                    offset,
                });
            }
            offset += atom_len(atom, index, offset, previous);
        }
    }
    values
}

fn atom_len(atom: &Atom, section: usize, offset: i64, values: &SymbolValues) -> i64 {
    match &atom.kind {
        AtomKind::Label(_) => 0,
        AtomKind::AlignInstruction => offset & 1,
        AtomKind::Instruction(instruction) => i64::from(instruction.len()),
        AtomKind::Data { size, items } => items
            .iter()
            .map(|item| match item {
                DataItem::Bytes(bytes) => bytes.len() as i64,
                DataItem::Expr(..) => i64::from(size.bytes()),
            })
            .sum(),
        AtomKind::Space { size, count, .. } => {
            let pc = Value {
                section: Some(section),
                offset,
            };
            let lookup = |id: u32| values[id as usize].or(Some(Value::abs(0)));
            match eval(count, pc, &lookup) {
                Ok(Value {
                    section: None,
                    offset: n,
                }) if (0..=MAX_SPACE).contains(&n) => {
                    n.saturating_mul(i64::from(size.bytes())).min(MAX_SPACE)
                }
                _ => 0,
            }
        }
    }
}

fn emit_atom(atom: &Atom, ctx: &mut EmitContext) {
    match &atom.kind {
        AtomKind::Label(_) => {}
        AtomKind::AlignInstruction => {
            if ctx.data.len() % 2 != 0 {
                ctx.diagnostics.push(Diagnostic::warning(
                    "instruction-aligned",
                    "instruction at an odd address has been aligned",
                    atom.span,
                ));
                ctx.data.push(0);
            }
        }
        AtomKind::Instruction(instruction) => encode(instruction, atom.span, ctx),
        AtomKind::Data { size, items } => {
            for item in items {
                emit_data_item(item, *size, ctx);
            }
        }
        AtomKind::Space { size, count, fill } => emit_space(atom, *size, count, fill, ctx),
    }
}

fn emit_data_item(item: &DataItem, size: Size, ctx: &mut EmitContext) {
    let (expr, span) = match item {
        DataItem::Bytes(bytes) => {
            ctx.data.extend_from_slice(bytes);
            return;
        }
        DataItem::Expr(expr, span) => (expr, *span),
    };
    let pc = Value {
        section: Some(ctx.section),
        offset: ctx.data.len() as i64,
    };
    let bytes = size.bytes();
    let Some(value) = ctx.evaluate(expr, pc, span) else {
        ctx.put_zero(bytes);
        return;
    };
    match size {
        Size::Byte | Size::Short if !(-0x80..=0xFF).contains(&value.offset) => {
            out_of_range(span, ctx)
        }
        Size::Word if !(-0x8000..=0xFFFF).contains(&value.offset) => out_of_range(span, ctx),
        Size::Long if !(-0x8000_0000..=0xFFFF_FFFF).contains(&value.offset) => {
            ctx.diagnostics.push(Diagnostic::warning(
                "data-truncated",
                "value does not fit in 32 bits and has been truncated",
                span,
            ));
        }
        _ => {}
    }
    ctx.put_value(value, bytes);
}

fn out_of_range(span: crate::Span, ctx: &mut EmitContext) {
    ctx.diagnostics
        .push(Diagnostic::error("out-of-range", "data out of range", span));
}

fn emit_space(
    atom: &Atom,
    size: Size,
    count: &crate::expr::Expr,
    fill: &Option<crate::expr::Expr>,
    ctx: &mut EmitContext,
) {
    let pc = Value {
        section: Some(ctx.section),
        offset: ctx.data.len() as i64,
    };
    let Some(value) = ctx.evaluate(count, pc, atom.span) else {
        return;
    };
    let total = value.offset.saturating_mul(i64::from(size.bytes()));
    if value.section.is_some() || !(0..=MAX_SPACE).contains(&value.offset) || total > MAX_SPACE {
        ctx.diagnostics.push(Diagnostic::error(
            "bad-count",
            "count must be a non-negative constant of reasonable size",
            atom.span,
        ));
        return;
    }
    let fill_value = match fill {
        Some(expr) => match ctx.evaluate(expr, pc, atom.span) {
            Some(value) if value.section.is_none() => value.offset,
            Some(_) => {
                ctx.diagnostics.push(Diagnostic::error(
                    "not-absolute",
                    "fill value must be a constant",
                    atom.span,
                ));
                0
            }
            None => 0,
        },
        None => 0,
    };
    let width = size.bytes() as usize;
    let pattern = (fill_value as u32).to_be_bytes();
    let pattern = &pattern[4 - width..];
    ctx.data.reserve(total as usize);
    for _ in 0..value.offset {
        ctx.data.extend_from_slice(pattern);
    }
}
