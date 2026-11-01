//! Shared types.

/// Operation or data size as written in source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    /// `.b`
    Byte,
    /// `.w`
    Word,
    /// `.l`
    Long,
    /// `.s`: the short form of a branch, or single precision on an FPU operand
    /// or in data. The parser turns it into [`Size::Single`] where that applies.
    Short,
    /// `.s` as single precision.
    Single,
    /// `.d`
    Double,
    /// `.x`
    Extended,
    /// `.p`
    Packed,
    /// `.q`
    Quad,
}

impl Size {
    /// Width of a data element in bytes. `.s` counts as a byte until the parser
    /// has decided that it means single precision.
    pub fn bytes(self) -> u32 {
        match self {
            Size::Byte | Size::Short => 1,
            Size::Word => 2,
            Size::Long | Size::Single => 4,
            Size::Double | Size::Quad => 8,
            Size::Extended | Size::Packed => 12,
        }
    }

    pub(crate) fn from_suffix(name: &str) -> Option<Size> {
        match name.to_ascii_lowercase().as_str() {
            "b" => Some(Size::Byte),
            "w" => Some(Size::Word),
            "l" => Some(Size::Long),
            "s" => Some(Size::Short),
            "d" => Some(Size::Double),
            "x" => Some(Size::Extended),
            "p" => Some(Size::Packed),
            "q" => Some(Size::Quad),
            _ => None,
        }
    }
}

/// Target CPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cpu {
    /// MC68000
    #[default]
    M68000,
    /// MC68010
    M68010,
    /// MC68020
    M68020,
    /// MC68030
    M68030,
    /// MC68040
    M68040,
    /// MC68060
    M68060,
}

impl Cpu {
    /// The CPU with this number, such as `68020`.
    pub fn from_number(number: u32) -> Option<Cpu> {
        match number {
            68000 => Some(Cpu::M68000),
            68010 => Some(Cpu::M68010),
            68020 => Some(Cpu::M68020),
            68030 => Some(Cpu::M68030),
            68040 => Some(Cpu::M68040),
            68060 => Some(Cpu::M68060),
            _ => None,
        }
    }

    /// Mask with only this CPU set.
    pub const fn mask(self) -> CpuSet {
        CpuSet(match self {
            Cpu::M68000 => 1,
            Cpu::M68010 => 2,
            Cpu::M68020 => 4,
            Cpu::M68030 => 8,
            Cpu::M68040 => 16,
            Cpu::M68060 => 32,
        })
    }
}

/// A set of CPUs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuSet(pub u8);

impl CpuSet {
    /// Every supported CPU.
    pub const ALL: CpuSet = CpuSet(0x3F);

    /// True if `cpu` is in the set.
    pub const fn contains(self, cpu: Cpu) -> bool {
        self.0 & cpu.mask().0 != 0
    }
}

/// Kind of a section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionKind {
    /// Code.
    Code,
    /// Initialised data.
    Data,
    /// Uninitialised data.
    Bss,
}

/// Amiga memory requirement of a section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemKind {
    /// No requirement.
    Any,
    /// Chip memory.
    Chip,
    /// Fast memory.
    Fast,
}
