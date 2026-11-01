//! vasm's generic optimisations: safe replacements of one instruction by others, which
//! depend on the values of its operands.

use crate::expr::{eval, Expr, Value};
use crate::isa::encode::{Instruction, Sizer};
use crate::isa::table::{rows_for, InsnRow};
use crate::isa::{Mode, Operand};
use crate::types::{Cpu, Size};

/// What an instruction is written as instead.
pub(crate) enum Rewritten {
    /// Nothing, because it has no effect.
    Deleted,
    One(Instruction),
    Two(Instruction, Instruction),
}

/// What an operand's value is known to be.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Known {
    Number(i64),
    /// An address that is only known when loading or linking.
    Address,
}

impl Instruction {
    fn known(&self, expr: &Expr, sizer: &Sizer) -> Option<Known> {
        let value: Value = eval(expr, sizer.address_of_start(), sizer.lookup).ok()?;
        Some(self.classify(value))
    }

    /// The row of `mnemonic` for these operands on this CPU.
    fn row_of(&self, mnemonic: &str, size: Size, operands: &[Operand]) -> Option<&'static InsnRow> {
        rows_for(mnemonic)?.iter().find(|row| {
            row.cpus.contains(self.cpu)
                && row.sizes.contains(&size)
                && row.operands.len() == operands.len()
                && row
                    .operands
                    .iter()
                    .zip(operands)
                    .all(|(set, operand)| set.intersects(operand.mode.class()))
        })
    }

    /// This as `mnemonic` with other operands, if the CPU has that.
    fn as_other(&self, mnemonic: &str, size: Size, operands: Vec<Operand>) -> Option<Instruction> {
        let row = self.row_of(mnemonic, size, &operands)?;
        Some(Instruction {
            row,
            size: Some(size),
            operands,
            rewrites: None,
            generic_rewrites: false,
            quick_move: None,
            shortens_absolutes: true,
            ..self.clone()
        })
    }

    /// Applies the generic optimisations, if this is an instruction they cover.
    pub(crate) fn generic_rewrite(&self, sizer: &Sizer) -> Option<Rewritten> {
        if !self.generic_rewrites {
            return None;
        }
        let size = self.size.unwrap_or(Size::Word);
        let one = |instruction: Option<Instruction>| instruction.map(Rewritten::One);
        let operand = |mode: Mode, like: &Operand| Operand {
            mode,
            span: like.span,
        };
        let operands = self.operands.as_slice();
        let mnemonic = self.row.mnemonic;
        match operands {
            [first, second] if mnemonic == "movem" => {
                self.rewrite_movem(first, second, size, sizer)
            }
            [source, destination] if matches!(source.mode, Mode::Immediate(_)) => {
                let Mode::Immediate(expr) = &source.mode else {
                    return None;
                };
                let known = self.known(expr, sizer)?;
                let to_address = matches!(destination.mode, Mode::AddrReg(_));
                let Known::Number(number) = known else {
                    let long_move = matches!(mnemonic, "move" | "movea") && size == Size::Long;
                    if long_move && to_address {
                        let label = if self.pc_relative {
                            Mode::AbsOrPc(expr.clone())
                        } else {
                            Mode::AbsLong(expr.clone())
                        };
                        let label = operand(label, source);
                        return one(self.as_other(
                            "lea",
                            Size::Long,
                            vec![label, destination.clone()],
                        ));
                    }
                    return None;
                };
                let zero = number == 0;
                let mask = match size {
                    Size::Byte => 0xFF,
                    Size::Word => 0xFFFF,
                    _ => 0xFFFF_FFFF,
                };
                // Like vasm, `and` wants the exact bits, except that a long can be -1.
                let and_ones = number == mask || (size == Size::Long && number == -1);
                let eor_ones = number == mask || number == -1 || number == 0xFFFF_FFFF;
                let unary = |name: &str| one(self.as_other(name, size, vec![destination.clone()]));
                // Clearing reads before it writes on a 68000, which may matter.
                let can_clear = self.cpu != Cpu::M68000;
                match mnemonic {
                    "cmp" | "cmpi" | "cmpa" if zero && to_address => {
                        one(self.as_other("tst", Size::Long, vec![destination.clone()]))
                    }
                    "cmp" | "cmpi" | "cmpa" if zero => unary("tst"),
                    "move" | "movea" if zero && to_address => {
                        let register = destination.clone();
                        one(self.as_other("suba", Size::Long, vec![register.clone(), register]))
                    }
                    "move" if zero && can_clear => unary("clr"),
                    "and" | "andi" if zero && can_clear => unary("clr"),
                    "and" | "andi" if and_ones => unary("tst"),
                    "or" | "ori" if zero => unary("tst"),
                    "eor" | "eori" if zero => unary("tst"),
                    "eor" | "eori" if eor_ones => unary("not"),
                    "add" | "adda" | "sub" | "suba" if zero && to_address => {
                        Some(Rewritten::Deleted)
                    }
                    "asl"
                        if number == 1
                            && self.cpu != Cpu::M68060
                            && matches!(destination.mode, Mode::DataReg(_)) =>
                    {
                        let register = destination.clone();
                        one(self.as_other("add", size, vec![register.clone(), register]))
                    }
                    _ => None,
                }
            }
            [register]
                if mnemonic == "asl"
                    && self.cpu != Cpu::M68060
                    && matches!(register.mode, Mode::DataReg(_)) =>
            {
                one(self.as_other("add", size, vec![register.clone(), register.clone()]))
            }
            [register] if mnemonic == "clr" && size == Size::Long => {
                if !matches!(register.mode, Mode::DataReg(_)) {
                    return None;
                }
                let zero = operand(Mode::Immediate(Expr::Num(0)), register);
                one(self.as_other("moveq", Size::Long, vec![zero, register.clone()]))
            }
            [register, value] if mnemonic == "link" && size == Size::Long => {
                let Mode::Immediate(expr) = &value.mode else {
                    return None;
                };
                match self.known(expr, sizer)? {
                    Known::Number(number) if (-0x8000..=0x7FFF).contains(&number) => one(
                        self.as_other("link", Size::Word, vec![register.clone(), value.clone()])
                    ),
                    _ => None,
                }
            }
            [source, target] if mnemonic == "lea" => self.rewrite_lea(source, target, sizer),
            _ => None,
        }
    }

    fn rewrite_movem(
        &self,
        first: &Operand,
        second: &Operand,
        size: Size,
        sizer: &Sizer,
    ) -> Option<Rewritten> {
        let mask = |operand: &Operand| match &operand.mode {
            Mode::RegList(mask) => Some(i64::from(*mask)),
            Mode::Immediate(expr) => match self.known(expr, sizer) {
                Some(Known::Number(number)) => Some(number),
                _ => None,
            },
            _ => None,
        };
        if mask(first) == Some(0) || mask(second) == Some(0) {
            return Some(Rewritten::Deleted);
        }
        // Two address registers from `(Ax)+`, unless it's one of them.
        if let (Mode::PostInc(from), Some(list)) = (&first.mode, mask(second)) {
            let source = u32::from(*from) + 8;
            if list.count_ones() == 2 && list & 0xFF == 0 && list & (1 << source) == 0 {
                let low = list.trailing_zeros() as u8 - 8;
                let high = 63 - list.leading_zeros() as u8 - 8;
                let target = |n: u8| Operand {
                    mode: Mode::AddrReg(n),
                    span: second.span,
                };
                let load = |n: u8| self.as_other("move", size, vec![first.clone(), target(n)]);
                return Some(Rewritten::Two(load(low)?, load(high)?));
            }
        }
        let single_address = match second.mode {
            Mode::AddrReg(n) => Some(n),
            _ => mask(second)
                .filter(|mask| mask.count_ones() == 1 && (0x100..=0x8000).contains(mask))
                .map(|mask| mask.trailing_zeros() as u8 - 8),
        }?;
        let target = Operand {
            mode: Mode::AddrReg(single_address),
            span: second.span,
        };
        self.as_other("move", size, vec![first.clone(), target])
            .map(Rewritten::One)
    }

    fn rewrite_lea(&self, source: &Operand, target: &Operand, sizer: &Sizer) -> Option<Rewritten> {
        let Mode::AddrReg(to) = target.mode else {
            return None;
        };
        let clear = || {
            self.as_other("suba", Size::Long, vec![target.clone(), target.clone()])
                .map(Rewritten::One)
        };
        match &source.mode {
            Mode::Indirect(from) if *from == to => Some(Rewritten::Deleted),
            Mode::AbsLong(expr) | Mode::AbsWord(expr) | Mode::AbsShortable(expr) => {
                match self.known(expr, sizer)? {
                    Known::Number(0) => clear(),
                    _ => None,
                }
            }
            Mode::Disp(expr, from) | Mode::DispWord(expr, from) => {
                let Known::Number(number) = self.known(expr, sizer)? else {
                    return None;
                };
                if number == 0 && *from == to {
                    return Some(Rewritten::Deleted);
                }
                if self.wide || (-0x8000..=0x7FFF).contains(&number) {
                    return None;
                }
                let immediate = Operand {
                    mode: Mode::Immediate(Expr::Num(number)),
                    span: source.span,
                };
                let add = self.as_other("adda", Size::Long, vec![immediate, target.clone()])?;
                if *from == to {
                    return Some(Rewritten::One(add));
                }
                let base = Operand {
                    mode: Mode::AddrReg(*from),
                    span: source.span,
                };
                let copy = self.as_other("movea", Size::Long, vec![base, target.clone()])?;
                Some(Rewritten::Two(copy, add))
            }
            _ => None,
        }
    }
}
