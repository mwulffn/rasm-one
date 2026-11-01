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
pub enum OutputError {
    /// Assembly reported errors, so no output can be written.
    #[error("assembly has errors")]
    HasErrors,
    /// Two sections would occupy the same addresses in a raw binary.
    #[error("sections {0} and {1} overlap")]
    Overlap(String, String),
}

/// Severity of a [`Diagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
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

    pub(crate) fn warning(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        Self {
            severity: Severity::Warning,
            ..Self::error(code, message, span)
        }
    }
}
