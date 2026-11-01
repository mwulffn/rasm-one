//! Expressions and their evaluation.

use crate::source::Span;

/// Index into the assembler's symbol list.
pub(crate) type SymId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnaryOp {
    Neg,
    Not,
    LogicalNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinaryOp {
    Shl,
    Shr,
    And,
    Xor,
    Or,
    Mul,
    Div,
    Rem,
    Add,
    Sub,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    LogicalAnd,
    LogicalOr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Expr {
    Num(i64),
    Sym(SymId, Span),
    /// `*`, the current address.
    Pc,
    Unary(UnaryOp, Box<Expr>),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
}

/// A value: a constant, or an offset from the start of a section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Value {
    pub section: Option<usize>,
    pub offset: i64,
}

impl Value {
    pub fn abs(offset: i64) -> Self {
        Self {
            section: None,
            offset,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EvalError {
    Undefined(SymId, Span),
    DivideByZero,
    NotAbsolute,
}

/// Evaluates `expr`. `pc` is the value of `*`; `lookup` returns a symbol's
/// value, or `None` if it has no definition.
pub(crate) fn eval(
    expr: &Expr,
    pc: Value,
    lookup: &dyn Fn(SymId) -> Option<Value>,
) -> Result<Value, EvalError> {
    match expr {
        Expr::Num(n) => Ok(Value::abs(*n)),
        Expr::Sym(id, span) => lookup(*id).ok_or(EvalError::Undefined(*id, *span)),
        Expr::Pc => Ok(pc),
        Expr::Unary(op, inner) => {
            let v = absolute(eval(inner, pc, lookup)?)?;
            Ok(Value::abs(match op {
                UnaryOp::Neg => v.wrapping_neg(),
                UnaryOp::Not => !v,
                UnaryOp::LogicalNot => truth(v == 0),
            }))
        }
        Expr::Binary(op, a, b) => {
            let a = eval(a, pc, lookup)?;
            let b = eval(b, pc, lookup)?;
            binary(*op, a, b)
        }
    }
}

fn binary(op: BinaryOp, a: Value, b: Value) -> Result<Value, EvalError> {
    match (op, a.section, b.section) {
        (BinaryOp::Add, Some(_), None) | (BinaryOp::Sub, Some(_), None) => {
            let delta = if op == BinaryOp::Add {
                b.offset
            } else {
                b.offset.wrapping_neg()
            };
            return Ok(Value {
                section: a.section,
                offset: a.offset.wrapping_add(delta),
            });
        }
        (BinaryOp::Add, None, Some(_)) => {
            return Ok(Value {
                section: b.section,
                offset: a.offset.wrapping_add(b.offset),
            });
        }
        (BinaryOp::Sub, Some(x), Some(y)) if x == y => {
            return Ok(Value::abs(a.offset.wrapping_sub(b.offset)));
        }
        _ => {}
    }
    let (a, b) = (absolute(a)?, absolute(b)?);
    Ok(Value::abs(match op {
        BinaryOp::Shl => shift(a, b, i64::wrapping_shl),
        BinaryOp::Shr => shift(a, b, i64::wrapping_shr),
        BinaryOp::And => a & b,
        BinaryOp::Xor => a ^ b,
        BinaryOp::Or => a | b,
        BinaryOp::Mul => a.wrapping_mul(b),
        BinaryOp::Div if b == 0 => return Err(EvalError::DivideByZero),
        BinaryOp::Rem if b == 0 => return Err(EvalError::DivideByZero),
        BinaryOp::Div => a.wrapping_div(b),
        BinaryOp::Rem => a.wrapping_rem(b),
        BinaryOp::Add => a.wrapping_add(b),
        BinaryOp::Sub => a.wrapping_sub(b),
        BinaryOp::Lt => truth(a < b),
        BinaryOp::Le => truth(a <= b),
        BinaryOp::Gt => truth(a > b),
        BinaryOp::Ge => truth(a >= b),
        BinaryOp::Eq => truth(a == b),
        BinaryOp::Ne => truth(a != b),
        BinaryOp::LogicalAnd => truth(a != 0 && b != 0),
        BinaryOp::LogicalOr => truth(a != 0 || b != 0),
    }))
}

fn shift(a: i64, count: i64, op: fn(i64, u32) -> i64) -> i64 {
    match u32::try_from(count) {
        Ok(count) if count < 64 => op(a, count),
        _ => 0,
    }
}

/// True is -1, as in vasm.
fn truth(b: bool) -> i64 {
    if b {
        -1
    } else {
        0
    }
}

fn absolute(value: Value) -> Result<i64, EvalError> {
    match value.section {
        None => Ok(value.offset),
        Some(_) => Err(EvalError::NotAbsolute),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evaluate(expr: &Expr) -> Result<Value, EvalError> {
        eval(expr, Value::abs(0), &|_| None)
    }

    fn bin(op: BinaryOp, a: i64, b: i64) -> Expr {
        Expr::Binary(op, Box::new(Expr::Num(a)), Box::new(Expr::Num(b)))
    }

    #[test]
    fn test_eval_division_truncates_and_zero_is_error() {
        assert_eq!(evaluate(&bin(BinaryOp::Div, -7, 2)), Ok(Value::abs(-3)));
        assert_eq!(
            evaluate(&bin(BinaryOp::Div, 1, 0)),
            Err(EvalError::DivideByZero)
        );
    }

    #[test]
    fn test_eval_comparison_true_is_minus_one() {
        assert_eq!(evaluate(&bin(BinaryOp::Lt, 1, 2)), Ok(Value::abs(-1)));
    }

    #[test]
    fn test_binary_section_difference_is_absolute() {
        let at = |offset| Value {
            section: Some(0),
            offset,
        };
        assert_eq!(binary(BinaryOp::Sub, at(6), at(2)), Ok(Value::abs(4)));
    }

    #[test]
    fn test_binary_different_sections_are_not_absolute() {
        let at = |section| Value {
            section: Some(section),
            offset: 0,
        };
        assert_eq!(
            binary(BinaryOp::Sub, at(0), at(1)),
            Err(EvalError::NotAbsolute)
        );
    }

    #[test]
    fn test_eval_undefined_symbol_is_error() {
        let span = Span {
            file: 0,
            line: 1,
            col: 1,
            len: 1,
        };
        assert_eq!(
            evaluate(&Expr::Sym(3, span)),
            Err(EvalError::Undefined(3, span))
        );
    }
}
