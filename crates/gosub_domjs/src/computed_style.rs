//! `getComputedStyle()` - the cascade's answer for one element, read-only.
//!
//! Where `element.style` is the `style` attribute and nothing else, this is the whole cascade:
//! the user-agent sheet, every `<style>` in the document, and the element's own attribute,
//! resolved through [`gosub_css3`]'s `properties_from_node`. Nothing is computed in this crate
//! - it asks the engine and reports what comes back.
//!
//! That makes it the instrument the specified-value bindings could never be. `element.style`
//! can only ever say what an author wrote; this says what the engine decided, which is where
//! `em` resolution, `calc()` evaluation and inheritance actually show up.

use cow_utils::CowUtils;
use gosub_css3::matcher::property_definitions::get_css_definitions;
use gosub_css3::matcher::styling::CssProperties;
use gosub_css3::stylesheet::CssValue;
use gosub_css3::system::{prop_is_inherit, Css3System};
use gosub_interface::css3::{CssPropertyMap, CssSystem};
use gosub_interface::document::Document as _;
use gosub_interface::node::NodeType;
use gosub_shared::node::NodeId;
use rquickjs::class::Trace;
use rquickjs::{Ctx, Function, JsLifetime, Result, Value};

use crate::{DocHandle, DomConfig};

/// Compute the cascade for `id`, walking the ancestors first so inheritance has a parent to
/// inherit from.
///
/// `properties_from_node` takes the parent's map, so a single element cannot be computed on its
/// own - an inherited property would come back as its initial value. The walk is root-first for
/// that reason, carrying the last map that resolved.
///
/// Only elements go in the chain. The document node is an ancestor of `<html>` but not an
/// element, and the cascade reads "no parent map" as "this is the root element" - which is what
/// makes a `rem` on `<html>` resolve against its own font-size rather than the initial one.
fn chain_maps(doc: &crate::Doc, id: NodeId, pseudo: Option<&str>) -> Vec<CssProperties> {
    let sheets = doc.stylesheets();

    let mut chain = vec![id];
    let mut current = id;
    while let Some(parent) = doc.parent(current) {
        if doc.node_type(parent) == NodeType::ElementNode {
            chain.push(parent);
        }
        current = parent;
    }
    chain.reverse();

    let mut maps: Vec<CssProperties> = Vec::with_capacity(chain.len());
    for node in chain {
        // The previous map is passed along because custom properties are scoped through it;
        // ordinary inheritance is not applied here (see `value_of`).
        let previous = maps.last();
        if let Some(mut map) = Css3System::properties_from_node::<DomConfig>(doc, node, sheets, previous) {
            // Resolve this map before the next element inherits from it: what a child inherits
            // is its parent's *computed* value, and the cascade reads it straight off the map.
            for (_, property) in map.iter_mut() {
                property.compute_value();
            }
            maps.push(map);
        }
    }

    if let Some(pseudo) = pseudo {
        // A pseudo-element's own cascade, with the originating element's map as its owner.
        let owner = maps.last();
        match Css3System::pseudo_properties_from_node::<DomConfig>(doc, id, sheets, pseudo, owner) {
            Some(map) => maps.push(map),
            // No pseudo box here; the element's own cascade is not its computed style.
            None => return Vec::new(),
        }
    }
    maps
}

/// The computed value of `name` for the last element in `maps`.
///
/// Mirrors how the render pipeline resolves a property: the element's own cascaded value, else
/// the parent's when the property inherits, else the spec's initial value. `properties_from_node`
/// returns only what was *declared* for an element - `insert_inherited` has no callers - so
/// inheritance has to happen at lookup, walking back up the chain the cascade was computed over.
fn value_of(maps: &mut [CssProperties], name: &str) -> Option<String> {
    // Blockification (css-display-3 §2.7): an absolutely positioned or floated element's
    // computed `display` is the block-level form of what it specified. Decided before the walk
    // below, which borrows the maps.
    let blockify = name == "display" && {
        let position = value_of(maps, "position").unwrap_or_default();
        let float = value_of(maps, "float").unwrap_or_default();
        matches!(position.as_str(), "absolute" | "fixed") || matches!(float.as_str(), "left" | "right")
    };
    let finish = |values: Vec<CssValue>| -> String {
        let values = if blockify {
            gosub_css3::matcher::property_definitions::blockified_display(values)
        } else {
            values
        };
        CssValue::from_vec(values).to_string()
    };

    let inherits = prop_is_inherit(name);

    for (depth, map) in maps.iter_mut().enumerate().rev() {
        if let Some(property) = map.get_mut(name) {
            // `compute_value` is what walks cascaded -> specified -> computed -> used -> actual.
            // A freshly cascaded property has all of those still `None` and is marked dirty, so
            // reading it without this reports "none" for everything.
            // Serialized in canonical form where the grammar knows one (`block flow` reads as
            // `block`); a computed value the grammar does not match, such as one already
            // resolved past the specified syntax, serializes as it is.
            let computed = property.compute_value().clone();
            let value = get_css_definitions()
                .find_property(name)
                .and_then(|definition| definition.canonical(computed.to_slice()))
                .map_or_else(|| computed.to_string(), &finish);
            // A declared value is the answer even when it is `none`: `display: none` is a
            // value, not an absence. This used to skip any "none" as if nothing had been
            // declared and report the initial `inline` for it.
            if !value.is_empty() && !property.declared.is_empty() {
                return Some(value);
            }
        }
        // Only an inherited property may take its value from an ancestor.
        if !inherits || depth == 0 {
            break;
        }
    }

    // Nothing declared anywhere: the property's initial value, which is what a browser reports
    // for an element no rule has touched.
    let definition = get_css_definitions().find_property(name)?;
    definition
        .has_initial_value()
        .then(|| finish(definition.initial_value().into_vec()))
}

/// The computed style of one element.
#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "CSSStyleDeclaration")]
pub struct GosubComputedStyle {
    #[qjs(skip_trace)]
    doc: DocHandle,
    #[qjs(skip_trace)]
    id: NodeId,
    #[qjs(skip_trace)]
    pseudo: Option<String>,
}

impl GosubComputedStyle {
    pub(crate) fn new(doc: DocHandle, id: NodeId, pseudo: Option<String>) -> Self {
        Self { doc, id, pseudo }
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl GosubComputedStyle {
    /// The computed value of `name`, or `""` when the cascade has nothing for it.
    pub fn get_property_value(&self, name: String) -> String {
        let name = name.cow_to_ascii_lowercase();
        let doc = self.doc.borrow();
        let mut maps = chain_maps(&doc, self.id, self.pseudo.as_deref());
        value_of(&mut maps, &name).unwrap_or_default()
    }

    /// Whether the engine knows this property at all.
    ///
    /// This answers the `in` operator, which wpt's `test_computed_value` checks before anything
    /// else: `assert_true(property in getComputedStyle(target))`. Answering from the property
    /// definitions rather than from the cascade is deliberate - a property the engine supports
    /// is "in" the computed style even when this element has no value for it, and reporting
    /// otherwise would fail every subtest on its first line.
    pub fn supports(&self, name: String) -> bool {
        get_css_definitions()
            .find_property(&name.cow_to_ascii_lowercase())
            .is_some()
    }
}

/// Backs the global `getComputedStyle`.
///
/// A class rather than a closure because `Function::new` cannot take one that is generic over
/// the context lifetime, which returning a `Value<'js>` requires; the method macro binds it.
#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "ComputedStyleFactory")]
pub struct ComputedStyleFactory {
    #[qjs(skip_trace)]
    doc: DocHandle,
}

#[rquickjs::methods]
impl ComputedStyleFactory {
    /// `getComputedStyle(element, pseudo)`. `null` for anything that is not one of our nodes.
    pub fn compute<'js>(&self, ctx: Ctx<'js>, target: Value<'js>, pseudo: Option<String>) -> Result<Value<'js>> {
        let Some(id) = crate::node::node_id_of(&target) else {
            return Ok(Value::new_null(ctx));
        };
        // The leading `::` is optional in the argument but not in the engine's pseudo names.
        let pseudo = pseudo
            .map(|p| p.trim_start_matches(':').to_string())
            .filter(|p| !p.is_empty());
        wrap(&ctx, &self.doc, id, pseudo)
    }
}

/// The JS helper that wraps a computed style in its property-access proxy.
pub(crate) const COMPUTED_PROXY: &str = "__gosub_computed_proxy";

/// Where the factory instance lives, for the `getComputedStyle` shim to call.
const COMPUTED_FACTORY: &str = "__gosub_computed_factory";

/// Read-only sibling of the `element.style` proxy.
///
/// Assignment is dropped rather than refused: `getComputedStyle(el).color = 'red'` is a no-op
/// in a browser, not an error, and a test that does it by accident should not die on it.
const COMPUTED_PROXY_SOURCE: &str = r#"
(function (computed) {
  const dashed = (name) => {
    if (name.startsWith("--")) return name;
    const css = name.replace(/[A-Z]/g, (c) => "-" + c.toLowerCase());
    if (css === "css-float") return "float";
    return /^(webkit|moz|ms|o)-/.test(css) ? "-" + css : css;
  };
  const bound = new Map();
  return new Proxy(computed, {
    get(target, key) {
      if (typeof key !== "string") return Reflect.get(target, key, target);
      if (key in target) {
        const value = Reflect.get(target, key, target);
        if (typeof value !== "function") return value;
        if (!bound.has(key)) bound.set(key, value.bind(target));
        return bound.get(key);
      }
      return target.getPropertyValue(dashed(key));
    },
    set() {
      return true;
    },
    has(target, key) {
      if (typeof key === "string" && !(key in target)) return target.supports(dashed(key));
      return Reflect.has(target, key);
    },
  });
})
"#;

/// Install the proxy factory and the global `getComputedStyle`. Called once per context.
pub(crate) fn install(ctx: &Ctx<'_>, doc: DocHandle) -> Result<()> {
    let factory: Function = ctx.eval(COMPUTED_PROXY_SOURCE)?;
    ctx.globals().set(COMPUTED_PROXY, factory)?;

    let instance = rquickjs::Class::instance(ctx.clone(), ComputedStyleFactory { doc })?;
    ctx.globals().set(COMPUTED_FACTORY, instance)?;
    // A shim so the global takes its second argument the way the spec does - optional, and
    // `null` when omitted.
    ctx.eval::<(), _>(format!(
        "globalThis.getComputedStyle = function (element, pseudo) {{ \
             return {COMPUTED_FACTORY}.compute(element, pseudo === undefined ? null : pseudo); \
         }};"
    ))?;
    Ok(())
}

/// Wrap an element's computed style for JS.
pub(crate) fn wrap<'js>(ctx: &Ctx<'js>, doc: &DocHandle, id: NodeId, pseudo: Option<String>) -> Result<Value<'js>> {
    let computed = rquickjs::Class::instance(ctx.clone(), GosubComputedStyle::new(doc.clone(), id, pseudo))?;
    let factory: Function<'js> = ctx.globals().get(COMPUTED_PROXY)?;
    factory.call((computed,))
}
