//! Raw binary output, for bootblocks and trackloaded code.

use crate::error::OutputError;
use crate::Assembly;

/// Writes a raw binary. Sections all start at address 0, so at most one may
/// hold any bytes.
pub fn write_bin(assembly: &Assembly) -> Result<Vec<u8>, OutputError> {
    if assembly.has_errors() {
        return Err(OutputError::HasErrors);
    }
    let mut filled = assembly.sections.iter().filter(|s| !s.data.is_empty());
    let first = filled.next();
    if let (Some(first), Some(second)) = (first, filled.next()) {
        return Err(OutputError::Overlap(
            first.name.clone(),
            second.name.clone(),
        ));
    }
    Ok(first.map(|s| s.data.clone()).unwrap_or_default())
}
