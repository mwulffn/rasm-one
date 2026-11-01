//! The instruction table, transcribed from the M68000 Family Programmer's
//! Reference Manual. A row is one form of an instruction on a set of CPUs;
//! forms whose encoding differs between CPUs get separate rows.

use crate::types::{CpuSet, Size};

/// A set of effective address classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EaSet(pub u16);

impl EaSet {
    /// `Dn`
    pub const DN: EaSet = EaSet(1 << 0);
    /// `An`
    pub const AN: EaSet = EaSet(1 << 1);
    /// `(An)`
    pub const IND: EaSet = EaSet(1 << 2);
    /// `(An)+`
    pub const POST: EaSet = EaSet(1 << 3);
    /// `-(An)`
    pub const PRE: EaSet = EaSet(1 << 4);
    /// `(d16,An)`
    pub const DISP: EaSet = EaSet(1 << 5);
    /// `(d8,An,Xn)`
    pub const INDEX: EaSet = EaSet(1 << 6);
    /// `(xxx).W`
    pub const ABS_W: EaSet = EaSet(1 << 7);
    /// `(xxx).L`
    pub const ABS_L: EaSet = EaSet(1 << 8);
    /// `(d16,PC)`
    pub const PC_DISP: EaSet = EaSet(1 << 9);
    /// `(d8,PC,Xn)`
    pub const PC_INDEX: EaSet = EaSet(1 << 10);
    /// `#<data>`
    pub const IMM: EaSet = EaSet(1 << 11);
    /// A branch target: an address expression written without a mode.
    pub const LABEL: EaSet = EaSet::ABS_L;

    /// Every class.
    pub const ALL: EaSet = EaSet(0x0FFF);
    /// Every class except `An`.
    pub const DATA: EaSet = EaSet(EaSet::ALL.0 & !EaSet::AN.0);
    /// `(An)` and the other modes usable by `lea`.
    pub const CONTROL: EaSet = EaSet(
        EaSet::IND.0
            | EaSet::DISP.0
            | EaSet::INDEX.0
            | EaSet::ABS_W.0
            | EaSet::ABS_L.0
            | EaSet::PC_DISP.0
            | EaSet::PC_INDEX.0,
    );
    /// Writable modes, excluding `An`.
    pub const DATA_ALTERABLE: EaSet = EaSet(
        EaSet::DN.0
            | EaSet::IND.0
            | EaSet::POST.0
            | EaSet::PRE.0
            | EaSet::DISP.0
            | EaSet::INDEX.0
            | EaSet::ABS_W.0
            | EaSet::ABS_L.0,
    );
    /// Writable modes, including `An`.
    pub const ALTERABLE: EaSet = EaSet(EaSet::DATA_ALTERABLE.0 | EaSet::AN.0);

    /// True if the classes overlap.
    pub const fn intersects(self, other: EaSet) -> bool {
        self.0 & other.0 != 0
    }
}

/// Coprocessor a row needs in addition to its CPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coprocessor {
    /// None.
    None,
    /// A floating-point unit.
    Fpu,
    /// A memory management unit.
    Mmu,
}

/// How a row's fields are combined into machine code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// Just the opcode word.
    Fixed,
    /// `move` and `movea`: size, destination EA, source EA.
    Move,
    /// `lea`: destination register, source EA.
    Lea,
    /// `Bcc`, `bra`, `bsr`: opcode with displacement.
    Branch,
}

/// One form of an instruction.
#[derive(Debug, Clone, Copy)]
pub struct InsnRow {
    /// Lower-case mnemonic.
    pub mnemonic: &'static str,
    /// Sizes that may be written.
    pub sizes: &'static [Size],
    /// Size used if none is written; `None` for unsized instructions.
    pub default_size: Option<Size>,
    /// Permitted classes per operand.
    pub operands: &'static [EaSet],
    /// CPUs with this form.
    pub cpus: CpuSet,
    /// Required coprocessor.
    pub coprocessor: Coprocessor,
    /// True if this form traps to software on the 68060.
    pub trap_060: bool,
    /// Base opcode word.
    pub opcode: u16,
    /// Encoding.
    pub encoding: Encoding,
    /// Execution cycles. Reserved; not filled in yet.
    pub cycles: Option<u16>,
}

const WL: &[Size] = &[Size::Word, Size::Long];
const L: &[Size] = &[Size::Long];
const B: &[Size] = &[Size::Byte];
const BRANCH: &[Size] = &[Size::Short, Size::Byte, Size::Word];

const fn row(
    mnemonic: &'static str,
    sizes: &'static [Size],
    default_size: Option<Size>,
    operands: &'static [EaSet],
    opcode: u16,
    encoding: Encoding,
) -> InsnRow {
    InsnRow {
        mnemonic,
        sizes,
        default_size,
        operands,
        cpus: CpuSet::ALL,
        coprocessor: Coprocessor::None,
        trap_060: false,
        opcode,
        encoding,
        cycles: None,
    }
}

const fn branch(mnemonic: &'static str, opcode: u16) -> InsnRow {
    row(
        mnemonic,
        BRANCH,
        Some(Size::Word),
        &[EaSet::LABEL],
        opcode,
        Encoding::Branch,
    )
}

/// All instruction forms. Rows for a mnemonic are adjacent.
pub const TABLE: &[InsnRow] = &[
    row("nop", &[], None, &[], 0x4E71, Encoding::Fixed),
    row("rts", &[], None, &[], 0x4E75, Encoding::Fixed),
    // `An` is not a valid byte operand, so the byte form is its own row.
    row(
        "move",
        B,
        Some(Size::Word),
        &[EaSet::DATA, EaSet::DATA_ALTERABLE],
        0,
        Encoding::Move,
    ),
    row(
        "move",
        WL,
        Some(Size::Word),
        &[EaSet::ALL, EaSet::ALTERABLE],
        0,
        Encoding::Move,
    ),
    row(
        "movea",
        WL,
        Some(Size::Word),
        &[EaSet::ALL, EaSet::AN],
        0,
        Encoding::Move,
    ),
    row(
        "lea",
        L,
        Some(Size::Long),
        &[EaSet::CONTROL, EaSet::AN],
        0x41C0,
        Encoding::Lea,
    ),
    branch("bra", 0x6000),
    branch("bsr", 0x6100),
    branch("bhi", 0x6200),
    branch("bls", 0x6300),
    branch("bcc", 0x6400),
    branch("bhs", 0x6400),
    branch("bcs", 0x6500),
    branch("blo", 0x6500),
    branch("bne", 0x6600),
    branch("beq", 0x6700),
    branch("bvc", 0x6800),
    branch("bvs", 0x6900),
    branch("bpl", 0x6A00),
    branch("bmi", 0x6B00),
    branch("bge", 0x6C00),
    branch("blt", 0x6D00),
    branch("bgt", 0x6E00),
    branch("ble", 0x6F00),
];

/// Returns the rows for a lower-case mnemonic, or `None` if it is unknown.
pub fn rows_for(mnemonic: &str) -> Option<&'static [InsnRow]> {
    let start = TABLE.iter().position(|r| r.mnemonic == mnemonic)?;
    let count = TABLE[start..]
        .iter()
        .take_while(|r| r.mnemonic == mnemonic)
        .count();
    Some(&TABLE[start..start + count])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rows_for_returns_adjacent_rows() {
        assert_eq!(rows_for("move").map(<[InsnRow]>::len), Some(2));
    }

    #[test]
    fn test_rows_for_unknown_mnemonic_is_none() {
        assert!(rows_for("bogus").is_none());
    }

    #[test]
    fn test_table_mnemonics_are_adjacent_and_lower_case() {
        for (i, row) in TABLE.iter().enumerate() {
            assert_eq!(row.mnemonic, row.mnemonic.to_ascii_lowercase());
            let seen_before = TABLE[..i].iter().any(|r| r.mnemonic == row.mnemonic);
            let previous_same = i > 0 && TABLE[i - 1].mnemonic == row.mnemonic;
            assert!(!seen_before || previous_same, "{}", row.mnemonic);
        }
    }
}
