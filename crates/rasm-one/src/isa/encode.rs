//! Turns matched instructions into machine code.
//!
//! An instruction's length depends only on its modes and size, never on
//! operand values. Writers therefore always emit the full width, even after
//! an error, and layout never disagrees with encoding.

use crate::error::Diagnostic;
use crate::expr::{eval, EvalError, Expr, SymId, Value};
use crate::isa::table::{Encoding, InsnRow};
use crate::isa::{IndexReg, Mode, Operand};
use crate::source::Span;
use crate::types::Size;
use crate::{Reloc, RelocKind};

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
        if self.row.encoding == Encoding::Branch {
            return match self.size {
                Some(Size::Short | Size::Byte) => 2,
                _ => 4,
            };
        }
        let size = self.size.unwrap_or(Size::Word);
        2 + self
            .operands
            .iter()
            .map(|op| extension_len(&op.mode, size))
            .sum::<u32>()
    }
}

fn extension_len(mode: &Mode, size: Size) -> u32 {
    match mode {
        Mode::Disp(..)
        | Mode::Index(..)
        | Mode::AbsWord(_)
        | Mode::PcDisp(_)
        | Mode::PcIndex(..) => 2,
        Mode::AbsLong(_) => 4,
        Mode::Immediate(_) => {
            if size == Size::Long {
                4
            } else {
                2
            }
        }
        _ => 0,
    }
}

/// Everything an encoder needs from the surrounding assembly.
pub(crate) struct EmitContext<'a> {
    pub section: usize,
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

    /// The value of `*` in an operand.
    fn star(&self) -> Value {
        Value {
            section: Some(self.section),
            offset: self.instruction_start,
        }
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

    fn put(&mut self, value: u32, bytes: u32) {
        let all = value.to_be_bytes();
        self.data.extend_from_slice(&all[4 - bytes as usize..]);
    }

    /// Records a relocation for `value` if it is section-relative, then
    /// writes it. Relocations always cover the field about to be written.
    pub fn put_value(&mut self, value: Value, bytes: u32) {
        if let Some(target) = value.section {
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

pub(crate) fn encode(insn: &Instruction, span: Span, ctx: &mut EmitContext) {
    ctx.instruction_start = ctx.offset();
    match insn.row.encoding {
        Encoding::Fixed => ctx.put(u32::from(insn.row.opcode), 2),
        Encoding::Branch => encode_branch(insn, span, ctx),
        Encoding::Move => {
            let (src, dst) = (&insn.operands[0], &insn.operands[1]);
            let size = insn.size.unwrap_or(Size::Word);
            let size_code: u16 = match size {
                Size::Byte | Size::Short => 1,
                Size::Long => 2,
                Size::Word => 3,
            };
            let (dst_mode, dst_reg) = mode_reg(&dst.mode);
            let opcode = size_code << 12 | dst_reg << 9 | dst_mode << 6 | mode_bits(&src.mode);
            ctx.put(u32::from(opcode), 2);
            emit_extension(src, size, ctx);
            emit_extension(dst, size, ctx);
        }
        Encoding::Lea => {
            let (src, dst) = (&insn.operands[0], &insn.operands[1]);
            let (_, reg) = mode_reg(&dst.mode);
            ctx.put(
                u32::from(insn.row.opcode | reg << 9 | mode_bits(&src.mode)),
                2,
            );
            emit_extension(src, Size::Long, ctx);
        }
    }
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
            let d = pc_relative(expr, op.span, ctx).unwrap_or(0);
            if !(-0x8000..=0x7FFF).contains(&d) {
                ctx.error("out-of-range", "displacement out of range", op.span);
            }
            ctx.put(d as u32, 2);
        }
        Mode::PcIndex(expr, index) => {
            let d = pc_relative(expr, op.span, ctx).unwrap_or(0);
            if !(-0x80..=0x7F).contains(&d) {
                ctx.error("out-of-range", "displacement out of range", op.span);
            }
            ctx.put(u32::from(index_bits(*index) | (d as u16 & 0xFF)), 2);
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

/// Returns the distance from the current address to the target. A constant
/// target is an address, not a displacement.
fn pc_relative(expr: &Expr, span: Span, ctx: &mut EmitContext) -> Option<i64> {
    let pc = ctx.star();
    let value = ctx.evaluate(expr, pc, span)?;
    match value.section {
        Some(section) if section != ctx.section => {
            ctx.error(
                "cross-section",
                "PC-relative reference to another section",
                span,
            );
            None
        }
        _ => Some(value.offset - ctx.offset()),
    }
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

fn encode_branch(insn: &Instruction, span: Span, ctx: &mut EmitContext) {
    let target = &insn.operands[0];
    let Mode::AbsLong(expr) = &target.mode else {
        // Row matching only lets a bare address through.
        return;
    };
    let pc_at_extension = ctx.offset() + 2;
    let pc = ctx.star();
    let opcode = u32::from(insn.row.opcode);
    let short = matches!(insn.size, Some(Size::Short | Size::Byte));
    let d = match ctx.evaluate(expr, pc, target.span) {
        Some(value) if value.section.is_some_and(|s| s != ctx.section) => {
            ctx.error("cross-section", "branch to another section", target.span);
            None
        }
        Some(value) => Some(value.offset - pc_at_extension),
        None => None,
    };
    if short {
        let d = d.unwrap_or(2);
        if d == 0 || !(-0x80..=0x7F).contains(&d) {
            ctx.error("out-of-range", "branch destination out of range", span);
        }
        ctx.put(opcode | (d as u32 & 0xFF), 2);
    } else {
        let d = d.unwrap_or(0);
        if !(-0x8000..=0x7FFF).contains(&d) {
            ctx.error("out-of-range", "branch destination out of range", span);
        }
        ctx.put(opcode, 2);
        ctx.put(d as u32 & 0xFFFF, 2);
    }
}
