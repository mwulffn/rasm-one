//! Source access and source positions.

use std::collections::HashMap;

use crate::error::FileError;

/// Index into [`crate::Assembly::files`].
pub type FileId = u32;

/// A position in source. Lines and columns are 1-based; columns count bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// File containing the span.
    pub file: FileId,
    /// Line number.
    pub line: u32,
    /// Column of the first byte.
    pub col: u32,
    /// Length in bytes.
    pub len: u32,
}

/// The only route by which the library reads files.
pub trait FileProvider {
    /// Returns the contents of `path`. `including_file` is the file that
    /// asked for it, or `None` for the main source.
    fn read(&self, path: &str, including_file: Option<&str>) -> Result<Vec<u8>, FileError>;
}

/// A [`FileProvider`] backed by memory, for browsers and tests.
#[derive(Debug, Clone, Default)]
pub struct MemoryProvider {
    files: HashMap<String, Vec<u8>>,
}

impl MemoryProvider {
    /// Creates an empty provider.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces a file.
    pub fn insert(&mut self, path: impl Into<String>, contents: impl Into<Vec<u8>>) {
        self.files.insert(path.into(), contents.into());
    }
}

impl FileProvider for MemoryProvider {
    fn read(&self, path: &str, _including_file: Option<&str>) -> Result<Vec<u8>, FileError> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| FileError::NotFound(path.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_provider_read_missing_file_is_error() {
        let provider = MemoryProvider::new();
        assert!(matches!(
            provider.read("x.s", None),
            Err(FileError::NotFound(_))
        ));
    }
}
