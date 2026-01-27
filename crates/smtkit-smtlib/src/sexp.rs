//! Tiny s-expression utilities for SMT-LIB2 I/O.

use std::fmt;

/// A minimal s-expression AST (atoms + lists).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sexp {
    /// An atom (symbol, numeral, keyword, etc).
    Atom(String),
    /// A list: `(a b c)`.
    List(Vec<Sexp>),
}

impl Sexp {
    /// Create an atom.
    pub fn atom(s: impl Into<String>) -> Self {
        Self::Atom(s.into())
    }

    /// Create a list.
    pub fn list(items: impl Into<Vec<Sexp>>) -> Self {
        Self::List(items.into())
    }
}

impl fmt::Display for Sexp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Sexp::Atom(a) => write!(f, "{a}"),
            Sexp::List(items) => {
                write!(f, "(")?;
                for (i, it) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{it}")?;
                }
                write!(f, ")")
            }
        }
    }
}

/// Parse a single s-expression from `input`.
///
/// This is intentionally small: enough to parse solver output like:
/// - `sat`
/// - `unsat`
/// - `((x 5) (y 3))`
pub fn parse_one(input: &str) -> Result<Sexp, ParseError> {
    let mut p = Parser::new(input);
    let sexp = p.parse_sexp()?;
    p.skip_ws();
    if p.peek().is_some() {
        return Err(ParseError::Trailing);
    }
    Ok(sexp)
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("unexpected end of input")]
    Eof,
    #[error("unexpected token: {0}")]
    Unexpected(char),
    #[error("trailing input after s-expression")]
    Trailing,
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            s: s.as_bytes(),
            i: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.i += 1;
        Some(b)
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            match b {
                b' ' | b'\t' | b'\n' | b'\r' => {
                    self.i += 1;
                }
                // SMT-LIB line comment: from ';' to end of line.
                b';' => {
                    self.i += 1;
                    while let Some(c) = self.peek() {
                        self.i += 1;
                        if c == b'\n' {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
    }

    fn parse_sexp(&mut self) -> Result<Sexp, ParseError> {
        self.skip_ws();
        match self.peek() {
            None => Err(ParseError::Eof),
            Some(b'(') => self.parse_list(),
            Some(_) => self.parse_atom(),
        }
    }

    fn parse_list(&mut self) -> Result<Sexp, ParseError> {
        match self.bump() {
            Some(b'(') => {}
            Some(c) => return Err(ParseError::Unexpected(c as char)),
            None => return Err(ParseError::Eof),
        }
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => return Err(ParseError::Eof),
                Some(b')') => {
                    self.i += 1;
                    break;
                }
                _ => items.push(self.parse_sexp()?),
            }
        }
        Ok(Sexp::List(items))
    }

    fn parse_atom(&mut self) -> Result<Sexp, ParseError> {
        self.skip_ws();
        match self.peek() {
            Some(b'|') => return self.parse_bar_quoted_symbol(),
            Some(b'"') => return self.parse_string_literal(),
            _ => {}
        }
        let start = self.i;
        while let Some(b) = self.peek() {
            match b {
                b'(' | b')' | b' ' | b'\t' | b'\n' | b'\r' | b';' => break,
                _ => self.i += 1,
            }
        }
        if self.i == start {
            return Err(ParseError::Eof);
        }
        let s = std::str::from_utf8(&self.s[start..self.i]).map_err(|_| ParseError::Eof)?;
        Ok(Sexp::Atom(s.to_string()))
    }

    /// Parse an SMT-LIB `|...|`-quoted symbol as a single atom.
    ///
    /// Note: we preserve the quotes in the returned atom string (e.g. `|a b|`) so `Display`
    /// can round-trip it without needing a richer AST.
    fn parse_bar_quoted_symbol(&mut self) -> Result<Sexp, ParseError> {
        self.skip_ws();
        let start = self.i;
        match self.bump() {
            Some(b'|') => {}
            Some(c) => return Err(ParseError::Unexpected(c as char)),
            None => return Err(ParseError::Eof),
        }
        loop {
            match self.peek() {
                None => return Err(ParseError::Eof),
                Some(b'|') => {
                    // Support `||` to embed a literal `|` inside a quoted symbol.
                    if self.s.get(self.i + 1) == Some(&b'|') {
                        self.i += 2;
                        continue;
                    }
                    self.i += 1;
                    break;
                }
                Some(_) => {
                    self.i += 1;
                }
            }
        }
        let s = std::str::from_utf8(&self.s[start..self.i]).map_err(|_| ParseError::Eof)?;
        Ok(Sexp::Atom(s.to_string()))
    }

    /// Parse an SMT-LIB string literal as a single atom.
    ///
    /// SMT-LIB uses `""` to represent a literal `"` inside a string. We also tolerate `\` escapes
    /// for robustness against solver-specific output.
    ///
    /// Note: we preserve the quotes in the returned atom string (e.g. `"hello world"`).
    fn parse_string_literal(&mut self) -> Result<Sexp, ParseError> {
        self.skip_ws();
        let start = self.i;
        match self.bump() {
            Some(b'"') => {}
            Some(c) => return Err(ParseError::Unexpected(c as char)),
            None => return Err(ParseError::Eof),
        }
        loop {
            match self.peek() {
                None => return Err(ParseError::Eof),
                Some(b'"') => {
                    // SMT-LIB escape for quote is `""`.
                    if self.s.get(self.i + 1) == Some(&b'"') {
                        self.i += 2;
                        continue;
                    }
                    self.i += 1;
                    break;
                }
                Some(b'\\') => {
                    // Best-effort: consume escape + following byte if present.
                    self.i += 1;
                    if self.peek().is_some() {
                        self.i += 1;
                    }
                }
                Some(_) => {
                    self.i += 1;
                }
            }
        }
        let s = std::str::from_utf8(&self.s[start..self.i]).map_err(|_| ParseError::Eof)?;
        Ok(Sexp::Atom(s.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_atom_with_comment_skips_comment() {
        let s = parse_one("(a b) ; comment here\n").unwrap();
        assert_eq!(s.to_string(), "(a b)");
    }

    #[test]
    fn parse_bar_quoted_symbol_atom() {
        let s = parse_one("(|a b| c)").unwrap();
        assert_eq!(s.to_string(), "(|a b| c)");
    }

    #[test]
    fn parse_string_literal_atom_with_space() {
        let s = parse_one("(\"hello world\" 3)").unwrap();
        assert_eq!(s.to_string(), "(\"hello world\" 3)");
    }

    #[test]
    fn parse_string_literal_atom_with_embedded_quote() {
        let s = parse_one("(\"a\"\"b\" x)").unwrap();
        assert_eq!(s.to_string(), "(\"a\"\"b\" x)");
    }
}
