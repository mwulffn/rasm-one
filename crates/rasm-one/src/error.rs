//! Diagnostics and error types.

use thiserror::Error;

use crate::source::Span;

/// Failure reported by a [`crate::FileProvider`].
#[derive(Debug, Error)]
pub enum FileError {
    /// The file does not exist.
    #[error("file not found: {0}")]
    NotFound(String),
    /// Any other failure, described by the provider.
    #[error("{0}")]
    Other(String),
}

/// Failure while writing an output format.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutputError {
    /// The assembly was made for a different kind of output, in [`crate::Options::target`].
    #[error("this was assembled for a different kind of output")]
    WrongTarget,
    /// Assembly reported errors, so no output can be written.
    #[error("assembly has errors")]
    HasErrors,
    /// Two sections would occupy the same addresses in a raw binary.
    #[error("sections {0} and {1} overlap")]
    Overlap(String, String),
    /// A raw binary would be larger than the limit, because of gaps between `org`s.
    #[error("the raw binary would exceed the size limit")]
    TooLarge,
    /// `org` gives fixed addresses, but this format is relocatable.
    #[error("org cannot be used in a relocatable format")]
    FixedAddress,
    /// A symbol from another object file is used, but this format can't link.
    #[error("external symbol {name} needs a linker; write an object file instead")]
    UnresolvedExternal {
        /// The symbol.
        name: String,
        /// Where it is used.
        span: Span,
    },
    /// The format cannot represent this relocation.
    #[error("{size}-byte relocation at {section}+{offset:#x} is not supported by this format")]
    UnsupportedReloc {
        /// Name of the section containing the field.
        section: String,
        /// Offset of the field in that section.
        offset: u32,
        /// Width of the field in bytes.
        size: u8,
        /// Source of the field.
        span: Span,
    },
}

/// Severity of a [`Diagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Severity {
    /// A message the source asked to show, with `printt` and similar.
    Info,
    /// Output can still be produced.
    Warning,
    /// Output cannot be produced.
    Error,
}

/// A secondary location that helps explain a diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Related {
    /// What this location is.
    pub message: String,
    /// Where it is.
    pub span: Span,
}

/// A structured message for editors and terminals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Severity.
    pub severity: Severity,
    /// Stable identifier, such as `"undefined-symbol"`.
    pub code: &'static str,
    /// Human-readable message.
    pub message: String,
    /// Primary location, if the problem has one.
    pub span: Option<Span>,
    /// Secondary locations.
    pub related: Vec<Related>,
}

impl Diagnostic {
    pub(crate) fn error(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        Self {
            severity: Severity::Error,
            code,
            message: message.into(),
            span: Some(span),
            related: Vec::new(),
        }
    }

    pub(crate) fn info(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        Self {
            severity: Severity::Info,
            ..Self::error(code, message, span)
        }
    }

    pub(crate) fn warning(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        Self {
            severity: Severity::Warning,
            ..Self::error(code, message, span)
        }
    }
}
