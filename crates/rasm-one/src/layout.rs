//! Assigns addresses to symbols and generates section contents.

use std::cell::Cell;

use crate::atom::{Atom, AtomKind, DataItem};
use crate::error::Diagnostic;
use crate::expr::{eval, Base, Value};
use crate::isa::encode::{encode, EmitContext};
use crate::parser::{add_expansion_chain, Program, SectionEntry};
use crate::types::{SectionKind, Size};
use crate::{Expansion, LineEntry, Reloc, Section, Symbol};

/// Forward references in `ds` counts settle quickly; this bound only catches
/// sources that can never settle.
const MAX_PASSES: usize = 32;

/// Largest permitted `ds`/`blk` size in bytes.
const MAX_SPACE: i64 = 0x7FFF_FFFF;

/// Current symbol values. Walking the source updates them in order, so a `set`
/// symbol has the value of its latest definition, and a forward reference sees
/// the value from the end of the previous walk.
type SymbolValues = Vec<Cell<Option<Value>>>;

/// Result of [`build`].
pub(crate) struct Built {
    pub sections: Vec<Section>,
    pub symbols: Vec<Symbol>,
    pub line_table: Vec<LineEntry>,
    pub diagnostics: Vec<Diagnostic>,
    pub files: Vec<String>,
    pub expansions: Vec<Expansion>,
}

/// Lays out a parsed program and generates its bytes.
pub(crate) fn build(program: Program) -> Built {
    let mut diagnostics = program.diagnostics;
    for entry in &program.symbols {
        if let (Some(span), None) = (entry.exported_at, entry.defined_at) {
            diagnostics.push(Diagnostic::error(
                "undefined-export",
                format!("xdef of {}, which is never defined", entry.name),
                span,
            ));
        }
    }
    let values: SymbolValues = program
        .symbols
        .iter()
        .enumerate()
        .map(|(id, entry)| {
            Cell::new(entry.external.then_some(Value {
                base: Base::External(id as u32),
                offset: 0,
            }))
        })
        .collect();
    let mut settled = false;
    for _ in 0..MAX_PASSES {
        let before = values.clone();
        layout_pass(&program.sections, &program.atoms, &values);
        settled = before == values;
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

    let lookup = |id: u32| values[id as usize].get();
    let name_of = |id: u32| program.symbols[id as usize].name.clone();
    let mut line_table = Vec::new();
    let mut datas: Vec<Vec<u8>> = vec![Vec::new(); program.sections.len()];
    let mut relocs: Vec<Vec<Reloc>> = vec![Vec::new(); program.sections.len()];
    for atom in &program.atoms {
        let index = atom.section;
        let entry = &program.sections[index];
        let start = datas[index].len() as u32;
        let first_new = diagnostics.len();
        let mut ctx = EmitContext {
            section: index,
            kind: entry.kind,
            origin: entry.origin,
            span: atom.span,
            instruction_start: 0,
            lookup: &lookup,
            name_of: &name_of,
            data: &mut datas[index],
            relocs: &mut relocs[index],
            diagnostics: &mut diagnostics,
        };
        emit_atom(atom, &values, &mut ctx);
        add_expansion_chain(
            &mut diagnostics[first_new..],
            atom.expansion,
            &program.expansions,
        );
        let len = datas[index].len() as u32 - start;
        if len > 0 {
            line_table.push(LineEntry {
                section: index,
                offset: start,
                len,
                span: atom.span,
                expansion: atom.expansion,
            });
        }
    }
    line_table.sort_by_key(|entry| (entry.section, entry.offset));

    let sections = program
        .sections
        .iter()
        .zip(datas)
        .zip(relocs)
        .map(|((entry, data), relocs)| Section {
            name: entry.name.clone(),
            kind: entry.kind,
            mem: entry.mem,
            origin: entry.origin.map(|origin| origin as u32),
            data,
            relocs,
        })
        .collect();

    let symbols = program
        .symbols
        .iter()
        .zip(&values)
        .filter_map(|(entry, value)| {
            let value = value.get()?;
            Some(Symbol {
                name: entry.name.clone(),
                section: match value.base {
                    Base::Section(section) => Some(section),
                    _ => None,
                },
                value: value.offset,
                span: entry.defined_at?,
                local: entry.name.starts_with('.') || entry.name.ends_with('$'),
                exported: entry.exported_at.is_some(),
            })
        })
        .collect();

    // Undefined symbols are reported where they are used, during emission.
    Built {
        sections,
        symbols,
        line_table,
        diagnostics,
        files: program.files,
        expansions: program.expansions,
    }
}

/// The address of a position in a section, as a value.
fn address(section: usize, entry: &SectionEntry, offset: i64) -> Value {
    match entry.origin {
        Some(origin) => Value::abs(origin + offset),
        None => Value {
            base: Base::Section(section),
            offset,
        },
    }
}

/// Walks the source once, assigning symbols. Forward references that are not
/// known yet count as zero when sizing.
fn layout_pass(sections: &[SectionEntry], atoms: &[Atom], values: &SymbolValues) {
    let mut offsets = vec![0i64; sections.len()];
    for atom in atoms {
        let section = atom.section;
        let entry = &sections[section];
        let offset = offsets[section];
        let here = address(section, entry, offset);
        match &atom.kind {
            AtomKind::Label(id) => values[*id as usize].set(Some(here)),
            AtomKind::Equate(id, expr) => {
                let lookup = |id: u32| values[id as usize].get();
                if let Ok(value) = eval(expr, here, &lookup) {
                    values[*id as usize].set(Some(value));
                }
            }
            _ => offsets[section] += atom_len(atom, here, offset, entry, values),
        }
    }
}

fn atom_len(
    atom: &Atom,
    here: Value,
    offset: i64,
    entry: &SectionEntry,
    values: &SymbolValues,
) -> i64 {
    let lookup = |id: u32| values[id as usize].get().or(Some(Value::abs(0)));
    match &atom.kind {
        AtomKind::Label(_) | AtomKind::Equate(..) => 0,
        AtomKind::AlignInstruction => here.offset & 1,
        AtomKind::Instruction(instruction) => i64::from(instruction.len()),
        AtomKind::Data { size, items } => items
            .iter()
            .map(|item| match item {
                DataItem::Bytes(bytes) => bytes.len() as i64,
                DataItem::Expr(..) => i64::from(size.bytes()),
            })
            .sum(),
        AtomKind::Space { size, count, .. } => match eval(count, here, &lookup) {
            Ok(Value {
                base: Base::Absolute,
                offset: n,
            }) if (0..=MAX_SPACE).contains(&n) => {
                n.saturating_mul(i64::from(size.bytes())).min(MAX_SPACE)
            }
            _ => 0,
        },
        AtomKind::Align {
            alignment, offset, ..
        } => padding(here.offset, *alignment, *offset),
        AtomKind::Rorg(expr) => match eval(expr, here, &lookup) {
            Ok(value) => rorg_padding(value, offset, entry.origin),
            Err(_) => 0,
        },
    }
}

/// Bytes from `address` to the next multiple of `alignment`, plus `offset`.
fn padding(address: i64, alignment: i64, offset: i64) -> i64 {
    if alignment <= 0 {
        return 0;
    }
    let rounded = (address + alignment - 1) / alignment * alignment;
    rounded + offset - address
}

/// Bytes from section position `offset` up to the address `target`, if any.
fn rorg_padding(target: Value, offset: i64, origin: Option<i64>) -> i64 {
    let position = target.offset - origin.unwrap_or(0);
    (position - offset).clamp(0, MAX_SPACE)
}

fn emit_atom(atom: &Atom, values: &SymbolValues, ctx: &mut EmitContext) {
    match &atom.kind {
        AtomKind::Label(id) => {
            let here = ctx.address_of(ctx.data.len() as i64);
            values[*id as usize].set(Some(here));
        }
        AtomKind::Equate(id, expr) => {
            let here = ctx.address_of(ctx.data.len() as i64);
            if let Some(value) = ctx.evaluate(expr, here, atom.span) {
                values[*id as usize].set(Some(value));
            }
        }
        AtomKind::AlignInstruction => {
            if ctx.address_of(ctx.data.len() as i64).offset % 2 != 0 {
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
        AtomKind::Align {
            alignment,
            offset,
            nops,
        } => {
            let start = ctx.address_of(ctx.data.len() as i64).offset;
            let count = padding(start, *alignment, *offset);
            let use_nops = *nops && ctx.kind == SectionKind::Code;
            emit_padding(start, count, use_nops, ctx);
        }
        AtomKind::Rorg(expr) => {
            let offset = ctx.data.len() as i64;
            let here = ctx.address_of(offset);
            if let Some(value) = ctx.evaluate(expr, here, atom.span) {
                let count = rorg_padding(value, offset, ctx.origin);
                ctx.data.resize(ctx.data.len() + count as usize, 0);
            }
        }
    }
}

/// Writes padding starting at `address`. NOPs only fit at even addresses, so
/// odd bytes at either end are zero.
fn emit_padding(address: i64, count: i64, nops: bool, ctx: &mut EmitContext) {
    let mut left = count;
    if nops && address % 2 != 0 && left > 0 {
        ctx.data.push(0);
        left -= 1;
    }
    if nops {
        while left >= 2 {
            ctx.data.extend_from_slice(&[0x4E, 0x71]);
            left -= 2;
        }
    }
    ctx.data.resize(ctx.data.len() + left as usize, 0);
}

fn emit_data_item(item: &DataItem, size: Size, ctx: &mut EmitContext) {
    let (expr, span) = match item {
        DataItem::Bytes(bytes) => {
            ctx.data.extend_from_slice(bytes);
            return;
        }
        DataItem::Expr(expr, span) => (expr, *span),
    };
    let pc = ctx.address_of(ctx.data.len() as i64);
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
    let pc = ctx.address_of(ctx.data.len() as i64);
    let Some(value) = ctx.evaluate(count, pc, atom.span) else {
        return;
    };
    let total = value.offset.saturating_mul(i64::from(size.bytes()));
    if value.base != Base::Absolute || !(0..=MAX_SPACE).contains(&value.offset) || total > MAX_SPACE
    {
        ctx.diagnostics.push(Diagnostic::error(
            "bad-count",
            "count must be a non-negative constant of reasonable size",
            atom.span,
        ));
        return;
    }
    let fill_value = match fill {
        Some(expr) => match ctx.evaluate(expr, pc, atom.span) {
            Some(value) if value.base == Base::Absolute => value.offset,
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
