//! Raw binary output, for bootblocks and trackloaded code.

use crate::error::OutputError;
use crate::{Assembly, RelocKind, RelocTarget};

/// Writes a raw binary starting at the lowest address used. Sections without
/// an `org` start at address 0 and are ignored if empty; gaps are zero-filled.
pub fn write_bin(assembly: &Assembly) -> Result<Vec<u8>, OutputError> {
    if assembly.has_errors() {
        return Err(OutputError::HasErrors);
    }
    for section in &assembly.sections {
        for reloc in &section.relocs {
            if let RelocTarget::External(name) = &reloc.target {
                return Err(OutputError::UnresolvedExternal {
                    name: name.clone(),
                    span: reloc.span,
                });
            }
            if reloc.kind == RelocKind::PcRelative {
                return Err(OutputError::UnsupportedReloc {
                    section: section.name.clone(),
                    offset: reloc.offset,
                    size: reloc.size,
                    span: reloc.span,
                });
            }
        }
    }
    let mut pieces: Vec<_> = assembly
        .sections
        .iter()
        .filter(|s| !s.dummy && (s.origin.is_some() || !s.data.is_empty()))
        .map(|s| (u64::from(s.origin.unwrap_or(0)), s))
        .collect();
    pieces.sort_by_key(|(start, _)| *start);

    let mut out = Vec::new();
    let mut previous: Option<(u64, &str)> = None;
    let first = pieces.first().map_or(0, |(start, _)| *start);
    let last = pieces
        .iter()
        .map(|(start, section)| start + section.data.len() as u64)
        .max()
        .unwrap_or(first);
    if (last - first) as i64 > assembly.max_output_bytes {
        return Err(OutputError::TooLarge);
    }
    for (start, section) in pieces {
        if let Some((end, name)) = previous {
            if start < end {
                return Err(OutputError::Overlap(name.to_string(), section.name.clone()));
            }
        }
        out.resize((start - first) as usize, 0);
        out.extend_from_slice(&section.data);
        previous = Some((start + section.data.len() as u64, &section.name));
    }
    Ok(out)
}
