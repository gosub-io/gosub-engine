//! `@supports` condition parsing and evaluation.
//!
//! Unlike `@media`, a supports condition asks about the *engine*, not the device, so its
//! answer cannot change while the browser runs. Conditions are therefore evaluated once, when
//! the stylesheet is built, and rules inside a false block are simply not collected - no
//! per-rule state and no cost in the cascade.
//!
//! The prelude reaches us as [`crate::node::NodeType::Raw`] text (the parser deliberately
//! does not structure it), so [`SupportsCondition::parse`] does its own scan. That keeps the
//! AST and the main parser's state machine untouched.

use crate::matcher::property_definitions::get_css_definitions;
use crate::Css3;
use cow_utils::CowUtils;
use gosub_interface::css3::CssOrigin;
use gosub_shared::config::ParserConfig;

/// Guard against a pathologically nested prelude - `((((((...))))))`. Real conditions nest two
/// or three deep at most.
const MAX_NESTING_DEPTH: usize = 32;

/// Declarations the CSS syntax tables accept but the render pipeline never acts on.
///
/// Reporting these as supported is worse than reporting nothing: a site that feature-detects
/// them takes its enhanced branch and skips the fallback that would actually have rendered.
/// The entries were checked against the property table the pipeline consumes
/// (`gosub_render_pipeline`'s `StyleProperty`, 78 properties); an entry with a `Some(value)`
/// denies only that keyword, because the property itself is otherwise honoured.
///
/// **Delete entries from this list as the features land.** It is a statement about what the
/// engine implements today, not about what is valid CSS.
const UNIMPLEMENTED: &[(&str, Option<&str>)] = &[
    // Layout the engine has no box model for at all.
    ("float", None),
    ("clear", None),
    // `position` is honoured, but `sticky` is not implemented.
    ("position", Some("sticky")),
    // Paint features with no display item behind them.
    ("transform", None),
    ("transform-style", None),
    ("perspective", None),
    ("box-shadow", None),
    ("text-shadow", None),
    ("filter", None),
    ("backdrop-filter", None),
    ("clip-path", None),
    ("mask", None),
    ("mask-image", None),
    ("object-fit", None),
    ("visibility", None),
    // Nothing animates yet.
    ("transition", None),
    ("animation", None),
    ("will-change", None),
    ("scroll-behavior", None),
    // Container queries are not evaluated (see the `@media` work for the query machinery
    // that would need extending).
    ("container-type", None),
    ("container-name", None),
];

/// Selectors the matcher does not implement, for `selector(...)` queries. Names are matched
/// as substrings of the queried selector, so `:has(` catches `:has(> img)` too.
const UNIMPLEMENTED_SELECTORS: &[&str] = &[":has(", ":nth-last-of-type(", "::backdrop", "::part("];

/// A parsed `@supports` condition.
#[derive(Debug, Clone, PartialEq)]
pub enum SupportsCondition {
    /// `(property: value)`
    Declaration {
        property: String,
        value: String,
    },
    /// `selector(<complex-selector>)`
    Selector(String),
    Not(Box<SupportsCondition>),
    All(Vec<SupportsCondition>),
    Any(Vec<SupportsCondition>),
    /// A `<general-enclosed>` production: a function or parenthesised blob this engine does
    /// not recognise. Always false, per spec - which makes `not (weird(1))` true.
    Unknown,
}

impl SupportsCondition {
    /// Parse a raw `@supports` prelude such as `(display: flex) and (gap: 1px)`.
    ///
    /// Never fails: anything unrecognised becomes [`SupportsCondition::Unknown`], which
    /// evaluates to false and so hides the block - the same outcome a real browser reaches
    /// for a condition it cannot understand.
    #[must_use]
    pub fn parse(input: &str) -> Self {
        Scanner::new(input).condition(0)
    }

    /// Parse the interior of an `@import`'s `supports(...)`.
    ///
    /// That production is laxer than an `@supports` prelude: it accepts a bare declaration
    /// (`supports(display: grid)`) as well as a full condition
    /// (`supports((a: b) or (c: d))`), so the parenthesised form the prelude requires is not
    /// there to recognise.
    #[must_use]
    pub fn parse_import_condition(input: &str) -> Self {
        match split_declaration(input) {
            Some((property, value)) => SupportsCondition::Declaration {
                property: property.to_string(),
                value: value.to_string(),
            },
            None => Self::parse(input),
        }
    }

    /// Whether this engine satisfies the condition.
    #[must_use]
    pub fn matches(&self) -> bool {
        match self {
            SupportsCondition::Declaration { property, value } => supports_declaration(property, value),
            SupportsCondition::Selector(selector) => supports_selector(selector),
            SupportsCondition::Not(inner) => !inner.matches(),
            SupportsCondition::All(list) => list.iter().all(SupportsCondition::matches),
            SupportsCondition::Any(list) => list.iter().any(SupportsCondition::matches),
            SupportsCondition::Unknown => false,
        }
    }
}

/// Whether the engine supports `property: value`, the question behind `@supports` and
/// (eventually) `CSS.supports()`.
///
/// Two gates: the declaration must be valid per the CSS syntax tables, and the property must
/// not be on the [`UNIMPLEMENTED`] list.
#[must_use]
pub fn supports_declaration(property: &str, value: &str) -> bool {
    let property = property.trim().cow_to_lowercase();
    let value = value.trim();
    if property.is_empty() || value.is_empty() {
        return false;
    }

    // Custom properties accept any value by definition, and the engine does cascade them.
    if property.starts_with("--") {
        return true;
    }

    if is_unimplemented(&property, value) {
        return false;
    }

    // Reuse the real declaration parser rather than re-implementing value parsing: wrap the
    // declaration in a throwaway rule, exactly as the inline-style path does.
    let config = ParserConfig {
        ignore_errors: true,
        ..Default::default()
    };
    let Ok(sheet) = Css3::parse_str(&format!("*{{{property}:{value}}}"), config, CssOrigin::Author, "") else {
        return false;
    };
    let Some(declaration) = sheet.rules.first().and_then(|rule| rule.declarations.first()) else {
        // The value did not survive parsing at all.
        return false;
    };

    let definitions = get_css_definitions();
    let Some(definition) = definitions.find_property(&property) else {
        // Not a property this engine knows.
        return false;
    };
    definition.matches(declaration.value.to_slice())
}

/// Whether `property: value` names something the engine parses but does not render.
fn is_unimplemented(property: &str, value: &str) -> bool {
    let value_lower = value.cow_to_lowercase();
    UNIMPLEMENTED.iter().any(|(name, keyword)| {
        *name == property
            && match keyword {
                None => true,
                // Match the keyword as a whole word so `sticky` does not fire on `stickyish`.
                Some(kw) => value_lower
                    .split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
                    .any(|word| word == *kw),
            }
    })
}

/// Whether the matcher can evaluate `selector`, for `@supports selector(...)`.
fn supports_selector(selector: &str) -> bool {
    let selector = selector.trim();
    if selector.is_empty() {
        return false;
    }
    let lowered = selector.cow_to_lowercase();
    if UNIMPLEMENTED_SELECTORS.iter().any(|s| lowered.contains(s)) {
        return false;
    }

    // A selector the parser accepts and turns into at least one part is one the matcher can
    // walk. `ignore_errors` stays off here: a silently-skipped rule would look like success.
    let config = ParserConfig {
        ignore_errors: false,
        ..Default::default()
    };
    let Ok(sheet) = Css3::parse_str(&format!("{selector}{{color:red}}"), config, CssOrigin::Author, "") else {
        return false;
    };
    sheet.rules.first().is_some_and(|rule| {
        rule.selectors
            .iter()
            .any(|sel| sel.parts.iter().any(|part| !part.is_empty()))
    })
}

/// Character scanner over a raw `@supports` prelude.
///
/// Works on the text rather than the token stream so that a group can be handed back as a
/// substring - which is what both the declaration parser and the recursive call want. It is
/// quote- and comment-aware, so parentheses inside `content: ")"` do not unbalance it.
struct Scanner<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Scanner<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn rest(&self) -> &'a str {
        self.input.get(self.pos..).unwrap_or("")
    }

    fn skip_trivia(&mut self) {
        loop {
            let rest = self.rest();
            let trimmed = rest.trim_start();
            self.pos += rest.len() - trimmed.len();
            if trimmed.starts_with("/*") {
                match trimmed.find("*/") {
                    Some(end) => self.pos += end + 2,
                    // Unterminated comment: everything left is comment.
                    None => self.pos = self.input.len(),
                }
                continue;
            }
            return;
        }
    }

    /// Consume an identifier if one is next, without consuming a following `(`.
    fn take_ident(&mut self) -> Option<&'a str> {
        let rest = self.rest();
        let len = rest
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
            .unwrap_or(rest.len());
        if len == 0 {
            return None;
        }
        let ident = rest.get(..len)?;
        self.pos += len;
        Some(ident)
    }

    /// Consume a balanced `( ... )` group starting at the current position, returning its
    /// interior. Returns `None` when the next character is not `(` or the group never closes.
    fn take_group(&mut self) -> Option<&'a str> {
        let rest = self.rest();
        if !rest.starts_with('(') {
            return None;
        }
        let inner_start = self.pos + 1;
        let mut depth = 0usize;
        let mut chars = rest.char_indices();
        while let Some((offset, ch)) = chars.next() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        let end = self.pos + offset;
                        self.pos = end + 1;
                        return self.input.get(inner_start..end);
                    }
                }
                '"' | '\'' => {
                    // Skip the string body so a paren inside it does not count.
                    let quote = ch;
                    let mut escaped = false;
                    for (_, c) in chars.by_ref() {
                        if escaped {
                            escaped = false;
                        } else if c == '\\' {
                            escaped = true;
                        } else if c == quote {
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
        // Unbalanced: consume the remainder so the caller cannot loop forever.
        self.pos = self.input.len();
        None
    }

    /// `supports-condition`: a `not`, or a chain of terms joined by `and` / `or`.
    fn condition(&mut self, depth: usize) -> SupportsCondition {
        if depth > MAX_NESTING_DEPTH {
            return SupportsCondition::Unknown;
        }
        self.skip_trivia();

        if self.eat_keyword("not") {
            return SupportsCondition::Not(Box::new(self.in_parens(depth + 1)));
        }

        let first = self.in_parens(depth + 1);
        let mut terms = vec![first];
        // The spec forbids mixing `and` and `or` without parentheses; the first operator seen
        // decides how the whole chain combines.
        let mut combinator: Option<bool> = None;

        loop {
            self.skip_trivia();
            let is_and = if self.eat_keyword("and") {
                true
            } else if self.eat_keyword("or") {
                false
            } else {
                break;
            };
            combinator.get_or_insert(is_and);

            self.skip_trivia();
            // `A and not (B)` is not strictly valid, but accept it rather than losing the rule.
            let term = if self.eat_keyword("not") {
                SupportsCondition::Not(Box::new(self.in_parens(depth + 1)))
            } else {
                self.in_parens(depth + 1)
            };
            terms.push(term);
        }

        match (terms.len(), combinator) {
            (1, _) => terms.swap_remove(0),
            (_, Some(false)) => SupportsCondition::Any(terms),
            _ => SupportsCondition::All(terms),
        }
    }

    /// Consume `keyword` if it is the next identifier. Case-insensitive, per CSS.
    fn eat_keyword(&mut self, keyword: &str) -> bool {
        let save = self.pos;
        self.skip_trivia();
        match self.take_ident() {
            Some(ident) if ident.eq_ignore_ascii_case(keyword) => true,
            _ => {
                self.pos = save;
                false
            }
        }
    }

    /// `supports-in-parens`: a declaration, a nested condition, `selector(...)`, or an
    /// unrecognised blob.
    fn in_parens(&mut self, depth: usize) -> SupportsCondition {
        self.skip_trivia();

        // A function form: `selector(...)`, `font-tech(...)`, or any other.
        let save = self.pos;
        if let Some(name) = self.take_ident() {
            self.skip_trivia();
            if self.rest().starts_with('(') {
                let inner = self.take_group().unwrap_or("");
                return if name.eq_ignore_ascii_case("selector") {
                    SupportsCondition::Selector(inner.to_string())
                } else {
                    // `<general-enclosed>`: parsed so it does not derail the scan, never true.
                    SupportsCondition::Unknown
                };
            }
            self.pos = save;
        }

        let Some(inner) = self.take_group() else {
            // Not a group at all - skip a token's worth so the caller makes progress.
            self.take_ident();
            return SupportsCondition::Unknown;
        };

        match split_declaration(inner) {
            Some((property, value)) => SupportsCondition::Declaration {
                property: property.to_string(),
                value: value.to_string(),
            },
            // No top-level colon, so this is a parenthesised sub-condition.
            None => Scanner::new(inner).condition(depth + 1),
        }
    }
}

/// Split `property: value` at its top-level colon, ignoring colons nested in parentheses
/// (`(width: calc(1px))`) or strings, and those starting a pseudo-class in a sub-condition
/// (`(not (a:b))` has no top-level colon at all).
///
/// Returns `None` when there is no top-level colon, which marks the group as a sub-condition
/// rather than a declaration.
fn split_declaration(inner: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    let mut chars = inner.char_indices();
    while let Some((offset, ch)) = chars.next() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            '"' | '\'' => {
                let quote = ch;
                let mut escaped = false;
                for (_, c) in chars.by_ref() {
                    if escaped {
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == quote {
                        break;
                    }
                }
            }
            ':' if depth == 0 => {
                let property = inner.get(..offset)?.trim();
                let value = inner.get(offset + 1..)?.trim();
                // A leading `:` means a pseudo-class, not a property.
                if property.is_empty() || value.is_empty() {
                    return None;
                }
                return Some((property, value));
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(condition: &str) -> bool {
        SupportsCondition::parse(condition).matches()
    }

    #[test]
    fn plain_declaration() {
        assert!(matches("(display: flex)"));
        assert!(matches("(display: grid)"));
        assert!(matches("(color: red)"));
    }

    #[test]
    fn invalid_declarations_are_unsupported() {
        assert!(!matches("(display: bogus-value)"));
        assert!(!matches("(not-a-property: 1px)"));
        assert!(!matches("(color: 42deg)"));
    }

    #[test]
    fn and_or_not() {
        assert!(matches("(display: flex) and (color: red)"));
        assert!(!matches("(display: flex) and (display: bogus-value)"));
        assert!(matches("(display: bogus-value) or (display: flex)"));
        assert!(!matches("(display: bogus-value) or (color: 42deg)"));
        assert!(matches("not (display: bogus-value)"));
        assert!(!matches("not (display: flex)"));
    }

    #[test]
    fn nested_groups() {
        assert!(matches("((display: flex))"));
        assert!(matches("((display: flex) or (display: bogus-value)) and (color: red)"));
        assert!(!matches("((display: bogus-value) and (color: red))"));
        assert!(matches("(not (display: bogus-value)) and (color: red)"));
    }

    #[test]
    fn general_enclosed_is_false_but_negates_to_true() {
        assert!(!matches("(weird-function(3))"));
        assert!(!matches("font-tech(color-COLRv1)"));
        assert!(matches("not font-tech(color-COLRv1)"));
    }

    #[test]
    fn custom_properties_are_always_supported() {
        assert!(matches("(--anything: whatever you like)"));
    }

    /// A colon inside a nested group must not be mistaken for a declaration separator.
    #[test]
    fn nested_colon_does_not_split() {
        assert_eq!(split_declaration("not (display: flex)"), None);
        assert_eq!(split_declaration("width: calc(1px)"), Some(("width", "calc(1px)")));
    }

    /// Parens inside a string must not unbalance the group scan.
    #[test]
    fn strings_do_not_unbalance_groups() {
        let mut scanner = Scanner::new(r#"(content: ")") and (color: red)"#);
        assert_eq!(scanner.take_group(), Some(r#"content: ")""#));
    }

    #[test]
    fn unimplemented_properties_report_false() {
        // Valid CSS the engine parses but never renders.
        assert!(!matches("(float: left)"));
        assert!(!matches("(transform: translateX(1px))"));
        assert!(!matches("(position: sticky)"));
        // ...while the same property in an implemented mode still works.
        assert!(matches("(position: absolute)"));
        // And the fallback branch a site would write is now reachable.
        assert!(matches("not (position: sticky)"));
    }

    #[test]
    fn selector_queries() {
        assert!(matches("selector(a > b)"));
        assert!(matches("selector(.cls)"));
        // Not implemented by the matcher yet.
        assert!(!matches("selector(a:has(> img))"));
    }

    #[test]
    fn malformed_input_terminates() {
        // Unbalanced and empty preludes must not hang or panic.
        assert!(!matches("(display: flex"));
        assert!(!matches(""));
        assert!(!matches("("));
        assert!(!matches(")"));
        assert!(!matches("and and and"));
    }
}
