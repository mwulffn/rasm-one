//! Lexer for a single source line.
//!
//! Lexing is on demand because whitespace ends the operand field and whatever
//! follows is a free-form comment that need not be tokenisable.

/// A punctuation or operator token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Punct {
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Dot,
    Hash,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Shl,
    Shr,
    Amp,
    AmpAmp,
    Caret,
    Pipe,
    PipePipe,
    Bang,
    BangEq,
    Tilde,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    EqEq,
    LtGt,
}

/// A token. `End` means whitespace, a `;` comment or the end of the line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Tok<'a> {
    End,
    Ident(&'a str),
    Num(i64),
    Str(Vec<u8>),
    Punct(Punct),
}

/// A lexical error: its message and 0-based column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LexError {
    pub message: String,
    pub pos: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_ident_cont(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a [u8]) -> Self {
        Self { src, pos: 0 }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn set_pos(&mut self, pos: usize) {
        self.pos = pos;
    }

    pub fn at_blank(&self) -> bool {
        matches!(self.src.get(self.pos), Some(b' ' | b'\t'))
    }

    pub fn skip_blanks(&mut self) {
        while self.at_blank() {
            self.pos += 1;
        }
    }

    /// Reads an unquoted word, which ends at a blank, `;` or the end of the line.
    pub fn word(&mut self) -> &'a [u8] {
        let start = self.pos;
        while self
            .src
            .get(self.pos)
            .is_some_and(|c| !matches!(c, b' ' | b'\t' | b';'))
        {
            self.pos += 1;
        }
        &self.src[start..self.pos]
    }

    /// Everything left on the line.
    pub fn rest(&self) -> &'a [u8] {
        self.src.get(self.pos..).unwrap_or(&[])
    }

    pub fn peek(&self) -> Result<Tok<'a>, LexError> {
        self.clone().next_tok()
    }

    pub fn next_tok(&mut self) -> Result<Tok<'a>, LexError> {
        let Some(&c) = self.src.get(self.pos) else {
            return Ok(Tok::End);
        };
        if matches!(c, b' ' | b'\t' | b';') {
            return Ok(Tok::End);
        }
        if is_ident_start(c) {
            return Ok(self.ident());
        }
        match c {
            b'0'..=b'9' if self.is_numeric_local_label() => Ok(self.ident()),
            b'0'..=b'9' => self.number(10),
            b'$' => {
                self.pos += 1;
                self.number(16)
            }
            b'@' => {
                self.pos += 1;
                self.number(8)
            }
            b'"' | b'\'' => self.string(c),
            _ => self.punct(c),
        }
    }

    /// Reads digits in `radix` after an operator-like prefix, such as the
    /// binary `%`, which only the parser can tell apart from modulo.
    pub fn number(&mut self, radix: u32) -> Result<Tok<'a>, LexError> {
        let start = self.pos;
        let mut value: i64 = 0;
        while let Some(digit) = self
            .src
            .get(self.pos)
            .and_then(|&c| (c as char).to_digit(36))
        {
            if digit >= radix {
                return Err(self.error_at("invalid digit in number", self.pos));
            }
            // Values up to 64 bits are valid; wider ones are not representable.
            let (next, overflow) = (value as u64).overflowing_mul(u64::from(radix));
            let (next, carry) = next.overflowing_add(u64::from(digit));
            if overflow || carry {
                return Err(self.error_at("number too large", start));
            }
            value = next as i64;
            self.pos += 1;
        }
        if self.pos == start {
            return Err(self.error_at("number expected", start));
        }
        Ok(Tok::Num(value))
    }

    /// True at digits ending in `$`, such as the local label `1$`.
    fn is_numeric_local_label(&self) -> bool {
        let rest = self.rest();
        let digits = rest.iter().take_while(|c| c.is_ascii_digit()).count();
        rest.get(digits) == Some(&b'$')
    }

    fn ident(&mut self) -> Tok<'a> {
        let start = self.pos;
        while self.src.get(self.pos).is_some_and(|&c| is_ident_cont(c)) {
            self.pos += 1;
        }
        // A final `$` makes it a local label.
        if self.src.get(self.pos) == Some(&b'$') {
            self.pos += 1;
        }
        // Identifier bytes are all ASCII, so this cannot fail.
        Tok::Ident(std::str::from_utf8(&self.src[start..self.pos]).unwrap_or(""))
    }

    /// Reads a string. A doubled quote stands for one quote; there are no
    /// backslash escapes.
    fn string(&mut self, quote: u8) -> Result<Tok<'a>, LexError> {
        let start = self.pos;
        self.pos += 1;
        let mut bytes = Vec::new();
        loop {
            match self.src.get(self.pos) {
                None => return Err(self.error_at("unterminated string", start)),
                Some(&c) if c == quote => {
                    if self.src.get(self.pos + 1) == Some(&quote) {
                        bytes.push(quote);
                        self.pos += 2;
                    } else {
                        self.pos += 1;
                        return Ok(Tok::Str(bytes));
                    }
                }
                Some(&c) => {
                    bytes.push(c);
                    self.pos += 1;
                }
            }
        }
    }

    fn punct(&mut self, c: u8) -> Result<Tok<'a>, LexError> {
        let next = self.src.get(self.pos + 1).copied();
        let (punct, len) = match (c, next) {
            (b'<', Some(b'<')) => (Punct::Shl, 2),
            (b'<', Some(b'=')) => (Punct::Le, 2),
            (b'<', Some(b'>')) => (Punct::LtGt, 2),
            (b'>', Some(b'>')) => (Punct::Shr, 2),
            (b'>', Some(b'=')) => (Punct::Ge, 2),
            (b'=', Some(b'=')) => (Punct::EqEq, 2),
            (b'!', Some(b'=')) => (Punct::BangEq, 2),
            (b'&', Some(b'&')) => (Punct::AmpAmp, 2),
            (b'|', Some(b'|')) => (Punct::PipePipe, 2),
            (b'[', _) => (Punct::LBracket, 1),
            (b']', _) => (Punct::RBracket, 1),
            (b'{', _) => (Punct::LBrace, 1),
            (b'}', _) => (Punct::RBrace, 1),
            (b'(', _) => (Punct::LParen, 1),
            (b')', _) => (Punct::RParen, 1),
            (b',', _) => (Punct::Comma, 1),
            (b':', _) => (Punct::Colon, 1),
            (b'.', _) => (Punct::Dot, 1),
            (b'#', _) => (Punct::Hash, 1),
            (b'+', _) => (Punct::Plus, 1),
            (b'-', _) => (Punct::Minus, 1),
            (b'*', _) => (Punct::Star, 1),
            (b'/', _) => (Punct::Slash, 1),
            (b'%', _) => (Punct::Percent, 1),
            (b'&', _) => (Punct::Amp, 1),
            (b'^', _) => (Punct::Caret, 1),
            (b'|', _) => (Punct::Pipe, 1),
            (b'!', _) => (Punct::Bang, 1),
            (b'~', _) => (Punct::Tilde, 1),
            (b'<', _) => (Punct::Lt, 1),
            (b'>', _) => (Punct::Gt, 1),
            (b'=', _) => (Punct::Eq, 1),
            _ => return Err(self.error_at("unexpected character", self.pos)),
        };
        self.pos += len;
        Ok(Tok::Punct(punct))
    }

    fn error_at(&self, message: &str, pos: usize) -> LexError {
        LexError {
            message: message.to_string(),
            pos,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_next_tok_blank_ends_field_before_untokenisable_comment() -> Result<(), LexError> {
        let mut lexer = Lexer::new(b"d0 it's a comment");
        assert_eq!(lexer.next_tok()?, Tok::Ident("d0"));
        assert_eq!(lexer.next_tok()?, Tok::End);
        Ok(())
    }

    #[test]
    fn test_next_tok_number_radixes() -> Result<(), LexError> {
        assert_eq!(Lexer::new(b"$fF").next_tok()?, Tok::Num(255));
        assert_eq!(Lexer::new(b"@17").next_tok()?, Tok::Num(15));
        assert_eq!(Lexer::new(b"10").next_tok()?, Tok::Num(10));
        Ok(())
    }

    #[test]
    fn test_next_tok_local_labels_are_identifiers() -> Result<(), LexError> {
        assert_eq!(Lexer::new(b"l$").next_tok()?, Tok::Ident("l$"));
        assert_eq!(Lexer::new(b"12$").next_tok()?, Tok::Ident("12$"));
        Ok(())
    }

    #[test]
    fn test_next_tok_number_followed_by_letters_is_error() {
        assert!(Lexer::new(b"1abc").next_tok().is_err());
    }

    #[test]
    fn test_next_tok_string_doubled_quote_and_no_escapes() -> Result<(), LexError> {
        assert_eq!(
            Lexer::new(br#""a""\n""#).next_tok()?,
            Tok::Str(br#"a"\n"#.to_vec())
        );
        Ok(())
    }

    #[test]
    fn test_next_tok_unterminated_string_is_error() {
        assert!(Lexer::new(b"\"a").next_tok().is_err());
    }
}
