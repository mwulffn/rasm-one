//! Atoms: the units a section is built from before layout.

use crate::expr::{Expr, SymId};
use crate::isa::encode::Instruction;
use crate::source::Span;
use crate::types::Size;

/// One element of a `dc` list.
#[derive(Debug, Clone)]
pub(crate) enum DataItem {
    /// A string in a `dc.b`, stored byte by byte.
    Bytes(Vec<u8>),
    Expr(Expr, Span),
}

#[derive(Debug, Clone)]
pub(crate) enum AtomKind {
    Label(SymId),
    /// Pads to an even address before an instruction, with a warning.
    AlignInstruction,
    Instruction(Instruction),
    Data {
        size: Size,
        items: Vec<DataItem>,
    },
    Space {
        size: Size,
        count: Expr,
        fill: Option<Expr>,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct Atom {
    pub kind: AtomKind,
    pub span: Span,
}
