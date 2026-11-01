//! Links assemblies into one, which [`crate::write_hunk_exe`] can then write.
//!
//! Sections with the same name, kind and memory type, or none, are joined, in the order of
//! the inputs, each padded to a longword as a hunk would be. As in vlink, code comes first in
//! the output, then data, then BSS.

use std::collections::HashMap;

use crate::error::{Diagnostic, Related};
use crate::source::Span;
use crate::types::{MemKind, SectionKind};
use crate::{Assembly, Reloc, RelocKind, RelocTarget, Section, Severity, Symbol, Target};

/// Where one input's section went.
#[derive(Clone, Copy)]
struct Placement {
    section: usize,
    offset: u32,
}

/// A symbol another input can refer to.
#[derive(Clone, Copy)]
struct Export {
    /// `None` for a constant.
    place: Option<(usize, u32)>,
    value: i64,
    span: Span,
}

fn kind_rank(kind: SectionKind) -> u8 {
    match kind {
        SectionKind::Code => 0,
        SectionKind::Data => 1,
        SectionKind::Bss => 2,
    }
}

/// Links assemblies made for [`Target::Relocatable`]. Problems are diagnostics in the result,
/// which keeps the files, lines and symbols of every input for debugging.
pub fn link(inputs: &[Assembly]) -> Assembly {
    let mut linked = Assembly {
        target: Target::Relocatable,
        ..Assembly::default()
    };
    linked.max_output_bytes = inputs
        .iter()
        .map(|input| input.max_output_bytes)
        .min()
        .unwrap_or(i64::MAX);
    let file_bases = merge_sources(inputs, &mut linked);
    check_inputs(inputs, &file_bases, &mut linked);
    if !fits_limit(inputs, linked.max_output_bytes) {
        linked.diagnostics.push(general_error(
            "limit-exceeded",
            format!(
                "linked output exceeds the limit of {} bytes",
                linked.max_output_bytes
            ),
        ));
        return linked;
    }
    let expansion_bases = merge_expansions(inputs, &file_bases, &mut linked);
    let placements = place_sections(inputs, &mut linked);
    let exports = collect_exports(inputs, &placements, &file_bases, &mut linked);
    merge_symbols(inputs, &placements, &file_bases, &mut linked);
    merge_lines(
        inputs,
        &placements,
        &file_bases,
        &expansion_bases,
        &mut linked,
    );
    apply_relocs(inputs, &placements, &file_bases, &exports, &mut linked);
    linked
}

/// Whether every section, padded to a longword, adds up to no more than the limit.
fn fits_limit(inputs: &[Assembly], limit: i64) -> bool {
    let sizes = inputs
        .iter()
        .flat_map(|input| &input.sections)
        .filter(|section| !section.dummy)
        .map(|section| (section.data.len() as i64).saturating_add(3) & !3);
    sizes.fold(0i64, i64::saturating_add) <= limit.min(i64::from(u32::MAX))
}

fn moved(span: Span, base: u32) -> Span {
    Span {
        file: span.file + base,
        ..span
    }
}

fn merge_sources(inputs: &[Assembly], linked: &mut Assembly) -> Vec<u32> {
    let mut bases = Vec::with_capacity(inputs.len());
    for input in inputs {
        bases.push(linked.files.len() as u32);
        linked.files.extend(input.files.iter().cloned());
        linked.sources.extend(input.sources.iter().cloned());
        linked.sources.resize(linked.files.len(), Vec::new());
    }
    bases
}

fn merge_expansions(inputs: &[Assembly], file_bases: &[u32], linked: &mut Assembly) -> Vec<u32> {
    let mut bases = Vec::with_capacity(inputs.len());
    for (input, file_base) in inputs.iter().zip(file_bases) {
        let base = linked.expansions.len() as u32;
        bases.push(base);
        linked
            .expansions
            .extend(input.expansions.iter().map(|expansion| {
                let mut expansion = *expansion;
                expansion.site = moved(expansion.site, *file_base);
                expansion.parent = expansion.parent.map(|parent| parent + base);
                expansion
            }));
    }
    bases
}

/// Joins sections, and returns where each input's sections went.
fn place_sections(inputs: &[Assembly], linked: &mut Assembly) -> Vec<Vec<Option<Placement>>> {
    let mut order: Vec<(&Section, usize)> = Vec::new();
    for input in inputs {
        for section in input.sections.iter().filter(|section| !section.dummy) {
            order.push((section, order.len()));
        }
    }
    // Sections of a kind keep the order they were first seen in.
    // Empty ones come last, so they don't affect the order or the memory type.
    order.sort_by_key(|(section, seen)| (section.data.is_empty(), kind_rank(section.kind), *seen));
    for (section, _) in &order {
        let number = joined_section(section, linked);
        if number.is_none() {
            linked.sections.push(Section {
                name: section.name.clone(),
                kind: section.kind,
                mem: section.mem,
                origin: None,
                dummy: false,
                data: Vec::new(),
                relocs: Vec::new(),
            });
        } else if let Some(number) = number {
            let target = &mut linked.sections[number];
            if target.mem == MemKind::Any && !section.data.is_empty() {
                target.mem = section.mem;
            }
        }
    }
    inputs
        .iter()
        .map(|input| {
            input
                .sections
                .iter()
                .map(|section| {
                    if section.dummy {
                        return None;
                    }
                    let number = joined_section(section, linked)?;
                    let target = &mut linked.sections[number];
                    let offset = target.data.len() as u32;
                    target.data.extend_from_slice(&section.data);
                    pad_to_longword(target);
                    Some(Placement {
                        section: number,
                        offset,
                    })
                })
                .collect()
        })
        .collect()
}

/// The section this one joins, which has its name and kind, and its memory type unless one
/// of them doesn't care.
fn joined_section(section: &Section, linked: &Assembly) -> Option<usize> {
    linked.sections.iter().position(|other| {
        other.name == section.name
            && other.kind == section.kind
            && (other.mem == section.mem
                || other.mem == MemKind::Any
                || section.mem == MemKind::Any)
    })
}

fn pad_to_longword(section: &mut Section) {
    let padding = (4 - section.data.len() % 4) % 4;
    if section.kind == SectionKind::Code && padding == 2 {
        section.data.extend_from_slice(&[0x4E, 0x71]);
    } else {
        section.data.resize(section.data.len() + padding, 0);
    }
}

/// Carries over diagnostics, and reports inputs that can't be linked.
fn check_inputs(inputs: &[Assembly], file_bases: &[u32], linked: &mut Assembly) {
    for (input, file_base) in inputs.iter().zip(file_bases) {
        for diagnostic in &input.diagnostics {
            let mut diagnostic = diagnostic.clone();
            diagnostic.span = diagnostic.span.map(|span| moved(span, *file_base));
            for related in &mut diagnostic.related {
                *related = Related {
                    message: related.message.clone(),
                    span: moved(related.span, *file_base),
                };
            }
            linked.diagnostics.push(diagnostic);
        }
        let name = input.files.first().map_or("input", String::as_str);
        if input.target != Target::Relocatable {
            linked.diagnostics.push(general_error(
                "link-target",
                format!("{name} was assembled for a raw binary, so it can't be linked"),
            ));
        }
        let uses_org = input
            .sections
            .iter()
            .any(|section| !section.dummy && section.origin.is_some());
        if uses_org {
            linked.diagnostics.push(general_error(
                "link-org",
                format!("{name} uses org, so it can't be linked"),
            ));
        }
    }
}

fn general_error(code: &'static str, message: String) -> Diagnostic {
    Diagnostic {
        severity: Severity::Error,
        code,
        message,
        span: None,
        related: Vec::new(),
    }
}

fn collect_exports(
    inputs: &[Assembly],
    placements: &[Vec<Option<Placement>>],
    file_bases: &[u32],
    linked: &mut Assembly,
) -> HashMap<String, Export> {
    let mut exports: HashMap<String, Export> = HashMap::new();
    for (index, input) in inputs.iter().enumerate() {
        for symbol in input.symbols.iter().filter(|symbol| symbol.exported) {
            let span = moved(symbol.span, file_bases[index]);
            let place = symbol
                .section
                .and_then(|section| placements[index].get(section).copied().flatten());
            let export = Export {
                place: place.map(|place| (place.section, place.offset)),
                value: symbol.value,
                span,
            };
            let earlier = exports.get(&symbol.name).copied();
            if earlier.is_none() {
                exports.insert(symbol.name.clone(), export);
            }
            if let Some(earlier) = earlier {
                let mut error = Diagnostic::error(
                    "duplicate-symbol",
                    format!("{} is defined more than once", symbol.name),
                    span,
                );
                error.related.push(Related {
                    message: "first defined here".to_string(),
                    span: earlier.span,
                });
                linked.diagnostics.push(error);
            }
        }
    }
    exports
}

fn merge_symbols(
    inputs: &[Assembly],
    placements: &[Vec<Option<Placement>>],
    file_bases: &[u32],
    linked: &mut Assembly,
) {
    for (index, input) in inputs.iter().enumerate() {
        for symbol in &input.symbols {
            let place = symbol
                .section
                .and_then(|section| placements[index].get(section).copied().flatten());
            if symbol.section.is_some() && place.is_none() {
                continue;
            }
            linked.symbols.push(Symbol {
                section: place.map(|place| place.section),
                value: symbol
                    .value
                    .saturating_add(place.map_or(0, |place| i64::from(place.offset))),
                span: moved(symbol.span, file_bases[index]),
                ..symbol.clone()
            });
        }
    }
}

fn merge_lines(
    inputs: &[Assembly],
    placements: &[Vec<Option<Placement>>],
    file_bases: &[u32],
    expansion_bases: &[u32],
    linked: &mut Assembly,
) {
    for (index, input) in inputs.iter().enumerate() {
        for entry in &input.line_table {
            let Some(place) = placements[index].get(entry.section).copied().flatten() else {
                continue;
            };
            let mut entry = *entry;
            entry.section = place.section;
            entry.offset += place.offset;
            entry.span = moved(entry.span, file_bases[index]);
            entry.expansion = entry.expansion.map(|e| e + expansion_bases[index]);
            linked.line_table.push(entry);
        }
    }
    linked
        .line_table
        .sort_by_key(|entry| (entry.section, entry.offset));
    for (index, input) in inputs.iter().enumerate() {
        for entry in &input.debug_lines {
            let Some(place) = placements[index].get(entry.section).copied().flatten() else {
                continue;
            };
            let mut entry = *entry;
            entry.section = place.section;
            entry.offset += place.offset;
            entry.span = moved(entry.span, file_bases[index]);
            entry.expansion = entry.expansion.map(|e| e + expansion_bases[index]);
            linked.debug_lines.push(entry);
        }
        linked.hunk_settings.line_debug |= input.hunk_settings.line_debug;
    }
}

fn apply_relocs(
    inputs: &[Assembly],
    placements: &[Vec<Option<Placement>>],
    file_bases: &[u32],
    exports: &HashMap<String, Export>,
    linked: &mut Assembly,
) {
    for (index, input) in inputs.iter().enumerate() {
        for (section_index, section) in input.sections.iter().enumerate() {
            let Some(place) = placements[index].get(section_index).copied().flatten() else {
                continue;
            };
            for reloc in &section.relocs {
                let span = moved(reloc.span, file_bases[index]);
                let Some(target) = resolve(reloc, &placements[index], exports) else {
                    if let RelocTarget::External(name) = &reloc.target {
                        linked.diagnostics.push(Diagnostic::error(
                            "undefined-symbol",
                            format!("{name} is not defined by any file"),
                            span,
                        ));
                    }
                    continue;
                };
                let moved_reloc = Reloc {
                    offset: reloc.offset + place.offset,
                    span,
                    ..reloc.clone()
                };
                apply_reloc(moved_reloc, target, place.section, linked);
            }
        }
    }
}

/// What a relocation refers to, after joining.
#[derive(Clone, Copy)]
enum Resolved {
    /// A position in a joined section.
    Place(usize, i64),
    Constant(i64),
}

fn resolve(
    reloc: &Reloc,
    placements: &[Option<Placement>],
    exports: &HashMap<String, Export>,
) -> Option<Resolved> {
    match &reloc.target {
        RelocTarget::Section(section) => {
            let place = placements.get(*section).copied().flatten()?;
            Some(Resolved::Place(place.section, i64::from(place.offset)))
        }
        RelocTarget::External(name) => {
            let export = exports.get(name)?;
            Some(match export.place {
                Some((section, offset)) => {
                    Resolved::Place(section, export.value.saturating_add(i64::from(offset)))
                }
                None => Resolved::Constant(export.value),
            })
        }
    }
}

/// Adds to the field, or leaves a relocation for the loader.
fn apply_reloc(reloc: Reloc, target: Resolved, section: usize, linked: &mut Assembly) {
    let addition = match (reloc.kind, target) {
        (RelocKind::PcRelative, Resolved::Place(other, position)) if other == section => {
            position - i64::from(reloc.offset)
        }
        // This needs a small data base, which vlink refuses without one defined too.
        (RelocKind::BaseRelative, _) => {
            linked.diagnostics.push(Diagnostic::error(
                "link-base-relative",
                "a label as a word displacement needs a small data base, which isn't supported",
                reloc.span,
            ));
            return;
        }
        (RelocKind::Absolute, Resolved::Constant(value)) => value,
        (_, Resolved::Place(other, position)) => {
            add_to_field(&reloc, position, section, linked);
            let target = RelocTarget::Section(other);
            linked.sections[section]
                .relocs
                .push(Reloc { target, ..reloc });
            return;
        }
        (_, Resolved::Constant(_)) => {
            linked.diagnostics.push(Diagnostic::error(
                "link-constant",
                "a PC-relative reference can't reach a constant",
                reloc.span,
            ));
            return;
        }
    };
    add_to_field(&reloc, addition, section, linked);
}

fn add_to_field(reloc: &Reloc, addition: i64, section: usize, linked: &mut Assembly) {
    if !matches!(reloc.size, 1 | 2 | 4) {
        linked.diagnostics.push(Diagnostic::error(
            "link-relocation",
            format!("a relocation can't have {} bytes", reloc.size),
            reloc.span,
        ));
        return;
    }
    let data = &mut linked.sections[section].data;
    let start = reloc.offset as usize;
    let Some(field) = data.get_mut(start..start + usize::from(reloc.size)) else {
        return;
    };
    let current = field
        .iter()
        .fold(0i64, |value, byte| value << 8 | i64::from(*byte));
    let bits = u32::from(reloc.size) * 8;
    let sign = 1i64 << (bits - 1);
    let current = (current ^ sign) - sign;
    let result = current.saturating_add(addition);
    let fits = match reloc.kind {
        // A signed number, or the same bits as an unsigned one.
        RelocKind::Absolute => (-sign..sign * 2).contains(&result),
        _ => (-sign..sign).contains(&result),
    };
    for (index, byte) in field.iter_mut().enumerate() {
        let shift = (usize::from(reloc.size) - 1 - index) * 8;
        *byte = (result >> shift) as u8;
    }
    if !fits {
        linked.diagnostics.push(Diagnostic::error(
            "out-of-range",
            format!("reference out of range after linking: {result}"),
            reloc.span,
        ));
    }
}
