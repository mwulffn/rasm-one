//! Turns matched instructions into machine code.
//!
//! An instruction's length depends only on its modes and size, never on
//! operand values. Writers therefore always emit the full width, even after
//! an error, and layout never disagrees with encoding.

use crate::error::Diagnostic;
use crate::expr::{eval, Base, EvalError, Expr, SymId, Value};
use crate::isa::table::{Encoding, InsnRow};
use crate::isa::{IndexReg, Mode, Operand};
use crate::source::Span;
use crate::types::{SectionKind, Size};
use crate::{Reloc, RelocKind, RelocTarget};

/// An instruction matched to its table row.
#[derive(Debug, Clone)]
pub(crate) struct Instruction {
    pub row: &'static InsnRow,
    pub size: Option<Size>,
    pub operands: Vec<Operand>,
}

impl Instruction {
    /// Length in bytes.
    pub fn len(&self) -> u32 {
        let size = self.size.unwrap_or(Size::Word);
        match self.row.encoding {
            Encoding::Branch => match self.size {
                Some(Size::Short | Size::Byte) => 2,
                _ => 4,
            },
            Encoding::DecrementBranch => 4,
            // The immediate is part of the opcode word.
            Encoding::Quick | Encoding::Shift | Encoding::Trap | Encoding::Moveq => {
                2 + extension_len(&self.operands[1..], size)
            }
            Encoding::MovepLoad | Encoding::MovepStore => 4,
            // The bit number is always a word, whatever the operation size.
            Encoding::BitImmediate => 4 + extension_len(&self.operands[1..], size),
            Encoding::MovemStore => 4 + extension_len(&self.operands[1..], size),
            Encoding::MovemLoad => 4 + extension_len(&self.operands[..1], size),
            _ => 2 + extension_len(&self.operands, size),
        }
    }
}

fn extension_len(operands: &[Operand], size: Size) -> u32 {
    operands
        .iter()
        .map(|op| match &op.mode {
            Mode::Disp(..)
            | Mode::Index(..)
            | Mode::AbsWord(_)
            | Mode::PcDisp(_)
            | Mode::PcIndex(..) => 2,
            Mode::AbsLong(_) => 4,
            Mode::Immediate(_) if size == Size::Long => 4,
            Mode::Immediate(_) => 2,
            _ => 0,
        })
        .sum()
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

pub(crate) fn encode(insn: &Instruction, span: Span, ctx: &mut EmitContext) {
    ctx.instruction_start = ctx.offset();
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
            emit_extension(op(0), size, ctx);
            emit_extension(op(1), size, ctx);
        }
        Encoding::Ea0 => {
            ctx.put_word(base | mode_bits(&op(0).mode));
            emit_extension(op(0), size, ctx);
        }
        Encoding::Ea1 => {
            ctx.put_word(base | mode_bits(&op(1).mode));
            emit_extension(op(1), size, ctx);
        }
        Encoding::SizeEa0 => {
            ctx.put_word(base | size_bits(size) | mode_bits(&op(0).mode));
            emit_extension(op(0), size, ctx);
        }
        Encoding::RegEa => {
            ctx.put_word(base | reg(1) << 9 | mode_bits(&op(0).mode));
            emit_extension(op(0), size, ctx);
        }
        Encoding::ToReg => {
            ctx.put_word(base | reg(1) << 9 | size_bits(size) | mode_bits(&op(0).mode));
            emit_extension(op(0), size, ctx);
        }
        Encoding::FromReg => {
            ctx.put_word(base | reg(0) << 9 | size_bits(size) | mode_bits(&op(1).mode));
            emit_extension(op(1), size, ctx);
        }
        Encoding::Address => {
            let opmode: u16 = if size == Size::Long { 0x1C0 } else { 0xC0 };
            ctx.put_word(base | reg(1) << 9 | opmode | mode_bits(&op(0).mode));
            emit_extension(op(0), size, ctx);
        }
        Encoding::Immediate => {
            ctx.put_word(base | size_bits(size) | mode_bits(&op(1).mode));
            emit_extension(op(0), size, ctx);
            emit_extension(op(1), size, ctx);
        }
        Encoding::ImmediateOnly => {
            ctx.put_word(base);
            emit_extension(op(0), size, ctx);
        }
        Encoding::Quick => {
            let count = opcode_constant(op(0), 1, 8, false, ctx) as u16 & 7;
            ctx.put_word(base | count << 9 | size_bits(size) | mode_bits(&op(1).mode));
            emit_extension(op(1), size, ctx);
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
            emit_extension(op(1), size, ctx);
        }
        Encoding::BitImmediate => {
            ctx.put_word(base | mode_bits(&op(1).mode));
            let bit = opcode_constant(op(0), 0, 255, false, ctx) as u16;
            ctx.put_word(bit);
            emit_extension(op(1), size, ctx);
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
            emit_extension(op(1), Size::Word, ctx);
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
            emit_extension(op(1), size, ctx);
        }
        Encoding::MovemLoad => {
            ctx.put_word(base | mode_bits(&op(0).mode));
            let mask = register_mask(op(1), ctx);
            ctx.put_word(mask);
            emit_extension(op(0), size, ctx);
        }
    }
}

fn encode_movep(opcode: u16, memory: &Operand, ctx: &mut EmitContext) {
    let (displacement, base) = match &memory.mode {
        Mode::Disp(expr, n) => (Some(expr), *n),
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
        // Not effective addresses, so never placed in an EA field.
        Mode::Sr | Mode::Ccr | Mode::Usp | Mode::RegList(_) => (0, 0),
    }
}

fn index_bits(index: IndexReg) -> u16 {
    u16::from(index.reg) << 12 | u16::from(index.long) << 11
}

fn emit_extension(op: &Operand, size: Size, ctx: &mut EmitContext) {
    let pc = ctx.star();
    match &op.mode {
        Mode::Disp(expr, _) => {
            emit_short_field(expr, "displacement out of range", op.span, ctx);
        }
        Mode::Index(expr, _, index) => {
            ctx.put(u32::from(index_bits(*index) >> 8), 1);
            match ctx.evaluate(expr, pc, op.span) {
                Some(value) => {
                    let d = sign_extend_32(value.offset);
                    if !(-0x80..=0x7F).contains(&d) {
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
        Mode::PcDisp(expr) => {
            let target = ctx.evaluate(expr, pc, op.span);
            let extension = ctx.offset();
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
            let extension = ctx.offset();
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
    let (min, max) = if bytes == 1 {
        (-0x80, 0x7F)
    } else {
        (-0x8000, 0x7FFF)
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
