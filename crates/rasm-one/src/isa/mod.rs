//! The instruction set: operands, the data-driven table and encoders.

pub(crate) mod encode;
pub mod table;

use crate::expr::Expr;
use crate::source::Span;

/// Index register of an indexed mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IndexReg {
    /// 0-7 for D0-D7, 8-15 for A0-A7.
    pub reg: u8,
    pub long: bool,
}

/// An addressing mode as parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mode {
    DataReg(u8),
    AddrReg(u8),
    Indirect(u8),
    PostInc(u8),
    PreDec(u8),
    Disp(Expr, u8),
    Index(Expr, u8, IndexReg),
    AbsWord(Expr),
    AbsLong(Expr),
    PcDisp(Expr),
    PcIndex(Expr, IndexReg),
    Immediate(Expr),
}

impl Mode {
    /// The class this mode belongs to, for matching table rows.
    pub(crate) fn class(&self) -> table::EaSet {
        use table::EaSet;
        match self {
            Mode::DataReg(_) => EaSet::DN,
            Mode::AddrReg(_) => EaSet::AN,
            Mode::Indirect(_) => EaSet::IND,
            Mode::PostInc(_) => EaSet::POST,
            Mode::PreDec(_) => EaSet::PRE,
            Mode::Disp(..) => EaSet::DISP,
            Mode::Index(..) => EaSet::INDEX,
            Mode::AbsWord(_) => EaSet::ABS_W,
            Mode::AbsLong(_) => EaSet::ABS_L,
            Mode::PcDisp(_) => EaSet::PC_DISP,
            Mode::PcIndex(..) => EaSet::PC_INDEX,
            Mode::Immediate(_) => EaSet::IMM,
        }
    }
}

/// An operand and where it was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Operand {
    pub mode: Mode,
    pub span: Span,
}
