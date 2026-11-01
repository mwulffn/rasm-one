//! The instruction table, transcribed from the M68000 Family Programmer's
//! Reference Manual. A row is one form of an instruction on a set of CPUs;
//! forms whose encoding differs between CPUs get separate rows.

use crate::types::{CpuSet, Size};

/// A set of effective address classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EaSet(pub u32);

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
    /// `SR`
    pub const SR: EaSet = EaSet(1 << 12);
    /// `CCR`
    pub const CCR: EaSet = EaSet(1 << 13);
    /// `USP`
    pub const USP: EaSet = EaSet(1 << 14);
    /// A register list of more than one register, such as `d0-d3/a0`.
    pub const LIST: EaSet = EaSet(1 << 15);
    /// A pair of data registers, such as `d0:d1`.
    pub const PAIR: EaSet = EaSet(1 << 16);
    /// A pair of register-indirect operands, such as `(a0):(d1)`.
    pub const INDIRECT_PAIR: EaSet = EaSet(1 << 17);
    /// A control register, for `movec`.
    pub const CONTROL_REG: EaSet = EaSet(1 << 18);
    /// A cache name, for `cinv` and `cpush`.
    pub const CACHE: EaSet = EaSet(1 << 19);
    /// A branch target: an address expression written without a mode.
    pub const LABEL: EaSet = EaSet::ABS_L;

    /// Every effective address class.
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
    /// Writable memory modes.
    pub const MEM_ALTERABLE: EaSet = EaSet(EaSet::DATA_ALTERABLE.0 & !EaSet::DN.0);
    /// Control modes that are writable, for `movem` to memory.
    pub const CONTROL_ALTERABLE: EaSet =
        EaSet(EaSet::CONTROL.0 & !EaSet::PC_DISP.0 & !EaSet::PC_INDEX.0);
    /// What `movem` accepts as a register list. A bare expression counts as a
    /// mask, which is the form symbolic register lists will need.
    pub const REG_MASK: EaSet =
        EaSet(EaSet::LIST.0 | EaSet::DN.0 | EaSet::AN.0 | EaSet::IMM.0 | EaSet::ABS_L.0);

    /// The bit-field form of these classes, such as `(a0){1:2}`.
    pub const fn bitfield(self) -> EaSet {
        EaSet(self.0 << 20)
    }

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

/// How a row's fields are combined into machine code. Names give the opcode
/// fields beyond `opcode`; `ea0` is operand 0 as an effective address, and
/// `reg0` is operand 0's register number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// Just the opcode word.
    Fixed,
    /// `move` and `movea`: size, destination EA, source EA.
    Move,
    /// `Bcc`, `bra`, `bsr`: opcode with displacement.
    Branch,
    /// `ea0`
    Ea0,
    /// `ea1`
    Ea1,
    /// size, `ea0`
    SizeEa0,
    /// `reg1 << 9`, `ea0`
    RegEa,
    /// `reg1 << 9`, size, `ea0`
    ToReg,
    /// `reg0 << 9`, size, `ea1`
    FromReg,
    /// `reg1 << 9`, operation size bit, `ea0`
    Address,
    /// size, `ea1`, followed by the immediate
    Immediate,
    /// Opcode followed by the immediate
    ImmediateOnly,
    /// Immediate 1-8 `<< 9`, size, `ea1`
    Quick,
    /// `reg1 << 9`, size, register-or-predecrement flag, `reg0`
    RegOrPredec,
    /// `reg1 << 9`, size, `reg0`
    Cmpm,
    /// Count (register or immediate 1-8) `<< 9`, size, flag, `reg1`
    Shift,
    /// Count of 1 `<< 9`, size, `reg0`
    ShiftOnce,
    /// `reg0 << 9`, `ea1`
    BitReg,
    /// `ea1`, preceded by the bit number
    BitImmediate,
    /// `reg0`, followed by a 16-bit displacement
    DecrementBranch,
    /// `exg`: the two registers
    Exchange,
    /// `reg0`
    Reg0,
    /// `reg1`
    Reg1,
    /// `reg0`, followed by a 16-bit immediate
    Link,
    /// Immediate 0-15
    Trap,
    /// Opcode followed by an unsigned 16-bit immediate
    Stop,
    /// `reg1 << 9`, 8-bit immediate
    Moveq,
    /// `reg1 << 9`, base register of `ea0`, displacement
    MovepLoad,
    /// `reg0 << 9`, base register of `ea1`, displacement
    MovepStore,
    /// Register mask, then `ea1`
    MovemStore,
    /// Register mask, then `ea0`
    MovemLoad,
    /// Opcode, then a 32-bit displacement
    BranchLong,
    /// `ea0`, then an extension word with the registers of operand 1
    MulDivLong,
    /// Size in bits 9-10, `ea0`, then an extension word with `reg1`
    Chk2,
    /// Size in bits 9-10, `ea2`, then an extension word with `reg0` and `reg1`
    Cas,
    /// Two extension words from the register pairs
    Cas2,
    /// `reg1 << 9`, register-or-predecrement flag, `reg0`, then a 16-bit value
    PackUnpack,
    /// `ea0`, with an extension word for its bit field
    BitField,
    /// `ea0`, with an extension word for its bit field and `reg1`
    BitFieldToReg,
    /// `ea1`, with an extension word for its bit field and `reg0`
    BitFieldFromReg,
    /// An extension word with `reg1` and the control register of operand 0
    MovecFrom,
    /// An extension word with `reg0` and the control register of operand 1
    MovecTo,
    /// Size, `ea0`, then an extension word with `reg1`
    MovesLoad,
    /// Size, `ea1`, then an extension word with `reg0`
    MovesStore,
    /// Immediate 0-7
    Bkpt,
    /// `ea1`, preceded by an 8-bit immediate in a word
    Callm,
    /// `reg0`, then an extension word with `reg1`
    Move16Registers,
    /// `reg0`, then the absolute address from operand 1
    Move16To,
    /// `reg1`, then the absolute address from operand 0
    Move16From,
    /// Cache in bits 6-7, and `reg1` if there is a second operand
    CacheOp,
    /// Opcode, extension word, then an unsigned 16-bit immediate
    Lpstop,
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
    /// Fixed bits of an extension word, for encodings that have one.
    pub extension: u16,
    /// Execution cycles. Reserved; not filled in yet.
    pub cycles: Option<u16>,
}

impl InsnRow {
    /// This row, available only on `cpus`.
    pub const fn on(mut self, cpus: CpuSet) -> InsnRow {
        self.cpus = cpus;
        self
    }

    /// This row, with fixed extension word bits.
    pub const fn ext(mut self, extension: u16) -> InsnRow {
        self.extension = extension;
        self
    }
}

const BWL: &[Size] = &[Size::Byte, Size::Word, Size::Long];
const WL: &[Size] = &[Size::Word, Size::Long];
const L: &[Size] = &[Size::Long];
const W: &[Size] = &[Size::Word];
const B: &[Size] = &[Size::Byte];
const BRANCH: &[Size] = &[Size::Short, Size::Byte, Size::Word];

const UP010: CpuSet = CpuSet(0x3E);
const UP020: CpuSet = CpuSet(0x3C);
const ONLY020: CpuSet = CpuSet(0x04);
const UP040: CpuSet = CpuSet(0x30);
const ONLY060: CpuSet = CpuSet(0x20);

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
        extension: 0,
        cycles: None,
    }
}

const WORD: Option<Size> = Some(Size::Word);
const BYTE: Option<Size> = Some(Size::Byte);
const LONG: Option<Size> = Some(Size::Long);

const DN: EaSet = EaSet::DN;
const AN: EaSet = EaSet::AN;
const IMM: EaSet = EaSet::IMM;
const ALL: EaSet = EaSet::ALL;
const DATA: EaSet = EaSet::DATA;
const MEM: EaSet = EaSet::MEM_ALTERABLE;
const DALT: EaSet = EaSet::DATA_ALTERABLE;
const CTRL: EaSet = EaSet::CONTROL;
const PRE: EaSet = EaSet::PRE;
const POST: EaSet = EaSet::POST;
const SR: EaSet = EaSet::SR;
const CCR: EaSet = EaSet::CCR;
const USP: EaSet = EaSet::USP;
const DISP: EaSet = EaSet(EaSet::DISP.0 | EaSet::IND.0);
const RN: EaSet = EaSet(DN.0 | AN.0);
const PC_MODES: EaSet = EaSet(EaSet::PC_DISP.0 | EaSet::PC_INDEX.0);
const PAIR: EaSet = EaSet::PAIR;
const CREG: EaSet = EaSet::CONTROL_REG;
const CACHE: EaSet = EaSet::CACHE;
const IND: EaSet = EaSet::IND;
const ABSL: EaSet = EaSet::ABS_L;
/// Bit fields that are only read.
const BF_READ: EaSet = EaSet(DN.0 | CTRL.0).bitfield();
/// Bit fields that are written.
const BF_WRITE: EaSet = EaSet(DN.0 | EaSet::CONTROL_ALTERABLE.0).bitfield();

const fn long_branch(mnemonic: &'static str, opcode: u16) -> InsnRow {
    row(
        mnemonic,
        L,
        WORD,
        &[EaSet::LABEL],
        opcode | 0xFF,
        Encoding::BranchLong,
    )
    .on(UP020)
}

const fn branch(mnemonic: &'static str, opcode: u16) -> InsnRow {
    row(
        mnemonic,
        BRANCH,
        WORD,
        &[EaSet::LABEL],
        opcode,
        Encoding::Branch,
    )
}

const fn set(mnemonic: &'static str, condition: u16) -> InsnRow {
    row(
        mnemonic,
        B,
        BYTE,
        &[DALT],
        0x50C0 | condition << 8,
        Encoding::Ea0,
    )
}

const fn decrement(mnemonic: &'static str, condition: u16) -> InsnRow {
    row(
        mnemonic,
        W,
        WORD,
        &[DN, EaSet::LABEL],
        0x50C8 | condition << 8,
        Encoding::DecrementBranch,
    )
}

const fn sized(mnemonic: &'static str, opcode: u16) -> InsnRow {
    row(mnemonic, BWL, WORD, &[DALT], opcode, Encoding::SizeEa0)
}

const fn word_to_reg(mnemonic: &'static str, opcode: u16) -> InsnRow {
    row(mnemonic, W, WORD, &[DATA, DN], opcode, Encoding::RegEa)
}

pub(crate) const CPU_020_UP: CpuSet = UP020;

const fn fixed(mnemonic: &'static str, opcode: u16) -> InsnRow {
    row(mnemonic, &[], None, &[], opcode, Encoding::Fixed)
}

/// Rows `lsl`-style shifts share: register form, one-register form, memory form.
macro_rules! shift {
    ($name:literal, $kind:literal, $left:literal) => {
        [
            row(
                $name,
                BWL,
                WORD,
                &[EaSet(DN.0 | IMM.0), DN],
                0xE000 | $left << 8 | $kind << 3,
                Encoding::Shift,
            ),
            row(
                $name,
                BWL,
                WORD,
                &[DN],
                0xE000 | $left << 8 | $kind << 3,
                Encoding::ShiftOnce,
            ),
            row(
                $name,
                W,
                WORD,
                &[MEM],
                0xE0C0 | $kind << 9 | $left << 8,
                Encoding::Ea0,
            ),
        ]
    };
}

const ASL: [InsnRow; 3] = shift!("asl", 0, 1);
const ASR: [InsnRow; 3] = shift!("asr", 0, 0);
const LSL: [InsnRow; 3] = shift!("lsl", 1, 1);
const LSR: [InsnRow; 3] = shift!("lsr", 1, 0);
const ROXL: [InsnRow; 3] = shift!("roxl", 2, 1);
const ROXR: [InsnRow; 3] = shift!("roxr", 2, 0);
const ROL: [InsnRow; 3] = shift!("rol", 3, 1);
const ROR: [InsnRow; 3] = shift!("ror", 3, 0);

/// All instruction forms. Rows for a mnemonic are adjacent and tried in order.
pub const TABLE: &[InsnRow] = &[
    fixed("nop", 0x4E71),
    fixed("rts", 0x4E75),
    fixed("rte", 0x4E73),
    fixed("rtr", 0x4E77),
    fixed("reset", 0x4E70),
    fixed("trapv", 0x4E76),
    fixed("illegal", 0x4AFC),
    // `An` is not a valid byte operand, so byte forms are separate rows.
    row("move", B, WORD, &[DATA, DALT], 0, Encoding::Move),
    row(
        "move",
        WL,
        WORD,
        &[ALL, EaSet::ALTERABLE],
        0,
        Encoding::Move,
    ),
    row("move", W, WORD, &[CCR, DALT], 0x42C0, Encoding::Ea1).on(UP010),
    row("move", W, WORD, &[SR, DALT], 0x40C0, Encoding::Ea1),
    row("move", W, WORD, &[DATA, CCR], 0x44C0, Encoding::Ea0),
    row("move", W, WORD, &[DATA, SR], 0x46C0, Encoding::Ea0),
    row("move", L, LONG, &[AN, USP], 0x4E60, Encoding::Reg0),
    row("move", L, LONG, &[USP, AN], 0x4E68, Encoding::Reg1),
    row("movea", WL, WORD, &[ALL, AN], 0, Encoding::Move),
    row("moveq", L, LONG, &[IMM, DN], 0x7000, Encoding::Moveq),
    row("movep", W, WORD, &[DISP, DN], 0x0108, Encoding::MovepLoad),
    row("movep", L, WORD, &[DISP, DN], 0x0148, Encoding::MovepLoad),
    row("movep", W, WORD, &[DN, DISP], 0x0188, Encoding::MovepStore),
    row("movep", L, WORD, &[DN, DISP], 0x01C8, Encoding::MovepStore),
    row(
        "movem",
        W,
        WORD,
        &[EaSet::REG_MASK, EaSet(EaSet::CONTROL_ALTERABLE.0 | PRE.0)],
        0x4880,
        Encoding::MovemStore,
    ),
    row(
        "movem",
        L,
        WORD,
        &[EaSet::REG_MASK, EaSet(EaSet::CONTROL_ALTERABLE.0 | PRE.0)],
        0x48C0,
        Encoding::MovemStore,
    ),
    row(
        "movem",
        W,
        WORD,
        &[EaSet(CTRL.0 | POST.0), EaSet::REG_MASK],
        0x4C80,
        Encoding::MovemLoad,
    ),
    row(
        "movem",
        L,
        WORD,
        &[EaSet(CTRL.0 | POST.0), EaSet::REG_MASK],
        0x4CC0,
        Encoding::MovemLoad,
    ),
    row("lea", L, LONG, &[CTRL, AN], 0x41C0, Encoding::RegEa),
    row("pea", L, LONG, &[CTRL], 0x4840, Encoding::Ea0),
    row("jmp", &[], None, &[CTRL], 0x4EC0, Encoding::Ea0),
    row("jsr", &[], None, &[CTRL], 0x4E80, Encoding::Ea0),
    row("add", B, WORD, &[DATA, DN], 0xD000, Encoding::ToReg),
    row("add", WL, WORD, &[ALL, DN], 0xD000, Encoding::ToReg),
    row("add", WL, WORD, &[ALL, AN], 0xD000, Encoding::Address),
    row("add", BWL, WORD, &[DN, MEM], 0xD100, Encoding::FromReg),
    row("add", BWL, WORD, &[IMM, MEM], 0x0600, Encoding::Immediate),
    row("adda", WL, WORD, &[ALL, AN], 0xD000, Encoding::Address),
    row("addi", BWL, WORD, &[IMM, DALT], 0x0600, Encoding::Immediate),
    row("addq", B, WORD, &[IMM, DALT], 0x5000, Encoding::Quick),
    row(
        "addq",
        WL,
        WORD,
        &[IMM, EaSet::ALTERABLE],
        0x5000,
        Encoding::Quick,
    ),
    row("addx", BWL, WORD, &[DN, DN], 0xD100, Encoding::RegOrPredec),
    row(
        "addx",
        BWL,
        WORD,
        &[PRE, PRE],
        0xD100,
        Encoding::RegOrPredec,
    ),
    row("sub", B, WORD, &[DATA, DN], 0x9000, Encoding::ToReg),
    row("sub", WL, WORD, &[ALL, DN], 0x9000, Encoding::ToReg),
    row("sub", WL, WORD, &[ALL, AN], 0x9000, Encoding::Address),
    row("sub", BWL, WORD, &[DN, MEM], 0x9100, Encoding::FromReg),
    row("sub", BWL, WORD, &[IMM, MEM], 0x0400, Encoding::Immediate),
    row("suba", WL, WORD, &[ALL, AN], 0x9000, Encoding::Address),
    row("subi", BWL, WORD, &[IMM, DALT], 0x0400, Encoding::Immediate),
    row("subq", B, WORD, &[IMM, DALT], 0x5100, Encoding::Quick),
    row(
        "subq",
        WL,
        WORD,
        &[IMM, EaSet::ALTERABLE],
        0x5100,
        Encoding::Quick,
    ),
    row("subx", BWL, WORD, &[DN, DN], 0x9100, Encoding::RegOrPredec),
    row(
        "subx",
        BWL,
        WORD,
        &[PRE, PRE],
        0x9100,
        Encoding::RegOrPredec,
    ),
    row("cmp", B, WORD, &[DATA, DN], 0xB000, Encoding::ToReg),
    row("cmp", WL, WORD, &[ALL, DN], 0xB000, Encoding::ToReg),
    row("cmp", WL, WORD, &[ALL, AN], 0xB000, Encoding::Address),
    row("cmp", BWL, WORD, &[IMM, MEM], 0x0C00, Encoding::Immediate),
    row(
        "cmp",
        BWL,
        WORD,
        &[IMM, PC_MODES],
        0x0C00,
        Encoding::Immediate,
    )
    .on(UP020),
    row("cmp", BWL, WORD, &[POST, POST], 0xB108, Encoding::Cmpm),
    row("cmpa", WL, WORD, &[ALL, AN], 0xB000, Encoding::Address),
    row("cmpi", BWL, WORD, &[IMM, DALT], 0x0C00, Encoding::Immediate),
    row(
        "cmpi",
        BWL,
        WORD,
        &[IMM, PC_MODES],
        0x0C00,
        Encoding::Immediate,
    )
    .on(UP020),
    row("cmpm", BWL, WORD, &[POST, POST], 0xB108, Encoding::Cmpm),
    row("and", BWL, WORD, &[DATA, DN], 0xC000, Encoding::ToReg),
    row("and", BWL, WORD, &[DN, MEM], 0xC100, Encoding::FromReg),
    row("and", BWL, WORD, &[IMM, MEM], 0x0200, Encoding::Immediate),
    row("and", B, BYTE, &[IMM, CCR], 0x023C, Encoding::ImmediateOnly),
    row("and", W, WORD, &[IMM, SR], 0x027C, Encoding::ImmediateOnly),
    row("andi", BWL, WORD, &[IMM, DALT], 0x0200, Encoding::Immediate),
    row(
        "andi",
        B,
        BYTE,
        &[IMM, CCR],
        0x023C,
        Encoding::ImmediateOnly,
    ),
    row("andi", W, WORD, &[IMM, SR], 0x027C, Encoding::ImmediateOnly),
    row("or", BWL, WORD, &[DATA, DN], 0x8000, Encoding::ToReg),
    row("or", BWL, WORD, &[DN, MEM], 0x8100, Encoding::FromReg),
    row("or", BWL, WORD, &[IMM, MEM], 0x0000, Encoding::Immediate),
    row("or", B, BYTE, &[IMM, CCR], 0x003C, Encoding::ImmediateOnly),
    row("or", W, WORD, &[IMM, SR], 0x007C, Encoding::ImmediateOnly),
    row("ori", BWL, WORD, &[IMM, DALT], 0x0000, Encoding::Immediate),
    row("ori", B, BYTE, &[IMM, CCR], 0x003C, Encoding::ImmediateOnly),
    row("ori", W, WORD, &[IMM, SR], 0x007C, Encoding::ImmediateOnly),
    row("eor", BWL, WORD, &[DN, DALT], 0xB100, Encoding::FromReg),
    row("eor", BWL, WORD, &[IMM, DALT], 0x0A00, Encoding::Immediate),
    row("eor", B, BYTE, &[IMM, CCR], 0x0A3C, Encoding::ImmediateOnly),
    row("eor", W, WORD, &[IMM, SR], 0x0A7C, Encoding::ImmediateOnly),
    row("eori", BWL, WORD, &[IMM, DALT], 0x0A00, Encoding::Immediate),
    row(
        "eori",
        B,
        BYTE,
        &[IMM, CCR],
        0x0A3C,
        Encoding::ImmediateOnly,
    ),
    row("eori", W, WORD, &[IMM, SR], 0x0A7C, Encoding::ImmediateOnly),
    row("abcd", B, BYTE, &[DN, DN], 0xC100, Encoding::RegOrPredec),
    row("abcd", B, BYTE, &[PRE, PRE], 0xC100, Encoding::RegOrPredec),
    row("sbcd", B, BYTE, &[DN, DN], 0x8100, Encoding::RegOrPredec),
    row("sbcd", B, BYTE, &[PRE, PRE], 0x8100, Encoding::RegOrPredec),
    row("nbcd", B, BYTE, &[DALT], 0x4800, Encoding::Ea0),
    row("tas", B, BYTE, &[DALT], 0x4AC0, Encoding::Ea0),
    sized("clr", 0x4200),
    sized("neg", 0x4400),
    sized("negx", 0x4000),
    sized("not", 0x4600),
    sized("tst", 0x4A00),
    row("tst", WL, WORD, &[AN], 0x4A00, Encoding::SizeEa0).on(UP020),
    row(
        "tst",
        BWL,
        WORD,
        &[EaSet(PC_MODES.0 | IMM.0)],
        0x4A00,
        Encoding::SizeEa0,
    )
    .on(UP020),
    word_to_reg("chk", 0x4180),
    row("chk", L, WORD, &[DATA, DN], 0x4100, Encoding::RegEa).on(UP020),
    word_to_reg("divu", 0x80C0),
    row("divu", L, WORD, &[DATA, DN], 0x4C40, Encoding::MulDivLong).on(UP020),
    row("divu", L, LONG, &[DATA, PAIR], 0x4C40, Encoding::MulDivLong)
        .on(UP020)
        .ext(0x0400),
    word_to_reg("divs", 0x81C0),
    row("divs", L, WORD, &[DATA, DN], 0x4C40, Encoding::MulDivLong)
        .on(UP020)
        .ext(0x0800),
    row("divs", L, LONG, &[DATA, PAIR], 0x4C40, Encoding::MulDivLong)
        .on(UP020)
        .ext(0x0C00),
    row(
        "divul",
        L,
        LONG,
        &[DATA, PAIR],
        0x4C40,
        Encoding::MulDivLong,
    )
    .on(UP020),
    row(
        "divsl",
        L,
        LONG,
        &[DATA, PAIR],
        0x4C40,
        Encoding::MulDivLong,
    )
    .on(UP020)
    .ext(0x0800),
    word_to_reg("mulu", 0xC0C0),
    row("mulu", L, WORD, &[DATA, DN], 0x4C00, Encoding::MulDivLong).on(UP020),
    row("mulu", L, LONG, &[DATA, PAIR], 0x4C00, Encoding::MulDivLong)
        .on(UP020)
        .ext(0x0400),
    word_to_reg("muls", 0xC1C0),
    row("muls", L, WORD, &[DATA, DN], 0x4C00, Encoding::MulDivLong)
        .on(UP020)
        .ext(0x0800),
    row("muls", L, LONG, &[DATA, PAIR], 0x4C00, Encoding::MulDivLong)
        .on(UP020)
        .ext(0x0C00),
    row("extb", L, LONG, &[DN], 0x49C0, Encoding::Reg0).on(UP020),
    row("chk2", BWL, WORD, &[CTRL, RN], 0x00C0, Encoding::Chk2)
        .on(UP020)
        .ext(0x0800),
    row("cmp2", BWL, WORD, &[CTRL, RN], 0x00C0, Encoding::Chk2).on(UP020),
    row("cas", BWL, WORD, &[DN, DN, MEM], 0x08C0, Encoding::Cas).on(UP020),
    row(
        "cas2",
        WL,
        WORD,
        &[PAIR, PAIR, EaSet::INDIRECT_PAIR],
        0x08FC,
        Encoding::Cas2,
    )
    .on(UP020),
    row(
        "pack",
        &[],
        None,
        &[DN, DN, IMM],
        0x8140,
        Encoding::PackUnpack,
    )
    .on(UP020),
    row(
        "pack",
        &[],
        None,
        &[PRE, PRE, IMM],
        0x8140,
        Encoding::PackUnpack,
    )
    .on(UP020),
    row(
        "unpk",
        &[],
        None,
        &[DN, DN, IMM],
        0x8180,
        Encoding::PackUnpack,
    )
    .on(UP020),
    row(
        "unpk",
        &[],
        None,
        &[PRE, PRE, IMM],
        0x8180,
        Encoding::PackUnpack,
    )
    .on(UP020),
    row("bftst", &[], None, &[BF_READ], 0xE8C0, Encoding::BitField).on(UP020),
    row(
        "bfextu",
        &[],
        None,
        &[BF_READ, DN],
        0xE9C0,
        Encoding::BitFieldToReg,
    )
    .on(UP020),
    row("bfchg", &[], None, &[BF_WRITE], 0xEAC0, Encoding::BitField).on(UP020),
    row(
        "bfexts",
        &[],
        None,
        &[BF_READ, DN],
        0xEBC0,
        Encoding::BitFieldToReg,
    )
    .on(UP020),
    row("bfclr", &[], None, &[BF_WRITE], 0xECC0, Encoding::BitField).on(UP020),
    row(
        "bfffo",
        &[],
        None,
        &[BF_READ, DN],
        0xEDC0,
        Encoding::BitFieldToReg,
    )
    .on(UP020),
    row("bfset", &[], None, &[BF_WRITE], 0xEEC0, Encoding::BitField).on(UP020),
    row(
        "bfins",
        &[],
        None,
        &[DN, BF_WRITE],
        0xEFC0,
        Encoding::BitFieldFromReg,
    )
    .on(UP020),
    row("callm", &[], None, &[IMM, CTRL], 0x06C0, Encoding::Callm).on(ONLY020),
    row("rtm", &[], None, &[RN], 0x06C0, Encoding::Ea0).on(ONLY020),
    row("rtd", &[], None, &[IMM], 0x4E74, Encoding::Stop).on(UP010),
    row("movec", L, LONG, &[CREG, RN], 0x4E7A, Encoding::MovecFrom).on(UP010),
    row("movec", L, LONG, &[RN, CREG], 0x4E7B, Encoding::MovecTo).on(UP010),
    row("moves", BWL, WORD, &[RN, MEM], 0x0E00, Encoding::MovesStore)
        .on(UP010)
        .ext(0x0800),
    row("moves", BWL, WORD, &[MEM, RN], 0x0E00, Encoding::MovesLoad).on(UP010),
    row("bkpt", &[], None, &[IMM], 0x4848, Encoding::Bkpt).on(UP010),
    row(
        "move16",
        &[],
        None,
        &[POST, POST],
        0xF620,
        Encoding::Move16Registers,
    )
    .on(UP040)
    .ext(0x8000),
    row(
        "move16",
        &[],
        None,
        &[POST, ABSL],
        0xF600,
        Encoding::Move16To,
    )
    .on(UP040),
    row(
        "move16",
        &[],
        None,
        &[ABSL, POST],
        0xF608,
        Encoding::Move16From,
    )
    .on(UP040),
    row(
        "move16",
        &[],
        None,
        &[IND, ABSL],
        0xF610,
        Encoding::Move16To,
    )
    .on(UP040),
    row(
        "move16",
        &[],
        None,
        &[ABSL, IND],
        0xF618,
        Encoding::Move16From,
    )
    .on(UP040),
    row("cinvl", &[], None, &[CACHE, IND], 0xF408, Encoding::CacheOp).on(UP040),
    row("cinvp", &[], None, &[CACHE, IND], 0xF410, Encoding::CacheOp).on(UP040),
    row("cinva", &[], None, &[CACHE], 0xF418, Encoding::CacheOp).on(UP040),
    row(
        "cpushl",
        &[],
        None,
        &[CACHE, IND],
        0xF428,
        Encoding::CacheOp,
    )
    .on(UP040),
    row(
        "cpushp",
        &[],
        None,
        &[CACHE, IND],
        0xF430,
        Encoding::CacheOp,
    )
    .on(UP040),
    row("cpusha", &[], None, &[CACHE], 0xF438, Encoding::CacheOp).on(UP040),
    row("lpstop", W, WORD, &[IMM], 0xF800, Encoding::Lpstop)
        .on(ONLY060)
        .ext(0x01C0),
    row("ext", W, WORD, &[DN], 0x4880, Encoding::Reg0),
    row("ext", L, WORD, &[DN], 0x48C0, Encoding::Reg0),
    row("swap", W, WORD, &[DN], 0x4840, Encoding::Reg0),
    row("unlk", &[], None, &[AN], 0x4E58, Encoding::Reg0),
    row("link", W, WORD, &[AN, IMM], 0x4E50, Encoding::Link),
    row("link", L, WORD, &[AN, IMM], 0x4808, Encoding::Link).on(UP020),
    row("trap", &[], None, &[IMM], 0x4E40, Encoding::Trap),
    row("stop", &[], None, &[IMM], 0x4E72, Encoding::Stop),
    row("exg", L, LONG, &[DN, DN], 0xC140, Encoding::Exchange),
    row("exg", L, LONG, &[AN, AN], 0xC148, Encoding::Exchange),
    row("exg", L, LONG, &[DN, AN], 0xC188, Encoding::Exchange),
    row("exg", L, LONG, &[AN, DN], 0xC188, Encoding::Exchange),
    // Bit operations are long on registers and byte on memory.
    row("btst", L, LONG, &[DN, DN], 0x0100, Encoding::BitReg),
    row(
        "btst",
        B,
        BYTE,
        &[DN, EaSet(DATA.0 & !DN.0)],
        0x0100,
        Encoding::BitReg,
    ),
    row("btst", L, LONG, &[IMM, DN], 0x0800, Encoding::BitImmediate),
    row(
        "btst",
        B,
        BYTE,
        &[IMM, EaSet(CTRL.0 | POST.0 | PRE.0)],
        0x0800,
        Encoding::BitImmediate,
    ),
    row("bchg", L, LONG, &[DN, DN], 0x0140, Encoding::BitReg),
    row("bchg", B, BYTE, &[DN, MEM], 0x0140, Encoding::BitReg),
    row("bchg", L, LONG, &[IMM, DN], 0x0840, Encoding::BitImmediate),
    row("bchg", B, BYTE, &[IMM, MEM], 0x0840, Encoding::BitImmediate),
    row("bclr", L, LONG, &[DN, DN], 0x0180, Encoding::BitReg),
    row("bclr", B, BYTE, &[DN, MEM], 0x0180, Encoding::BitReg),
    row("bclr", L, LONG, &[IMM, DN], 0x0880, Encoding::BitImmediate),
    row("bclr", B, BYTE, &[IMM, MEM], 0x0880, Encoding::BitImmediate),
    row("bset", L, LONG, &[DN, DN], 0x01C0, Encoding::BitReg),
    row("bset", B, BYTE, &[DN, MEM], 0x01C0, Encoding::BitReg),
    row("bset", L, LONG, &[IMM, DN], 0x08C0, Encoding::BitImmediate),
    row("bset", B, BYTE, &[IMM, MEM], 0x08C0, Encoding::BitImmediate),
    ASL[0],
    ASL[1],
    ASL[2],
    ASR[0],
    ASR[1],
    ASR[2],
    LSL[0],
    LSL[1],
    LSL[2],
    LSR[0],
    LSR[1],
    LSR[2],
    ROXL[0],
    ROXL[1],
    ROXL[2],
    ROXR[0],
    ROXR[1],
    ROXR[2],
    ROL[0],
    ROL[1],
    ROL[2],
    ROR[0],
    ROR[1],
    ROR[2],
    branch("bra", 0x6000),
    long_branch("bra", 0x6000),
    branch("bsr", 0x6100),
    long_branch("bsr", 0x6100),
    branch("bhi", 0x6200),
    long_branch("bhi", 0x6200),
    branch("bls", 0x6300),
    long_branch("bls", 0x6300),
    branch("bcc", 0x6400),
    long_branch("bcc", 0x6400),
    branch("bhs", 0x6400),
    long_branch("bhs", 0x6400),
    branch("bcs", 0x6500),
    long_branch("bcs", 0x6500),
    branch("blo", 0x6500),
    long_branch("blo", 0x6500),
    branch("bne", 0x6600),
    long_branch("bne", 0x6600),
    branch("beq", 0x6700),
    long_branch("beq", 0x6700),
    branch("bvc", 0x6800),
    long_branch("bvc", 0x6800),
    branch("bvs", 0x6900),
    long_branch("bvs", 0x6900),
    branch("bpl", 0x6A00),
    long_branch("bpl", 0x6A00),
    branch("bmi", 0x6B00),
    long_branch("bmi", 0x6B00),
    branch("bge", 0x6C00),
    long_branch("bge", 0x6C00),
    branch("blt", 0x6D00),
    long_branch("blt", 0x6D00),
    branch("bgt", 0x6E00),
    long_branch("bgt", 0x6E00),
    branch("ble", 0x6F00),
    long_branch("ble", 0x6F00),
    row("trapt", &[], None, &[], 0x50FC, Encoding::Fixed).on(UP020),
    row("trapt", W, WORD, &[IMM], 0x50FA, Encoding::ImmediateOnly).on(UP020),
    row("trapt", L, WORD, &[IMM], 0x50FB, Encoding::ImmediateOnly).on(UP020),
    row("trapf", &[], None, &[], 0x51FC, Encoding::Fixed).on(UP020),
    row("trapf", W, WORD, &[IMM], 0x51FA, Encoding::ImmediateOnly).on(UP020),
    row("trapf", L, WORD, &[IMM], 0x51FB, Encoding::ImmediateOnly).on(UP020),
    row("traphi", &[], None, &[], 0x52FC, Encoding::Fixed).on(UP020),
    row("traphi", W, WORD, &[IMM], 0x52FA, Encoding::ImmediateOnly).on(UP020),
    row("traphi", L, WORD, &[IMM], 0x52FB, Encoding::ImmediateOnly).on(UP020),
    row("trapls", &[], None, &[], 0x53FC, Encoding::Fixed).on(UP020),
    row("trapls", W, WORD, &[IMM], 0x53FA, Encoding::ImmediateOnly).on(UP020),
    row("trapls", L, WORD, &[IMM], 0x53FB, Encoding::ImmediateOnly).on(UP020),
    row("trapcc", &[], None, &[], 0x54FC, Encoding::Fixed).on(UP020),
    row("trapcc", W, WORD, &[IMM], 0x54FA, Encoding::ImmediateOnly).on(UP020),
    row("trapcc", L, WORD, &[IMM], 0x54FB, Encoding::ImmediateOnly).on(UP020),
    row("traphs", &[], None, &[], 0x54FC, Encoding::Fixed).on(UP020),
    row("traphs", W, WORD, &[IMM], 0x54FA, Encoding::ImmediateOnly).on(UP020),
    row("traphs", L, WORD, &[IMM], 0x54FB, Encoding::ImmediateOnly).on(UP020),
    row("trapcs", &[], None, &[], 0x55FC, Encoding::Fixed).on(UP020),
    row("trapcs", W, WORD, &[IMM], 0x55FA, Encoding::ImmediateOnly).on(UP020),
    row("trapcs", L, WORD, &[IMM], 0x55FB, Encoding::ImmediateOnly).on(UP020),
    row("traplo", &[], None, &[], 0x55FC, Encoding::Fixed).on(UP020),
    row("traplo", W, WORD, &[IMM], 0x55FA, Encoding::ImmediateOnly).on(UP020),
    row("traplo", L, WORD, &[IMM], 0x55FB, Encoding::ImmediateOnly).on(UP020),
    row("trapne", &[], None, &[], 0x56FC, Encoding::Fixed).on(UP020),
    row("trapne", W, WORD, &[IMM], 0x56FA, Encoding::ImmediateOnly).on(UP020),
    row("trapne", L, WORD, &[IMM], 0x56FB, Encoding::ImmediateOnly).on(UP020),
    row("trapeq", &[], None, &[], 0x57FC, Encoding::Fixed).on(UP020),
    row("trapeq", W, WORD, &[IMM], 0x57FA, Encoding::ImmediateOnly).on(UP020),
    row("trapeq", L, WORD, &[IMM], 0x57FB, Encoding::ImmediateOnly).on(UP020),
    row("trapvc", &[], None, &[], 0x58FC, Encoding::Fixed).on(UP020),
    row("trapvc", W, WORD, &[IMM], 0x58FA, Encoding::ImmediateOnly).on(UP020),
    row("trapvc", L, WORD, &[IMM], 0x58FB, Encoding::ImmediateOnly).on(UP020),
    row("trapvs", &[], None, &[], 0x59FC, Encoding::Fixed).on(UP020),
    row("trapvs", W, WORD, &[IMM], 0x59FA, Encoding::ImmediateOnly).on(UP020),
    row("trapvs", L, WORD, &[IMM], 0x59FB, Encoding::ImmediateOnly).on(UP020),
    row("trappl", &[], None, &[], 0x5AFC, Encoding::Fixed).on(UP020),
    row("trappl", W, WORD, &[IMM], 0x5AFA, Encoding::ImmediateOnly).on(UP020),
    row("trappl", L, WORD, &[IMM], 0x5AFB, Encoding::ImmediateOnly).on(UP020),
    row("trapmi", &[], None, &[], 0x5BFC, Encoding::Fixed).on(UP020),
    row("trapmi", W, WORD, &[IMM], 0x5BFA, Encoding::ImmediateOnly).on(UP020),
    row("trapmi", L, WORD, &[IMM], 0x5BFB, Encoding::ImmediateOnly).on(UP020),
    row("trapge", &[], None, &[], 0x5CFC, Encoding::Fixed).on(UP020),
    row("trapge", W, WORD, &[IMM], 0x5CFA, Encoding::ImmediateOnly).on(UP020),
    row("trapge", L, WORD, &[IMM], 0x5CFB, Encoding::ImmediateOnly).on(UP020),
    row("traplt", &[], None, &[], 0x5DFC, Encoding::Fixed).on(UP020),
    row("traplt", W, WORD, &[IMM], 0x5DFA, Encoding::ImmediateOnly).on(UP020),
    row("traplt", L, WORD, &[IMM], 0x5DFB, Encoding::ImmediateOnly).on(UP020),
    row("trapgt", &[], None, &[], 0x5EFC, Encoding::Fixed).on(UP020),
    row("trapgt", W, WORD, &[IMM], 0x5EFA, Encoding::ImmediateOnly).on(UP020),
    row("trapgt", L, WORD, &[IMM], 0x5EFB, Encoding::ImmediateOnly).on(UP020),
    row("traple", &[], None, &[], 0x5FFC, Encoding::Fixed).on(UP020),
    row("traple", W, WORD, &[IMM], 0x5FFA, Encoding::ImmediateOnly).on(UP020),
    row("traple", L, WORD, &[IMM], 0x5FFB, Encoding::ImmediateOnly).on(UP020),
    set("st", 0),
    set("sf", 1),
    set("shi", 2),
    set("sls", 3),
    set("scc", 4),
    set("shs", 4),
    set("scs", 5),
    set("slo", 5),
    set("sne", 6),
    set("seq", 7),
    set("svc", 8),
    set("svs", 9),
    set("spl", 10),
    set("smi", 11),
    set("sge", 12),
    set("slt", 13),
    set("sgt", 14),
    set("sle", 15),
    decrement("dbt", 0),
    decrement("dbf", 1),
    decrement("dbra", 1),
    decrement("dbhi", 2),
    decrement("dbls", 3),
    decrement("dbcc", 4),
    decrement("dbhs", 4),
    decrement("dbcs", 5),
    decrement("dblo", 5),
    decrement("dbne", 6),
    decrement("dbeq", 7),
    decrement("dbvc", 8),
    decrement("dbvs", 9),
    decrement("dbpl", 10),
    decrement("dbmi", 11),
    decrement("dbge", 12),
    decrement("dblt", 13),
    decrement("dbgt", 14),
    decrement("dble", 15),
];

/// A control register for `movec`: name, code and the CPUs that have it.
pub const CONTROL_REGISTERS: &[(&str, u16, CpuSet)] = &[
    ("sfc", 0x000, UP010),
    ("dfc", 0x001, UP010),
    ("usp", 0x800, UP010),
    ("vbr", 0x801, UP010),
    ("cacr", 0x002, UP020),
    ("caar", 0x802, CpuSet(0x0C)),
    ("msp", 0x803, CpuSet(0x1C)),
    ("isp", 0x804, CpuSet(0x1C)),
    ("tc", 0x003, UP040),
    ("itt0", 0x004, UP040),
    ("itt1", 0x005, UP040),
    ("dtt0", 0x006, UP040),
    ("dtt1", 0x007, UP040),
    ("iacr0", 0x004, UP040),
    ("iacr1", 0x005, UP040),
    ("dacr0", 0x006, UP040),
    ("dacr1", 0x007, UP040),
    ("mmusr", 0x805, CpuSet(0x10)),
    ("urp", 0x806, UP040),
    ("srp", 0x807, UP040),
    ("buscr", 0x008, ONLY060),
    ("pcr", 0x808, ONLY060),
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
        assert_eq!(rows_for("lea").map(<[InsnRow]>::len), Some(1));
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
