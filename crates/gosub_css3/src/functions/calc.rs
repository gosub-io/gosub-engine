//! `calc()` arithmetic.
//!
//! The parser once kept a `calc()` body as raw text and nothing ever did anything with it:
//! `width: calc(10px + 20px)` reached layout still spelled `calc(10px + 20px)`, and every
//! consumer that wanted a length got a string it could not use. This is the arithmetic that was
//! missing.
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
//! # Input
//!
//! A math expression arrives as the `CssValue`s the parser built - `Unit`, `Number`, a `String`
//! holding an operator, a nested `Function` - and is read by `lex_values`. It used to arrive as
//! *text*: the parser rebuilt its own tokens into a string (see [`crate::parser::calc`]) and a
//! byte scanner here tokenized that string all over again. Two tokenizers for one input, and
//! they had to agree to stay correct. They did not: the CSS tokenizer folds a leading `+` into
//! the number after it, and a dimension prints without a positive sign, so `calc(1px +2px)`
//! reached the scanner as `1px 2px`.
//!
//! Whitespace was the thing that round trip existed to carry, because css-values-4 §10.1 makes
//! it load-bearing around `+` and `-`. It is now recorded on the operator nodes themselves
//! ([`crate::node::NodeType::Operator`]) and checked once, when a declaration is parsed. By the
//! time a stream reaches this module its spacing has already been ruled valid, which is why
//! `lex_values` does not need whitespace to be representable in a `CssValue`.
//!
//! The result goes back into a `CssValue::Function("calc", body)` whose body is the values the
//! sum came down to ([`Sum::to_values`]), so nothing downstream has to tokenize anything again.

use cow_utils::CowUtils;
use std::collections::BTreeMap;
#[cfg(test)]
use std::fmt::Write as _;

use crate::stylesheet::CssValue;
use crate::tokenizer::NumberKind;

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
            "" => CssValue::Number(value, NumberKind::Integer),
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

    /// The body, as the values that make it up: the `calc()` arguments this sum becomes.
    ///
    /// The stored form of a partially-simplified `calc()` used to be the body's *text*, which
    /// meant every later reader had to tokenize it again. Values cost the same to serialize -
    /// `CssValue::Function`'s `Display` writes the separating spaces itself, so a sum of terms
    /// still comes back as `2em + 10px` - and cost nothing to read.
    #[must_use]
    pub fn to_values(&self) -> Vec<CssValue> {
        let mut out = Vec::new();
        for (i, (unit, value)) in self.terms.iter().enumerate() {
            // A non-finite term carries its sign inside the keyword, so it is never written as
            // the right-hand side of a subtraction.
            if i > 0 {
                let subtract = value.is_finite() && *value < 0.0;
                out.push(CssValue::String(if subtract { "-" } else { "+" }.to_string()));
                if subtract {
                    out.extend(term_values(-*value, unit));
                    continue;
                }
            }
            out.extend(term_values(*value, unit));
        }
        out
    }

    /// The body text, without the surrounding `calc(` and `)`.
    ///
    /// Only tests read this: a sum is *stored* as [`Sum::to_values`], and the text is whatever
    /// those values serialize to. It stays because an expected value is far easier to read as
    /// `"2em + 10px"` than as the three values that spell it.
    #[cfg(test)]
    #[must_use]
    pub fn serialize(&self) -> String {
        let mut out = String::new();
        for (i, value) in self.to_values().iter().enumerate() {
            if i > 0 {
                out.push(' ');
            }
            let _ = write!(out, "{value}");
        }
        out
    }
}

/// One term of a sum, as the values it serializes to.
fn term_values(value: f64, unit: &str) -> Vec<CssValue> {
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
        let keyword = CssValue::String(keyword.to_string());
        return match unit {
            "" => vec![keyword],
            unit => vec![
                keyword,
                CssValue::String("*".to_string()),
                CssValue::Unit(1.0, unit.to_string()),
            ],
        };
    }
    // Narrowed before storing: the sum is carried in f64 so intermediate steps do not accumulate
    // error, but the value it becomes is an f32, and keeping f64 width would report seventeen
    // digits of a precision the stored value does not have.
    #[expect(clippy::cast_possible_truncation, reason = "the value it becomes is an f32")]
    let value = value as f32;
    match unit {
        "" => vec![CssValue::Number(value, NumberKind::Integer)],
        "%" => vec![CssValue::Percentage(value)],
        unit => vec![CssValue::Unit(value, unit.to_string())],
    }
}

/// Simplify the body of a `calc()`, or `None` when it cannot be reduced at all.
///
/// `None` means "leave the value exactly as it was": an unknown function inside the body, a
/// `var()` that has not been substituted, a malformed expression. None of those are reported as
/// errors here - rejecting a declaration is the matcher's job, and this stage has to be safe to
/// run over anything that parsed.
#[must_use]
pub fn simplify(body: &[CssValue], units: &Units) -> Option<Sum> {
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

fn simplify_with(body: &[CssValue], units: &Units, mode: Mode) -> Option<Sum> {
    let mut tokens = Vec::new();
    lex_values(body, &mut tokens)?;
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
pub fn evaluate(body: &[CssValue], units: &Units, unwrap: bool) -> Option<CssValue> {
    let sum = simplify(body, units)?;
    if unwrap {
        if let Some(value) = sum.single_value() {
            return Some(make_finite(value));
        }
    }
    Some(CssValue::Function("calc".to_string(), sum.to_values()))
}

/// Evaluate a whole math-function call - `min(1px, 2px)`, `progress(100px, 0px, 100px)` - and
/// `calc()` itself, whose arguments are its body.
///
/// Only `calc()` used to be folded, because only `calc()` held its expression as text. Everything
/// else kept its arguments as values and reached serialization untouched, so
/// `progress(100px, 0px, 100px)` came back as itself where it should read `calc(1)`.
///
/// `None` means it did not reduce, and the call should be left exactly as it was.
#[must_use]
pub fn evaluate_call(name: &str, args: &[CssValue], units: &Units, unwrap: bool) -> Option<CssValue> {
    if !is_evaluable(name) {
        return None;
    }
    // A `calc()` is its body, not a call: its parentheses group, they do not take an argument
    // list. Everything else is wrapped back up so the grammar sees the call it expects.
    let sum = if name.eq_ignore_ascii_case("calc") {
        simplify(args, units)?
    } else {
        simplify(&[CssValue::Function(name.to_string(), args.to_vec())], units)?
    };
    if unwrap {
        if let Some(value) = sum.single_value() {
            return Some(make_finite(value));
        }
    }
    // A reduced math function serializes as `calc()`, whatever function it started as - the
    // expression is gone, and what is left is a plain value in a math context.
    Some(CssValue::Function("calc".to_string(), sum.to_values()))
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
        CssValue::Number(n, kind) if !n.is_finite() => CssValue::Number(finite(n), kind),
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
/// Every function css-values-4 defines as a math function, whether or not this module can yet
/// evaluate one.
///
/// The distinction matters: a math function's *syntax* rules - the whitespace required around
/// `+` and `-`, most of all - apply to all of them, while [`is_evaluable`] names only the subset
/// whose value this module can work out. Names are matched case-insensitively and without a
/// vendor prefix, which callers strip.
#[must_use]
pub fn is_math_function_name(name: &str) -> bool {
    matches!(
        name.cow_to_ascii_lowercase().as_ref(),
        "calc"
            | "calc-size"
            | "min"
            | "max"
            | "clamp"
            | "progress"
            | "round"
            | "mod"
            | "rem"
            | "abs"
            | "sign"
            | "pow"
            | "sqrt"
            | "hypot"
            | "log"
            | "exp"
            | "sin"
            | "cos"
            | "tan"
            | "asin"
            | "acos"
            | "atan"
            | "atan2"
    )
}

fn is_evaluable(name: &str) -> bool {
    matches!(
        name.cow_to_ascii_lowercase().as_ref(),
        "calc"
            | "min"
            | "max"
            | "clamp"
            | "progress"
            | "round"
            | "mod"
            | "rem"
            | "sin"
            | "cos"
            | "tan"
            | "asin"
            | "acos"
            | "atan"
            | "atan2"
    )
}

/// Whether anything in here is a function this cannot evaluate.
fn has_unevaluable(values: &[CssValue]) -> bool {
    values.iter().any(|value| match value {
        // A parenthesized group is a call with no name, and it is transparent: it evaluates to
        // whatever is inside it. Reading the empty name as "a function nobody implements" made
        // every grouped expression answer "cannot tell" instead of being type-checked, so
        // `width: calc((1% * 1deg) / 1px)` was accepted while the ungrouped `calc(1% * 1deg)`
        // was correctly rejected.
        CssValue::Function(name, args) => (!name.is_empty() && !is_evaluable(name)) || has_unevaluable(args),
        CssValue::List(items) => has_unevaluable(items),
        _ => false,
    })
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
    let call = [CssValue::Function(name.to_string(), args.to_vec())];
    let Some(sum) = simplify_with(&call, units, Mode::TypeOnly) else {
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
/// Convert one value to its canonical unit, for the cascade.
///
/// The multiplication happens in f64 and narrows once at the end, exactly as it does inside a
/// `calc()` body. Narrowing the *factor* first and multiplying in f32 instead puts the two paths
/// one ULP apart - `12cm` came out `453.54333px` where `round(10cm, 6cm)` gave `453.5433px`,
/// which is the very comparison this is here to make agree.
#[must_use]
pub fn to_canonical(value: f32, unit: &str, units: &Units) -> Option<(String, f32)> {
    let (name, factor) = canonical(unit, units)?;
    #[expect(clippy::cast_possible_truncation, reason = "CssValue holds f32")]
    Some((name, (f64::from(value) * factor) as f32))
}

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
    // A leading `-` belongs to the keyword rather than being an operator, because the CSS
    // tokenizer reads `-infinity` as one identifier - an identifier may start with a hyphen.
    // css-values-4 names `-infinity` as a keyword in its own right for exactly that reason. The
    // byte scanner this replaced never saw the two joined up, so it took the `-` for a minus and
    // got the same answer by a different route; `-pi` needs the sign handled here to keep it.
    let (sign, name) = match name.strip_prefix('-') {
        Some(rest) => (-1.0, rest),
        None => (1.0, name),
    };

    // Case-insensitive, as css-values-4 says: `nan`, `NaN` and `nAn` are one keyword. This used
    // to be handled by the lexer, which lowercased every identifier before it got here.
    let value = match name.cow_to_ascii_lowercase().as_ref() {
        "pi" => std::f64::consts::PI,
        "e" => std::f64::consts::E,
        "infinity" => f64::INFINITY,
        "nan" => f64::NAN,
        _ => return None,
    };

    Some(sign * value)
}

/// A token and whether whitespace came before it, which `+` and `-` need: CSS requires them to
/// be surrounded by whitespace precisely so that `calc(1px -2px)` is a syntax error rather than a
/// subtraction, since `-2px` on its own is a single number token.
#[derive(Debug, Clone)]
struct Lexed {
    tok: Tok,
    space_before: bool,
}

/// Turn a math function's arguments into the token stream the expression parser reads.
///
/// The arguments arrive already parsed - `CssValue::Unit`, `CssValue::Function` and the rest -
/// because the CSS tokenizer built them on the way in. This used to re-serialize them into a
/// string and tokenize that string a second time, with a byte scanner of its own that had to
/// agree with the real tokenizer to stay correct. It did not always: the tokenizer folds a
/// leading `+` into the number it precedes, and a dimension prints without a positive sign, so
/// `calc(1px +2px)` reached the scanner as `1px 2px`.
///
/// # Precision
///
/// A number arrives as the `f32` the tokenizer narrowed it to, and is widened back with
/// `f64::from`, which keeps the `f32`'s exact value rather than the decimal behind it. That is
/// the wrong answer for `calc(-80px + 25.4mm)`, where `f64::from(25.4f32)` is 25.399999618530273
/// and the sum comes out as `15.999998px` instead of `16px`. Reading the shortest decimal back
/// instead (`25.4`) fixes that case and four like it, and breaks twelve others - `tan(0.78539816)`
/// among them, which needs the exact `f32` to round to 1. Neither is winnable here: the loss
/// happens in the tokenizer, whose `Number` is an `f32`, and only widening *that* fixes both.
///
/// # Whitespace
///
/// Every token is marked as having whitespace before it, which is what lets the `+`/`-` rule in
/// [`Parser::sum`] pass. That is sound rather than a shortcut: whitespace is checked once, at
/// parse time, against the real whitespace on the operator nodes (see `math_spacing_is_valid` in
/// `crate::ast`), and a declaration that fails the check never becomes a `CssValue` at all. By
/// the time a stream reaches here its spacing has already been ruled valid, so re-deciding it
/// from values that no longer carry whitespace would only be able to get it wrong.
fn lex_values(values: &[CssValue], out: &mut Vec<Lexed>) -> Option<()> {
    for value in values {
        let tok = match value {
            CssValue::Zero => Tok::Value(0.0, String::new()),
            CssValue::Number(number, _) => Tok::Value(f64::from(*number), String::new()),
            CssValue::Percentage(percentage) => Tok::Value(f64::from(*percentage), "%".to_string()),
            CssValue::Unit(number, unit) => Tok::Value(f64::from(*number), unit.cow_to_ascii_lowercase().into_owned()),
            CssValue::Comma => Tok::Comma,
            // The parser lowers an operator to a plain string, so this is where `+` stops being
            // an identifier and becomes arithmetic. Anything else is a keyword: a numeric
            // constant if it names one, else an identifier the grammar may or may not allow
            // (`no-clamp`, a `round()` strategy).
            CssValue::String(text) => match text.as_str() {
                "+" => Tok::Plus,
                "-" => Tok::Minus,
                "*" => Tok::Star,
                "/" => Tok::Slash,
                _ => match constant(text) {
                    Some(number) => Tok::Value(number, String::new()),
                    None => Tok::Ident(text.clone()),
                },
            },
            CssValue::Function(name, args) => {
                // `calc()` is transparent - its parentheses are just parentheses, and so is a
                // bare group, which the parser records as a call with no name. Everything else
                // is a call whose arguments the grammar reads for itself.
                let opener = if name.is_empty() || name.eq_ignore_ascii_case("calc") {
                    Tok::Open
                } else if is_evaluable(name) {
                    Tok::Func(name.cow_to_ascii_lowercase().into_owned())
                } else {
                    // `var()` before substitution, or a function this module does not implement.
                    // Not knowing what it is worth means the whole expression is unevaluable.
                    return None;
                };
                out.push(Lexed {
                    tok: opener,
                    space_before: true,
                });
                lex_values(args, out)?;
                Tok::Close
            }
            // A colour, a url, a nested list: none of them are arithmetic.
            _ => return None,
        };

        out.push(Lexed {
            tok,
            space_before: true,
        });
    }

    Some(())
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
                // `round()` may open with its rounding strategy, and only there: `round(1,
                // nearest)` names a multiple that does not exist.
                let mut strategy = Strategy::Nearest;
                if let Some(Tok::Ident(word)) = self.peek() {
                    if let Some(named) = Strategy::from_keyword(word) {
                        if !name.eq_ignore_ascii_case("round") {
                            return None;
                        }
                        strategy = named;
                        self.pos += 1;
                        // The strategy is followed by the value it applies to, not by the end of
                        // the argument list.
                        match self.peek() {
                            Some(Tok::Comma) => self.pos += 1,
                            _ => return None,
                        }
                    }
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
                if matches!(name.cow_to_ascii_lowercase().as_ref(), "round" | "mod" | "rem") {
                    return fold_stepped(&name, &args, strategy, self.mode);
                }
                if is_trig(&name) {
                    return fold_trig(&name, &args, self.mode);
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
/// Whether `name` is one of the trigonometric functions, and what it does with its arguments.
fn is_trig(name: &str) -> bool {
    matches!(
        name.cow_to_ascii_lowercase().as_ref(),
        "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "atan2"
    )
}

/// Fold `sin()`, `cos()`, `tan()` and the inverses - css-values-4 §10.6.
///
/// The two halves take and give opposite things, which is the whole of the type rule:
/// `sin`/`cos`/`tan` accept an angle *or* a bare number (read as radians) and produce a number;
/// `asin`/`acos`/`atan` accept a number and produce an angle. So `rotate(tan(45deg))` is invalid -
/// `tan()` gives a number where `rotate()` wants an angle - while `rotate(atan(1))` is fine.
///
/// `atan2()` is the exception on both counts: two arguments, which may be any one datatype as
/// long as they agree, and an angle out.
fn fold_trig(name: &str, args: &[Sum], mode: Mode) -> Option<Sum> {
    let name = name.cow_to_ascii_lowercase();
    let is_atan2 = name == "atan2";
    // Degrees, because that is the canonical angle unit everything else here has already been
    // converted to.
    let out_unit = if name == "sin" || name == "cos" || name == "tan" {
        ""
    } else {
        "deg"
    };

    match (is_atan2, args.len()) {
        (true, 2) | (false, 1) => {}
        _ => return None,
    }

    // What each argument is allowed to be.
    let accepts = |kind: &str| match name.as_ref() {
        "sin" | "cos" | "tan" => kind == "number" || kind == "angle",
        "asin" | "acos" | "atan" => kind == "number",
        // `atan2` takes a ratio, so the units cancel and any datatype will do.
        _ => true,
    };

    if mode == Mode::TypeOnly {
        // Per *argument*, not per unit: an argument that is itself a `<length-percentage>` names
        // two datatypes and is still one type. Comparing unit by unit made
        // `atan2(round(1px, 100%), round(1px, 100%))` look mismatched against itself, while
        // `atan2(90px, 100%)` - two arguments that really are different things - has to stay
        // invalid.
        let mut first_kinds: Option<Vec<&'static str>> = None;
        for arg in args {
            let mut kinds: Vec<&'static str> = Vec::new();
            for unit in arg.terms.keys() {
                let kind = unit_datatype(unit)?;
                if !accepts(kind) {
                    return None;
                }
                if !kinds.contains(&kind) {
                    kinds.push(kind);
                }
            }
            kinds.sort_unstable();
            match &first_kinds {
                None => first_kinds = Some(kinds),
                Some(first) if is_atan2 && *first != kinds => return None,
                Some(_) => {}
            }
        }
        return Some(Sum::term(out_unit, 0.0));
    }

    let (first_unit, first) = args[0].single_term()?;
    if !accepts(unit_datatype(&first_unit)?) {
        return None;
    }

    if is_atan2 {
        let (second_unit, second) = args[1].single_term()?;
        if first_unit != second_unit {
            return None;
        }
        return Some(Sum::term(out_unit, first.atan2(second).to_degrees()));
    }

    let result = match name.as_ref() {
        // An angle arrives in degrees; a bare number is already radians, which is what the
        // library functions want.
        "sin" | "cos" | "tan" => {
            let radians = if first_unit == "deg" { first.to_radians() } else { first };
            match name.as_ref() {
                "sin" => radians.sin(),
                "cos" => radians.cos(),
                _ => radians.tan(),
            }
        }
        "asin" => first.asin().to_degrees(),
        "acos" => first.acos().to_degrees(),
        _ => first.atan().to_degrees(),
    };
    Some(Sum::term(out_unit, result))
}

/// Whether two datatypes may appear in one expression.
///
/// They agree, or one is a percentage and the other a dimension - `<length-percentage>` and its
/// kin. A percentage never combines with a plain number: there is no such type.
///
/// `atan2()` is deliberately stricter and does not use this; its two arguments have to be the
/// same thing as each other, so `atan2(90px, 100%)` is invalid where `round(1px, 100%)` is not.
fn kinds_combine(a: &str, b: &str) -> bool {
    a == b || ((a == "percentage" || b == "percentage") && a != "number" && b != "number")
}

/// Which multiple `round()` picks when the value falls between two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Strategy {
    /// The closest, ties going to the multiple nearer +infinity - so `round(-1.5)` is `-1`, not
    /// `-2`. That is not what `f64::round` does, which takes ties away from zero.
    Nearest,
    Up,
    Down,
    ToZero,
}

impl Strategy {
    fn from_keyword(word: &str) -> Option<Self> {
        Some(match word {
            "nearest" => Self::Nearest,
            "up" => Self::Up,
            "down" => Self::Down,
            "to-zero" => Self::ToZero,
            _ => return None,
        })
    }
}

/// Fold `round()`, `mod()` and `rem()` - the stepped-value functions of css-values-4 §10.5.
///
/// All the arguments have to be the same kind of thing, and so is the answer. `round()` with no
/// step rounds to whole numbers, which is why `round(0px)` is invalid rather than a no-op: the
/// implicit step is a *number*, and a length cannot be a multiple of one.
fn fold_stepped(name: &str, args: &[Sum], strategy: Strategy, mode: Mode) -> Option<Sum> {
    let is_round = name.eq_ignore_ascii_case("round");
    // The step `round()` leaves out is 1, and it is a number - which is what makes the type
    // check reject a lone length.
    let implicit_step = Sum::term("", 1.0);
    let args: Vec<&Sum> = match (is_round, args.len()) {
        (true, 1) => vec![&args[0], &implicit_step],
        (_, 2) => vec![&args[0], &args[1]],
        _ => return None,
    };

    if mode == Mode::TypeOnly {
        let mut kind: Option<&'static str> = None;
        let mut carried = Sum { terms: BTreeMap::new() };
        for arg in &args {
            for unit in arg.terms.keys() {
                let found = unit_datatype(unit)?;
                // A percentage rides along with a dimension - `round(1px, 100%)` is a
                // `<length-percentage>`, and `flex-basis` takes one. It never rides along with a
                // plain number, which is why `round(1, 1%)` is not valid.
                match kind {
                    None => kind = Some(found),
                    Some(known) if !kinds_combine(known, found) => return None,
                    // Once a dimension has been seen it is the type of the whole expression;
                    // a percentage does not replace it.
                    Some("percentage") => kind = Some(found),
                    Some(_) => {}
                }
                carried.terms.insert(unit.clone(), 0.0);
            }
        }
        return (!carried.terms.is_empty()).then_some(carried);
    }

    let (value_unit, value) = args[0].single_term()?;
    let (step_unit, step) = args[1].single_term()?;
    if value_unit != step_unit {
        return None;
    }

    let result = if is_round {
        round_to(value, step, strategy)
    } else if name.eq_ignore_ascii_case("mod") {
        modulo(value, step)
    } else {
        remainder(value, step)
    };
    Some(Sum::term(&value_unit, result))
}

/// `value` rounded to a multiple of `step`.
fn round_to(value: f64, step: f64, strategy: Strategy) -> f64 {
    if step == 0.0 || (value.is_infinite() && step.is_infinite()) {
        return f64::NAN;
    }
    // An infinite value is already every multiple away; it stays itself.
    if value.is_infinite() {
        return value;
    }
    if step.is_infinite() {
        // The multiples of infinity are -infinity, -0, +0 and +infinity, and nothing between.
        let zero = if value.is_sign_negative() { -0.0 } else { 0.0 };
        return match strategy {
            Strategy::Nearest | Strategy::ToZero => zero,
            Strategy::Up if value > 0.0 => f64::INFINITY,
            Strategy::Down if value < 0.0 => f64::NEG_INFINITY,
            Strategy::Up | Strategy::Down => zero,
        };
    }
    // The multiples of a step and of its magnitude are the same set, and working from the
    // magnitude keeps the tie-break pointing at +infinity whichever sign was written.
    let step = step.abs();
    let quotient = value / step;
    let multiple = match strategy {
        Strategy::Nearest => (quotient + 0.5).floor(),
        Strategy::Up => quotient.ceil(),
        Strategy::Down => quotient.floor(),
        Strategy::ToZero => quotient.trunc(),
    };
    multiple * step
}

/// `mod()`: the remainder takes the sign of the *divisor*, so `mod(-18, 5)` is `2`.
fn modulo(value: f64, step: f64) -> f64 {
    if step == 0.0 || value.is_infinite() {
        return f64::NAN;
    }
    if step.is_infinite() {
        // Everything is within one infinite step of zero, but only on the divisor's side of it.
        return if value == 0.0 || value.is_sign_negative() == step.is_sign_negative() {
            value
        } else {
            f64::NAN
        };
    }
    value - step * (value / step).floor()
}

/// `rem()`: the remainder takes the sign of the *dividend*, so `rem(-18, 5)` is `-3`. That is
/// what `%` already does.
fn remainder(value: f64, step: f64) -> f64 {
    if step == 0.0 || value.is_infinite() {
        return f64::NAN;
    }
    if step.is_infinite() {
        return value;
    }
    value % step
}

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

    /// The values a `calc()` body of this text is made of, as the parser produces them.
    fn values(input: &str) -> Vec<CssValue> {
        crate::parse_calc_body(input).expect("calc body should parse")
    }

    fn body(input: &str, units: &Units) -> Option<String> {
        simplify(&values(input), units).map(|sum| sum.serialize())
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
            return CssValue::Number(number, NumberKind::Integer);
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
        // `pow()` stands in for whatever is not implemented yet; swap it when it is.
        assert_eq!(math_function_type("pow", &[], &Units::none()), MathType::Unknown);
        // Including one inside a `calc()` body, which is values like any other argument list -
        // it used to be text, and this was answered by scanning that text for a `name(`.
        assert_eq!(
            math_function_type("calc", &values("1px + pow(2, 3)"), &Units::none()),
            MathType::Unknown
        );
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
    fn round_picks_a_multiple_of_the_step() {
        assert_eq!(parsed("round(100, 10)").as_deref(), Some("100"));
        assert_eq!(parsed("round(up, 101, 10)").as_deref(), Some("110"));
        assert_eq!(parsed("round(down, 106, 10)").as_deref(), Some("100"));
        assert_eq!(parsed("round(to-zero, 105, 10)").as_deref(), Some("100"));
        assert_eq!(parsed("round(to-zero, -105, 10)").as_deref(), Some("-100"));
        assert_eq!(parsed("round(up, -103, 10)").as_deref(), Some("-100"));
        assert_eq!(parsed("round(10px, 6px)").as_deref(), Some("12px"));
        // Ties go to the multiple nearer +infinity, which is not what `f64::round` does - it
        // takes ties away from zero, and would make this -2.
        assert_eq!(parsed("round(-1.5)").as_deref(), Some("-1"));
        assert_eq!(parsed("round(1.5)").as_deref(), Some("2"));
        // The step written negative names the same set of multiples.
        assert_eq!(parsed("round(105, -10)").as_deref(), Some("110"));
    }

    #[test]
    fn mod_takes_the_sign_of_the_divisor_and_rem_of_the_dividend() {
        assert_eq!(parsed("mod(18, 5)").as_deref(), Some("3"));
        assert_eq!(parsed("rem(18, 5)").as_deref(), Some("3"));
        assert_eq!(parsed("mod(-18, 5)").as_deref(), Some("2"));
        assert_eq!(parsed("rem(-18, 5)").as_deref(), Some("-3"));
        assert_eq!(parsed("mod(140, -90)").as_deref(), Some("-40"));
        assert_eq!(parsed("rem(140, -90)").as_deref(), Some("50"));
        assert_eq!(parsed("mod(-140, -90)").as_deref(), Some("-50"));
        assert_eq!(parsed("mod(10px, 6px)").as_deref(), Some("4px"));
    }

    #[test]
    fn stepped_functions_over_infinities() {
        assert_eq!(parsed("round(infinity, infinity)").as_deref(), Some("NaN"));
        assert_eq!(parsed("round(-infinity, 5)").as_deref(), Some("-infinity"));
        assert_eq!(parsed("round(infinity, -5)").as_deref(), Some("infinity"));
        // The multiples of infinity are the infinities and zero, and nothing between.
        assert_eq!(parsed("round(5, infinity)").as_deref(), Some("0"));
        assert_eq!(parsed("round(up, 1, infinity)").as_deref(), Some("infinity"));
        assert_eq!(parsed("round(down, -1, infinity)").as_deref(), Some("-infinity"));
        assert_eq!(parsed("round(down, 1, infinity)").as_deref(), Some("0"));
        // A zero step has no multiples to land on.
        assert_eq!(parsed("round(1, 0)").as_deref(), Some("NaN"));
        assert_eq!(parsed("mod(1, 0)").as_deref(), Some("NaN"));
        assert_eq!(parsed("rem(1, 0)").as_deref(), Some("NaN"));
    }

    #[test]
    fn stepped_functions_check_their_shape() {
        // The step is a *number* when it is left out, so a lone length has nothing to be a
        // multiple of.
        assert_eq!(comparison("round", &["0px"]), MathType::Invalid);
        assert_eq!(comparison("round", &["1", "1%"]), MathType::Invalid);
        assert_eq!(comparison("round", &["1", "0s"]), MathType::Invalid);
        assert_eq!(comparison("round", &["1.5"]), MathType::Resolved(vec!["number"]));
        // `mod()` and `rem()` take exactly two.
        assert_eq!(comparison("mod", &["0px"]), MathType::Invalid);
        assert_eq!(comparison("mod", &["1", "2", "3"]), MathType::Invalid);
        assert_eq!(comparison("rem", &["1"]), MathType::Invalid);
        // Malformed argument lists.
        assert_eq!(comparison("round", &[]), MathType::Invalid);
        assert_eq!(comparison("round", &["1", ""]), MathType::Invalid);
        assert_eq!(comparison("round", &["1 2"]), MathType::Invalid);
        assert_eq!(parsed("round(1, nearest)"), None);
        assert_eq!(parsed("round(1, nearest, 12)"), None);
        assert_eq!(parsed("round(nearest, 1, nearest)"), None);
        // The strategy belongs to `round()` alone.
        assert_eq!(parsed("mod(up, 1, 2)"), None);
    }

    /// Trig results are irrational; compare on the value rather than on its spelling.
    ///
    /// The tolerance is relative and sized to `f32`, because `f32` is the precision CSS text can
    /// actually carry here: the tokenizer narrows every number it reads, so an argument written
    /// as `0.7853981633974483rad` reaches the evaluator as the nearest `f32` and no answer can
    /// be better than that. These used to be compared against a flat 1e-6 and passed, because
    /// the test handed its own text straight to a byte scanner that read it at `f64` width - a
    /// precision no stylesheet could ever have delivered.
    fn approx(input: &str, expected: f64, unit: &str) {
        let sum = simplify(&values(input), &Units::none()).unwrap_or_else(|| panic!("{input} should evaluate"));
        let (got_unit, got) = sum.single_term().expect("one term");
        assert_eq!(got_unit, unit, "{input}");
        let tolerance = f64::from(f32::EPSILON) * expected.abs().max(1.0) * 4.0;
        assert!(
            (got - expected).abs() <= tolerance,
            "{input}: expected {expected}, got {got} (tolerance {tolerance})"
        );
    }

    #[test]
    fn sin_cos_tan_take_an_angle_or_a_number() {
        approx("cos(0)", 1.0, "");
        approx("sin(0)", 0.0, "");
        // A bare number is radians.
        approx("sin(pi / 2)", 1.0, "");
        approx("cos(pi)", -1.0, "");
        // An angle in any unit: all of these are a quarter turn.
        approx("sin(90deg)", 1.0, "");
        approx("sin(100grad)", 1.0, "");
        approx("sin(0.25turn)", 1.0, "");
        approx("sin(1.5707963267948966rad)", 1.0, "");
        // Angles add before the function sees them.
        approx("sin(30deg + 1.0471975511965976rad)", 1.0, "");
        approx("tan(45deg)", 1.0, "");
    }

    #[test]
    fn the_inverse_functions_give_back_an_angle() {
        approx("acos(1)", 0.0, "deg");
        approx("asin(0)", 0.0, "deg");
        approx("atan(0)", 0.0, "deg");
        approx("asin(1)", 90.0, "deg");
        approx("atan(1)", 45.0, "deg");
        approx("atan2(0, 1)", 0.0, "deg");
        approx("atan2(1, -1)", 135.0, "deg");
        approx("atan2(-1, 1)", -45.0, "deg");
        // Round trips.
        approx("asin(sin(0.25turn))", 90.0, "deg");
        approx("atan(tan(0.7853981633974483rad))", 45.0, "deg");
    }

    #[test]
    fn trig_types_run_in_both_directions() {
        // A number out of the forward functions, an angle out of the inverse ones - which is
        // what makes `rotate(tan(45deg))` invalid and `rotate(atan(1))` fine.
        assert_eq!(comparison("sin", &["45deg"]), MathType::Resolved(vec!["number"]));
        assert_eq!(comparison("tan", &["45deg"]), MathType::Resolved(vec!["number"]));
        assert_eq!(comparison("asin", &["1"]), MathType::Resolved(vec!["angle"]));
        assert_eq!(comparison("atan2", &["1", "2"]), MathType::Resolved(vec!["angle"]));

        // A length is neither an angle nor a number.
        assert_eq!(comparison("sin", &["90px"]), MathType::Invalid);
        // The inverse functions take a number, not an angle.
        assert_eq!(comparison("asin", &["1deg"]), MathType::Invalid);
        assert_eq!(comparison("acos", &["1deg"]), MathType::Invalid);
        // Arity, and `atan2`'s two arguments agreeing with each other.
        assert_eq!(comparison("atan2", &["90px"]), MathType::Invalid);
        assert_eq!(comparison("atan2", &["90px", "100%"]), MathType::Invalid);
        assert_eq!(comparison("sin", &["1deg", "0"]), MathType::Invalid);
        assert_eq!(comparison("sin", &[]), MathType::Invalid);
        assert_eq!(comparison("cos", &["1deg 2deg"]), MathType::Invalid);
        // Not a unit at all.
        assert_eq!(comparison("tan", &["1dag"]), MathType::Invalid);
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
    fn nesting_is_capped_by_the_parser_that_produces_the_body() {
        // The evaluator used to be handed a body as text, so it had to survive any depth on its
        // own. Now the body comes from the parser, and the parser's own recursion cap is what
        // keeps a `calc(calc(calc(...` from ever reaching here - the depth that once needed
        // defending against is refused before evaluation begins.
        let within = 32;
        let body = format!("{}1px{}", "calc(".repeat(within), ")".repeat(within));
        assert_eq!(parsed(&body).as_deref(), Some("1px"));

        let beyond = 200;
        let body = format!("{}1px{}", "calc(".repeat(beyond), ")".repeat(beyond));
        assert_eq!(crate::parse_calc_body(&body), None);
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
            evaluate(&values("1px * NaN"), &units, true),
            Some(CssValue::Unit(0.0, "px".to_string()))
        );
        // Whatever unit the NaN carried: there is nothing left to take a percentage of.
        assert_eq!(
            evaluate(&values("1% * NaN"), &units, true),
            Some(CssValue::Unit(0.0, "px".to_string()))
        );
        assert_eq!(
            evaluate(&values("1px * infinity"), &units, true),
            Some(CssValue::Unit(MAX_FINITE, "px".to_string()))
        );
        assert_eq!(
            evaluate(&values("1px * -infinity"), &units, true),
            Some(CssValue::Unit(-MAX_FINITE, "px".to_string()))
        );
        // Unwrapped only when it came down to one term, so the specified path is untouched.
        assert_eq!(
            evaluate(&values("1px * NaN"), &units, false),
            Some(CssValue::Function(
                "calc".to_string(),
                vec![
                    CssValue::String("NaN".to_string()),
                    CssValue::String("*".to_string()),
                    CssValue::Unit(1.0, "px".to_string()),
                ]
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
            evaluate(&values("10px + 20px"), &units, true),
            Some(CssValue::Unit(30.0, "px".to_string()))
        );
        assert_eq!(
            evaluate(&values("10px + 20px"), &units, false),
            Some(CssValue::Function(
                "calc".to_string(),
                vec![CssValue::Unit(30.0, "px".to_string())]
            ))
        );
        // Two terms left, so there is nothing to unwrap to either way.
        assert_eq!(
            evaluate(&values("10px + 20%"), &units, true),
            Some(CssValue::Function(
                "calc".to_string(),
                vec![
                    CssValue::Percentage(20.0),
                    CssValue::String("+".to_string()),
                    CssValue::Unit(10.0, "px".to_string()),
                ]
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
