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
//! what [`Sum`] is. A term carries a unit but no exponent, which is the model's one real limit:
//! `calc(100px * 1px / 1px)` is valid CSS - the units cancel before the expression ends, so the
//! *result* is a length - but it needs `px²` to exist along the way, and this cannot express
//! that. Such an expression is left unevaluated rather than answered wrongly. Lifting it means
//! keeping an exponent per unit in the key; the `calc-mixed-units-*` suites are what measure it.
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
    terms: BTreeMap<String, f64>,
}

impl Sum {
    fn term(unit: &str, value: f64) -> Self {
        let mut terms = BTreeMap::new();
        terms.insert(unit.to_string(), value);
        Self { terms }
    }

    /// The coefficient when this is a plain number and nothing else, which is what
    /// multiplication and division require of one of their operands.
    fn as_number(&self) -> Option<f64> {
        match self.terms.len() {
            1 => self.terms.get("").copied(),
            _ => None,
        }
    }

    fn add(mut self, other: &Self, sign: f64) -> Self {
        for (unit, value) in &other.terms {
            *self.terms.entry(unit.clone()).or_insert(0.0) += value * sign;
        }
        self
    }

    fn scale(mut self, factor: f64) -> Self {
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
        // The arithmetic above runs in f64 and only narrows here. `CssValue` holds f32, and
        // doing the sums in f32 made `2ms + 3ms` come out `0.0050000004s`: each operand picks up
        // its own error converting to seconds, and there is no width left to absorb it.
        #[expect(clippy::cast_possible_truncation, reason = "CssValue is f32; see above")]
        let value = *value as f32;
        Some(match unit.as_str() {
            "" => CssValue::Number(value),
            "%" => CssValue::Percentage(value),
            unit => CssValue::Unit(value, unit.to_string()),
        })
    }

    /// The unit and coefficient this sum came down to, if it came down to one term.
    fn single_term(&self) -> Option<(String, f64)> {
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

fn format_term(value: f64, unit: &str) -> String {
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
    // Narrowed before printing: the sum is carried in f64 so intermediate steps do not
    // accumulate error, but the value this becomes is an f32, and serializing at f64 width would
    // print seventeen digits of a precision the stored value does not have.
    #[expect(clippy::cast_possible_truncation, reason = "the value it serializes is an f32")]
    let value = value as f32;
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
    simplify_with(body, units, Mode::Evaluate)
}

/// What the caller wants out of a simplification.
///
/// The two questions are not the same, and answering the second with the first is what made
/// `calc(min(1em, 21px) + 10px)` look invalid: at parse time those operands are a length in two
/// units nothing can compare yet, so it cannot be *folded* - but it is perfectly well *typed*,
/// and the matcher only ever needed the type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Reduce to a value. A comparison whose operands are not yet comparable does not reduce.
    Evaluate,
    /// Establish the datatype. A comparison over operands of one datatype has that datatype,
    /// whatever their units, and stands in for itself with a zero of the canonical unit -
    /// nothing reads the magnitude in this mode.
    TypeOnly,
}

fn simplify_with(body: &str, units: &Units, mode: Mode) -> Option<Sum> {
    let tokens = lex(body)?;
    let mut parser = Parser {
        tokens: &tokens,
        pos: 0,
        units,
        mode,
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

/// Evaluate a whole math-function call - `min(1px, 2px)`, `progress(100px, 0px, 100px)` - rather
/// than a `calc()` body.
///
/// Only `calc()` used to be folded, because only `calc()` held its expression as text. Everything
/// else kept its arguments as values and reached serialization untouched, so
/// `progress(100px, 0px, 100px)` came back as itself where it should read `calc(1)`.
///
/// `None` means it did not reduce, and the call should be left exactly as it was.
#[must_use]
pub fn evaluate_call(name: &str, args: &[CssValue], units: &Units, unwrap: bool) -> Option<CssValue> {
    if !is_evaluable(name) || name.eq_ignore_ascii_case("calc") || has_unevaluable(args) {
        return None;
    }
    let text = CssValue::Function(name.to_string(), args.to_vec()).to_string();
    let sum = simplify(&text, units)?;
    if unwrap {
        if let Some(value) = sum.single_value() {
            return Some(make_finite(value));
        }
    }
    // A reduced math function serializes as `calc()`, whatever function it started as - the
    // expression is gone, and what is left is a plain value in a math context.
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

/// The datatype a unit denotes, named as the property grammars name it.
///
/// `""` is a plain number and `"%"` a percentage. `None` is an identifier that is not a unit at
/// all, which is how `min(1py)` is caught.
#[must_use]
pub fn unit_datatype(unit: &str) -> Option<&'static str> {
    Some(match unit {
        "" => "number",
        "%" => "percentage",
        "px" | "in" | "cm" | "mm" | "q" | "pt" | "pc" => "length",
        // Font-relative. Unresolved at parse time, but still lengths.
        "em" | "rem" | "ex" | "rex" | "ch" | "rch" | "cap" | "rcap" | "ic" | "ric" | "lh" | "rlh" => "length",
        // Viewport-relative, in all four variants css-values-4 defines.
        "vw" | "vh" | "vi" | "vb" | "vmin" | "vmax" => "length",
        "svw" | "svh" | "svi" | "svb" | "svmin" | "svmax" => "length",
        "lvw" | "lvh" | "lvi" | "lvb" | "lvmin" | "lvmax" => "length",
        "dvw" | "dvh" | "dvi" | "dvb" | "dvmin" | "dvmax" => "length",
        // Container-relative.
        "cqw" | "cqh" | "cqi" | "cqb" | "cqmin" | "cqmax" => "length",
        "deg" | "grad" | "rad" | "turn" => "angle",
        "s" | "ms" => "time",
        "hz" | "khz" => "frequency",
        "dpi" | "dpcm" | "dppx" | "x" => "resolution",
        "fr" => "flex",
        _ => return None,
    })
}

/// What a math function's arguments come to, as far as this can tell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MathType {
    /// The datatypes present across the arguments. More than one means a mix, which is only
    /// legal for `<length-percentage>` and its kin.
    Resolved(Vec<&'static str>),
    /// Not decidable here: an unsubstituted `var()`, or a math function this does not evaluate.
    /// The caller has to let it through rather than reject something it cannot read.
    Unknown,
    /// Not valid CSS for any property - malformed, or arguments that cannot be compared.
    Invalid,
}

/// The math functions whose arguments this can evaluate. Anything else - `sin()`, `round()`,
/// `var()` - makes the whole expression undecidable rather than invalid.
fn is_evaluable(name: &str) -> bool {
    matches!(
        name.cow_to_ascii_lowercase().as_ref(),
        "calc" | "min" | "max" | "clamp" | "progress"
    )
}

/// Whether anything in here is a function this cannot evaluate.
fn has_unevaluable(values: &[CssValue]) -> bool {
    values.iter().any(|value| match value {
        CssValue::Function(name, args) => !is_evaluable(name) || has_unevaluable(args),
        CssValue::List(items) => has_unevaluable(items),
        // A `calc()` body is text, so its functions are not `CssValue`s to walk.
        CssValue::String(text) => text_has_unevaluable(text),
        _ => false,
    })
}

/// The same question for a `calc()` body, which is held as text.
fn text_has_unevaluable(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_alphabetic() && bytes[i] != b'-' && bytes[i] != b'_' {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b'_') {
            i += 1;
        }
        if bytes.get(i) == Some(&b'(') && !is_evaluable(&text[start..i]) {
            return true;
        }
    }
    false
}

/// Type-check a math function's arguments against nothing in particular.
///
/// The syntax matcher used to accept any math function wherever a numeric datatype was allowed,
/// checking the *name* and never the arguments - so `width: min(red, 50px)` was valid, and so was
/// `min(1px 2px)`. It could not do better while a `calc()` body was opaque text nobody evaluated;
/// now that it is evaluated, the type it comes to is knowable, and so is whether it parses at all.
#[must_use]
pub fn math_function_type(name: &str, args: &[CssValue], units: &Units) -> MathType {
    if !is_evaluable(name) {
        return MathType::Unknown;
    }
    if has_unevaluable(args) {
        return MathType::Unknown;
    }

    // Evaluate the call as a whole rather than reasoning about its arguments here. The grammar
    // lives in one place that way - arity, `clamp()`'s `none` bounds, `progress()`'s `no-clamp`
    // and the fact that `progress()` returns a *number* whatever its arguments were. Picking
    // arguments apart separately got all four of those wrong.
    //
    // `Display` on a `CssValue` is its CSS serialization, so writing the call back out and
    // re-reading it is exact.
    let text = CssValue::Function(name.to_string(), args.to_vec()).to_string();
    let Some(sum) = simplify_with(&text, units, Mode::TypeOnly) else {
        return MathType::Invalid;
    };

    let mut kinds: Vec<&'static str> = Vec::new();
    for unit in sum.terms.keys() {
        let Some(kind) = unit_datatype(unit) else {
            return MathType::Invalid;
        };
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    if kinds.is_empty() {
        return MathType::Invalid;
    }
    kinds.sort_unstable();
    MathType::Resolved(kinds)
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
fn canonical(unit: &str, units: &Units) -> Option<(String, f64)> {
    let px = |factor: f64| Some(("px".to_string(), factor));
    match unit {
        "px" => px(1.0),
        // 1in is 96px by definition, and every other absolute length is a fraction of an inch.
        "in" => px(96.0),
        "pt" => px(96.0 / 72.0),
        "pc" => px(96.0 / 6.0),
        "cm" => px(96.0 / 2.54),
        "mm" => px(96.0 / 25.4),
        "q" => px(96.0 / 101.6),
        "em" => units.em_px.map(f64::from).and_then(px),
        "rem" => units.rem_px.map(f64::from).and_then(px),
        "vw" | "svw" | "lvw" | "dvw" => units.viewport.then(|| f64::from(viewport().0) / 100.0).and_then(px),
        "vh" | "svh" | "lvh" | "dvh" => units.viewport.then(|| f64::from(viewport().1) / 100.0).and_then(px),
        "vmin" => units
            .viewport
            .then(|| {
                let (w, h) = viewport();
                f64::from(w.min(h)) / 100.0
            })
            .and_then(px),
        "vmax" => units
            .viewport
            .then(|| {
                let (w, h) = viewport();
                f64::from(w.max(h)) / 100.0
            })
            .and_then(px),
        "deg" => Some(("deg".to_string(), 1.0)),
        "grad" => Some(("deg".to_string(), 0.9)),
        "rad" => Some(("deg".to_string(), 180.0 / std::f64::consts::PI)),
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
    Value(f64, String),
    Plus,
    Minus,
    Star,
    Slash,
    /// `(` or `calc(` - the two are the same thing to the grammar.
    Open,
    /// A comparison function that takes this expression as one of its arguments.
    Func(String),
    /// A bare identifier. Only `no-clamp`, at the head of a `progress()`, means anything.
    Ident(String),
    Comma,
    Close,
}

/// The numeric constants css-values-4 allows wherever a `<number>` may appear inside a math
/// function. They are keywords rather than identifiers, and ASCII case-insensitive - `nan`,
/// `NaN` and `nAn` are the same token.
fn constant(name: &str) -> Option<f64> {
    match name {
        "pi" => Some(std::f64::consts::PI),
        "e" => Some(std::f64::consts::E),
        "infinity" => Some(f64::INFINITY),
        "nan" => Some(f64::NAN),
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
                    "min" | "max" | "clamp" | "progress" if opens => {
                        i += 1;
                        Tok::Func(name)
                    }
                    _ if opens => return None,
                    // A constant, else an identifier the grammar may or may not allow here.
                    _ => match constant(&name) {
                        Some(value) => Tok::Value(value, String::new()),
                        None => Tok::Ident(name),
                    },
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
fn scan_number(bytes: &[u8]) -> Option<(f64, usize)> {
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
    Some((text.parse::<f64>().ok()?, i))
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
    mode: Mode,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos).map(|l| &l.tok)
    }

    /// Consume a `none` standing alone as a whole `clamp()` argument, if that is what is here.
    ///
    /// It has to be the entire argument - `clamp(none + 1px, ...)` is not a bound with a keyword
    /// in it - so the token after it must end the argument.
    fn take_none_bound(&mut self) -> bool {
        if !matches!(self.peek(), Some(Tok::Ident(word)) if word == "none") {
            return false;
        }
        if !matches!(
            self.tokens.get(self.pos + 1).map(|l| &l.tok),
            Some(Tok::Comma | Tok::Close)
        ) {
            return false;
        }
        self.pos += 1;
        true
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
                // `progress()` may open with `no-clamp`, which turns off the clamping it
                // otherwise does on its first argument.
                let mut clamped = true;
                if matches!(self.peek(), Some(Tok::Ident(word)) if word == "no-clamp") {
                    if !name.eq_ignore_ascii_case("progress") {
                        return None;
                    }
                    clamped = false;
                    self.pos += 1;
                }
                let is_clamp = name.eq_ignore_ascii_case("clamp");
                // `None` is `clamp()`'s `none` bound: no limit on that side (css-values-5).
                let mut args: Vec<Option<Sum>> = Vec::new();
                loop {
                    if is_clamp && self.take_none_bound() {
                        args.push(None);
                    } else {
                        args.push(Some(self.sum()?));
                    }
                    match self.peek() {
                        Some(Tok::Comma) => self.pos += 1,
                        Some(Tok::Close) => {
                            self.pos += 1;
                            break;
                        }
                        _ => return None,
                    }
                }

                // An unbounded side is simply not compared against, so `clamp()` with one comes
                // out as the comparison that remains. That keeps the arity check honest and
                // means nothing downstream has to know about the keyword.
                if is_clamp {
                    let [low, value, high] = args.as_slice() else {
                        return None;
                    };
                    return match (low, high) {
                        (None, None) => value.clone(),
                        (None, Some(high)) => fold_comparison("min", &[value.clone()?, high.clone()], self.mode),
                        (Some(low), None) => fold_comparison("max", &[low.clone(), value.clone()?], self.mode),
                        (Some(low), Some(high)) => {
                            fold_comparison("clamp", &[low.clone(), value.clone()?, high.clone()], self.mode)
                        }
                    };
                }

                let args: Option<Vec<Sum>> = args.into_iter().collect();
                let args = args?;
                if name.eq_ignore_ascii_case("progress") {
                    return fold_progress(&args, clamped, self.mode);
                }
                fold_comparison(&name, &args, self.mode)
            }
            // A bare identifier is not a value. `no-clamp` is handled above, where it belongs.
            Tok::Ident(_) | Tok::Star | Tok::Slash | Tok::Close | Tok::Comma => None,
        }
    }
}

/// Fold `progress(V, S, E)` - how far `V` has got from `S` towards `E`, as a `<number>`.
///
/// All three arguments have to be the same kind of thing, and the answer never is: three lengths
/// give a number, and that is the point of the function.
///
/// By default `V` is clamped into the range before the division, *not* the result afterwards.
/// The two agree everywhere except when `S` and `E` are the same, and that is the case the wpt
/// suite pins down: `progress(2rad, 1rad, 1rad)` is `0`, where clamping the result would make it
/// `1` (the raw value is `+infinity`, which `no-clamp` does report). `no-clamp` skips it.
fn fold_progress(args: &[Sum], clamped: bool, mode: Mode) -> Option<Sum> {
    let [value, start, end] = args else {
        return None;
    };

    if mode == Mode::TypeOnly {
        // Whatever the arguments are, they must agree, and the result is a number either way.
        let mut kind: Option<&'static str> = None;
        for arg in args {
            for unit in arg.terms.keys() {
                let found = unit_datatype(unit)?;
                if *kind.get_or_insert(found) != found {
                    return None;
                }
            }
        }
        return Some(Sum::term("", 0.0));
    }

    let (value_unit, value) = value.single_term()?;
    let (start_unit, start) = start.single_term()?;
    let (end_unit, end) = end.single_term()?;
    if value_unit != start_unit || start_unit != end_unit {
        return None;
    }

    // Written to hold however the range is ordered, rather than assuming `S <= E`.
    let value = if clamped {
        value.max(start.min(end)).min(start.max(end))
    } else {
        value
    };
    let progress = (value - start) / (end - start);
    // `0 / 0` is NaN, and `progress(1rad, 1rad, 1rad)` is defined to be 0 - the value did get
    // all the way from the start to the end, the distance was just zero. The other two
    // degenerate cases fall out of the division as the infinities the suite expects.
    Some(Sum::term("", if progress.is_nan() { 0.0 } else { progress }))
}

/// Fold `min()`, `max()` or `clamp()` over arguments that have already been simplified.
///
/// To *evaluate* one, every argument has to have come down to a single term in the same unit:
/// comparing a length with a number is not a thing CSS can do, and a sum still holding two terms
/// (a percentage against a length) has no order yet either.
///
/// To *type* one is a weaker question, and the one the syntax matcher asks. `min(1em, 21px)`
/// cannot be compared before a font-size exists, and `min(1em + 1px, 22px)` has not even come
/// down to one term - but both are plainly lengths, and both are valid CSS. In that mode the
/// units the arguments mention are carried out, all with a zero nobody reads, so the caller can
/// see the datatypes involved.
fn fold_comparison(name: &str, args: &[Sum], mode: Mode) -> Option<Sum> {
    if mode == Mode::TypeOnly {
        let mut carried = Sum { terms: BTreeMap::new() };
        for arg in args {
            for unit in arg.terms.keys() {
                // An identifier that is not a unit at all - `min(1py)`, `min(red, 50px)` - has no
                // datatype, and no property accepts it.
                unit_datatype(unit)?;
                carried.terms.insert(unit.clone(), 0.0);
            }
        }
        return (!carried.terms.is_empty()).then_some(carried);
    }

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

    // Percentages have no order of their own. The basis one is taken of can be negative, and
    // then the larger percentage is the smaller length - so `min(1%, 2%)` cannot be decided
    // until there is something to be a percentage *of*, and has to survive to layout as written.
    // (`progress()` is untouched by this: its basis cancels in the ratio.)
    if unit == "%" {
        return None;
    }

    // NaN is contagious through a comparison, which `f32::min` and `f32::max` are not: they are
    // defined to *ignore* it and return the other operand, so `max(NaN, 0)` would come out 0
    // where CSS requires NaN.
    if values.iter().any(|v| v.is_nan()) {
        return Some(Sum::term(&unit, f64::NAN));
    }

    let folded = match name {
        "min" => values.into_iter().reduce(f64::min)?,
        "max" => values.into_iter().reduce(f64::max)?,
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

    /// `min(a, b, ...)` as the parser hands it over: values with the commas kept among them.
    fn comparison(name: &str, args: &[&str]) -> MathType {
        let mut values = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                values.push(CssValue::Comma);
            }
            // Each argument is written as it would be in CSS and re-read through the parser's
            // own value shapes, which is what the matcher will be holding.
            for (j, token) in arg.split_whitespace().enumerate() {
                if j > 0 || !token.is_empty() {
                    values.push(token_value(token));
                }
            }
        }
        math_function_type(name, &values, &Units::none())
    }

    fn token_value(token: &str) -> CssValue {
        if let Some(number) = token.strip_suffix('%') {
            return number.parse().map_or_else(
                |_| CssValue::String(token.to_string()),
                |n: f32| CssValue::Percentage(n),
            );
        }
        if let Ok(number) = token.parse::<f32>() {
            return CssValue::Number(number);
        }
        let split = token.find(|c: char| c.is_ascii_alphabetic());
        match split.filter(|i| *i > 0).and_then(|i| {
            token[..i]
                .parse::<f32>()
                .ok()
                .map(|n| CssValue::Unit(n, token[i..].to_string()))
        }) {
            Some(unit) => unit,
            None => CssValue::String(token.to_string()),
        }
    }

    #[test]
    fn a_comparison_is_typed_by_its_arguments() {
        // The matcher used to accept any math function on its *name*, so this was valid for
        // `width` - a colour where a length belongs.
        assert_eq!(comparison("min", &["red", "50px"]), MathType::Invalid);
        assert_eq!(comparison("min", &["1px", "2px"]), MathType::Resolved(vec!["length"]));
        assert_eq!(comparison("min", &["0"]), MathType::Resolved(vec!["number"]));
        assert_eq!(comparison("min", &["0s"]), MathType::Resolved(vec!["time"]));
        assert_eq!(comparison("max", &["0dpi"]), MathType::Resolved(vec!["resolution"]));
        // Not a unit at all.
        assert_eq!(comparison("min", &["1py"]), MathType::Invalid);
    }

    #[test]
    fn a_malformed_comparison_is_invalid() {
        assert_eq!(comparison("min", &[]), MathType::Invalid);
        assert_eq!(comparison("min", &["", ""]), MathType::Invalid);
        assert_eq!(comparison("min", &["1px", ""]), MathType::Invalid);
        assert_eq!(comparison("min", &["", "1px"]), MathType::Invalid);
        // Two values with no operator between them.
        assert_eq!(comparison("min", &["1px 2px"]), MathType::Invalid);
        // An operator with nothing after it.
        assert_eq!(comparison("min", &["1px +"]), MathType::Invalid);
        // `clamp()` is MIN, VAL, MAX and nothing else.
        assert_eq!(comparison("clamp", &["1px", "2px"]), MathType::Invalid);
    }

    #[test]
    fn typing_is_a_weaker_question_than_evaluating() {
        // Neither of these can be compared before a font-size exists, and the second has not
        // even come down to one term - but both are plainly lengths, and both are valid CSS.
        // Answering the type question with the evaluation one rejected them.
        assert_eq!(comparison("min", &["1em", "21px"]), MathType::Resolved(vec!["length"]));
        assert_eq!(
            comparison("min", &["1em + 1px", "22px"]),
            MathType::Resolved(vec!["length"])
        );
        // And the evaluation question still answers "not yet".
        assert_eq!(parsed("min(1em, 21px)"), None);
    }

    #[test]
    fn a_length_may_be_compared_with_a_percentage() {
        // Legal in a `<length-percentage>`. Both datatypes are reported so the caller can tell.
        assert_eq!(
            comparison("min", &["1px", "20%"]),
            MathType::Resolved(vec!["length", "percentage"])
        );
    }

    #[test]
    fn clamp_takes_none_for_either_bound() {
        // css-values-5: `none` means "do not clamp on this side". It is a keyword, so it carries
        // no datatype of its own.
        assert_eq!(
            comparison("clamp", &["none", "1px", "2px"]),
            MathType::Resolved(vec!["length"])
        );
        assert_eq!(
            comparison("clamp", &["1px", "2px", "none"]),
            MathType::Resolved(vec!["length"])
        );
        // Not in the middle, which is the value being clamped.
        assert_eq!(comparison("clamp", &["1px", "none", "2px"]), MathType::Invalid);
    }

    #[test]
    fn what_cannot_be_read_is_not_called_invalid() {
        // A substitution that has not happened yet, and a function this does not evaluate.
        // Neither is decidable here, and refusing them would reject valid CSS.
        let args = vec![CssValue::Function(
            "var".to_string(),
            vec![CssValue::String("--x".to_string())],
        )];
        assert_eq!(math_function_type("min", &args, &Units::none()), MathType::Unknown);
        assert_eq!(math_function_type("sin", &[], &Units::none()), MathType::Unknown);
        // Including one inside a `calc()` body, which is text rather than values.
        let body = vec![CssValue::String("1px + sin(45deg)".to_string())];
        assert_eq!(math_function_type("calc", &body, &Units::none()), MathType::Unknown);
    }

    #[test]
    fn progress_compares_across_units_of_one_datatype() {
        // Arguments need to be the same *kind* of thing, not the same unit. Anything with a
        // fixed conversion is canonicalised before the comparison, so these mix freely.
        assert_eq!(parsed("progress(1in, 0px, 192px)").as_deref(), Some("0.5"));
        assert_eq!(parsed("progress(96px, 0in, 2in)").as_deref(), Some("0.5"));
        assert_eq!(parsed("progress(0.5turn, 0deg, 360deg)").as_deref(), Some("0.5"));
        assert_eq!(parsed("progress(500ms, 0s, 1s)").as_deref(), Some("0.5"));

        // A unit whose conversion is not known yet waits, rather than being wrong: `em` and
        // `vw` become px once there is a font-size and a viewport.
        let computed = Units::computed(10.0, 16.0);
        assert_eq!(parsed("progress(10em, 0px, 10em)"), None);
        assert_eq!(body("progress(10em, 0px, 10em)", &computed).as_deref(), Some("1"));
        assert_eq!(parsed("progress(1vw, 0px, 10px)"), None);

        // `ch` depends on a font this never sees, so it stays unresolved at every stage.
        assert_eq!(parsed("progress(1ch, 0px, 10px)"), None);
        assert_eq!(body("progress(1ch, 0px, 10px)", &computed), None);

        // Different datatypes are not a question of conversion - there is no answer.
        assert_eq!(parsed("progress(10deg, 0, 10)"), None);
        assert_eq!(parsed("progress(1px, 0%, 100%)"), None);
    }

    #[test]
    fn percentages_have_no_order_to_compare() {
        // The basis a percentage is taken of can be negative, and then the larger percentage is
        // the smaller length - so a comparison over percentages cannot be decided until there is
        // something to be a percentage *of*, and has to reach layout as written.
        assert_eq!(parsed("min(1%, 2%, 3%)"), None);
        assert_eq!(parsed("max(-1%, 1%)"), None);
        assert_eq!(parsed("clamp(1%, 2%, 3%)"), None);
        // A percentage against a length is not comparable either, for the same reason.
        assert_eq!(parsed("min(1%, 2px)"), None);
        // `progress()` is untouched: its basis cancels in the ratio.
        assert_eq!(parsed("progress(1%, (10% - 10%), 100%)").as_deref(), Some("0.01"));
    }

    #[test]
    fn progress_measures_how_far_a_value_got() {
        assert_eq!(parsed("progress(100px, 0px, 100px)").as_deref(), Some("1"));
        assert_eq!(parsed("progress(1%, (10% - 10%), 100%)").as_deref(), Some("0.01"));
        // Three lengths in, a number out - which is the point of the function.
        assert_eq!(
            parsed("calc(50px * progress(100px, 0px, 100px))").as_deref(),
            Some("50px")
        );
        // Arguments that disagree are not comparable.
        assert_eq!(parsed("progress(10deg, 0, 10)"), None);
        assert_eq!(parsed("progress(10, 0px, 10)"), None);
        assert_eq!(parsed("progress(1px, 2px)"), None);
    }

    #[test]
    fn progress_clamps_its_input_by_default() {
        // Out of range in both directions.
        assert_eq!(
            parsed("calc(0.5 * progress(200px, 0px, 100px))").as_deref(),
            Some("0.5")
        );
        assert_eq!(parsed("calc(0.5 * progress(-100px, 0px, 100px))").as_deref(), Some("0"));
        // `no-clamp` reports the raw value instead.
        assert_eq!(
            parsed("calc(0.5 * progress(no-clamp 200px, 0px, 100px))").as_deref(),
            Some("1")
        );
        assert_eq!(
            parsed("calc(0.5 * progress(no-clamp -100px, 0px, 100px))").as_deref(),
            Some("-0.5")
        );
        // `no-clamp` belongs to `progress()` alone.
        assert_eq!(parsed("min(no-clamp 1px, 2px)"), None);
    }

    #[test]
    fn progress_over_an_empty_range() {
        // Clamping the *input* is what makes these zero: the value is pulled onto the start, so
        // it has travelled the whole of a zero-length range. Clamping the result instead would
        // make the first one 1, since the raw value is +infinity.
        assert_eq!(parsed("progress(2rad, 1rad, 1rad)").as_deref(), Some("0"));
        assert_eq!(parsed("progress(1rad, 1rad, 1rad)").as_deref(), Some("0"));
        assert_eq!(parsed("progress(0rad, 1rad, 1rad)").as_deref(), Some("0"));
        // Unclamped, the division says what it says - and `0 / 0` is the one that is defined to
        // be zero rather than NaN.
        assert_eq!(
            parsed("progress(no-clamp 2rad, 1rad, 1rad)").as_deref(),
            Some("infinity")
        );
        assert_eq!(parsed("progress(no-clamp 1rad, 1rad, 1rad)").as_deref(), Some("0"));
        assert_eq!(
            parsed("progress(no-clamp 0rad, 1rad, 1rad)").as_deref(),
            Some("-infinity")
        );
    }

    #[test]
    fn progress_waits_for_the_cascade_like_everything_else() {
        // `em` against `px` cannot be compared before a font-size exists, so the expression
        // stays as written - which is exactly what its specified serialization should be.
        assert_eq!(parsed("progress(10em, 0px, 10em)"), None);
        let units = Units::computed(10.0, 16.0);
        assert_eq!(body("progress(10em, 0px, 10em)", &units).as_deref(), Some("1"));
        assert_eq!(body("progress(10em, 0px, 10rem)", &units).as_deref(), Some("0.625"));
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
