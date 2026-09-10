//! `element.style` - the specified-value half of `CSSStyleDeclaration`.
//!
//! The declaration block lives in the element's own `style` attribute rather than in this
//! crate, so every read and write goes through the real document, and whether a value is
//! accepted at all is decided by [`gosub_css3`]: the declaration is parsed by the real
//! parser and then checked against the property's syntax definition. A test that goes green
//! here says the CSS parser accepted the value, not that this binding did.
//!
//! What is **not** here is CSSOM serialization. `getPropertyValue` gives back the text the
//! author wrote, because `CssValue`'s `Display` is a debug rendering rather than a CSS
//! serializer (a `List` prints as `List(a, b, c)`), and inventing a serializer in the
//! binding would make the wpt parsing tests measure this file instead of the engine. So the
//! suites that assert a canonical form - `1E2px` serializing as `100px`, `rgb(0,0,0)` as
//! `rgb(0, 0, 0)` - fail, and they should: that work belongs in `gosub_css3`.

use cow_utils::CowUtils;
use gosub_css3::matcher::property_definitions::get_css_definitions;
use gosub_css3::stylesheet::CssValue;
use gosub_css3::Css3;
use gosub_interface::css3::CssOrigin;
use gosub_interface::document::Document as _;
use gosub_shared::config::ParserConfig;
use gosub_shared::node::NodeId;
use rquickjs::class::Trace;
use rquickjs::{Ctx, Function, JsLifetime, Result, Value};

use crate::DocHandle;

/// Parse declarations the way a `style` attribute is parsed: an invalid declaration is
/// dropped rather than taking the rest of the block with it.
fn parser_config() -> ParserConfig {
    ParserConfig {
        ignore_errors: true,
        ..Default::default()
    }
}

/// CSS property names are ASCII case-insensitive, but custom properties are not: `--Foo` and
/// `--foo` are two different properties, so folding their case would merge them into one slot.
fn normalize_name(name: &str) -> String {
    if name.starts_with("--") {
        name.to_string()
    } else {
        name.cow_to_ascii_lowercase().into_owned()
    }
}

/// Parse one `name: value` pair through the real parser, and return its value when the
/// property's syntax definition accepts it.
///
/// Both halves matter. The parser rejects what is not a declaration at all (`width: ;`,
/// unbalanced brackets), and the definition rejects what parses but means nothing for this
/// property (`width: solid`). A custom property (`--x`) has no definition and no grammar to
/// check against, so it is accepted whenever it parses.
///
/// The value has to account for the whole of the block it is spliced into, not just the start
/// of it. `setProperty("width", "10px} *{color:red")` closes the rule and opens another, and
/// reading only the first rule would call that a valid `width: 10px` - after which the raw text
/// goes into the `style` attribute, where the renderer parses it as the injected pair of rules
/// instead. Requiring exactly one rule holding exactly one declaration is what rules that out,
/// and it also rejects a value smuggling a second declaration past a `;`.
fn parse_declaration(name: &str, value: &str) -> Option<CssValue> {
    let sheet = Css3::parse_str(&format!("*{{{name}:{value}}}"), parser_config(), CssOrigin::Author, "").ok()?;
    let [rule] = sheet.rules.as_slice() else {
        return None;
    };
    let [declaration] = rule.declarations().as_slice() else {
        return None;
    };
    if !declaration.property.eq_ignore_ascii_case(name) {
        return None;
    }

    if name.starts_with("--") {
        return Some(declaration.value.clone());
    }
    let definition = get_css_definitions().find_property(&name.cow_to_ascii_lowercase())?;
    definition
        .matches(declaration.value.to_slice())
        .then(|| declaration.value.clone())
}

/// Split a `style` attribute into its declarations, as text.
///
/// This walks the attribute's own punctuation - `;` between declarations, `:` between a name
/// and its value - and nothing else. Strings, brackets, comments and backslash escapes are
/// tracked only so that a `;` inside `url(a;b)`, `content: "a\";b"` or `/* ; */` is not taken
/// for a separator: this rewrites the attribute on every `setProperty`, so a `;` mistaken there
/// truncates the declaration it was inside and writes the damage back. Every value it hands
/// back still goes to [`parse_declaration`], so no judgement about what CSS *means* is made.
fn split_declarations(attribute: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let (mut depth, mut quote) = (0u32, None::<char>);
    let mut current = String::new();

    let flush = |text: &str, out: &mut Vec<(String, String)>| {
        let Some((name, value)) = text.split_once(':') else {
            return;
        };
        let (name, value) = (name.trim(), value.trim());
        if !name.is_empty() && !value.is_empty() {
            out.push((normalize_name(name), value.to_string()));
        }
    };

    let mut chars = attribute.chars().peekable();
    while let Some(ch) = chars.next() {
        // A backslash escapes whatever follows it, in or out of a string, so neither character
        // can close a quote or end a declaration.
        if ch == '\\' {
            current.push(ch);
            if let Some(escaped) = chars.next() {
                current.push(escaped);
            }
            continue;
        }
        // Comments nest inside neither strings nor each other, so they only start outside one.
        if quote.is_none() && ch == '/' && chars.peek() == Some(&'*') {
            current.push(ch);
            current.push(chars.next().unwrap_or('*'));
            let mut previous = '\0';
            for inner in chars.by_ref() {
                current.push(inner);
                if previous == '*' && inner == '/' {
                    break;
                }
                previous = inner;
            }
            continue;
        }
        match (quote, ch) {
            (Some(open), _) => {
                if ch == open {
                    quote = None;
                }
            }
            (None, '"' | '\'') => quote = Some(ch),
            (None, '(') => depth += 1,
            (None, ')') => depth = depth.saturating_sub(1),
            (None, ';') if depth == 0 => {
                flush(&current, &mut out);
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(ch);
    }
    flush(&current, &mut out);
    out
}

/// Render a declaration list back into a `style` attribute.
fn join_declarations(declarations: &[(String, String)]) -> String {
    declarations
        .iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The `style` attribute of one element, seen as a declaration block.
#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "CSSStyleDeclaration")]
pub struct GosubCssStyleDeclaration {
    #[qjs(skip_trace)]
    doc: DocHandle,
    #[qjs(skip_trace)]
    id: NodeId,
}

impl GosubCssStyleDeclaration {
    pub(crate) fn new(doc: DocHandle, id: NodeId) -> Self {
        Self { doc, id }
    }

    /// The block, with a property that appears more than once collapsed onto its last value.
    ///
    /// A declaration block holds each property once: `width: 10px; width: 20px` is one entry
    /// worth `20px`, not two. Collapsing on the way out rather than on the way in means an
    /// attribute written by hand reads correctly without the attribute being rewritten first.
    fn declarations(&self) -> Vec<(String, String)> {
        let parsed = self
            .doc
            .borrow()
            .attribute(self.id, "style")
            .map(split_declarations)
            .unwrap_or_default();

        let mut out: Vec<(String, String)> = Vec::with_capacity(parsed.len());
        for (name, value) in parsed {
            match out.iter_mut().find(|(existing, _)| *existing == name) {
                Some(slot) => slot.1 = value,
                None => out.push((name, value)),
            }
        }
        out
    }

    fn store(&self, declarations: &[(String, String)]) {
        let mut doc = self.doc.borrow_mut();
        if declarations.is_empty() {
            doc.remove_attribute(self.id, "style");
        } else {
            doc.set_attribute(self.id, "style", &join_declarations(declarations));
        }
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl GosubCssStyleDeclaration {
    #[qjs(get)]
    pub fn length(&self) -> usize {
        self.declarations().len()
    }

    /// The name at index `index`, or `""` past the end - the CSSOM's answer for an index that
    /// is not there.
    pub fn item(&self, index: usize) -> String {
        self.declarations()
            .get(index)
            .map(|(name, _)| name.clone())
            .unwrap_or_default()
    }

    pub fn get_property_value(&self, name: String) -> String {
        let name = normalize_name(&name);
        // Last, not first: a block written from `cssText` can carry the same property twice, and
        // the later declaration is the one that wins.
        self.declarations()
            .into_iter()
            .rfind(|(property, _)| *property == name)
            .map(|(_, value)| value)
            .unwrap_or_default()
    }

    /// `setProperty` is also what an assignment through the proxy lands on, so this is the
    /// one place a value is accepted or refused. A value the engine will not parse leaves the
    /// block untouched, which is what makes `test_invalid_value` meaningful.
    pub fn set_property(&self, name: String, value: String) {
        let name = normalize_name(&name);
        // Assigning the empty string removes the declaration - `test_valid_value` clears the
        // property this way before setting the value it is actually testing.
        if value.trim().is_empty() {
            self.remove_property(name);
            return;
        }
        if parse_declaration(&name, &value).is_none() {
            return;
        }

        let mut declarations = self.declarations();
        let stored = value.trim().to_string();
        match declarations.iter_mut().find(|(property, _)| *property == name) {
            // Setting a property that is already there keeps its position in the block, which
            // is what `item()` and the iteration order are read against.
            Some(slot) => slot.1 = stored,
            None => declarations.push((name, stored)),
        }
        self.store(&declarations);
    }

    pub fn remove_property(&self, name: String) -> String {
        let name = normalize_name(&name);
        let mut declarations = self.declarations();
        let Some(index) = declarations.iter().position(|(property, _)| *property == name) else {
            return String::new();
        };
        let previous = declarations.remove(index).1;
        self.store(&declarations);
        previous
    }

    #[qjs(get, rename = "cssText")]
    pub fn css_text(&self) -> String {
        let declarations = self.declarations();
        if declarations.is_empty() {
            return String::new();
        }
        format!("{};", join_declarations(&declarations))
    }

    /// Assigning `cssText` replaces the whole block, dropping the declarations the engine
    /// will not accept rather than refusing the lot - that is how a `style` attribute parses.
    #[qjs(set, rename = "cssText")]
    pub fn set_css_text(&self, text: String) {
        let kept: Vec<(String, String)> = split_declarations(&text)
            .into_iter()
            .filter(|(name, value)| parse_declaration(name, value).is_some())
            .collect();
        self.store(&kept);
    }
}

/// The name of the JS helper that wraps a declaration in its property-access proxy.
pub(crate) const STYLE_PROXY: &str = "__gosub_style_proxy";

/// `style[name]` and `style.name` have to reach an arbitrary CSS property, which a fixed set
/// of class accessors cannot do - and defining one accessor per known property on every
/// declaration object would cost several hundred per element. A `Proxy` forwards the names
/// that are not real members to `getPropertyValue`/`setProperty` instead, which is also where
/// the IDL-attribute spelling (`fontSize`) is turned back into the CSS one (`font-size`).
const STYLE_PROXY_SOURCE: &str = r#"
(function (declaration) {
  const dashed = (name) => {
    if (name.startsWith("--")) return name;             // custom properties are literal
    const css = name.replace(/[A-Z]/g, (c) => "-" + c.toLowerCase());
    // `cssFloat` is the one IDL attribute whose name is not its CSS property, and the
    // `webkitFoo` spelling of a prefixed property gains the leading dash the CSS name has.
    if (css === "css-float") return "float";
    return /^(webkit|moz|ms|o)-/.test(css) ? "-" + css : css;
  };
  // A method reached through the proxy would be called with the proxy as `this`, which the
  // native class cannot unwrap, so each one is handed out bound to the declaration itself.
  // Cached, so that `style.setProperty === style.setProperty` still holds.
  const bound = new Map();
  return new Proxy(declaration, {
    get(target, key, receiver) {
      if (typeof key !== "string") return Reflect.get(target, key, target);
      if (key in target) {
        const value = Reflect.get(target, key, target);
        if (typeof value !== "function") return value;
        if (!bound.has(key)) bound.set(key, value.bind(target));
        return bound.get(key);
      }
      // An index reads out a property name, the way `item()` does.
      if (/^\d+$/.test(key)) return target.item(Number(key));
      return target.getPropertyValue(dashed(key));
    },
    set(target, key, value) {
      if (typeof key !== "string" || key in target) return Reflect.set(target, key, value, target);
      target.setProperty(dashed(key), value === null || value === undefined ? "" : String(value));
      return true;
    },
    has(target, key) {
      if (typeof key === "string" && !(key in target)) return target.getPropertyValue(dashed(key)) !== "";
      return Reflect.has(target, key);
    },
  });
})
"#;

/// The JS `Map` holding one style proxy per element, so `el.style === el.style` holds. It lives
/// on the globals, like the node wrapper cache, so the proxies stay reachable for the GC.
const STYLE_CACHE: &str = "__gosub_style_wrappers";

/// Install the proxy factory and its cache. Called once per context, before any `style` is
/// handed out.
pub(crate) fn install(ctx: &Ctx<'_>) -> Result<()> {
    let factory: Function = ctx.eval(STYLE_PROXY_SOURCE)?;
    ctx.globals().set(STYLE_PROXY, factory)?;
    ctx.globals().set(STYLE_CACHE, ctx.eval::<Value, _>("new Map()")?)?;
    Ok(())
}

/// Wrap a declaration for `element.style`, reusing the element's existing proxy.
///
/// `style` is `[SameObject]` in the CSSOM: it has to be the same object every time, or
/// `el.style === el.style` is false and anything a test hangs on the object is lost between
/// reads. The block itself still lives in the `style` attribute, so the cache is only about
/// identity - two proxies over one element would already have agreed on every value.
pub(crate) fn wrap<'js>(ctx: &Ctx<'js>, doc: &DocHandle, id: NodeId) -> Result<Value<'js>> {
    let cache: rquickjs::Object<'js> = ctx.globals().get(STYLE_CACHE)?;
    let key = id.as_usize() as f64;

    let existing: Value<'js> = cache
        .get::<_, Function>("get")?
        .call((rquickjs::function::This(cache.clone()), key))?;
    if !existing.is_undefined() {
        return Ok(existing);
    }

    let declaration = rquickjs::Class::instance(ctx.clone(), GosubCssStyleDeclaration::new(doc.clone(), id))?;
    let factory: Function<'js> = ctx.globals().get(STYLE_PROXY)?;
    let proxy: Value<'js> = factory.call((declaration,))?;
    cache
        .get::<_, Function>("set")?
        .call::<_, ()>((rquickjs::function::This(cache), key, proxy.clone()))?;
    Ok(proxy)
}

/// The declaration block itself, for the paths that operate on it without going through JS.
pub(crate) fn declaration(doc: &DocHandle, id: NodeId) -> GosubCssStyleDeclaration {
    GosubCssStyleDeclaration::new(doc.clone(), id)
}
