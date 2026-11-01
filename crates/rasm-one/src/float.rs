//! Converts decimal floating-point text to the 68k's formats, exactly.
//!
//! Extended precision has more bits than Rust's floats, so it is done with a
//! small big integer. Single and double use Rust's own correct conversions.

use crate::types::Size;

/// A number too large for this conversion is an error, which also bounds the
/// big integers involved. Extended precision reaches about 1e4932.
const MAX_DECIMAL_EXPONENT: i64 = 5000;

/// A floating-point literal, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FloatLiteral {
    pub text: String,
}

/// An unsigned big integer, least significant limb first.
#[derive(Clone, PartialEq, Eq)]
struct Big(Vec<u32>);

impl Big {
    fn from_digits(digits: &str) -> Big {
        let mut big = Big(vec![0]);
        for digit in digits.bytes() {
            big.mul_small(10);
            big.add_small(u32::from(digit - b'0'));
        }
        big
    }

    fn mul_small(&mut self, factor: u32) {
        let mut carry = 0u64;
        for limb in &mut self.0 {
            let product = u64::from(*limb) * u64::from(factor) + carry;
            *limb = product as u32;
            carry = product >> 32;
        }
        if carry != 0 {
            self.0.push(carry as u32);
        }
    }

    fn add_small(&mut self, addend: u32) {
        let mut carry = u64::from(addend);
        for limb in &mut self.0 {
            let sum = u64::from(*limb) + carry;
            *limb = sum as u32;
            carry = sum >> 32;
            if carry == 0 {
                return;
            }
        }
        if carry != 0 {
            self.0.push(carry as u32);
        }
    }

    fn is_zero(&self) -> bool {
        self.0.iter().all(|&limb| limb == 0)
    }

    fn bit_length(&self) -> i64 {
        for (index, &limb) in self.0.iter().enumerate().rev() {
            if limb != 0 {
                return index as i64 * 32 + i64::from(32 - limb.leading_zeros());
            }
        }
        0
    }

    fn shift_left(&self, bits: i64) -> Big {
        let limbs = (bits / 32) as usize;
        let rest = (bits % 32) as u32;
        let mut out = vec![0u32; limbs];
        let mut carry = 0u32;
        for &limb in &self.0 {
            out.push(if rest == 0 {
                limb
            } else {
                limb << rest | carry
            });
            carry = if rest == 0 { 0 } else { limb >> (32 - rest) };
        }
        if carry != 0 {
            out.push(carry);
        }
        Big(out)
    }

    fn at_least(&self, other: &Big) -> bool {
        let len = self.0.len().max(other.0.len());
        for index in (0..len).rev() {
            let a = self.0.get(index).copied().unwrap_or(0);
            let b = other.0.get(index).copied().unwrap_or(0);
            if a != b {
                return a > b;
            }
        }
        true
    }

    fn subtract(&mut self, other: &Big) {
        let mut borrow = 0i64;
        for (index, limb) in self.0.iter_mut().enumerate() {
            let subtrahend = i64::from(other.0.get(index).copied().unwrap_or(0)) + borrow;
            let difference = i64::from(*limb) - subtrahend;
            borrow = i64::from(difference < 0);
            *limb = difference.rem_euclid(1 << 32) as u32;
        }
    }
}

/// A decimal literal split into sign, digits and power of ten.
struct Decimal {
    negative: bool,
    digits: String,
    exponent: i64,
}

fn parse_decimal(text: &str) -> Option<Decimal> {
    let (negative, unsigned) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let lower = unsigned.to_ascii_lowercase();
    let (mantissa, written_exponent) = match lower.split_once('e') {
        Some((mantissa, exponent)) => (mantissa, exponent.parse::<i64>().ok()?),
        None => (lower.as_str(), 0),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{whole}{fraction}");
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let exponent = written_exponent.checked_sub(fraction.len() as i64)?;
    if exponent.abs() > MAX_DECIMAL_EXPONENT + digits.len() as i64 {
        return None;
    }
    Some(Decimal {
        negative,
        digits,
        exponent,
    })
}

/// Rounds to 64 significant bits, half to even. Returns the mantissa with its
/// top bit set and the binary exponent of that top bit, or `None` for zero.
fn to_binary(decimal: &Decimal) -> Option<(u64, i64)> {
    let mut numerator = Big::from_digits(&decimal.digits);
    if numerator.is_zero() {
        return None;
    }
    let mut denominator = Big(vec![1]);
    for _ in 0..decimal.exponent.unsigned_abs() {
        if decimal.exponent > 0 {
            numerator.mul_small(10);
        } else {
            denominator.mul_small(10);
        }
    }
    // Scale so the quotient of the next step has its top bit in a known place.
    let mut exponent = numerator.bit_length() - denominator.bit_length();
    if exponent >= 0 {
        denominator = denominator.shift_left(exponent);
    } else {
        numerator = numerator.shift_left(-exponent);
    }
    if !numerator.at_least(&denominator) {
        numerator = numerator.shift_left(1);
        exponent -= 1;
    }
    let mut mantissa = 0u64;
    for _ in 0..64 {
        mantissa <<= 1;
        if numerator.at_least(&denominator) {
            numerator.subtract(&denominator);
            mantissa |= 1;
        }
        numerator = numerator.shift_left(1);
    }
    let above_half = numerator.at_least(&denominator);
    let exactly_half = above_half && {
        let mut rest = numerator.clone();
        rest.subtract(&denominator);
        rest.is_zero()
    };
    if above_half && (!exactly_half || mantissa & 1 == 1) {
        let (rounded, overflow) = mantissa.overflowing_add(1);
        if overflow {
            return Some((1 << 63, exponent + 1));
        }
        mantissa = rounded;
    }
    Some((mantissa, exponent))
}

fn extended_bytes(decimal: &Decimal) -> Option<Vec<u8>> {
    let sign: u16 = if decimal.negative { 0x8000 } else { 0 };
    let (mantissa, biased) = match to_binary(decimal) {
        Some((mantissa, exponent)) => {
            let biased = exponent + 16383;
            if !(1..0x7FFF).contains(&biased) {
                return None;
            }
            (mantissa, biased as u16)
        }
        None => (0, 0),
    };
    let mut out = Vec::with_capacity(12);
    out.extend_from_slice(&(sign | biased).to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&mantissa.to_be_bytes());
    Some(out)
}

/// Packed decimal: one digit before the point and sixteen after, rounded half
/// up, with a three-digit decimal exponent.
fn packed_bytes(decimal: &Decimal) -> Option<Vec<u8>> {
    let trimmed = decimal.digits.trim_start_matches('0');
    let mut digits: Vec<u8> = trimmed.bytes().map(|byte| byte - b'0').collect();
    let mut exponent = if digits.is_empty() {
        0
    } else {
        decimal.exponent + digits.len() as i64 - 1
    };
    if digits.len() > 17 {
        let round_up = digits[17] >= 5;
        digits.truncate(17);
        if round_up {
            let mut index = 16;
            loop {
                if digits[index] < 9 {
                    digits[index] += 1;
                    break;
                }
                digits[index] = 0;
                if index == 0 {
                    digits.insert(0, 1);
                    digits.truncate(17);
                    exponent += 1;
                    break;
                }
                index -= 1;
            }
        }
    }
    digits.resize(17, 0);
    if exponent.abs() > 999 {
        return None;
    }
    let magnitude = exponent.unsigned_abs() as u32;
    let exponent_bcd = ((magnitude / 100) << 8) | ((magnitude / 10 % 10) << 4) | (magnitude % 10);
    let mut first = exponent_bcd << 16 | u32::from(digits[0]);
    if decimal.negative {
        first |= 0x8000_0000;
    }
    if exponent < 0 {
        first |= 0x4000_0000;
    }
    let mut out = first.to_be_bytes().to_vec();
    for pair in digits[1..].chunks(2) {
        out.push(pair[0] << 4 | pair[1]);
    }
    Some(out)
}

/// The bytes of a literal in a floating-point format, or `None` if it is out of
/// range or `size` isn't one.
pub(crate) fn float_bytes(text: &str, size: Size) -> Option<Vec<u8>> {
    let decimal = parse_decimal(text)?;
    match size {
        Size::Single => {
            let value: f32 = text.parse().ok()?;
            value.is_finite().then(|| value.to_be_bytes().to_vec())
        }
        Size::Double => {
            let value: f64 = text.parse().ok()?;
            value.is_finite().then(|| value.to_be_bytes().to_vec())
        }
        Size::Extended => extended_bytes(&decimal),
        Size::Packed => packed_bytes(&decimal),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(text: &str, size: Size) -> Option<String> {
        float_bytes(text, size).map(|bytes| {
            bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .concat()
        })
    }

    #[test]
    fn test_float_bytes_extended_exact_values() {
        let cases = [
            ("1", "3fff00008000000000000000"),
            ("1.5", "3fff0000c000000000000000"),
            ("-2", "c00000008000000000000000"),
            ("0", "000000000000000000000000"),
            ("0.25", "3ffd00008000000000000000"),
        ];
        for (text, expected) in cases {
            assert_eq!(
                hex(text, Size::Extended).as_deref(),
                Some(expected),
                "{text}"
            );
        }
    }

    /// These need more precision than a double, which is the point of Big.
    #[test]
    fn test_float_bytes_extended_needs_full_precision() {
        // The 68881's own constant ROM has pi as 4000 C90FDAA22168C235.
        let pi = "3.14159265358979323846264338327950288";
        assert_eq!(
            hex(pi, Size::Extended).as_deref(),
            Some("40000000c90fdaa22168c235")
        );
        // 0.1 is 3FFB CCCCCCCCCCCCCCCD, rounded up from a repeating pattern.
        assert_eq!(
            hex("0.1", Size::Extended).as_deref(),
            Some("3ffb0000cccccccccccccccd")
        );
        // 1 + 2^-63 is exactly representable.
        let just_above_one = "1.000000000000000000108420217248550443400745280086994171142578125";
        assert_eq!(
            hex(just_above_one, Size::Extended).as_deref(),
            Some("3fff00008000000000000001")
        );
    }

    #[test]
    fn test_float_bytes_extended_rounds_half_to_even() {
        // 1 + 2^-64 is halfway, and rounds down to the even mantissa.
        let halfway = "1.0000000000000000000542101086242752217003726400434970855712890625";
        assert_eq!(
            hex(halfway, Size::Extended).as_deref(),
            Some("3fff00008000000000000000")
        );
    }

    #[test]
    fn test_float_bytes_extended_carry_on_rounding() {
        // Just below 2 rounds all the way up to 2.
        let below_two = "1.99999999999999999999999999";
        assert_eq!(
            hex(below_two, Size::Extended).as_deref(),
            Some("400000008000000000000000")
        );
    }

    #[test]
    fn test_float_bytes_exponents() {
        assert_eq!(
            hex("1e2", Size::Extended).as_deref(),
            Some("40050000c800000000000000")
        );
        assert_eq!(hex("1e5000", Size::Extended), None);
        assert_eq!(hex("1e400", Size::Double), None);
    }

    #[test]
    fn test_float_bytes_single_and_double() {
        assert_eq!(hex("1.5", Size::Single).as_deref(), Some("3fc00000"));
        assert_eq!(
            hex("1.5", Size::Double).as_deref(),
            Some("3ff8000000000000")
        );
    }

    #[test]
    fn test_float_bytes_packed() {
        assert_eq!(
            hex("1", Size::Packed).as_deref(),
            Some("000000010000000000000000")
        );
        assert_eq!(
            hex("1.5", Size::Packed).as_deref(),
            Some("000000015000000000000000")
        );
        assert_eq!(
            hex("-250", Size::Packed).as_deref(),
            Some("800200025000000000000000")
        );
        assert_eq!(
            hex("0.25", Size::Packed).as_deref(),
            Some("400100025000000000000000")
        );
    }
}
