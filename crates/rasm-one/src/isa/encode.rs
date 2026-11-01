//! Turns matched instructions into machine code.
//!
//! Displacements can grow on 68020 and later, so an instruction's length can
//! depend on operand values. `plan_extension` makes that choice for both layout
//! and emission. Writers always emit the full planned width, even after an error,
//! so layout and emission can't disagree.

use crate::error::Diagnostic;
use crate::expr::{eval, Base, EvalError, Expr, SymId, Value};
use crate::isa::table::{Encoding, InsnRow};
use crate::isa::{
    BitFieldArg, FullBase, FullDisp, FullMode, IndexReg, Indirection, Mode, Operand, Width,
};
use crate::source::Span;
use crate::types::{SectionKind, Size};
use crate::{Reloc, RelocKind, RelocTarget};

/// An instruction matched to its table row.
#[derive(Debug, Clone)]
pub(crate) struct Instruction {
    pub row: &'static InsnRow,
    pub size: Option<Size>,
    pub operands: Vec<Operand>,
    /// True on 68020 and later, where displacements can grow.
    pub wide: bool,
    /// True if labels are final addresses, as in a raw binary.
    pub fixed_addresses: bool,
}

/// How to find an operand's values, for choosing extension forms.
pub(crate) struct Sizer<'a> {
    pub section: usize,
    pub origin: Option<i64>,
    /// Section position of the instruction.
    pub start: i64,
    pub lookup: &'a dyn Fn(SymId) -> Option<Value>,
}

impl Sizer<'_> {
    fn address_of(&self, offset: i64) -> Value {
        match self.origin {
            Some(origin) => Value::abs(origin + offset),
            None => Value {
                base: Base::Section(self.section),
                offset,
            },
        }
    }

    /// The value of an expression; anything not known yet counts as zero.
    fn value(&self, expr: &Expr) -> Value {
        eval(expr, self.address_of(self.start), self.lookup).unwrap_or(Value::abs(0))
    }
}

/// What a displacement's value means for the choice of its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DispKind {
    /// A number that is known now.
    Number(i64),
    /// A section address that a loader will relocate, needing 32 bits.
    Relocated,
    /// Something resolved later whose size the instruction doesn't dictate.
    Deferred,
}

fn fits_byte(number: i64) -> bool {
    (-0x80..=0x7F).contains(&number)
}

fn fits_word(number: i64) -> bool {
    (-0x8000..=0x7FFF).contains(&number)
}

/// Classifies a displacement from an address register, or none at all.
fn plain_disp_kind(value: Value, fixed: bool) -> DispKind {
    match value.base {
        Base::Absolute => DispKind::Number(sign_extend_32(value.offset)),
        Base::Section(_) if fixed => DispKind::Number(sign_extend_32(value.offset)),
        Base::Section(_) => DispKind::Relocated,
        Base::External(_) => DispKind::Deferred,
    }
}

/// Classifies a PC-relative displacement, whose PC is at section position `pc`.
fn pc_disp_kind(value: Value, pc: i64, sizer: &Sizer) -> DispKind {
    match value.base {
        Base::Absolute => DispKind::Number(value.offset - sizer.address_of(pc).offset),
        Base::Section(section) if section == sizer.section => {
            DispKind::Number(value.offset - sizer.address_of(pc).offset)
        }
        _ => DispKind::Deferred,
    }
}

/// The widths chosen for a full-format extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FullPlan {
    base: Option<Width>,
    outer: Option<Width>,
}

impl FullPlan {
    fn bytes(self) -> i64 {
        let width_bytes = |width: Option<Width>| match width {
            None => 0,
            Some(Width::Word) => 2,
            Some(Width::Long) => 4,
        };
        2 + width_bytes(self.base) + width_bytes(self.outer)
    }
}

/// How one operand's extension words will be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExtensionPlan {
    /// The brief forms and everything else of a fixed size.
    Fixed(i64),
    Full(FullPlan),
}

impl ExtensionPlan {
    fn bytes(self) -> i64 {
        match self {
            ExtensionPlan::Fixed(bytes) => bytes,
            ExtensionPlan::Full(plan) => plan.bytes(),
        }
    }
}

/// The width of a displacement that would not fit the brief form.
fn grown_width(number: i64) -> Width {
    if fits_word(number) {
        Width::Word
    } else {
        Width::Long
    }
}

/// The width of a full-format displacement written without a size.
fn auto_width(kind: DispKind, deferred: Width) -> Width {
    match kind {
        DispKind::Number(number) => grown_width(number),
        DispKind::Relocated => Width::Long,
        DispKind::Deferred => deferred,
    }
}

fn full_plan(full: &FullMode, pc: i64, insn: &Instruction, sizer: &Sizer) -> FullPlan {
    let is_pc = full.base == FullBase::Pc;
    let base = full.base_disp.as_ref().map(|disp| {
        disp.width.unwrap_or_else(|| {
            let value = sizer.value(&disp.expr);
            if is_pc {
                auto_width(pc_disp_kind(value, pc, sizer), Width::Word)
            } else {
                auto_width(plain_disp_kind(value, insn.fixed_addresses), Width::Long)
            }
        })
    });
    let outer = full.outer_disp.as_ref().map(|disp| {
        disp.width.unwrap_or_else(|| {
            let value = sizer.value(&disp.expr);
            auto_width(plain_disp_kind(value, insn.fixed_addresses), Width::Long)
        })
    });
    FullPlan { base, outer }
}

/// A brief mode that grew, as a full-format plan with only a base displacement.
fn grown_plan(width: Width) -> ExtensionPlan {
    ExtensionPlan::Full(FullPlan {
        base: Some(width),
        outer: None,
    })
}

/// Decides how to write an operand's extension, which starts at section
/// position `pc`. Before the 68020 only the brief forms exist, and a value that
/// doesn't fit is an error during emission.
fn plan_extension(
    mode: &Mode,
    size: Size,
    pc: i64,
    insn: &Instruction,
    sizer: &Sizer,
) -> ExtensionPlan {
    let fixed = insn.fixed_addresses;
    match mode {
        Mode::Disp(expr, _) if insn.wide => match plain_disp_kind(sizer.value(expr), fixed) {
            DispKind::Number(number) if !fits_word(number) => grown_plan(Width::Long),
            DispKind::Relocated => grown_plan(Width::Long),
            _ => ExtensionPlan::Fixed(2),
        },
        Mode::Index(expr, _, _) if insn.wide => match plain_disp_kind(sizer.value(expr), fixed) {
            DispKind::Number(number) if !fits_byte(number) => grown_plan(grown_width(number)),
            DispKind::Relocated => grown_plan(Width::Long),
            _ => ExtensionPlan::Fixed(2),
        },
        Mode::PcDisp(expr) if insn.wide => match pc_disp_kind(sizer.value(expr), pc, sizer) {
            DispKind::Number(number) if !fits_word(number) => grown_plan(Width::Long),
            _ => ExtensionPlan::Fixed(2),
        },
        Mode::PcIndex(expr, _) if insn.wide => match pc_disp_kind(sizer.value(expr), pc, sizer) {
            DispKind::Number(number) if !fits_byte(number) => grown_plan(grown_width(number)),
            _ => ExtensionPlan::Fixed(2),
        },
        Mode::Full(full) => ExtensionPlan::Full(full_plan(full, pc, insn, sizer)),
        Mode::BitField { ea, .. } => plan_extension(ea, size, pc, insn, sizer),
        Mode::Disp(..)
        | Mode::DispWord(..)
        | Mode::Index(..)
        | Mode::AbsWord(_)
        | Mode::PcDisp(_)
        | Mode::PcDispWord(_)
        | Mode::PcIndex(..) => ExtensionPlan::Fixed(2),
        Mode::AbsLong(_) => ExtensionPlan::Fixed(4),
        Mode::Immediate(_) if size == Size::Long => ExtensionPlan::Fixed(4),
        Mode::Immediate(_) => ExtensionPlan::Fixed(2),
        _ => ExtensionPlan::Fixed(0),
    }
}

impl Instruction {
    /// The operands that have extension words in order, after any leading
    /// `lead` bytes of fixed extension, and the size their immediates use.
    fn extension_layout(&self) -> (i64, &[Operand]) {
        let operands = self.operands.as_slice();
        match self.row.encoding {
            Encoding::Quick | Encoding::Shift | Encoding::Trap | Encoding::Moveq => {
                (0, operands.get(1..).unwrap_or(&[]))
            }
            Encoding::BitImmediate | Encoding::Callm | Encoding::MovemStore => {
                (2, operands.get(1..).unwrap_or(&[]))
            }
            Encoding::MovesStore | Encoding::BitFieldFromReg => {
                (2, operands.get(1..).unwrap_or(&[]))
            }
            Encoding::MovemLoad
            | Encoding::MulDivLong
            | Encoding::Chk2
            | Encoding::MovesLoad
            | Encoding::BitField
            | Encoding::BitFieldToReg => (2, operands.get(..1).unwrap_or(&[])),
            Encoding::Cas => (2, operands.get(2..).unwrap_or(&[])),
            Encoding::Cas2 => (4, &[]),
            Encoding::MovecFrom | Encoding::MovecTo | Encoding::Move16Registers => (2, &[]),
            Encoding::MovepLoad | Encoding::MovepStore | Encoding::PackUnpack => (2, &[]),
            Encoding::Lpstop => (4, &[]),
            Encoding::Bkpt | Encoding::Stop | Encoding::CacheOp => (
                if self.row.encoding == Encoding::Stop {
                    2
                } else {
                    0
                },
                &[],
            ),
            _ => (0, operands),
        }
    }

    /// Length in bytes, with symbols as they are known now.
    pub fn len(&self, sizer: &Sizer) -> i64 {
        let size = self.size.unwrap_or(Size::Word);
        match self.row.encoding {
            Encoding::Branch => {
                return match self.size {
                    Some(Size::Short | Size::Byte) => 2,
                    _ => 4,
                };
            }
            Encoding::BranchLong => return 6,
            Encoding::DecrementBranch => return 4,
            _ => {}
        }
        let (lead, operands) = self.extension_layout();
        let mut position = sizer.start + 2 + lead;
        for operand in operands {
            position += plan_extension(&operand.mode, size, position, self, sizer).bytes();
        }
        position - sizer.start
    }
}

/// Everything an encoder needs from the surrounding assembly.
pub(crate) struct EmitContext<'a> {
    pub section: usize,
    /// Kind of that section.
    pub kind: SectionKind,
    /// Fixed address of that section, if it has one. Its symbols are then
    /// constants, so addresses have to include this.
    pub origin: Option<i64>,
    /// Source of the atom being emitted.
    pub span: Span,
    /// Start of the current instruction, which is what `*` means in operands.
    pub instruction_start: i64,
    pub lookup: &'a dyn Fn(SymId) -> Option<Value>,
    pub name_of: &'a dyn Fn(SymId) -> String,
    pub data: &'a mut Vec<u8>,
    pub relocs: &'a mut Vec<Reloc>,
    pub diagnostics: &'a mut Vec<Diagnostic>,
}

impl EmitContext<'_> {
    fn offset(&self) -> i64 {
        self.data.len() as i64
    }

    /// The address of a position in the section, as a value.
    pub fn address_of(&self, offset: i64) -> Value {
        match self.origin {
            Some(origin) => Value::abs(origin + offset),
            None => Value {
                base: Base::Section(self.section),
                offset,
            },
        }
    }

    /// The value of `*` in an operand.
    fn star(&self) -> Value {
        self.address_of(self.instruction_start)
    }

    fn error(&mut self, code: &'static str, message: impl Into<String>, span: Span) {
        self.diagnostics
            .push(Diagnostic::error(code, message, span));
    }

    /// Evaluates an expression whose `*` is `pc`, reporting any failure.
    pub fn evaluate(&mut self, expr: &Expr, pc: Value, span: Span) -> Option<Value> {
        match eval(expr, pc, self.lookup) {
            Ok(value) => Some(value),
            Err(EvalError::Undefined(id, sym_span)) => {
                let name = (self.name_of)(id);
                self.error(
                    "undefined-symbol",
                    format!("undefined symbol {name}"),
                    sym_span,
                );
                None
            }
            Err(EvalError::DivideByZero) => {
                self.error("divide-by-zero", "division by zero", span);
                None
            }
            Err(EvalError::NotAbsolute) => {
                self.error(
                    "not-absolute",
                    "expression cannot be represented as a constant or a relocation",
                    span,
                );
                None
            }
        }
    }

    fn put_word(&mut self, value: u16) {
        self.put(u32::from(value), 2);
    }

    fn put(&mut self, value: u32, bytes: u32) {
        let all = value.to_be_bytes();
        self.data.extend_from_slice(&all[4 - bytes as usize..]);
    }

    /// What a value must be relocated against, if anything.
    fn reloc_target(&self, base: Base) -> Option<RelocTarget> {
        match base {
            Base::Absolute => None,
            Base::Section(section) => Some(RelocTarget::Section(section)),
            Base::External(id) => Some(RelocTarget::External((self.name_of)(id))),
        }
    }

    /// Records a relocation for `value` if it needs one, then writes it.
    /// Relocations always cover the field about to be written.
    pub fn put_value(&mut self, value: Value, bytes: u32) {
        if let Some(target) = self.reloc_target(value.base) {
            self.relocs.push(Reloc {
                offset: self.data.len() as u32,
                size: bytes as u8,
                target,
                kind: RelocKind::Absolute,
                span: self.span,
            });
        }
        self.put(value.offset as u32, bytes);
    }

    /// Writes zeros in place of a value that failed to evaluate.
    pub fn put_zero(&mut self, bytes: u32) {
        self.put(0, bytes);
    }
}

/// Maps the 32-bit view of a negative number to its signed value, so that
/// `$ffff8000` works as a short address or displacement.
fn sign_extend_32(v: i64) -> i64 {
    if (0x8000_0000..=0xFFFF_FFFF).contains(&v) {
        v - (1 << 32)
    } else {
        v
    }
}

fn size_bits(size: Size) -> u16 {
    match size {
        Size::Byte | Size::Short => 0,
        Size::Word => 1 << 6,
        Size::Long => 2 << 6,
    }
}

/// The registers of a `Dn:Dn` operand.
fn pair(mode: &Mode) -> (u16, u16) {
    match mode {
        Mode::RegPair(a, b) => (u16::from(*a), u16::from(*b)),
        Mode::IndirectPair(a, b) => (u16::from(*a), u16::from(*b)),
        _ => (0, 0),
    }
}

/// A register number 0-15, with address registers from 8.
fn full_reg(mode: &Mode) -> u16 {
    match mode {
        Mode::DataReg(n) => u16::from(*n),
        Mode::AddrReg(n) => u16::from(*n) + 8,
        _ => 0,
    }
}

pub(crate) fn encode(insn: &Instruction, span: Span, ctx: &mut EmitContext) {
    ctx.instruction_start = ctx.offset();
    let resolved = with_growth_resolved(insn, &sizer_for(ctx));
    let insn = resolved.as_ref().unwrap_or(insn);
    let size = insn.size.unwrap_or(Size::Word);
    let base = insn.row.opcode;
    let op = |index: usize| &insn.operands[index];
    let reg = |index: usize| mode_reg(&insn.operands[index].mode).1;
    match insn.row.encoding {
        Encoding::Fixed => ctx.put_word(base),
        Encoding::Branch => encode_branch(insn, span, ctx),
        Encoding::Move => {
            let size_code: u16 = match size {
                Size::Byte | Size::Short => 1,
                Size::Long => 2,
                Size::Word => 3,
            };
            let (dst_mode, dst_reg) = mode_reg(&op(1).mode);
            ctx.put_word(size_code << 12 | dst_reg << 9 | dst_mode << 6 | mode_bits(&op(0).mode));
            emit_extension(op(0), size, insn, ctx);
            emit_extension(op(1), size, insn, ctx);
        }
        Encoding::Ea0 => {
            ctx.put_word(base | mode_bits(&op(0).mode));
            emit_extension(op(0), size, insn, ctx);
        }
        Encoding::Ea1 => {
            ctx.put_word(base | mode_bits(&op(1).mode));
            emit_extension(op(1), size, insn, ctx);
        }
        Encoding::SizeEa0 => {
            ctx.put_word(base | size_bits(size) | mode_bits(&op(0).mode));
            emit_extension(op(0), size, insn, ctx);
        }
        Encoding::RegEa => {
            ctx.put_word(base | reg(1) << 9 | mode_bits(&op(0).mode));
            emit_extension(op(0), size, insn, ctx);
        }
        Encoding::ToReg => {
            ctx.put_word(base | reg(1) << 9 | size_bits(size) | mode_bits(&op(0).mode));
            emit_extension(op(0), size, insn, ctx);
        }
        Encoding::FromReg => {
            ctx.put_word(base | reg(0) << 9 | size_bits(size) | mode_bits(&op(1).mode));
            emit_extension(op(1), size, insn, ctx);
        }
        Encoding::Address => {
            let opmode: u16 = if size == Size::Long { 0x1C0 } else { 0xC0 };
            ctx.put_word(base | reg(1) << 9 | opmode | mode_bits(&op(0).mode));
            emit_extension(op(0), size, insn, ctx);
        }
        Encoding::Immediate => {
            ctx.put_word(base | size_bits(size) | mode_bits(&op(1).mode));
            emit_extension(op(0), size, insn, ctx);
            emit_extension(op(1), size, insn, ctx);
        }
        Encoding::ImmediateOnly => {
            ctx.put_word(base);
            emit_extension(op(0), size, insn, ctx);
        }
        Encoding::Quick => {
            let count = opcode_constant(op(0), 1, 8, false, ctx) as u16 & 7;
            ctx.put_word(base | count << 9 | size_bits(size) | mode_bits(&op(1).mode));
            emit_extension(op(1), size, insn, ctx);
        }
        Encoding::RegOrPredec => {
            let memory = u16::from(matches!(op(0).mode, Mode::PreDec(_)));
            ctx.put_word(base | reg(1) << 9 | size_bits(size) | memory << 3 | reg(0));
        }
        Encoding::Cmpm => ctx.put_word(base | reg(1) << 9 | size_bits(size) | reg(0)),
        Encoding::Shift => {
            let (count, register) = match op(0).mode {
                Mode::DataReg(n) => (u16::from(n), 1),
                _ => (opcode_constant(op(0), 1, 8, false, ctx) as u16 & 7, 0),
            };
            ctx.put_word(base | count << 9 | size_bits(size) | register << 5 | reg(1));
        }
        Encoding::ShiftOnce => ctx.put_word(base | 1 << 9 | size_bits(size) | reg(0)),
        Encoding::BitReg => {
            ctx.put_word(base | reg(0) << 9 | mode_bits(&op(1).mode));
            emit_extension(op(1), size, insn, ctx);
        }
        Encoding::BitImmediate => {
            ctx.put_word(base | mode_bits(&op(1).mode));
            let bit = opcode_constant(op(0), 0, 255, false, ctx) as u16;
            ctx.put_word(bit);
            emit_extension(op(1), size, insn, ctx);
        }
        Encoding::DecrementBranch => {
            ctx.put_word(base | reg(0));
            emit_branch_word(op(1), span, ctx);
        }
        Encoding::Exchange => {
            // A mixed exchange always puts the data register first.
            let (x, y) = match op(0).mode {
                Mode::AddrReg(_) if matches!(op(1).mode, Mode::DataReg(_)) => (reg(1), reg(0)),
                _ => (reg(0), reg(1)),
            };
            ctx.put_word(base | x << 9 | y);
        }
        Encoding::Stop => {
            ctx.put_word(base);
            let status = opcode_constant(op(0), 0, 0xFFFF, false, ctx) as u16;
            ctx.put_word(status);
        }
        Encoding::Reg0 => ctx.put_word(base | reg(0)),
        Encoding::Reg1 => ctx.put_word(base | reg(1)),
        Encoding::Link => {
            ctx.put_word(base | reg(0));
            emit_extension(op(1), size, insn, ctx);
        }
        Encoding::BranchLong => {
            ctx.put_word(base);
            let value = branch_target(op(0), ctx);
            let pc = ctx.instruction_start + 2;
            emit_pc_field(value, pc, 4, BRANCH_RANGE, span, ctx);
        }
        Encoding::MulDivLong => {
            ctx.put_word(base | mode_bits(&op(0).mode));
            let (high, low) = match op(1).mode {
                Mode::RegPair(high, low) => (u16::from(high), u16::from(low)),
                Mode::DataReg(n) => (u16::from(n), u16::from(n)),
                _ => (0, 0),
            };
            ctx.put_word(low << 12 | insn.row.extension | high);
            emit_extension(op(0), size, insn, ctx);
        }
        Encoding::Chk2 => {
            let size_field: u16 = match size {
                Size::Byte | Size::Short => 0,
                Size::Word => 1,
                Size::Long => 2,
            };
            ctx.put_word(base | size_field << 9 | mode_bits(&op(0).mode));
            ctx.put_word(full_reg(&op(1).mode) << 12 | insn.row.extension);
            emit_extension(op(0), size, insn, ctx);
        }
        Encoding::Cas => {
            let size_field: u16 = match size {
                Size::Byte | Size::Short => 1,
                Size::Word => 2,
                Size::Long => 3,
            };
            ctx.put_word(base | size_field << 9 | mode_bits(&op(2).mode));
            ctx.put_word(reg(1) << 6 | reg(0));
            emit_extension(op(2), size, insn, ctx);
        }
        Encoding::Cas2 => {
            let size_field: u16 = if size == Size::Long { 3 } else { 2 };
            ctx.put_word(base | size_field << 9);
            let (compare1, compare2) = pair(&op(0).mode);
            let (update1, update2) = pair(&op(1).mode);
            let (address1, address2) = pair(&op(2).mode);
            ctx.put_word(address1 << 12 | update1 << 6 | compare1);
            ctx.put_word(address2 << 12 | update2 << 6 | compare2);
        }
        Encoding::PackUnpack => {
            let memory = u16::from(matches!(op(0).mode, Mode::PreDec(_)));
            ctx.put_word(base | reg(1) << 9 | memory << 3 | reg(0));
            let adjustment = opcode_constant(op(2), 0, 0xFFFF, false, ctx) as u16;
            ctx.put_word(adjustment);
        }
        Encoding::BitField => {
            ctx.put_word(base | mode_bits(&op(0).mode));
            emit_bitfield_word(op(0), 0, ctx);
            emit_extension(op(0), size, insn, ctx);
        }
        Encoding::BitFieldToReg => {
            ctx.put_word(base | mode_bits(&op(0).mode));
            emit_bitfield_word(op(0), reg(1) << 12, ctx);
            emit_extension(op(0), size, insn, ctx);
        }
        Encoding::BitFieldFromReg => {
            ctx.put_word(base | mode_bits(&op(1).mode));
            emit_bitfield_word(op(1), reg(0) << 12, ctx);
            emit_extension(op(1), size, insn, ctx);
        }
        Encoding::MovecFrom | Encoding::MovecTo => {
            ctx.put_word(base);
            let (register, control) = if insn.row.encoding == Encoding::MovecFrom {
                (op(1), op(0))
            } else {
                (op(0), op(1))
            };
            let code = match control.mode {
                Mode::ControlReg(code) => code,
                _ => 0,
            };
            ctx.put_word(full_reg(&register.mode) << 12 | code);
        }
        Encoding::MovesLoad => {
            ctx.put_word(base | size_bits(size) | mode_bits(&op(0).mode));
            ctx.put_word(full_reg(&op(1).mode) << 12 | insn.row.extension);
            emit_extension(op(0), size, insn, ctx);
        }
        Encoding::MovesStore => {
            ctx.put_word(base | size_bits(size) | mode_bits(&op(1).mode));
            ctx.put_word(full_reg(&op(0).mode) << 12 | insn.row.extension);
            emit_extension(op(1), size, insn, ctx);
        }
        Encoding::Bkpt => {
            let vector = opcode_constant(op(0), 0, 7, false, ctx) as u16;
            ctx.put_word(base | vector);
        }
        Encoding::Callm => {
            ctx.put_word(base | mode_bits(&op(1).mode));
            let count = opcode_constant(op(0), 0, 255, false, ctx) as u16;
            ctx.put_word(count);
            emit_extension(op(1), size, insn, ctx);
        }
        Encoding::Move16Registers => {
            ctx.put_word(base | reg(0));
            ctx.put_word(insn.row.extension | reg(1) << 12);
        }
        Encoding::Move16To => {
            ctx.put_word(base | reg(0));
            emit_extension(op(1), size, insn, ctx);
        }
        Encoding::Move16From => {
            ctx.put_word(base | reg(1));
            emit_extension(op(0), size, insn, ctx);
        }
        Encoding::CacheOp => {
            let cache = match op(0).mode {
                Mode::Cache(cache) => u16::from(cache),
                _ => 0,
            };
            let register = insn
                .operands
                .get(1)
                .map_or(0, |operand| mode_reg(&operand.mode).1);
            ctx.put_word(base | cache << 6 | register);
        }
        Encoding::Lpstop => {
            ctx.put_word(base);
            ctx.put_word(insn.row.extension);
            let value = opcode_constant(op(0), 0, 0xFFFF, false, ctx) as u16;
            ctx.put_word(value);
        }
        Encoding::Trap => {
            let vector = opcode_constant(op(0), 0, 15, false, ctx) as u16;
            ctx.put_word(base | vector);
        }
        Encoding::Moveq => {
            let data = sign_extend_32(opcode_constant(op(0), -128, 255, true, ctx));
            if data > 127 {
                ctx.diagnostics.push(Diagnostic::warning(
                    "moveq-unsigned",
                    format!(
                        "moveq sign-extends its operand, so {data} becomes {}",
                        data - 256
                    ),
                    op(0).span,
                ));
            }
            ctx.put_word(base | reg(1) << 9 | (data as u16 & 0xFF));
        }
        Encoding::MovepLoad => encode_movep(base | reg(1) << 9, op(0), ctx),
        Encoding::MovepStore => encode_movep(base | reg(0) << 9, op(1), ctx),
        Encoding::MovemStore => {
            ctx.put_word(base | mode_bits(&op(1).mode));
            // An immediate mask is used as is. Anything else is in register order
            // and is mapped to the reversed bit order predecrement uses.
            let literal = matches!(op(0).mode, Mode::Immediate(_));
            let predecrement = !literal && matches!(op(1).mode, Mode::PreDec(_));
            let mask = register_mask(op(0), ctx);
            ctx.put_word(if predecrement {
                mask.reverse_bits()
            } else {
                mask
            });
            emit_extension(op(1), size, insn, ctx);
        }
        Encoding::MovemLoad => {
            ctx.put_word(base | mode_bits(&op(0).mode));
            let mask = register_mask(op(1), ctx);
            ctx.put_word(mask);
            emit_extension(op(0), size, insn, ctx);
        }
    }
}

fn encode_movep(opcode: u16, memory: &Operand, ctx: &mut EmitContext) {
    let (displacement, base) = match &memory.mode {
        Mode::Disp(expr, n) | Mode::DispWord(expr, n) => (Some(expr), *n),
        Mode::Indirect(n) => (None, *n),
        // Row matching only lets these two modes through.
        _ => return,
    };
    ctx.put_word(opcode | u16::from(base));
    match displacement {
        Some(expr) => emit_short_field(expr, "displacement out of range", memory.span, ctx),
        None => ctx.put_zero(2),
    }
}

fn register_mask(op: &Operand, ctx: &mut EmitContext) -> u16 {
    match &op.mode {
        Mode::DataReg(n) => 1 << *n,
        Mode::AddrReg(n) => 1 << (*n + 8),
        Mode::RegList(mask) => *mask,
        _ => opcode_constant(op, 0, 0xFFFF, false, ctx) as u16,
    }
}

/// Evaluates a value that is encoded in a fixed field rather than as an
/// extension, which must be a constant in `min..=max`. Optionally, the 32-bit
/// view of a negative number counts as that number.
fn opcode_constant(
    op: &Operand,
    min: i64,
    max: i64,
    allow_32_bit_negative: bool,
    ctx: &mut EmitContext,
) -> i64 {
    let (Mode::Immediate(expr) | Mode::AbsLong(expr)) = &op.mode else {
        return 0;
    };
    let pc = ctx.star();
    let Some(value) = ctx.evaluate(expr, pc, op.span) else {
        return 0;
    };
    if value.base != Base::Absolute {
        ctx.error("not-absolute", "immediate must be a constant", op.span);
        return 0;
    }
    let constant = if allow_32_bit_negative {
        sign_extend_32(value.offset)
    } else {
        value.offset
    };
    if !(min..=max).contains(&constant) {
        ctx.error("out-of-range", "immediate operand out of range", op.span);
    }
    constant
}

fn mode_bits(mode: &Mode) -> u16 {
    let (mode, reg) = mode_reg(mode);
    mode << 3 | reg
}

fn mode_reg(mode: &Mode) -> (u16, u16) {
    match mode {
        Mode::DataReg(r) => (0, u16::from(*r)),
        Mode::AddrReg(r) => (1, u16::from(*r)),
        Mode::Indirect(r) => (2, u16::from(*r)),
        Mode::PostInc(r) => (3, u16::from(*r)),
        Mode::PreDec(r) => (4, u16::from(*r)),
        Mode::Disp(_, r) => (5, u16::from(*r)),
        Mode::Index(_, r, _) => (6, u16::from(*r)),
        Mode::AbsWord(_) => (7, 0),
        Mode::AbsLong(_) => (7, 1),
        Mode::PcDisp(_) => (7, 2),
        Mode::PcIndex(..) => (7, 3),
        Mode::Immediate(_) => (7, 4),
        Mode::DispWord(_, r) => (5, u16::from(*r)),
        Mode::PcDispWord(_) => (7, 2),
        Mode::Full(full) => match full.base {
            FullBase::Addr(r) | FullBase::SuppressedAddr(r) => (6, u16::from(r)),
            FullBase::None => (6, 0),
            FullBase::Pc | FullBase::SuppressedPc => (7, 3),
        },
        Mode::BitField { ea, .. } => mode_reg(ea),
        // Not effective addresses, so never placed in an EA field.
        Mode::Sr
        | Mode::Ccr
        | Mode::Usp
        | Mode::RegList(_)
        | Mode::RegPair(..)
        | Mode::IndirectPair(..)
        | Mode::ControlReg(_)
        | Mode::Cache(_) => (0, 0),
    }
}

fn index_bits(index: IndexReg) -> u16 {
    u16::from(index.reg) << 12 | u16::from(index.long) << 11 | u16::from(index.scale) << 9
}

/// The sizer for the instruction now being emitted.
fn sizer_for<'a>(ctx: &'a EmitContext) -> Sizer<'a> {
    Sizer {
        section: ctx.section,
        origin: ctx.origin,
        start: ctx.instruction_start,
        lookup: ctx.lookup,
    }
}

fn emit_extension(op: &Operand, size: Size, insn: &Instruction, ctx: &mut EmitContext) {
    let pc = ctx.star();
    let extension = ctx.offset();
    let plan = plan_extension(&op.mode, size, extension, insn, &sizer_for(ctx));
    match &op.mode {
        Mode::BitField { ea, .. } => {
            let inner = Operand {
                mode: (**ea).clone(),
                span: op.span,
            };
            emit_extension(&inner, size, insn, ctx);
        }
        Mode::Full(full) => {
            if let ExtensionPlan::Full(plan) = plan {
                emit_full(full, plan, op.span, ctx);
            }
        }
        Mode::Disp(expr, _) | Mode::DispWord(expr, _) => {
            emit_short_field(expr, "displacement out of range", op.span, ctx);
        }
        Mode::Index(expr, _, index) => {
            ctx.put(u32::from(index_bits(*index) >> 8), 1);
            match ctx.evaluate(expr, pc, op.span) {
                Some(value) => {
                    let d = sign_extend_32(value.offset);
                    if !fits_byte(d) {
                        ctx.error("out-of-range", "displacement out of range", op.span);
                    }
                    ctx.put_value(Value { offset: d, ..value }, 1);
                }
                None => ctx.put_zero(1),
            }
        }
        Mode::AbsWord(expr) => {
            emit_short_field(expr, "absolute short address out of range", op.span, ctx);
        }
        Mode::AbsLong(expr) => match ctx.evaluate(expr, pc, op.span) {
            Some(value) => ctx.put_value(value, 4),
            None => ctx.put_zero(4),
        },
        Mode::PcDisp(expr) | Mode::PcDispWord(expr) => {
            let target = ctx.evaluate(expr, pc, op.span);
            emit_pc_field(
                target,
                extension,
                2,
                "displacement out of range",
                op.span,
                ctx,
            );
        }
        Mode::PcIndex(expr, index) => {
            let target = ctx.evaluate(expr, pc, op.span);
            ctx.put(u32::from(index_bits(*index) >> 8), 1);
            emit_pc_field(
                target,
                extension,
                1,
                "displacement out of range",
                op.span,
                ctx,
            );
        }
        Mode::Immediate(expr) => emit_immediate(expr, size, op.span, ctx),
        _ => {}
    }
}

/// The full-format mode that a brief mode grows into.
fn grown_mode(expr: &Expr, base: FullBase, index: Option<IndexReg>, width: Option<Width>) -> Mode {
    Mode::Full(Box::new(FullMode {
        base,
        index,
        indirection: Indirection::None,
        base_disp: Some(FullDisp {
            expr: expr.clone(),
            width,
        }),
        outer_disp: None,
    }))
}

/// Replaces a mode that has grown with its full-format equivalent, so its mode
/// bits and extension words agree.
fn resolve_growth(mode: &Mode, plan: ExtensionPlan) -> Option<Mode> {
    let ExtensionPlan::Full(plan) = plan else {
        return None;
    };
    match mode {
        Mode::Disp(expr, r) => Some(grown_mode(expr, FullBase::Addr(*r), None, plan.base)),
        Mode::Index(expr, r, index) => Some(grown_mode(
            expr,
            FullBase::Addr(*r),
            Some(*index),
            plan.base,
        )),
        Mode::PcDisp(expr) => Some(grown_mode(expr, FullBase::Pc, None, plan.base)),
        Mode::PcIndex(expr, index) => Some(grown_mode(expr, FullBase::Pc, Some(*index), plan.base)),
        Mode::BitField { ea, offset, width } => {
            resolve_growth(ea, ExtensionPlan::Full(plan)).map(|ea| Mode::BitField {
                ea: Box::new(ea),
                offset: offset.clone(),
                width: width.clone(),
            })
        }
        _ => None,
    }
}

/// A copy of the instruction with any grown modes resolved, if there are any.
fn with_growth_resolved(insn: &Instruction, sizer: &Sizer) -> Option<Instruction> {
    if !insn.wide {
        return None;
    }
    let size = insn.size.unwrap_or(Size::Word);
    let (lead, operands) = insn.extension_layout();
    let mut position = sizer.start + 2 + lead;
    let mut resolved = None;
    for operand in operands {
        let plan = plan_extension(&operand.mode, size, position, insn, sizer);
        position += plan.bytes();
        if let Some(mode) = resolve_growth(&operand.mode, plan) {
            let copy = resolved.get_or_insert_with(|| insn.clone());
            let index = insn
                .operands
                .iter()
                .position(|candidate| std::ptr::eq(candidate, operand));
            if let Some(index) = index {
                let target: &mut Operand = &mut copy.operands[index];
                target.mode = mode;
            }
        }
    }
    resolved
}

fn width_code(width: Option<Width>) -> u16 {
    match width {
        None => 1,
        Some(Width::Word) => 2,
        Some(Width::Long) => 3,
    }
}

fn width_bytes(width: Width) -> u32 {
    match width {
        Width::Word => 2,
        Width::Long => 4,
    }
}

/// Writes a full-format extension word and its displacements.
fn emit_full(full: &FullMode, plan: FullPlan, span: Span, ctx: &mut EmitContext) {
    let extension = ctx.offset();
    let index_part = full.index.map_or(0, index_bits);
    let suppress_base = matches!(
        full.base,
        FullBase::SuppressedAddr(_) | FullBase::SuppressedPc | FullBase::None
    );
    let suppress_index = full.index.is_none_or(|index| index.suppressed);
    let outer = width_code(plan.outer);
    let selection = match full.indirection {
        Indirection::None => 0,
        Indirection::PostIndexed if !suppress_index => 4 + outer,
        Indirection::PreIndexed | Indirection::PostIndexed => outer,
    };
    ctx.put_word(
        index_part
            | 0x0100
            | u16::from(suppress_base) << 7
            | u16::from(suppress_index) << 6
            | width_code(plan.base) << 4
            | selection,
    );
    let first_reloc = ctx.relocs.len();
    if let (Some(disp), Some(width)) = (&full.base_disp, plan.base) {
        let star = ctx.star();
        let value = ctx.evaluate(&disp.expr, star, span);
        if full.base == FullBase::Pc {
            emit_pc_field(
                value,
                extension,
                width_bytes(width),
                "displacement out of range",
                span,
                ctx,
            );
        } else {
            emit_plain_disp(value, width, span, ctx);
        }
    }
    let before_outer = ctx.relocs.len();
    if let (Some(disp), Some(width)) = (&full.outer_disp, plan.outer) {
        let star = ctx.star();
        let value = ctx.evaluate(&disp.expr, star, span);
        emit_plain_disp(value, width, span, ctx);
    }
    // vasm lists the outer displacement's relocation first, which only shows in
    // the order of an object file's external references.
    if ctx.relocs.len() > before_outer && before_outer > first_reloc {
        ctx.relocs[first_reloc..].rotate_right(1);
    }
}

/// Writes a displacement that isn't relative to the PC.
fn emit_plain_disp(value: Option<Value>, width: Width, span: Span, ctx: &mut EmitContext) {
    let bytes = width_bytes(width);
    let Some(value) = value else {
        ctx.put_zero(bytes);
        return;
    };
    let offset = sign_extend_32(value.offset);
    if width == Width::Word && !fits_word(offset) {
        ctx.error("out-of-range", "displacement out of range", span);
    }
    ctx.put_value(Value { offset, ..value }, bytes);
}

/// Writes the extension word of a bit field. Registers give the offset or width
/// at run time; constants must be 0-31 for an offset and 1-32 for a width.
fn emit_bitfield_word(op: &Operand, register: u16, ctx: &mut EmitContext) {
    let Mode::BitField { offset, width, .. } = &op.mode else {
        return;
    };
    let offset_bits = match offset {
        BitFieldArg::Reg(n) => 0x0800 | u16::from(*n) << 6,
        BitFieldArg::Value(expr) => bitfield_constant(expr, 0, 31, op.span, ctx) << 6,
    };
    let width_bits = match width {
        BitFieldArg::Reg(n) => 0x0020 | u16::from(*n),
        BitFieldArg::Value(expr) => bitfield_constant(expr, 1, 32, op.span, ctx) & 0x1F,
    };
    ctx.put_word(register | offset_bits | width_bits);
}

fn bitfield_constant(expr: &Expr, min: i64, max: i64, span: Span, ctx: &mut EmitContext) -> u16 {
    let star = ctx.star();
    let Some(value) = ctx.evaluate(expr, star, span) else {
        return 0;
    };
    if value.base != Base::Absolute || !(min..=max).contains(&value.offset) {
        ctx.error("out-of-range", "illegal bit field offset or width", span);
        return 0;
    }
    value.offset as u16
}

/// Writes a signed 16-bit displacement or short address.
fn emit_short_field(expr: &Expr, range_message: &str, span: Span, ctx: &mut EmitContext) {
    let pc = ctx.star();
    let Some(value) = ctx.evaluate(expr, pc, span) else {
        ctx.put_zero(2);
        return;
    };
    let field = sign_extend_32(value.offset);
    if !(-0x8000..=0x7FFF).contains(&field) {
        ctx.error("out-of-range", range_message, span);
    }
    ctx.put_value(
        Value {
            offset: field,
            ..value
        },
        2,
    );
}

/// Writes a PC-relative field, whose PC is at section position `pc`. A target
/// in the same section, or a constant address, gives a known distance. Anything
/// else needs a relocation, and the field holds what the linker must add to the
/// target's address: the offset, adjusted for where the field is relative to PC.
/// Returns the distance if it is known.
fn emit_pc_field(
    target: Option<Value>,
    pc: i64,
    bytes: u32,
    range_message: &str,
    span: Span,
    ctx: &mut EmitContext,
) -> Option<i64> {
    let Some(value) = target else {
        ctx.put_zero(bytes);
        return None;
    };
    let is_local = match value.base {
        Base::Absolute => true,
        Base::Section(section) => section == ctx.section,
        Base::External(_) => false,
    };
    if !is_local {
        if let Some(target) = ctx.reloc_target(value.base) {
            ctx.relocs.push(Reloc {
                offset: ctx.data.len() as u32,
                size: bytes as u8,
                target,
                kind: RelocKind::PcRelative,
                span,
            });
        }
        ctx.put((value.offset + ctx.offset() - pc) as u32, bytes);
        return None;
    }
    let distance = value.offset - ctx.address_of(pc).offset;
    let (min, max) = match bytes {
        1 => (-0x80, 0x7F),
        2 => (-0x8000, 0x7FFF),
        _ => (i64::MIN, i64::MAX),
    };
    if !(min..=max).contains(&distance) {
        ctx.error("out-of-range", range_message, span);
    }
    ctx.put(distance as u32, bytes);
    Some(distance)
}

fn emit_immediate(expr: &Expr, size: Size, span: Span, ctx: &mut EmitContext) {
    let pc = ctx.star();
    let width = if size == Size::Long { 4 } else { 2 };
    let Some(value) = ctx.evaluate(expr, pc, span) else {
        ctx.put_zero(width);
        return;
    };
    let (min, max) = match size {
        Size::Byte | Size::Short => (-0x80, 0xFF),
        Size::Word => (-0x8000, 0xFFFF),
        Size::Long => (-0x8000_0000, 0xFFFF_FFFF),
    };
    if !(min..=max).contains(&value.offset) {
        ctx.error("out-of-range", "immediate operand out of range", span);
    }
    // A byte immediate occupies the low half of its extension word.
    if matches!(size, Size::Byte | Size::Short) {
        ctx.put_zero(1);
        ctx.put_value(value, 1);
    } else {
        ctx.put_value(value, width);
    }
}

fn branch_target(target: &Operand, ctx: &mut EmitContext) -> Option<Value> {
    let Mode::AbsLong(expr) = &target.mode else {
        // Row matching only lets a bare address through.
        return None;
    };
    let pc = ctx.star();
    ctx.evaluate(expr, pc, target.span)
}

const BRANCH_RANGE: &str = "branch destination out of range";

fn emit_branch_word(target: &Operand, span: Span, ctx: &mut EmitContext) {
    let value = branch_target(target, ctx);
    let pc = ctx.instruction_start + 2;
    emit_pc_field(value, pc, 2, BRANCH_RANGE, span, ctx);
}

fn encode_branch(insn: &Instruction, span: Span, ctx: &mut EmitContext) {
    let target = &insn.operands[0];
    let opcode = insn.row.opcode;
    if matches!(insn.size, Some(Size::Short | Size::Byte)) {
        let value = branch_target(target, ctx);
        let pc = ctx.instruction_start + 2;
        ctx.put(u32::from(opcode >> 8), 1);
        // A zero displacement would mean a word branch.
        if emit_pc_field(value, pc, 1, BRANCH_RANGE, span, ctx) == Some(0) {
            ctx.error("out-of-range", BRANCH_RANGE, span);
        }
    } else {
        ctx.put_word(opcode);
        emit_branch_word(target, span, ctx);
    }
}
