//! The instruction set: operands, the data-driven table and encoders.

pub(crate) mod encode;
pub mod table;

use crate::expr::Expr;
use crate::float::FloatLiteral;
use crate::source::Span;

/// Index register of an indexed mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IndexReg {
    /// 0-7 for D0-D7, 8-15 for A0-A7.
    pub reg: u8,
    pub long: bool,
    /// Scale as a power of two: 0 for `*1` up to 3 for `*8`.
    pub scale: u8,
    /// True if written with a `z` prefix, so it adds nothing.
    pub suppressed: bool,
}

/// The size written on a displacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Width {
    Word,
    Long,
}

/// A displacement in a full-format extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FullDisp {
    pub expr: Expr,
    pub width: Option<Width>,
}

/// The base of a full-format mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FullBase {
    Addr(u8),
    Pc,
    /// `zaN`: the register is named but adds nothing.
    SuppressedAddr(u8),
    /// `zpc`
    SuppressedPc,
    /// No base written at all.
    None,
}

/// Memory indirection in a full-format mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Indirection {
    #[default]
    None,
    /// The index is applied before the memory read.
    PreIndexed,
    /// The index, if any, is applied after the memory read.
    PostIndexed,
}

/// A 68020 full-format addressing mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FullMode {
    pub base: FullBase,
    pub index: Option<IndexReg>,
    pub indirection: Indirection,
    pub base_disp: Option<FullDisp>,
    pub outer_disp: Option<FullDisp>,
}

/// An offset or width in a bit field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BitFieldArg {
    Reg(u8),
    Value(Expr),
}

/// An addressing mode as parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mode {
    DataReg(u8),
    AddrReg(u8),
    Indirect(u8),
    PostInc(u8),
    PreDec(u8),
    /// A displacement that may grow to a full-format one on 68020 and later.
    Disp(Expr, u8),
    /// A displacement written with `.w`, which always stays 16 bits.
    DispWord(Expr, u8),
    /// An index that may grow to a full-format one on 68020 and later.
    Index(Expr, u8, IndexReg),
    AbsWord(Expr),
    AbsLong(Expr),
    /// An address without a size where a word would also do, under optimisation.
    /// Encoding sees this only for rows that have both, and resolves it.
    AbsShortable(Expr),
    PcDisp(Expr),
    PcDispWord(Expr),
    PcIndex(Expr, IndexReg),
    Full(Box<FullMode>),
    Immediate(Expr),
    Sr,
    Ccr,
    Usp,
    /// Bit 0 is D0 and bit 15 is A7.
    RegList(u16),
    /// `d0:d1`
    RegPair(u8, u8),
    /// `(a0):(d1)`, with each register numbered 0-15.
    IndirectPair(u8, u8),
    ControlReg(u16),
    Cache(u8),
    /// An MMU register for `pmove` or `pvalid`, as its bits in the command word.
    MmuReg(u16),
    /// A function code named `sfc` or `dfc`, as its bits in the command word.
    FunctionCode(u16),
    /// A floating-point literal, as an immediate.
    Float(FloatLiteral),
    FpReg(u8),
    /// `fpc:fps`
    FpPair(u8, u8),
    /// A list of floating-point registers; bit 0 is FP0.
    FpList(u8),
    /// One of FPCR, FPSR and FPIAR, as its bit in a control register list.
    FpControl(u16),
    /// More than one of FPCR, FPSR and FPIAR.
    FpControlList(u16),
    /// `<ea>{#k}` or `<ea>{Dn}`, for `fmove.p`.
    PackedStore {
        ea: Box<Mode>,
        factor: BitFieldArg,
    },
    BitField {
        ea: Box<Mode>,
        offset: BitFieldArg,
        width: BitFieldArg,
    },
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
            Mode::Disp(..) | Mode::DispWord(..) => EaSet::DISP,
            Mode::Index(..) => EaSet::INDEX,
            Mode::AbsWord(_) => EaSet::ABS_W,
            Mode::AbsLong(_) | Mode::AbsShortable(_) => EaSet::ABS_L,
            Mode::PcDisp(_) | Mode::PcDispWord(_) => EaSet::PC_DISP,
            Mode::PcIndex(..) => EaSet::PC_INDEX,
            Mode::Full(full) => match full.base {
                FullBase::Pc | FullBase::SuppressedPc => EaSet::PC_INDEX,
                _ => EaSet::INDEX,
            },
            Mode::Immediate(_) => EaSet::IMM,
            Mode::Sr => EaSet::SR,
            Mode::Ccr => EaSet::CCR,
            Mode::Usp => EaSet::USP,
            Mode::RegList(_) => EaSet::LIST,
            Mode::RegPair(..) => EaSet::PAIR,
            Mode::IndirectPair(..) => EaSet::INDIRECT_PAIR,
            Mode::ControlReg(_) => EaSet::CONTROL_REG,
            Mode::Cache(_) => EaSet::CACHE,
            // The 68851's PCSR can only be read.
            Mode::MmuReg(0x6400) => EaSet::MMU_REG_READ_ONLY,
            Mode::MmuReg(0x5400) => EaSet::MMU_VAL,
            // SRP, CRP and DRP hold 64 bits, so no register can take them.
            Mode::MmuReg(0x4400 | 0x4800 | 0x4C00) => EaSet::MMU_QUAD,
            Mode::MmuReg(0x6000) => EaSet::MMU_STATUS,
            Mode::MmuReg(_) => EaSet::MMU_REG,
            Mode::FunctionCode(_) => EaSet::FUNCTION_CODE,
            Mode::Float(_) => EaSet::FLOAT,
            Mode::FpReg(_) => EaSet::FP,
            Mode::FpPair(..) => EaSet::FP_PAIR,
            Mode::FpList(_) => EaSet::FP_LIST,
            // Only FPIAR can move to and from an address register.
            Mode::FpControl(0x0400) => EaSet::FP_IAR,
            Mode::FpControl(_) => EaSet::FP_CONTROL,
            Mode::FpControlList(_) => EaSet::FP_CONTROL_LIST,
            Mode::PackedStore { ea, .. } => ea.class().packed_store(),
            Mode::BitField { ea, .. } => ea.class().bitfield(),
        }
    }
}

/// An operand and where it was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Operand {
    pub mode: Mode,
    pub span: Span,
}
