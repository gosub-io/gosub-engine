//! Parsing and evaluation of CSS `calc()` expressions for the Taffy layouter.
//!
//! Taffy's `calc` feature lets us encode a `calc()` value as an opaque pointer in the
//! [`Dimension`]/[`LengthPercentage`]/[`LengthPercentageAuto`] tagged-pointer types. Taffy then
//! calls back into [`LayoutPartialTree::resolve_calc_value`] with that pointer and a basis size
//! when it needs a concrete f32. The pointer must be non-null and 8-byte aligned (Taffy uses the
//! low 3 bits as a tag).
//!
//! We parse the calc expression once (when styles are converted to Taffy [`Style`]s), box it
//! into a [`CalcExpr`] (which is forced to 8-byte alignment), and store the box in the per-node
//! layout cache so the pointer stays valid for the duration of layout.
//!
//! [`Dimension`]: taffy::Dimension
//! [`LengthPercentage`]: taffy::LengthPercentage
//! [`LengthPercentageAuto`]: taffy::LengthPercentageAuto
//! [`LayoutPartialTree::resolve_calc_value`]: taffy::LayoutPartialTree::resolve_calc_value
//! [`Style`]: taffy::Style

use std::iter::Peekable;
use std::str::Chars;

/// A parsed CSS `calc()` expression.
///
/// Forced to 8-byte alignment so the raw pointer we hand to Taffy has its low 3 bits clear,
/// satisfying the contract on `Dimension::calc(ptr)` and friends.
#[repr(C, align(8))]
#[derive(Debug, Clone, PartialEq)]
pub struct CalcExpr(pub CalcNode);

impl CalcExpr {
    /// Evaluate the expression against `basis` (the size used to resolve percentages).
    pub fn resolve(&self, basis: f32) -> f32 {
        self.0.resolve(basis)
    }
}

/// A node in a parsed calc expression tree.
#[derive(Debug, Clone, PartialEq)]
pub enum CalcNode {
    /// An absolute length, already converted to pixels.
    Length(f32),
    /// A percentage of the basis, stored as a fraction in `[0.0, 1.0]`.
    Percent(f32),
    /// A dimensionless number (used as a multiplier).
    Number(f32),
    Add(Box<CalcNode>, Box<CalcNode>),
    Sub(Box<CalcNode>, Box<CalcNode>),
    Mul(Box<CalcNode>, Box<CalcNode>),
    Div(Box<CalcNode>, Box<CalcNode>),
}

impl CalcNode {
    fn resolve(&self, basis: f32) -> f32 {
        match self {
            CalcNode::Length(v) => *v,
            CalcNode::Percent(p) => p * basis,
            CalcNode::Number(n) => *n,
            CalcNode::Add(l, r) => l.resolve(basis) + r.resolve(basis),
            CalcNode::Sub(l, r) => l.resolve(basis) - r.resolve(basis),
            CalcNode::Mul(l, r) => l.resolve(basis) * r.resolve(basis),
            CalcNode::Div(l, r) => {
                let rhs = r.resolve(basis);
                if rhs == 0.0 {
                    0.0
                } else {
                    l.resolve(basis) / rhs
                }
            }
        }
    }
}

/// Parse a calc body (the contents of `calc(...)`, with or without the wrapper).
///
/// Returns `None` if the input cannot be parsed into a valid calc expression.
pub fn parse(input: &str) -> Option<CalcExpr> {
    let body = strip_wrapper(input);
    let mut tokens = Tokenizer::new(body).peekable();
    let node = parse_expr(&mut tokens)?;
    if tokens.peek().is_some() {
        return None;
    }
    Some(CalcExpr(node))
}

/// Resolve a raw pointer (as passed to Taffy via `Dimension::calc(ptr)`) back to a value.
///
/// # Safety
/// The pointer must originate from a live [`CalcExpr`] (one boxed and kept alive by the layout
/// cache). Callers receive these pointers only from `LayoutPartialTree::resolve_calc_value`
/// during layout, where this invariant holds.
#[allow(unsafe_code)]
pub fn resolve(ptr: *const (), basis: f32) -> f32 {
    if ptr.is_null() {
        return 0.0;
    }
    let expr = unsafe { &*(ptr as *const CalcExpr) };
    expr.resolve(basis)
}

fn strip_wrapper(input: &str) -> &str {
    let trimmed = input.trim();
    // Be lenient: the body we receive may or may not include the surrounding `calc(...)`,
    // and our css3 parser is known to slice through the closing paren.
    let without_prefix = trimmed
        .strip_prefix("calc(")
        .or_else(|| trimmed.strip_prefix("CALC("))
        .unwrap_or(trimmed);
    without_prefix.strip_suffix(')').unwrap_or(without_prefix).trim()
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f32, Unit),
    Plus,
    Minus,
    Star,
    Slash,
    LParen,
    RParen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    None,
    Px,
    Em,
    Rem,
    Percent,
    /// Any other unit we don't yet handle (treated as a unitless number).
    Other,
}

struct Tokenizer<'a> {
    chars: Peekable<Chars<'a>>,
}

impl<'a> Tokenizer<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            chars: input.chars().peekable(),
        }
    }

    fn tokenize_number(&mut self, first: char) -> Option<Token> {
        let mut num_str = String::new();
        num_str.push(first);
        while let Some(&c) = self.chars.peek() {
            if c.is_ascii_digit() || c == '.' {
                num_str.push(c);
                self.chars.next();
            } else {
                break;
            }
        }
        let value: f32 = num_str.parse().ok()?;

        let mut unit_str = String::new();
        while let Some(&c) = self.chars.peek() {
            if c.is_ascii_alphabetic() || c == '%' {
                unit_str.push(c);
                self.chars.next();
            } else {
                break;
            }
        }
        let unit = match unit_str.as_str() {
            "" => Unit::None,
            "px" => Unit::Px,
            "em" => Unit::Em,
            "rem" => Unit::Rem,
            "%" => Unit::Percent,
            _ => Unit::Other,
        };
        Some(Token::Number(value, unit))
    }
}

impl Iterator for Tokenizer<'_> {
    type Item = Token;

    fn next(&mut self) -> Option<Token> {
        loop {
            let c = self.chars.next()?;
            if c.is_ascii_whitespace() {
                continue;
            }
            return Some(match c {
                '+' => Token::Plus,
                '-' => Token::Minus,
                '*' => Token::Star,
                '/' => Token::Slash,
                '(' => Token::LParen,
                ')' => Token::RParen,
                c if c.is_ascii_digit() || c == '.' => self.tokenize_number(c)?,
                _ => continue,
            });
        }
    }
}

type TokenIter<'a> = Peekable<Tokenizer<'a>>;

fn parse_expr(tokens: &mut TokenIter<'_>) -> Option<CalcNode> {
    let mut lhs = parse_term(tokens)?;
    while let Some(token) = tokens.peek() {
        let op = match token {
            Token::Plus => Token::Plus,
            Token::Minus => Token::Minus,
            _ => break,
        };
        tokens.next();
        let rhs = parse_term(tokens)?;
        lhs = match op {
            Token::Plus => CalcNode::Add(Box::new(lhs), Box::new(rhs)),
            Token::Minus => CalcNode::Sub(Box::new(lhs), Box::new(rhs)),
            _ => unreachable!(),
        };
    }
    Some(lhs)
}

fn parse_term(tokens: &mut TokenIter<'_>) -> Option<CalcNode> {
    let mut lhs = parse_atom(tokens)?;
    while let Some(token) = tokens.peek() {
        let op = match token {
            Token::Star => Token::Star,
            Token::Slash => Token::Slash,
            _ => break,
        };
        tokens.next();
        let rhs = parse_atom(tokens)?;
        lhs = match op {
            Token::Star => CalcNode::Mul(Box::new(lhs), Box::new(rhs)),
            Token::Slash => CalcNode::Div(Box::new(lhs), Box::new(rhs)),
            _ => unreachable!(),
        };
    }
    Some(lhs)
}

fn parse_atom(tokens: &mut TokenIter<'_>) -> Option<CalcNode> {
    match tokens.next()? {
        Token::Number(value, unit) => Some(match unit {
            Unit::Px | Unit::Other => CalcNode::Length(value),
            Unit::Em | Unit::Rem => CalcNode::Length(value * 16.0),
            Unit::Percent => CalcNode::Percent(value / 100.0),
            Unit::None => CalcNode::Number(value),
        }),
        Token::LParen => {
            let inner = parse_expr(tokens)?;
            match tokens.next()? {
                Token::RParen => Some(inner),
                _ => None,
            }
        }
        // Unary minus: parse as `0 - atom`.
        Token::Minus => {
            let inner = parse_atom(tokens)?;
            Some(CalcNode::Sub(Box::new(CalcNode::Number(0.0)), Box::new(inner)))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(input: &str, basis: f32) -> f32 {
        parse(input).expect("parse failed").resolve(basis)
    }

    #[test]
    fn plain_length() {
        assert_eq!(r("calc(10px)", 100.0), 10.0);
    }

    #[test]
    fn addition() {
        assert_eq!(r("calc(10px + 20px)", 0.0), 30.0);
    }

    #[test]
    fn percentage_against_basis() {
        assert_eq!(r("calc(50%)", 200.0), 100.0);
    }

    #[test]
    fn mixed_percentage_and_length() {
        assert_eq!(r("calc(50% - 10px)", 200.0), 90.0);
    }

    #[test]
    fn precedence() {
        // 10px + 2 * 5px = 20px
        assert_eq!(r("calc(10px + 2 * 5px)", 0.0), 20.0);
    }

    #[test]
    fn parentheses() {
        // (10px + 2) * 5px = 12 * 5 = 60
        assert_eq!(r("calc((10px + 2) * 5px)", 0.0), 60.0);
    }

    #[test]
    fn alignment_is_eight() {
        let boxed = Box::new(CalcExpr(CalcNode::Length(1.0)));
        let ptr = (&*boxed) as *const CalcExpr as usize;
        assert_eq!(ptr & 0b111, 0, "CalcExpr pointer must be 8-byte aligned");
    }

    #[test]
    fn body_without_wrapper() {
        // We accept the body alone as well.
        assert_eq!(r("10px + 20px", 0.0), 30.0);
    }
}
