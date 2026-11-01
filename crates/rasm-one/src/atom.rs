//! Atoms: the units a section is built from before layout.

use crate::expr::{Expr, SymId};
use crate::float::FloatLiteral;
use crate::isa::encode::Instruction;
use crate::source::Span;
use crate::types::Size;

/// One element of a `dc` list.
#[derive(Debug, Clone)]
pub(crate) enum DataItem {
    /// A string in a `dc.b`, stored byte by byte.
    Bytes(Vec<u8>),
    Expr(Expr, Span),
    /// A floating-point literal in a `dc.s`, `.d`, `.x` or `.p`.
    Float(FloatLiteral, Span),
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
    /// Gives a symbol a value at this point in the source.
    Equate(SymId, Expr),
    /// Pads to the next multiple of `alignment`, then on by `offset`. Pads
    /// with NOPs in code sections if `nops` is set, and otherwise with zeros.
    Align {
        alignment: i64,
        offset: i64,
        nops: bool,
    },
    /// Pads with zeros up to an address, if it isn't already past it.
    Rorg(Expr),
}

#[derive(Debug, Clone)]
pub(crate) struct Atom {
    pub kind: AtomKind,
    pub span: Span,
    /// Index of the section it belongs to.
    pub section: usize,
    /// The include or macro expansion it came from, if any.
    pub expansion: Option<u32>,
    /// True under `opt p+`, where absolute references are errors.
    pub position_independent: bool,
}
