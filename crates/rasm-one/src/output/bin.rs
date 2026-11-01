//! Raw binary output, for bootblocks and trackloaded code.

use crate::error::OutputError;
use crate::Assembly;

/// Writes a raw binary starting at the lowest address used. Sections without
/// an `org` start at address 0 and are ignored if empty; gaps are zero-filled.
pub fn write_bin(assembly: &Assembly) -> Result<Vec<u8>, OutputError> {
    if assembly.has_errors() {
        return Err(OutputError::HasErrors);
    }
    let mut pieces: Vec<_> = assembly
        .sections
        .iter()
        .filter(|s| s.origin.is_some() || !s.data.is_empty())
        .map(|s| (u64::from(s.origin.unwrap_or(0)), s))
        .collect();
    pieces.sort_by_key(|(start, _)| *start);

    let mut out = Vec::new();
    let mut previous: Option<(u64, &str)> = None;
    let first = pieces.first().map_or(0, |(start, _)| *start);
    for (start, section) in pieces {
        if let Some((end, name)) = previous {
            // Sections that hold no bytes can't collide with anything.
            if start < end && !section.data.is_empty() {
                return Err(OutputError::Overlap(name.to_string(), section.name.clone()));
            }
        }
        out.resize((start - first) as usize, 0);
        out.extend_from_slice(&section.data);
        previous = Some((start + section.data.len() as u64, &section.name));
    }
    Ok(out)
}
