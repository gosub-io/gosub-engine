//! `calc()` arithmetic.
//!
//! The parser keeps a `calc()` body as raw text (see [`crate::parser::calc`]) and nothing ever
//! did anything with it: `width: calc(10px + 20px)` reached layout still spelled
//! `calc(10px + 20px)`, and every consumer that wanted a length got a string it could not use.
//! This is the arithmetic that was missing.
//!
//! Evaluation happens twice, against different knowledge:
//!
//! * When a declaration is parsed, only the maths is known. Absolute lengths collapse (`1in` and
//!   `1px` are the same kind of thing), but `em` has no value yet and a percentage has no
//!   containing block, so those stay symbolic. `calc(1in + 1px)` becomes `calc(97px)`;
//!   `calc(50px + 40%)` stays a sum of two terms.
//! * When a property is computed, the element's font-size and the viewport are known, so `em`,
//!   `rem` and the viewport units resolve too. What is left is usually a single term, and a
//!   `calc()` that has come down to one value *is* that value - `width: calc(2em + 10px)` on a
//!   20px element computes to `50px`, not to `calc(50px)`.
//!
//! Percentages never resolve here. They need a containing block, which is layout's to know.
//!
//! # Representation
//!
//! css-values-4 defines simplification as reducing to a sum with one term per unit, and that is
//! what [`Sum`] is. Multiplication is only defined when one side is a plain number, and division
//! only by a plain number, so a term never needs an exponent - `px²` cannot arise from valid CSS.
//!
//! The result goes back into the same `CssValue::Function("calc", [String(body)])` the parser
//! produced, with the body rewritten in canonical form. Giving `CssValue` a typed variant would
//! be tidier, but `calc()` flows through the syntax matcher as a function like any other, and
//! every `match` over `CssValue` in the workspace would have to grow an arm to gain nothing that
//! is not already recoverable by re-reading the (now canonical, and short) body.

use cow_utils::CowUtils;
use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::stylesheet::CssValue;

/// What the relative units are worth at the point of evaluation.
///
/// Everything is optional because the same evaluator runs before the cascade knows any of it.
/// An unresolvable unit is not an error - the term simply survives into the output.
#[derive(Clone, Copy, Debug, Default)]
pub struct Units {
    /// px per `em`: the element's own computed font-size.
    pub em_px: Option<f32>,
    /// px per `rem`: the root element's computed font-size.
    pub rem_px: Option<f32>,
    /// Whether `vw` and friends may be resolved against the current layout viewport. False while
    /// parsing, where a resize must still be able to invalidate the declaration.
    pub viewport: bool,
}

impl Units {
    /// The knowledge available when a stylesheet is parsed: arithmetic only.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// The knowledge available when a property is computed.
    #[must_use]
    pub fn computed(em_px: f32, rem_px: f32) -> Self {
        Self {
            em_px: Some(em_px),
            rem_px: Some(rem_px),
            viewport: true,
        }
    }
}

/// A simplified `calc()` body: a sum of terms, at most one per unit.
///
/// Terms are keyed by unit, with `""` for a plain number and `"%"` for a percentage. The map is
/// ordered, and that ordering is the serialization css-values-4 asks for - number, then
/// percentage, then dimensions by unit - because `""` sorts before `"%"` sorts before any
/// letter.
#[derive(Clone, Debug, PartialEq)]
pub struct Sum {
    terms: BTreeMap<String, f32>,
}

impl Sum {
    fn term(unit: &str, value: f32) -> Self {
        let mut terms = BTreeMap::new();
        terms.insert(unit.to_string(), value);
        Self { terms }
    }

    /// The coefficient when this is a plain number and nothing else, which is what
    /// multiplication and division require of one of their operands.
    fn as_number(&self) -> Option<f32> {
        match self.terms.len() {
            1 => self.terms.get("").copied(),
            _ => None,
        }
    }

    fn add(mut self, other: &Self, sign: f32) -> Self {
        for (unit, value) in &other.terms {
            *self.terms.entry(unit.clone()).or_insert(0.0) += value * sign;
        }
        self
    }

    fn scale(mut self, factor: f32) -> Self {
        for value in self.terms.values_mut() {
            *value *= factor;
        }
        self
    }

    /// A number added to anything else is a type error - `calc(1px + 1)` is not valid CSS - and
    /// emitting it as a simplified body would invent a value nobody wrote.
    fn is_well_typed(&self) -> bool {
        self.terms.len() == 1 || !self.terms.contains_key("")
    }

    /// The single value this sum came down to, if it came down to one.
    #[must_use]
    pub fn single_value(&self) -> Option<CssValue> {
        let (unit, value) = match self.terms.len() {
            1 => self.terms.iter().next()?,
            _ => return None,
        };
        Some(match unit.as_str() {
            "" => CssValue::Number(*value),
            "%" => CssValue::Percentage(*value),
            unit => CssValue::Unit(*value, unit.to_string()),
        })
    }

    /// The unit and coefficient this sum came down to, if it came down to one term.
    fn single_term(&self) -> Option<(String, f32)> {
        match self.terms.len() {
            1 => self.terms.iter().next().map(|(unit, value)| (unit.clone(), *value)),
            _ => None,
        }
    }

    /// The body text, without the surrounding `calc(` and `)`.
    #[must_use]
    pub fn serialize(&self) -> String {
        let mut out = String::new();
        for (i, (unit, value)) in self.terms.iter().enumerate() {
            // A non-finite term carries its sign inside the keyword, so it is never written as
            // the right-hand side of a subtraction.
            if i > 0 && value.is_finite() && *value < 0.0 {
                let _ = write!(out, " - {}", format_term(-*value, unit));
                continue;
            }
            if i > 0 {
                out.push_str(" + ");
            }
            let _ = write!(out, "{}", format_term(*value, unit));
        }
        out
    }
}

fn format_term(value: f32, unit: &str) -> String {
    // css-values-4 serializes a non-finite dimension as a product with a one-unit multiplier -
    // `calc(NaN * 1px)`, never `NaNpx` - because `NaN` and `infinity` are `<number>` keywords
    // and cannot carry a unit themselves. A plain number is just the keyword.
    if !value.is_finite() {
        let keyword = if value.is_nan() {
            "NaN"
        } else if value > 0.0 {
            "infinity"
        } else {
            "-infinity"
        };
        return match unit {
            "" => keyword.to_string(),
            unit => format!("{keyword} * 1{unit}"),
        };
    }
    match unit {
        "" => format!("{value}"),
        unit => format!("{value}{unit}"),
    }
}

/// Simplify the body of a `calc()`, or `None` when it cannot be reduced at all.
///
/// `None` means "leave the value exactly as it was": an unknown function inside the body, a
/// `var()` that has not been substituted, a malformed expression. None of those are reported as
/// errors here - rejecting a declaration is the matcher's job, and this stage has to be safe to
/// run over anything that parsed.
#[must_use]
pub fn simplify(body: &str, units: &Units) -> Option<Sum> {
    let tokens = lex(body)?;
    let mut parser = Parser {
        tokens: &tokens,
        pos: 0,
        units,
    };
    let sum = parser.sum()?;
    if parser.pos != parser.tokens.len() {
        return None;
    }
    sum.is_well_typed().then_some(sum)
}

/// Evaluate a `calc()` body into the value it should become.
///
/// Collapses to a bare value when the sum has one term and `unwrap` is set - which is right for a
/// computed value, where `calc(50px)` *is* `50px`, and wrong for a specified one, where the
/// `calc()` wrapper is part of what the author wrote and what the CSSOM must give back.
#[must_use]
pub fn evaluate(body: &str, units: &Units, unwrap: bool) -> Option<CssValue> {
    let sum = simplify(body, units)?;
    if unwrap {
        if let Some(value) = sum.single_value() {
            return Some(make_finite(value));
        }
    }
    Some(CssValue::Function(
        "calc".to_string(),
        vec![CssValue::String(sum.serialize())],
    ))
}

/// The largest length this engine will admit, which is what an infinity becomes once a value has
/// to be a real number.
///
/// css-values-4 says an infinite computed value is clamped to "the maximum value the UA
/// supports" without naming one, so this matches what Chrome uses. It has to be far enough below
/// `f32::MAX` that arithmetic downstream - a layout adding two of them - cannot overflow back to
/// infinity, and far enough above any real page that clamping is never visible.
pub const MAX_FINITE: f32 = 33_554_428.0;

/// Replace a non-finite computed value with the finite one css-values-4 requires.
///
/// A specified value keeps `NaN` and `infinity` (they serialize as keywords, and the
/// `calc-infinity-nan-serialize-*` suites check exactly that), but a *computed* value is a real
/// number that layout will do arithmetic on. NaN becomes zero; an infinity is clamped.
///
/// The zero is a plain `0px` whatever unit the NaN carried, which is what a browser reports for
/// `width: calc(NaN * 1%)` - there is nothing left to take a percentage of.
fn make_finite(value: CssValue) -> CssValue {
    match value {
        CssValue::Number(n) if !n.is_finite() => CssValue::Number(finite(n)),
        CssValue::Percentage(p) if !p.is_finite() => {
            if p.is_nan() {
                CssValue::Unit(0.0, "px".to_string())
            } else {
                CssValue::Percentage(finite(p))
            }
        }
        CssValue::Unit(n, unit) if !n.is_finite() => {
            if n.is_nan() {
                CssValue::Unit(0.0, "px".to_string())
            } else {
                CssValue::Unit(finite(n), unit)
            }
        }
        other => other,
    }
}

fn finite(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else if value > 0.0 {
        MAX_FINITE
    } else {
        -MAX_FINITE
    }
}

/// How many of the canonical unit one of `unit` is worth, and which canonical unit that is.
///
/// Returns `None` for a unit this cannot reduce (`ch`, `lh`, the container-query units), which
/// keeps it as its own term rather than guessing at a value.
fn canonical(unit: &str, units: &Units) -> Option<(String, f32)> {
    let px = |factor: f32| Some(("px".to_string(), factor));
    match unit {
        "px" => px(1.0),
        // 1in is 96px by definition, and every other absolute length is a fraction of an inch.
        "in" => px(96.0),
        "pt" => px(96.0 / 72.0),
        "pc" => px(96.0 / 6.0),
        "cm" => px(96.0 / 2.54),
        "mm" => px(96.0 / 25.4),
        "q" => px(96.0 / 101.6),
        "em" => units.em_px.and_then(px),
        "rem" => units.rem_px.and_then(px),
        "vw" | "svw" | "lvw" | "dvw" => units.viewport.then(|| viewport().0 / 100.0).and_then(px),
        "vh" | "svh" | "lvh" | "dvh" => units.viewport.then(|| viewport().1 / 100.0).and_then(px),
        "vmin" => units
            .viewport
            .then(|| {
                let (w, h) = viewport();
                w.min(h) / 100.0
            })
            .and_then(px),
        "vmax" => units
            .viewport
            .then(|| {
                let (w, h) = viewport();
                w.max(h) / 100.0
            })
            .and_then(px),
        "deg" => Some(("deg".to_string(), 1.0)),
        "grad" => Some(("deg".to_string(), 0.9)),
        "rad" => Some(("deg".to_string(), 180.0 / std::f32::consts::PI)),
        "turn" => Some(("deg".to_string(), 360.0)),
        "s" => Some(("s".to_string(), 1.0)),
        "ms" => Some(("s".to_string(), 0.001)),
        "hz" => Some(("hz".to_string(), 1.0)),
        "khz" => Some(("hz".to_string(), 1000.0)),
        "dppx" | "x" => Some(("dppx".to_string(), 1.0)),
        "dpi" => Some(("dppx".to_string(), 1.0 / 96.0)),
        "dpcm" => Some(("dppx".to_string(), 2.54 / 96.0)),
        _ => None,
    }
}

fn viewport() -> (f32, f32) {
    let env = crate::media_query::media_environment();
    (env.width, env.height)
}

// --- lexing ----------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    /// A number, with its unit: `""` plain, `"%"` a percentage, else a dimension.
    Value(f32, String),
    Plus,
    Minus,
    Star,
    Slash,
    /// `(` or `calc(` - the two are the same thing to the grammar.
    Open,
    /// A comparison function that takes this expression as one of its arguments.
    Func(String),
    Comma,
    Close,
}

/// The numeric constants css-values-4 allows wherever a `<number>` may appear inside a math
/// function. They are keywords rather than identifiers, and ASCII case-insensitive - `nan`,
/// `NaN` and `nAn` are the same token.
fn constant(name: &str) -> Option<f32> {
    match name {
        "pi" => Some(std::f32::consts::PI),
        "e" => Some(std::f32::consts::E),
        "infinity" => Some(f32::INFINITY),
        "nan" => Some(f32::NAN),
        _ => None,
    }
}

/// A token and whether whitespace came before it, which `+` and `-` need: CSS requires them to
/// be surrounded by whitespace precisely so that `calc(1px -2px)` is a syntax error rather than a
/// subtraction, since `-2px` on its own is a single number token.
#[derive(Debug, Clone)]
struct Lexed {
    tok: Tok,
    space_before: bool,
}

fn lex(body: &str) -> Option<Vec<Lexed>> {
    let bytes = body.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut space = false;

    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            space = true;
            i += 1;
            continue;
        }

        let tok = match c {
            b'(' => {
                i += 1;
                Tok::Open
            }
            b')' => {
                i += 1;
                Tok::Close
            }
            b',' => {
                i += 1;
                Tok::Comma
            }
            b'*' => {
                i += 1;
                Tok::Star
            }
            b'/' => {
                i += 1;
                Tok::Slash
            }
            // A sign only introduces a number when a number can start here; after a value it is
            // an operator. The parser sorts that out - both are emitted as operator tokens and
            // unary signs are handled by the grammar.
            b'+' => {
                i += 1;
                Tok::Plus
            }
            b'-' if !starts_number(&bytes[i..]) => {
                i += 1;
                Tok::Minus
            }
            // An identifier is either a numeric constant or the name of a nested function.
            // `calc()` is transparent - its parentheses are just parentheses - while the
            // comparison functions take an argument list and are folded when it closes.
            // Anything else (`var()`, `attr()`, a function this does not implement) means the
            // expression cannot be evaluated here, and the whole body is left alone.
            c if c.is_ascii_alphabetic() => {
                let (name, used) = scan_unit(&bytes[i..]);
                let opens = bytes.get(i + used) == Some(&b'(');
                i += used;
                match name.as_str() {
                    "calc" if opens => {
                        i += 1;
                        Tok::Open
                    }
                    "min" | "max" | "clamp" if opens => {
                        i += 1;
                        Tok::Func(name)
                    }
                    _ if opens => return None,
                    _ => Tok::Value(constant(&name)?, String::new()),
                }
            }
            _ => {
                let (value, rest) = scan_number(&bytes[i..])?;
                i += rest;
                let (unit, used) = scan_unit(&bytes[i..]);
                i += used;
                Tok::Value(value, unit)
            }
        };

        out.push(Lexed {
            tok,
            space_before: space,
        });
        space = false;
    }

    Some(out)
}

/// Whether a number token starts here, so that a leading `-` belongs to it rather than being an
/// operator.
fn starts_number(bytes: &[u8]) -> bool {
    match bytes {
        [b'-' | b'+', rest @ ..] => starts_number(rest),
        [b'.', d, ..] => d.is_ascii_digit(),
        [d, ..] => d.is_ascii_digit(),
        [] => false,
    }
}

/// Read a CSS `<number>`: an optional sign, digits around an optional point, an optional
/// exponent.
fn scan_number(bytes: &[u8]) -> Option<(f32, usize)> {
    let mut i = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        i += 1;
    }
    let digits_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i == digits_start {
        return None;
    }
    // An `e` only starts an exponent when digits follow it, else it is the start of a unit.
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        let mut j = i + 1;
        if matches!(bytes.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if bytes.get(j).is_some_and(u8::is_ascii_digit) {
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    let text = std::str::from_utf8(&bytes[..i]).ok()?;
    Some((text.parse::<f32>().ok()?, i))
}

/// Read the unit after a number: `%`, an identifier, or nothing.
fn scan_unit(bytes: &[u8]) -> (String, usize) {
    if bytes.first() == Some(&b'%') {
        return ("%".to_string(), 1);
    }
    let mut i = 0;
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b'_') {
        i += 1;
    }
    let unit = String::from_utf8_lossy(&bytes[..i])
        .cow_to_ascii_lowercase()
        .into_owned();
    (unit, i)
}

// --- parsing ---------------------------------------------------------------------------------

struct Parser<'a> {
    tokens: &'a [Lexed],
    pos: usize,
    units: &'a Units,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos).map(|l| &l.tok)
    }

    fn sum(&mut self) -> Option<Sum> {
        let mut acc = self.product()?;
        while let Some(lexed) = self.tokens.get(self.pos) {
            let sign = match lexed.tok {
                Tok::Plus => 1.0,
                Tok::Minus => -1.0,
                _ => break,
            };
            // Both sides, per css-values-4: without the space `calc(1px -2px)` is two adjacent
            // values, not a subtraction, and treating it as one would accept invalid CSS.
            if !lexed.space_before || !self.tokens.get(self.pos + 1).is_some_and(|next| next.space_before) {
                return None;
            }
            self.pos += 1;
            let rhs = self.product()?;
            acc = acc.add(&rhs, sign);
        }
        Some(acc)
    }

    fn product(&mut self) -> Option<Sum> {
        let mut acc = self.value()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Star) => Tok::Star,
                Some(Tok::Slash) => Tok::Slash,
                _ => break,
            };
            self.pos += 1;
            let rhs = self.value()?;
            acc = match op {
                // Only one side of a product may carry a unit; `px * px` has no meaning in CSS.
                Tok::Star => match (acc.as_number(), rhs.as_number()) {
                    (Some(n), _) => rhs.scale(n),
                    (None, Some(n)) => acc.scale(n),
                    (None, None) => return None,
                },
                // And you can only divide by a plain number - but dividing by *zero* is fine.
                // css-values-4 makes that infinity rather than an error, so `100px / 0` is a
                // valid length of `calc(infinity * 1px)`, and `100px * 0 / 0` is NaN. Rejecting
                // it here would have thrown away a declaration the spec says to keep.
                _ => acc.scale(1.0 / rhs.as_number()?),
            };
        }
        Some(acc)
    }

    fn value(&mut self) -> Option<Sum> {
        match self.peek()? {
            // A leading sign on a parenthesised group: `calc(-(1px + 2px))`.
            Tok::Plus => {
                self.pos += 1;
                self.value()
            }
            Tok::Minus => {
                self.pos += 1;
                Some(self.value()?.scale(-1.0))
            }
            Tok::Open => {
                self.pos += 1;
                let inner = self.sum()?;
                match self.peek() {
                    Some(Tok::Close) => self.pos += 1,
                    _ => return None,
                }
                Some(inner)
            }
            Tok::Value(value, unit) => {
                let (value, unit) = (*value, unit.clone());
                self.pos += 1;
                Some(match canonical(&unit, self.units) {
                    Some((canonical_unit, factor)) => Sum::term(&canonical_unit, value * factor),
                    // `%` and anything this does not know (`ch`, `cqw`, `lh`) stay as themselves.
                    None => Sum::term(&unit, value),
                })
            }
            Tok::Func(name) => {
                let name = name.clone();
                self.pos += 1;
                let mut args = Vec::new();
                loop {
                    args.push(self.sum()?);
                    match self.peek() {
                        Some(Tok::Comma) => self.pos += 1,
                        Some(Tok::Close) => {
                            self.pos += 1;
                            break;
                        }
                        _ => return None,
                    }
                }
                fold_comparison(&name, &args)
            }
            Tok::Star | Tok::Slash | Tok::Close | Tok::Comma => None,
        }
    }
}

/// Fold `min()`, `max()` or `clamp()` over arguments that have already been simplified.
///
/// Every argument has to have come down to a single term in the same unit - comparing a length
/// with a number is not a thing CSS can do, and a sum that still has two terms in it (a
/// percentage against a length, say) has no order yet either.
fn fold_comparison(name: &str, args: &[Sum]) -> Option<Sum> {
    let mut unit: Option<String> = None;
    let mut values = Vec::with_capacity(args.len());
    for arg in args {
        let (term_unit, value) = arg.single_term()?;
        if unit.get_or_insert_with(|| term_unit.clone()) != &term_unit {
            return None;
        }
        values.push(value);
    }
    let unit = unit?;

    // NaN is contagious through a comparison, which `f32::min` and `f32::max` are not: they are
    // defined to *ignore* it and return the other operand, so `max(NaN, 0)` would come out 0
    // where CSS requires NaN.
    if values.iter().any(|v| v.is_nan()) {
        return Some(Sum::term(&unit, f32::NAN));
    }

    let folded = match name {
        "min" => values.into_iter().reduce(f32::min)?,
        "max" => values.into_iter().reduce(f32::max)?,
        // clamp(MIN, VAL, MAX) is max(MIN, min(VAL, MAX)).
        "clamp" => match values[..] {
            [low, value, high] => value.min(high).max(low),
            _ => return None,
        },
        _ => return None,
    };
    Some(Sum::term(&unit, folded))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(input: &str, units: &Units) -> Option<String> {
        simplify(input, units).map(|sum| sum.serialize())
    }

    fn parsed(input: &str) -> Option<String> {
        body(input, &Units::none())
    }

    #[test]
    fn adds_and_subtracts_absolute_lengths() {
        assert_eq!(parsed("10px + 20px").as_deref(), Some("30px"));
        assert_eq!(parsed("10px - 20px").as_deref(), Some("-10px"));
        // 1in is 96px, so this is 97px - the point of canonicalising at parse time.
        assert_eq!(parsed("1in + 1px").as_deref(), Some("97px"));
    }

    #[test]
    fn multiplies_and_divides_by_numbers() {
        assert_eq!(parsed("2 * 50px").as_deref(), Some("100px"));
        assert_eq!(parsed("50px * 2").as_deref(), Some("100px"));
        assert_eq!(parsed("150px*2/3").as_deref(), Some("100px"));
        assert_eq!(parsed("(1px + 2px) * 3").as_deref(), Some("9px"));
    }

    #[test]
    fn rejects_products_of_two_dimensions() {
        assert_eq!(parsed("2px * 3px"), None);
        assert_eq!(parsed("6px / 2px"), None);
    }

    #[test]
    fn nested_calc_is_transparent() {
        assert_eq!(parsed("calc(50px)").as_deref(), Some("50px"));
        assert_eq!(parsed("20px + calc(80px)").as_deref(), Some("100px"));
        assert_eq!(parsed("calc(2 * calc(calc(3)) + 4) * 10px").as_deref(), Some("100px"));
        assert_eq!(
            parsed("10px + calc(10px + calc(10px + calc(10px + 1px)))").as_deref(),
            Some("41px")
        );
    }

    #[test]
    fn deep_nesting_does_not_blow_the_stack() {
        // The parser has its own recursion limit for the `calc(` it must open; this one only has
        // to survive whatever body it is handed.
        let depth = 200;
        let body = format!("{}1px{}", "calc(".repeat(depth), ")".repeat(depth));
        assert_eq!(parsed(&body).as_deref(), Some("1px"));
    }

    #[test]
    fn percentages_stay_symbolic_and_sort_first() {
        // A percentage needs a containing block, so it survives - and css-values-4 puts it ahead
        // of every dimension in the serialization.
        assert_eq!(parsed("50px + 40%").as_deref(), Some("40% + 50px"));
        assert_eq!(parsed("40% - 50px").as_deref(), Some("40% - 50px"));
    }

    #[test]
    fn dimensions_sort_by_unit() {
        assert_eq!(
            parsed("1vw + 1em + 1px + 1ch").as_deref(),
            Some("1ch + 1em + 1px + 1vw")
        );
    }

    #[test]
    fn relative_units_wait_for_the_cascade() {
        // Nothing knows what an em is worth while a stylesheet is being parsed.
        assert_eq!(parsed("2em + 10px").as_deref(), Some("2em + 10px"));
        // Once the element's font-size is known it is just arithmetic.
        let units = Units::computed(20.0, 16.0);
        assert_eq!(body("2em + 10px", &units).as_deref(), Some("50px"));
        assert_eq!(body("2rem + 10px", &units).as_deref(), Some("42px"));
    }

    #[test]
    fn plus_and_minus_need_their_whitespace() {
        // `1px -2px` is two adjacent values, which is invalid - not a subtraction.
        assert_eq!(parsed("1px -2px"), None);
        assert_eq!(parsed("1px- 2px"), None);
        // Multiplication has no such rule.
        assert_eq!(parsed("2*3px").as_deref(), Some("6px"));
    }

    #[test]
    fn unary_signs_are_not_operators() {
        assert_eq!(parsed("-5px + 10px").as_deref(), Some("5px"));
        assert_eq!(parsed("10px + -5px").as_deref(), Some("5px"));
        assert_eq!(parsed("2 * -3px").as_deref(), Some("-6px"));
    }

    #[test]
    fn a_number_cannot_be_added_to_a_dimension() {
        // `calc(1px + 1)` is a type error; simplifying it would invent `calc(1 + 1px)`.
        assert_eq!(parsed("1px + 1"), None);
        assert_eq!(parsed("1 + 2").as_deref(), Some("3"));
    }

    #[test]
    fn unresolved_substitutions_are_left_alone() {
        assert_eq!(parsed("100% - var(--x)"), None);
        assert_eq!(parsed("attr(data-w) + 3px"), None);
        // A bare identifier that is not one of the constants is not a value either.
        assert_eq!(parsed("1px + banana"), None);
    }

    #[test]
    fn the_numeric_constants_are_values() {
        assert_eq!(parsed("pi").as_deref(), Some("3.1415927"));
        assert_eq!(parsed("e").as_deref(), Some("2.7182817"));
        assert_eq!(parsed("2 * pi").as_deref(), Some("6.2831855"));
        // Keywords, so ASCII case-insensitive.
        assert_eq!(parsed("1px * iNFinIty").as_deref(), Some("infinity * 1px"));
        assert_eq!(parsed("1px * nAn").as_deref(), Some("NaN * 1px"));
    }

    #[test]
    fn a_non_finite_dimension_serializes_as_a_product() {
        // `NaN` and `infinity` are `<number>` keywords and cannot carry a unit, so css-values-4
        // writes a non-finite length as a product with a one-unit multiplier.
        assert_eq!(parsed("1px * NaN").as_deref(), Some("NaN * 1px"));
        assert_eq!(parsed("1in * NaN").as_deref(), Some("NaN * 1px"));
        assert_eq!(parsed("1rad * NaN").as_deref(), Some("NaN * 1deg"));
        assert_eq!(parsed("1% * NaN").as_deref(), Some("NaN * 1%"));
        assert_eq!(parsed("1px * infinity").as_deref(), Some("infinity * 1px"));
        assert_eq!(parsed("1px * -infinity").as_deref(), Some("-infinity * 1px"));
        // A plain number is just the keyword.
        assert_eq!(parsed("NaN").as_deref(), Some("NaN"));
        assert_eq!(parsed("-infinity").as_deref(), Some("-infinity"));
    }

    #[test]
    fn infinities_cancel_into_nan() {
        for body in [
            "1px * infinity / infinity",
            "1px * 0 * infinity",
            "1px * (infinity + -infinity)",
            "1px * (infinity - infinity)",
        ] {
            assert_eq!(parsed(body).as_deref(), Some("NaN * 1px"), "{body}");
        }
        assert_eq!(parsed("1px * infinity * infinity").as_deref(), Some("infinity * 1px"));
        assert_eq!(parsed("1px * -infinity * -infinity").as_deref(), Some("infinity * 1px"));
        assert_eq!(parsed("1px * (1 / infinity)").as_deref(), Some("0px"));
    }

    #[test]
    fn dividing_by_zero_is_infinity_not_an_error() {
        // css-values-4 keeps the declaration and makes it non-finite; rejecting it would throw
        // away a value the spec says is valid.
        assert_eq!(parsed("100px / 0").as_deref(), Some("infinity * 1px"));
        assert_eq!(parsed("100px / (2 - 2)").as_deref(), Some("infinity * 1px"));
        assert_eq!(parsed("100px * 0 / 0").as_deref(), Some("NaN * 1px"));
    }

    #[test]
    fn comparison_functions_fold_inside_calc() {
        assert_eq!(parsed("1px * max(1/0, 0)").as_deref(), Some("infinity * 1px"));
        assert_eq!(parsed("1px * min(1/0, 0)").as_deref(), Some("0px"));
        assert_eq!(parsed("min(1px, 2px) + 3px").as_deref(), Some("4px"));
        assert_eq!(parsed("clamp(-infinity, 0, infinity)").as_deref(), Some("0"));
        assert_eq!(parsed("clamp(-infinity, infinity, 10)").as_deref(), Some("10"));
        // Comparing a length with a number has no meaning, so the body stays as written.
        assert_eq!(parsed("min(1px, 2)"), None);
    }

    #[test]
    fn a_computed_value_is_always_finite() {
        // A *specified* value keeps the keywords - that is what the serialization above is - but
        // a computed one is a real number layout will do arithmetic on. NaN becomes zero, and an
        // infinity is clamped to something a layout can add to without coming back to infinity.
        let units = Units::computed(16.0, 16.0);
        assert_eq!(
            evaluate("1px * NaN", &units, true),
            Some(CssValue::Unit(0.0, "px".to_string()))
        );
        // Whatever unit the NaN carried: there is nothing left to take a percentage of.
        assert_eq!(
            evaluate("1% * NaN", &units, true),
            Some(CssValue::Unit(0.0, "px".to_string()))
        );
        assert_eq!(
            evaluate("1px * infinity", &units, true),
            Some(CssValue::Unit(MAX_FINITE, "px".to_string()))
        );
        assert_eq!(
            evaluate("1px * -infinity", &units, true),
            Some(CssValue::Unit(-MAX_FINITE, "px".to_string()))
        );
        // Unwrapped only when it came down to one term, so the specified path is untouched.
        assert_eq!(
            evaluate("1px * NaN", &units, false),
            Some(CssValue::Function(
                "calc".to_string(),
                vec![CssValue::String("NaN * 1px".to_string())]
            ))
        );
    }

    #[test]
    fn nan_is_contagious_through_a_comparison() {
        // `f32::min`/`max` are defined to *ignore* NaN and return the other operand, which is
        // the opposite of what CSS requires.
        assert_eq!(parsed("max(NaN, min(0, 10))").as_deref(), Some("NaN"));
        assert_eq!(parsed("max(0, min(10, NaN))").as_deref(), Some("NaN"));
        assert_eq!(parsed("clamp(NaN, 0, 10)").as_deref(), Some("NaN"));
        assert_eq!(parsed("clamp(0, 10, NaN)").as_deref(), Some("NaN"));
        assert_eq!(parsed("clamp(0, NaN, 10)").as_deref(), Some("NaN"));
    }

    #[test]
    fn angles_times_and_resolutions_canonicalise() {
        assert_eq!(parsed("90deg + 0.25turn").as_deref(), Some("180deg"));
        assert_eq!(parsed("1s + 500ms").as_deref(), Some("1.5s"));
        assert_eq!(parsed("1dppx + 96dpi").as_deref(), Some("2dppx"));
    }

    #[test]
    fn evaluate_unwraps_only_when_asked() {
        let units = Units::computed(16.0, 16.0);
        assert_eq!(
            evaluate("10px + 20px", &units, true),
            Some(CssValue::Unit(30.0, "px".to_string()))
        );
        assert_eq!(
            evaluate("10px + 20px", &units, false),
            Some(CssValue::Function(
                "calc".to_string(),
                vec![CssValue::String("30px".to_string())]
            ))
        );
        // Two terms left, so there is nothing to unwrap to either way.
        assert_eq!(
            evaluate("10px + 20%", &units, true),
            Some(CssValue::Function(
                "calc".to_string(),
                vec![CssValue::String("20% + 10px".to_string())]
            ))
        );
    }

    #[test]
    fn a_corrupt_body_is_returned_untouched() {
        // `calc(10px+20px)` loses its operator in the parser today, leaving `10px20px`. That
        // reads as one dimension with a nonsense unit, which is left exactly as it is rather
        // than being silently "simplified" into something the author never wrote.
        assert_eq!(parsed("10px20px").as_deref(), Some("10px20px"));
    }
}
