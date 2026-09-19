use crate::stylesheet::{CssValue, Specificity};
use crate::tokenizer::NumberKind;
use gosub_interface::css3::CssOrigin;
use std::collections::hash_map::Entry;

use crate::matcher::property_definitions::{get_css_definitions, CssDefinitions, PropertyDefinition};
use crate::matcher::styling::{CssProperties, CssProperty, DeclarationProperty};
use crate::matcher::syntax::GroupCombinators::Juxtaposition;
use crate::matcher::syntax::{GroupCombinators, SyntaxComponent, SyntaxComponentMultiplier};
use crate::matcher::syntax_matcher::CssSyntaxTree;

impl CssSyntaxTree {
    pub fn has_property_syntax(&self, property: &str) -> Option<Shorthand> {
        let component = self.components.first()?;

        let mut path = Vec::with_capacity(1);

        if component.has_property_syntax(property, &mut path) {
            Some(Shorthand {
                name: property.to_string(),
                components: path,
            })
        } else {
            None
        }
    }
}

impl SyntaxComponent {
    pub fn has_property_syntax(&self, prop: &str, path: &mut Vec<usize>) -> bool {
        match self {
            SyntaxComponent::Definition { datatype, quoted, .. } if *quoted => prop == datatype,
            SyntaxComponent::Group { components, .. } => {
                for (i, component) in components.iter().enumerate() {
                    path.push(i);
                    if component.has_property_syntax(prop, path) {
                        return true;
                    }
                    path.pop();
                }
                false
            }
            _ => false,
        }
    }

    #[must_use]
    pub fn multipliers(&self) -> &[SyntaxComponentMultiplier] {
        match self {
            SyntaxComponent::GenericKeyword { multipliers, .. } => multipliers,
            SyntaxComponent::Function { multipliers, .. } => multipliers,
            SyntaxComponent::Definition { multipliers, .. } => multipliers,
            SyntaxComponent::Inherit { multipliers, .. } => multipliers,
            SyntaxComponent::Initial { multipliers, .. } => multipliers,
            SyntaxComponent::Unset { multipliers, .. } => multipliers,
            SyntaxComponent::Literal { multipliers, .. } => multipliers,
            SyntaxComponent::Value { multipliers, .. } => multipliers,
            SyntaxComponent::Group { multipliers, .. } => multipliers,
            SyntaxComponent::Unit { multipliers, .. } => multipliers,
            SyntaxComponent::Builtin { multipliers, .. } => multipliers,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Multiplier {
    None, // component has no multiplier TODO: do we need this?
    NextProp,
    QuadMulti, // we can use everything that matches this component, and go to the next if we have more (e.g, border-radius)
    DuoMulti,
    OnlyMatched, // we need to extract values out of the matched components (e.g, background)
}

impl Multiplier {
    fn get_names(self, completed: Vec<&str>, multi: usize) -> Option<Vec<&str>> {
        match self {
            Multiplier::NextProp => Some(vec![completed.get(multi)?]),

            Multiplier::DuoMulti => {
                if multi == 0 {
                    return Some(completed);
                }

                Some(vec![completed.get(1)?])
            }

            Multiplier::QuadMulti => match multi {
                0 => Some(completed),
                1 => Some(completed.get(1..3)?.to_vec()),
                2 => Some(vec![completed.first()?]),
                3 => Some(vec![completed.get(1)?]),

                _ => None,
            },

            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Shorthands {
    multiplier: Multiplier,
    shorthands: Vec<Shorthand>,
    name: String,
    /// The shorthand is a comma-separated list of layers (`background`, `transition`,
    /// `animation`): every longhand collects one value per layer.
    layered: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FixList {
    list: Vec<(String, Vec<DeclarationProperty>)>,
    multipliers: Vec<(String, usize)>,
    /// The longhands the declaration currently being expanded has set, so
    /// [`FixList::reset_unmentioned`] can tell which of a shorthand's longhands it left out.
    /// Cleared by [`FixList::set_info`], which every expansion calls first.
    touched: Vec<String>,
    /// Set while a layered shorthand is being expanded: values go to `layer_values` instead
    /// of `list`, and [`FixList::reset_unmentioned`] assembles the comma lists at the end.
    layered: bool,
    /// The layer the values now being recorded belong to, counted from 0.
    layer: usize,
    /// `(layer, longhand, value)` recorded so far for a layered shorthand.
    layer_values: Vec<(usize, String, CssValue)>,

    current_info: Option<FixListInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FixListInfo {
    origin: CssOrigin,
    important: bool,
    location: String,
    specificity: Specificity,
    /// Shadow depth of the declaring sheet, carried through shorthand expansion so the
    /// longhands it produces keep the cross-tree half of the cascade.
    shadow_depth: u16,
    /// Document-order position of the shorthand being expanded. The longhands inherit it, so a
    /// longhand declared *after* the shorthand still wins the cascade even though every
    /// expansion is applied after all the direct declarations.
    order: u32,
    /// Cascade layer of the shorthand, carried through for the same reason as the depth.
    layer: Option<u32>,
    /// Whether the shorthand came from the element's `style` attribute.
    attached: bool,
}

impl FixListInfo {
    #[must_use]
    #[allow(clippy::too_many_arguments, reason = "one cascade fact per argument")]
    pub fn new(
        origin: CssOrigin,
        important: bool,
        location: String,
        specificity: Specificity,
        shadow_depth: u16,
        order: u32,
        layer: Option<u32>,
        attached: bool,
    ) -> Self {
        Self {
            origin,
            important,
            location,
            specificity,
            shadow_depth,
            order,
            layer,
            attached,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Shorthand {
    name: String,
    components: Vec<usize>,
}

#[derive(Debug)]
pub struct ShorthandResolver<'a> {
    name: &'a str,
    pub multiplier: Multiplier,
    fix_list: &'a mut FixList,
    shorthands: Vec<ResolveShorthand<'a>>,
    /// How many grammar levels below the shorthand's root this resolver sits.
    depth: usize,
    /// Whether the shorthand is a list of layers; see [`Shorthands::layered`].
    layered: bool,
}

pub fn copy_resolver<'a>(res: &'a mut Option<ShorthandResolver>) -> Option<ShorthandResolver<'a>> {
    if let Some(resolver) = res {
        Some(ShorthandResolver {
            multiplier: resolver.multiplier,
            fix_list: resolver.fix_list,
            shorthands: resolver
                .shorthands
                .iter()
                .map(|s| ResolveShorthand {
                    name: s.name,
                    components: s.components,
                })
                .collect(),
            name: resolver.name,
            depth: resolver.depth,
            layered: resolver.layered,
        })
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ResolveShorthand<'a> {
    pub name: &'a str,
    pub components: &'a [usize],
}

pub struct CompleteStep<'a> {
    list: &'a mut FixList,
    name: Vec<&'a str>,
    completed: bool,
    snapshot: Option<Snapshot>,
}

impl Drop for CompleteStep<'_> {
    fn drop(&mut self) {
        if !self.completed {
            if let Some(snap) = self.snapshot.take() {
                *self.list = snap.fix_list;
            }
        }
    }
}

impl Shorthands {
    pub fn get_resolver<'a>(&'a self, fix_list: &'a mut FixList) -> ShorthandResolver<'a> {
        fix_list.layered = self.layered;
        ShorthandResolver {
            multiplier: self.multiplier,
            fix_list,
            shorthands: self.shorthands.iter().map(Shorthand::resolver).collect(),
            name: &self.name,
            depth: 0,
            layered: self.layered,
        }
    }
}

impl Shorthand {
    #[must_use]
    pub fn resolver(&self) -> ResolveShorthand<'_> {
        ResolveShorthand {
            name: &self.name,
            components: &self.components,
        }
    }
}

pub struct Snapshot {
    fix_list: FixList,
}

impl<'a> ShorthandResolver<'a> {
    #[allow(clippy::result_large_err)]
    pub fn step(&'a mut self, idx: usize) -> Result<Option<Self>, CompleteStep<'a>> {
        let snapshot = Some(self.snapshot());

        let mut shorthands = Vec::with_capacity(self.shorthands.len());

        if matches!(
            self.multiplier,
            Multiplier::QuadMulti | Multiplier::DuoMulti | Multiplier::NextProp
        ) {
            let mut complete = Vec::with_capacity(self.shorthands.len());

            for shorthand in &self.shorthands {
                match shorthand.step_complete(idx) {
                    Some(Some(elem)) => {
                        shorthands.push(elem);
                    }
                    Some(None) => {
                        complete.push(shorthand.name);
                    }
                    None => {}
                }
            }

            if !complete.is_empty() {
                let idx = self.fix_list.multipliers.iter_mut().find(|m| m.0 == self.name);

                if let Some(idx) = idx {
                    let Some(items) = self.multiplier.get_names(complete.clone(), idx.1) else {
                        return Ok(None);
                    };

                    idx.1 += 1;

                    return Err(CompleteStep {
                        list: self.fix_list,
                        name: items,
                        snapshot,
                        completed: false,
                    });
                }

                let Some(items) = self.multiplier.get_names(complete.clone(), 0) else {
                    return Ok(None);
                };

                self.fix_list.multipliers.push((self.name.to_string(), 1));

                return Err(CompleteStep {
                    list: self.fix_list,
                    name: items,
                    snapshot,
                    completed: false,
                });
            }
        }

        for shorthand in &self.shorthands {
            match shorthand.step_complete(idx) {
                Some(Some(elem)) => {
                    shorthands.push(elem);
                }
                Some(None) => {
                    return Err(CompleteStep {
                        list: self.fix_list,
                        name: vec![shorthand.name],
                        snapshot,
                        completed: false,
                    });
                }
                None => {}
            }
        }

        if shorthands.is_empty() {
            return Ok(None);
        }

        Ok(Some(Self {
            multiplier: self.multiplier,
            fix_list: self.fix_list,
            shorthands,
            name: self.name,
            depth: self.depth + 1,
            layered: self.layered,
        }))
    }

    /// The matcher consumed a comma of a `#` list. At the root of a layered shorthand
    /// (`transition: a 1s, b 2s`) that comma separates two layers; anywhere deeper it is a
    /// list inside one value (`font-family: a, b` inside `font`) and means nothing here.
    pub fn layer_separator(&mut self) {
        if self.layered && self.depth == 0 {
            self.fix_list.next_layer();
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            fix_list: self.fix_list.clone(),
        }
    }
}

impl<'a> ResolveShorthand<'a> {
    fn step_complete<'c>(&'c self, idx: usize) -> Option<Option<ResolveShorthand<'a>>> {
        if self.components.is_empty() {
            return Some(None);
        }

        if self.components.first().copied() == Some(idx) {
            let components = &self.components[1..];

            if components.is_empty() {
                return Some(None);
            }

            return Some(Some(Self {
                name: self.name,
                components,
            }));
        }

        None
    }
}

impl Default for FixList {
    fn default() -> Self {
        Self::new()
    }
}

impl FixList {
    #[must_use]
    pub fn new() -> Self {
        Self {
            list: Vec::new(),
            multipliers: Vec::new(),
            touched: Vec::new(),
            layered: false,
            layer: 0,
            layer_values: Vec::new(),
            current_info: None,
        }
    }

    /// Start expanding a declaration: the cascade facts every longhand it produces will carry.
    pub fn set_info(&mut self, info: FixListInfo) {
        self.current_info = Some(info);
        self.touched.clear();
        self.layered = false;
        self.layer = 0;
        self.layer_values.clear();
    }

    /// Start the next layer of a layered shorthand.
    pub fn next_layer(&mut self) {
        self.layer += 1;
    }

    /// The value layer `layer` recorded for `name`, if any.
    fn layer_value(&self, layer: usize, name: &str) -> Option<&CssValue> {
        self.layer_values
            .iter()
            .find(|(l, n, _)| *l == layer && n == name)
            .map(|(_, _, v)| v)
    }

    /// Build the longhands of a layered shorthand from the per-layer values and record them.
    ///
    /// A longhand that is itself a list (`background-image: <bg-image>#`) gets one value per
    /// layer, the layer's own or the initial value where the layer left it out, joined by
    /// commas. One that is not (`background-color`) takes the last layer that set it, or the
    /// initial value. The list form of a single layer is the value itself.
    fn assemble_layers(&mut self, shorthand: &PropertyDefinition, definitions: &CssDefinitions) {
        let layers = self.layer + 1;
        // One box in a background layer sets both origin and clip (css-backgrounds-3 §2.10.1);
        // the grammar hands it to origin.
        for (origin, clip) in [("background-origin", "background-clip"), ("mask-origin", "mask-clip")] {
            for layer in 0..layers {
                if let (Some(value), None) = (self.layer_value(layer, origin).cloned(), self.layer_value(layer, clip)) {
                    self.layer_values.push((layer, clip.to_string(), value));
                }
            }
        }

        let mut assembled: Vec<(String, CssValue)> = Vec::new();
        for name in shorthand.expanded_properties() {
            let Some(def) = definitions.find_property(&name) else {
                continue;
            };
            let is_list = def.syntax().components.first().is_some_and(takes_comma_list);
            let value = if is_list {
                let mut items = Vec::with_capacity(layers * 2);
                for layer in 0..layers {
                    let item = self
                        .layer_value(layer, &name)
                        .cloned()
                        .or_else(|| def.initial_value.clone());
                    let Some(item) = item else {
                        log::debug!(
                            "{}: layer {layer} leaves {name} unset and it has no initial value",
                            shorthand.name()
                        );
                        items.clear();
                        break;
                    };
                    if !items.is_empty() {
                        items.push(CssValue::Comma);
                    }
                    items.push(item);
                }
                if items.is_empty() {
                    continue;
                }
                CssValue::from_vec(items)
            } else {
                let last = (0..layers)
                    .rev()
                    .find_map(|layer| self.layer_value(layer, &name).cloned());
                match last.or_else(|| def.initial_value.clone()) {
                    Some(value) => value,
                    None => continue,
                }
            };
            assembled.push((name, value));
        }

        self.layered = false;
        for (name, value) in assembled {
            self.insert(name, value);
        }
    }

    /// Give every longhand of `shorthand` that the declaration did not set its initial value.
    ///
    /// A shorthand sets *all* of its longhands (css-cascade-5 §2.5): the ones it does not
    /// mention are reset to their initial value, which is the whole point of writing one -
    /// `border: 1px solid` makes the colour `currentColor`, and `font: 12px serif` undoes an
    /// earlier `font-weight: bold`. The resolver only records what consumed a value, so without
    /// this an unmentioned longhand kept whatever it had.
    ///
    /// A longhand that is itself a shorthand (`border-color` under `border`) is walked into, so
    /// the reset lands on the real longhands. One whose initial value the definitions give only
    /// as prose (`font-family` is "depends on user agent") is left alone; it cannot be reset to
    /// anything, and nothing an author writes leaves it out anyway.
    pub fn reset_unmentioned(
        &mut self,
        shorthand: &PropertyDefinition,
        input: &[CssValue],
        definitions: &CssDefinitions,
    ) {
        if !shorthand.is_shorthand() {
            return;
        }
        // `flex` is the one common shorthand whose omitted values are not the longhand initials
        // (css-flexbox-1 §7.1.1): a lone `flex: 1` means `1 1 0`, not `1 1 auto`, and the
        // difference is whether three `flex: 1` columns come out equal or sized by their content.
        if shorthand.name() == "flex" {
            self.reset_flex(input);
            return;
        }
        // A shorthand whose grammar the resolver cannot map records nothing at all - `background`
        // is `[ <bg-layer> , ]* <final-bg-layer>`, beyond what the resolver follows. Resetting
        // every longhand of a shorthand this has not expanded would erase the value the author
        // gave (`background: #c22` would end as `background-color: transparent`), so an expansion
        // that set nothing is left alone: the consumer reads the stored shorthand instead.
        if self.touched.is_empty() {
            return;
        }
        if self.layered {
            self.assemble_layers(shorthand, definitions);
            return;
        }
        // css-lists-3 §3.5: a `none` in `list-style` means both `list-style-image: none` and
        // `list-style-type: none`, unless a type is given as well. The grammar hands the `none`
        // to the image, so the type has to be filled in here before the general reset would give
        // it `disc` - and `ul { list-style: none }` is how every page hides its bullets.
        if shorthand.name() == "list-style"
            && !self.touched.iter().any(|t| t == "list-style-type")
            && self.list.iter().any(|(name, decls)| {
                name == "list-style-image"
                    && decls.last().is_some_and(|d| {
                        matches!(&d.value, CssValue::None)
                            || matches!(&d.value, CssValue::String(s) if s.eq_ignore_ascii_case("none"))
                    })
            })
        {
            self.insert("list-style-type".to_string(), CssValue::String("none".to_string()));
        }
        for name in shorthand.expanded_properties() {
            if self.touched.contains(&name) {
                continue;
            }
            let Some(def) = definitions.find_property(&name) else {
                continue;
            };
            if def.is_shorthand() {
                self.reset_unmentioned(def, &[], definitions);
                continue;
            }
            match &def.initial_value {
                Some(initial) => self.insert(name, initial.clone()),
                None => log::debug!("{}: no initial value to reset {name} to", shorthand.name()),
            }
        }
    }

    /// Reset the {1,4}-multiplier counter for a specific shorthand name.
    /// Must be called before each CSS declaration is expanded so that the counter
    /// from a prior rule (e.g. `* { margin: 0 }`) does not bleed into the current
    /// rule (e.g. `.container { margin: 0 auto }`), causing wrong TRBL assignment.
    pub fn reset_multiplier(&mut self, name: &str) {
        self.multipliers.retain(|m| m.0 != name);
    }

    fn get_declaration(&self, value: CssValue) -> DeclarationProperty {
        if let Some(info) = &self.current_info {
            DeclarationProperty {
                value,
                origin: info.origin,
                important: info.important,
                specificity: info.specificity,
                location: info.location.clone(),
                shadow_depth: info.shadow_depth,
                order: info.order,
                layer: info.layer,
                attached: info.attached,
            }
        } else {
            DeclarationProperty {
                value,
                origin: CssOrigin::Author,
                important: false,
                specificity: Specificity::new(0, 0, 0),
                location: String::new(),
                // A declaration with no info is a synthesized default, not something an
                // author wrote, and it carries no tree of its own. Depth 0 would read as
                // "from the document" - the *winning* end of the cross-tree comparison for
                // normal declarations - and would then outrank the real declarations of the
                // shadow tree being expanded, which is how a shadow-tree `border-left` lost
                // to its own `border` shorthand. These entries are never `important`, so
                // parking them at the far end makes them lose that comparison instead and
                // fall back to losing on specificity, exactly as they did before there was
                // a cross-tree comparison at all.
                shadow_depth: u16::MAX,
                order: 0,
                // No layer and not element-attached: the same reasoning as the depth above
                // puts a synthesized default at the losing end of every comparison it can be.
                layer: None,
                attached: false,
            }
        }
    }

    /// The omitted-value defaults of the `flex` shorthand (css-flexbox-1 §7.1.1): `none` is
    /// `0 0 auto`; otherwise a missing grow or shrink is `1` and a missing basis is `0%` - the
    /// spec prose says `0`, but every browser serializes the omitted basis as `0%` and the WPT
    /// shorthand suite asserts exactly that.
    fn reset_flex(&mut self, input: &[CssValue]) {
        let is_none = matches!(input, [CssValue::None])
            || matches!(input, [CssValue::String(s)] if s.eq_ignore_ascii_case("none"));
        let one = || CssValue::Number(1.0, NumberKind::Integer);
        let zero = || CssValue::Number(0.0, NumberKind::Integer);
        let defaults: [(&str, CssValue); 3] = if is_none {
            [
                ("flex-grow", zero()),
                ("flex-shrink", zero()),
                ("flex-basis", CssValue::String("auto".to_string())),
            ]
        } else {
            [
                ("flex-grow", one()),
                ("flex-shrink", one()),
                ("flex-basis", CssValue::Percentage(0.0)),
            ]
        };
        for (name, value) in defaults {
            if is_none || !self.touched.iter().any(|t| t == name) {
                self.insert(name.to_string(), value);
            }
        }
    }

    pub fn insert(&mut self, name: String, value: CssValue) {
        if !self.touched.contains(&name) {
            self.touched.push(name.clone());
        }
        if self.layered {
            let layer = self.layer;
            if let Some(slot) = self.layer_values.iter_mut().find(|(l, n, _)| *l == layer && *n == name) {
                slot.2 = value;
            } else {
                self.layer_values.push((layer, name, value));
            }
            return;
        }
        let value = self.get_declaration(value);

        for (k, v) in &mut self.list {
            if *k == name {
                // Keep the cascade-winning declaration for this longhand. A higher- or
                // equal-priority declaration replaces the existing one: "equal" covers the
                // within-declaration TRBL re-assignment (last write wins), while a higher origin
                // (author > user-agent) or specificity wins regardless of stylesheet order. This
                // is why `body { margin: 0 }` (author) correctly overrides the UA `margin: 8px`.
                if v.last().is_none_or(|existing| value >= *existing) {
                    v.clear();
                    v.push(value);
                }
                return;
            }
        }

        self.list.push((name, vec![value]));
    }

    pub fn resolve_nested(&mut self, definitions: &CssDefinitions) {
        let mut fix_list = FixList::new();

        let mut had_shorthands = false;

        for (name, decl) in &self.list {
            let Some(prop) = definitions.find_property(name) else {
                continue;
            };

            if !prop.is_shorthand() {
                continue;
            }

            let Some(decl) = decl.iter().max() else { continue };

            had_shorthands = true;

            // The longhands a *nested* shorthand expands to are still the author's declaration and
            // must carry its cascade metadata. Without this they fell through to the synthesized
            // default below - author origin, zero specificity, order 0, depth `u16::MAX` - so for
            // `border-left-width: 4px; border: 1px solid` the `border-left-width` produced by the
            // second declaration lost to the first, and the earlier longhand won.
            fix_list.set_info(FixListInfo::new(
                decl.origin,
                decl.important,
                decl.location.clone(),
                decl.specificity,
                decl.shadow_depth,
                decl.order,
                decl.layer,
                decl.attached,
            ));

            if prop.matches_and_shorthands(decl.value.to_slice(), &mut fix_list) {
                fix_list.reset_unmentioned(prop, decl.value.to_slice(), definitions);
            }
        }

        if had_shorthands {
            fix_list.resolve_nested(definitions);
        }

        self.append(fix_list);
    }

    pub fn append(&mut self, mut other: FixList) {
        self.list.append(&mut other.list);
    }

    pub fn apply(&mut self, props: &mut CssProperties) {
        for (name, value) in &self.list {
            let Some(decl) = value.iter().max().cloned() else {
                continue;
            };

            match props.properties.entry(name.clone()) {
                Entry::Occupied(mut entry) => {
                    let prop = entry.get_mut();

                    prop.declared.push(decl);
                }
                Entry::Vacant(entry) => {
                    let mut prop = CssProperty::new(name);

                    prop.declared.push(decl);

                    entry.insert(prop);
                }
            }
        }
    }
}

impl CompleteStep<'_> {
    pub fn complete(mut self, value: Vec<CssValue>) {
        // The step is complete either way, so the snapshot is not restored. But an optional
        // piece that matched nothing (`<'flex-shrink'>?` in `flex: 1`) has not set its
        // longhand: recording it as a `None` value here made `flex-shrink` look mentioned, and
        // the reset then left it at `none` instead of the `1` the shorthand's defaults give it.
        self.completed = true;
        // The layer-separating comma of `[ <layer> , ]* <final-layer>` is mapped to the empty
        // name: matching it starts the next layer rather than recording a value.
        if self.name.iter().any(|name| name.is_empty()) {
            self.list.next_layer();
            return;
        }
        if value.is_empty() {
            return;
        }
        let val = CssValue::from_vec(value);

        for name in self.name.clone() {
            self.list.insert(name.to_string(), val.clone());
        }
    }
}

/// Reorder a 4-longhand box list into the [bottom, left, right, top] order QuadMulti's
/// value->side mapping expects. Lists that are not four distinct sides (or already carry no
/// side keywords) are returned unchanged - only the order is normalised, never the names.
fn quad_side_order(computed: &[String]) -> Vec<String> {
    if computed.len() != 4 {
        return computed.to_vec();
    }
    let side_of = |name: &str| -> Option<usize> {
        // Match the side as a hyphen-delimited segment so e.g. `border-top-color`
        // and bare `top` both classify, but unrelated names never do.
        for (i, side) in ["bottom", "left", "right", "top"].iter().enumerate() {
            if name.split('-').any(|seg| seg == *side) {
                return Some(i);
            }
        }
        None
    };
    let mut ordered: [Option<&String>; 4] = [None; 4];
    for name in computed {
        match side_of(name) {
            Some(i) if ordered[i].is_none() => ordered[i] = Some(name),
            // Missing or duplicate side: not a box shorthand - keep the original order.
            _ => return computed.to_vec(),
        }
    }
    ordered.into_iter().flatten().cloned().collect()
}

/// Component paths for the `font` shorthand's longhands, validated against the inlined grammar
/// `[ [ style || variant || weight || stretch ]? <font-size> [ / <line-height> ]? <font-family># ]
/// | <system-font>`. The shape checks guard against a regenerated definitions file silently
/// changing the tree: on mismatch the shorthand expands to nothing again (and the unit test that
/// asserts the expansion catches it).
fn font_shorthands(syntax: &CssSyntaxTree, name: &str) -> Option<Shorthands> {
    // Top level: `main-form | system-font` (ExactlyOne). The main form is component 0.
    let SyntaxComponent::Group {
        components: top,
        combinator: GroupCombinators::ExactlyOne,
        ..
    } = syntax.components.first()?
    else {
        return None;
    };
    let SyntaxComponent::Group {
        components: main,
        combinator: GroupCombinators::Juxtaposition,
        ..
    } = top.first()?
    else {
        return None;
    };
    if main.len() != 4 {
        return None;
    }

    // main[0]: the optional `||` prelude with exactly style/variant/weight/stretch operands.
    let SyntaxComponent::Group {
        components: prelude,
        combinator: GroupCombinators::AtLeastOneAnyOrder,
        ..
    } = &main[0]
    else {
        return None;
    };
    if prelude.len() != 4 {
        return None;
    }

    // main[2]: the `[ / <line-height> ]` group - a juxtaposition starting with the `/` literal.
    let SyntaxComponent::Group { components: slash, .. } = &main[2] else {
        return None;
    };
    if !matches!(slash.first(), Some(SyntaxComponent::Literal { literal, .. }) if literal == "/") {
        return None;
    }

    let paths: &[(&str, &[usize])] = &[
        ("font-style", &[0, 0, 0]),
        ("font-variant", &[0, 0, 1]),
        ("font-weight", &[0, 0, 2]),
        ("font-stretch", &[0, 0, 3]),
        ("font-size", &[0, 1]),
        ("line-height", &[0, 2, 1]),
        ("font-family", &[0, 3]),
    ];

    Some(Shorthands {
        multiplier: Multiplier::None,
        name: name.to_string(),
        layered: false,
        shorthands: paths
            .iter()
            .map(|(prop, path)| Shorthand {
                name: (*prop).to_string(),
                components: path.to_vec(),
            })
            .collect(),
    })
}

/// Whether `def` is a shorthand this crate can expand into longhands: one with a shape map, or
/// one of the few whose longhands are placed by position instead ([`EXPANDED_BY_HAND`]).
#[must_use]
pub(crate) fn is_expandable_shorthand(def: &PropertyDefinition) -> bool {
    def.is_shorthand() && (def.shorthands.is_some() || EXPANDED_BY_HAND.contains(&def.name()))
}

/// The longhands `property` ultimately sets, in definition order, with nested shorthands
/// flattened: `border` gives the twelve `border-<side>-<width|style|color>`. Empty for a
/// longhand or an unknown property.
///
/// A nested shorthand the resolver cannot expand is kept as a leaf, so what it carries is not
/// lost. So is `background-position`, which the resolver *can* expand: this list is the CSSOM's
/// view of a declaration block, and the CSSOM cannot yet serialize a shorthand back from its
/// longhands - so flattening it here would make `e.style.background = "... 1px 2px ..."` answer
/// nothing at all for `background-position`. The cascade does not read this list, and expands it
/// there as it should.
#[must_use]
pub fn longhands_of(property: &str) -> Vec<String> {
    fn walk(definitions: &CssDefinitions, name: &str, depth: usize, out: &mut Vec<String>) {
        let Some(def) = definitions.find_property(name) else {
            return;
        };
        let expandable = is_expandable_shorthand(def) && depth < 8 && !(depth > 0 && name == "background-position");
        if !expandable {
            if depth > 0 {
                out.push(name.to_string());
            }
            return;
        }
        for longhand in def.expanded_properties() {
            walk(definitions, &longhand, depth + 1, out);
        }
    }
    let mut out = Vec::new();
    walk(get_css_definitions(), property, 0, &mut out);
    out
}

/// Expand one shorthand declaration into the longhands it sets, each with its value: what the
/// cascade records for it, the ones the value leaves out at their initial value, in the order
/// of [`longhands_of`]. `None` when `property` is not a shorthand the resolver can expand, or
/// when `value` does not match its grammar.
///
/// This is the CSSOM's view of a shorthand: `element.style.gap = "10px 20px"` is stored as
/// `row-gap: 10px; column-gap: 20px`.
#[must_use]
pub fn expand_shorthand(property: &str, value: &CssValue) -> Option<Vec<(String, CssValue)>> {
    let definitions = get_css_definitions();
    let def = definitions.find_property(property)?;
    if !is_expandable_shorthand(def) {
        return None;
    }
    let input = value.to_slice();
    let mut fix_list = FixList::new();
    fix_list.set_info(FixListInfo::new(
        CssOrigin::Author,
        false,
        String::new(),
        Specificity::new(0, 0, 0),
        0,
        0,
        None,
        false,
    ));
    if !def.matches_and_shorthands(input, &mut fix_list) {
        return None;
    }
    fix_list.reset_unmentioned(def, input, definitions);
    fix_list.resolve_nested(definitions);
    let recorded: std::collections::HashMap<&str, &CssValue> = fix_list
        .list
        .iter()
        .filter_map(|(name, declared)| declared.last().map(|d| (name.as_str(), &d.value)))
        .collect();
    // Nothing recorded means this shorthand has neither a shape map nor positional rules, so
    // the CSSOM keeps the declaration as written rather than reporting that it sets no
    // longhands at all.
    if recorded.is_empty() {
        return None;
    }
    Some(
        longhands_of(property)
            .into_iter()
            .filter_map(|longhand| {
                recorded
                    .get(longhand.as_str())
                    .map(|v| (longhand.clone(), (*v).clone()))
            })
            .collect(),
    )
}

/// The shorthands no shape map can describe, expanded from the spec's own rules in
/// [`expand_by_hand`] instead.
///
/// Two things put a shorthand here. Its longhands may share one grammar and be told apart by
/// position (`grid-area`'s four are all a `<grid-line>`), or its grammar may mix references to
/// other shorthands with keywords that steer the expansion rather than supply a value
/// (`auto-flow` in `grid` says which longhand the track size belongs to).
///
/// [`CssDefinitions::resolve_shorthands`] stops before its fallbacks for these. Left to them,
/// `grid-row` picked up a map that handed `grid-row-end` the `/` along with the line after it.
pub(crate) const EXPANDED_BY_HAND: &[&str] = &[
    "grid-row",
    "grid-column",
    "grid-area",
    "grid",
    "grid-template",
    "background-position",
];

/// The shorthands whose longhands cannot be told apart by grammar shape, and so are expanded
/// from the spec's own positional rules instead.
///
/// [`CssDefinitions::map_by_shape`] pairs a piece of a shorthand's grammar with the longhand
/// whose whole grammar has the same shape. That works whenever each longhand accepts something
/// the others do not, which is most of them - but not when several longhands share one grammar
/// and are told apart by *where* they sit. All four of `grid-area`'s longhands are a
/// `<grid-line>`; which one a value means is decided by how many values there are and which
/// side of the slash they are on. No shape can say that, so the map comes back ambiguous and
/// the shorthand used to expand to nothing at all - leaving `grid-row-start: 3; grid-area: 1 /
/// 2` with the 3 still in place.
pub(crate) fn expand_by_hand(name: &str, input: &[CssValue], fix_list: &mut FixList) {
    match name {
        "grid-row" => grid_line_pair(input, "grid-row-start", "grid-row-end", fix_list),
        "grid-column" => grid_line_pair(input, "grid-column-start", "grid-column-end", fix_list),
        "grid-area" => grid_area(input, fix_list),
        "grid-template" => {
            if let Some(longhands) = grid_template(input) {
                for (name, value) in longhands {
                    fix_list.insert(name, value);
                }
            }
        }
        "grid" => grid(input, fix_list),
        "background-position" => background_position(input, fix_list),
        _ => {}
    }
}

/// Which axis a `<position>` keyword belongs to. `center` takes whichever axis is still free.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
    Either,
}

fn position_keyword(value: &CssValue) -> Option<Axis> {
    let CssValue::String(word) = value else {
        return None;
    };
    for (keyword, axis) in [
        ("left", Axis::Horizontal),
        ("right", Axis::Horizontal),
        ("top", Axis::Vertical),
        ("bottom", Axis::Vertical),
        ("center", Axis::Either),
    ] {
        if word.eq_ignore_ascii_case(keyword) {
            return Some(axis);
        }
    }
    None
}

/// Split one `<bg-position>` into its horizontal and vertical halves (css-values-4 §9.5).
///
/// The two halves are what `background-position-x` and `background-position-y` each hold, and
/// they are not simply the first and second value: the keywords may be written in either order
/// (`top left`), a single value leaves the other axis at `center`, and the three- and four-value
/// forms pair each keyword with the offset that follows it (`left 10px top`).
fn bg_position_axes(layer: &[CssValue]) -> Option<(Vec<CssValue>, Vec<CssValue>)> {
    let center = || vec![CssValue::String("center".to_string())];
    match layer {
        [] => None,
        // One value sets one axis and centres the other.
        [single] => match position_keyword(single) {
            Some(Axis::Vertical) => Some((center(), vec![single.clone()])),
            _ => Some((vec![single.clone()], center())),
        },
        // Two values are one per axis, in either order when both are keywords.
        [first, second] => {
            let swapped =
                position_keyword(first) == Some(Axis::Vertical) || position_keyword(second) == Some(Axis::Horizontal);
            if swapped {
                Some((vec![second.clone()], vec![first.clone()]))
            } else {
                Some((vec![first.clone()], vec![second.clone()]))
            }
        }
        // Three or four values are keyword-and-offset pairs, which say their own axis.
        _ => {
            let mut components: Vec<(Axis, Vec<CssValue>)> = Vec::with_capacity(2);
            let mut index = 0;
            while index < layer.len() {
                let axis = position_keyword(&layer[index])?;
                let mut component = vec![layer[index].clone()];
                index += 1;
                // An offset may follow, but never after `center`.
                if axis != Axis::Either && layer.get(index).is_some_and(|next| position_keyword(next).is_none()) {
                    component.push(layer[index].clone());
                    index += 1;
                }
                components.push((axis, component));
            }

            let mut horizontal: Option<Vec<CssValue>> = None;
            let mut vertical: Option<Vec<CssValue>> = None;
            // The keywords that name an axis are placed first: `center` takes whichever one is
            // left, and in `center right 7%` that is only known once `right` has been read.
            for (axis, component) in components
                .iter()
                .filter(|(axis, _)| *axis != Axis::Either)
                .chain(components.iter().filter(|(axis, _)| *axis == Axis::Either))
            {
                let slot = match axis {
                    Axis::Horizontal => &mut horizontal,
                    Axis::Vertical => &mut vertical,
                    Axis::Either if horizontal.is_none() => &mut horizontal,
                    Axis::Either => &mut vertical,
                };
                if slot.is_some() {
                    return None;
                }
                *slot = Some(component.clone());
            }
            Some((horizontal.unwrap_or_else(center), vertical.unwrap_or_else(center)))
        }
    }
}

/// `background-position`: `<bg-position>#` over `background-position-x` and
/// `background-position-y` (css-backgrounds-4 §3.6).
///
/// One `<bg-position>` covers both axes at once, so no piece of the shorthand's grammar has the
/// shape of either longhand and the shape mapper finds nothing. Each comma-separated layer is
/// split, and the halves are collected back into one comma list per longhand.
fn background_position(input: &[CssValue], fix_list: &mut FixList) {
    let mut horizontal: Vec<CssValue> = Vec::new();
    let mut vertical: Vec<CssValue> = Vec::new();
    for (index, layer) in input.split(|value| matches!(value, CssValue::Comma)).enumerate() {
        let Some((x, y)) = bg_position_axes(layer) else {
            return;
        };
        if index > 0 {
            horizontal.push(CssValue::Comma);
            vertical.push(CssValue::Comma);
        }
        horizontal.extend(x);
        vertical.extend(y);
    }
    if horizontal.is_empty() {
        return;
    }
    fix_list.insert("background-position-x".to_string(), CssValue::from_vec(horizontal));
    fix_list.insert("background-position-y".to_string(), CssValue::from_vec(vertical));
}

/// `grid-template`: `none | <'grid-template-rows'> / <'grid-template-columns'> | [ <line-names>?
/// <string> <track-size>? <line-names>? ]+ [ / <explicit-track-list> ]?` (css-grid-2 §7.3).
///
/// `None` when the declaration is the third form, the one that draws the grid as rows of area
/// names. Reconstructing `grid-template-rows` from it means merging the line names on either
/// side of each string into one bracketed group, and the strings themselves are not yet told
/// apart from identifiers by the value parser. Returning nothing leaves the declaration
/// unexpanded, which is what the resolver does with any shorthand it cannot take apart.
fn grid_template(input: &[CssValue]) -> Option<Vec<(String, CssValue)>> {
    let none = || CssValue::String("none".to_string());
    if matches!(input, [CssValue::None]) || matches!(input, [CssValue::String(s)] if s.eq_ignore_ascii_case("none")) {
        return Some(vec![
            ("grid-template-rows".to_string(), none()),
            ("grid-template-columns".to_string(), none()),
            ("grid-template-areas".to_string(), none()),
        ]);
    }
    let parts = split_on_solidus(input);
    let (rows, columns) = match parts.as_slice() {
        [rows, columns] if !rows.is_empty() && !columns.is_empty() => (*rows, Some(*columns)),
        // The row-of-names form may leave the column track list out entirely.
        [rows] if !rows.is_empty() => (*rows, None),
        _ => return None,
    };

    if rows.iter().any(is_area_string) {
        return grid_template_areas(rows, columns);
    }
    let columns = columns?;
    Some(vec![
        ("grid-template-rows".to_string(), part_value(rows)),
        ("grid-template-columns".to_string(), part_value(columns)),
        ("grid-template-areas".to_string(), none()),
    ])
}

/// The `grid-template` form that draws the grid as rows of area names: `[ <line-names>?
/// <string> <track-size>? <line-names>? ]+ [ / <explicit-track-list> ]?` (css-grid-2 §7.3).
///
/// Each string is one row of `grid-template-areas`. `grid-template-rows` is rebuilt from what
/// surrounds them: the names before a row, then its track size or `auto` when it has none. The
/// names written after one row and before the next name the same grid line, so they are emitted
/// as a single bracketed group - which is why this cannot be a straight copy of the input.
fn grid_template_areas(rows: &[CssValue], columns: Option<&[CssValue]>) -> Option<Vec<(String, CssValue)>> {
    let is_bracket = |value: &CssValue, bracket: &str| matches!(value, CssValue::String(s) if s == bracket);

    let mut areas: Vec<CssValue> = Vec::new();
    let mut tracks: Vec<CssValue> = Vec::new();
    let mut names: Vec<CssValue> = Vec::new();
    let mut index = 0;
    while index < rows.len() {
        if is_bracket(&rows[index], "[") {
            index += 1;
            while index < rows.len() && !is_bracket(&rows[index], "]") {
                names.push(rows[index].clone());
                index += 1;
            }
            // An unclosed group is not a line-name list; leave the declaration unexpanded.
            if index >= rows.len() {
                return None;
            }
            index += 1;
            continue;
        }
        if !is_area_string(&rows[index]) {
            return None;
        }
        areas.push(rows[index].clone());
        index += 1;

        // Everything collected since the previous row names this row's start line. An empty
        // group names nothing and is left out.
        if !names.is_empty() {
            tracks.push(CssValue::String("[".to_string()));
            tracks.append(&mut names);
            tracks.push(CssValue::String("]".to_string()));
        }
        // A track size may follow the row. Anything that is not a name group or the next row is
        // one; a row with none is `auto`.
        let size = rows
            .get(index)
            .filter(|value| !is_bracket(value, "[") && !is_area_string(value));
        match size {
            Some(size) => {
                tracks.push(size.clone());
                index += 1;
            }
            None => tracks.push(CssValue::String("auto".to_string())),
        }
    }
    if areas.is_empty() {
        return None;
    }
    // Names after the last row name the grid's final line.
    if !names.is_empty() {
        tracks.push(CssValue::String("[".to_string()));
        tracks.append(&mut names);
        tracks.push(CssValue::String("]".to_string()));
    }

    let columns = match columns {
        Some(columns) => part_value(columns),
        None => CssValue::String("none".to_string()),
    };
    Some(vec![
        ("grid-template-rows".to_string(), CssValue::from_vec(tracks)),
        ("grid-template-columns".to_string(), columns),
        ("grid-template-areas".to_string(), CssValue::from_vec(areas)),
    ])
}

/// Whether a value in a `grid-template` track position can only be one of the area strings.
///
/// A quoted string and an identifier are the same `CssValue` today, so this reads the position
/// instead: the grammar allows no bare identifier there apart from the few keywords below, and
/// a track size is never a plain word.
fn is_area_string(value: &CssValue) -> bool {
    let CssValue::String(text) = value else {
        return false;
    };
    !["none", "auto", "subgrid", "min-content", "max-content", "masonry", "/"]
        .iter()
        .any(|keyword| text.eq_ignore_ascii_case(keyword))
}

/// `grid` (css-grid-2 §7.4). Three forms: a `grid-template`, or one of the two that name
/// `auto-flow` on the side of the slash whose tracks are generated automatically.
fn grid(input: &[CssValue], fix_list: &mut FixList) {
    let keyword = |name: &str| CssValue::String(name.to_string());
    let is = |value: &CssValue, name: &str| matches!(value, CssValue::String(s) if s.eq_ignore_ascii_case(name));

    let parts = split_on_solidus(input);
    let auto_flow_side = parts
        .iter()
        .position(|part| part.iter().any(|value| is(value, "auto-flow")));

    // No `auto-flow` anywhere: this is a `grid-template`, and the automatic-track longhands take
    // their initial values.
    let Some(side) = auto_flow_side else {
        let Some(longhands) = grid_template(input) else {
            return;
        };
        for (name, value) in longhands {
            fix_list.insert(name, value);
        }
        fix_list.insert("grid-auto-rows".to_string(), keyword("auto"));
        fix_list.insert("grid-auto-columns".to_string(), keyword("auto"));
        fix_list.insert("grid-auto-flow".to_string(), keyword("row"));
        return;
    };

    let [first, second] = parts.as_slice() else {
        return;
    };
    // `auto-flow && dense?`: both orders are allowed, and what is left over is the track size
    // for the automatically generated tracks - `auto` when it is left out.
    let (flow_side, track_side) = if side == 0 {
        (*first, *second)
    } else {
        (*second, *first)
    };
    let dense = flow_side.iter().any(|value| is(value, "dense"));
    let sizes: Vec<CssValue> = flow_side
        .iter()
        .filter(|value| !is(value, "auto-flow") && !is(value, "dense"))
        .cloned()
        .collect();
    if track_side.is_empty() {
        return;
    }
    let auto_tracks = if sizes.is_empty() {
        keyword("auto")
    } else {
        part_value(&sizes)
    };

    // `auto-flow` on the left generates rows and the right-hand side is the column template;
    // on the right it is the other way round.
    let (flow, auto_name, template_name, empty_template) = if side == 0 {
        ("row", "grid-auto-rows", "grid-template-columns", "grid-template-rows")
    } else {
        (
            "column",
            "grid-auto-columns",
            "grid-template-rows",
            "grid-template-columns",
        )
    };
    let flow = if dense {
        CssValue::List(vec![keyword(flow), keyword("dense")])
    } else {
        keyword(flow)
    };

    fix_list.insert("grid-auto-flow".to_string(), flow);
    fix_list.insert(auto_name.to_string(), auto_tracks);
    fix_list.insert(template_name.to_string(), part_value(track_side));
    fix_list.insert(empty_template.to_string(), keyword("none"));
    fix_list.insert("grid-template-areas".to_string(), keyword("none"));
    // The other automatic track size is the one this form does not name.
    let other_auto = if side == 0 {
        "grid-auto-columns"
    } else {
        "grid-auto-rows"
    };
    fix_list.insert(other_auto.to_string(), keyword("auto"));
}

/// Whether a `<grid-line>` is a lone `<custom-ident>`, which is what decides an omitted end
/// line (css-grid-2 §8.3): `grid-row: foo` spans from `foo` to `foo`, `grid-row: 1` from line 1
/// to `auto`. `auto` and `span` are keywords of the grammar, not names.
fn is_line_name(part: &[CssValue]) -> bool {
    matches!(part, [CssValue::String(name)] if !name.eq_ignore_ascii_case("auto") && !name.eq_ignore_ascii_case("span"))
}

/// Split a value on the `/` that separates the lines of a grid placement shorthand.
fn split_on_solidus(input: &[CssValue]) -> Vec<&[CssValue]> {
    let mut parts = Vec::with_capacity(4);
    let mut rest = input;
    while let Some(at) = rest
        .iter()
        .position(|value| matches!(value, CssValue::String(s) if s == "/"))
    {
        parts.push(&rest[..at]);
        rest = &rest[at + 1..];
    }
    parts.push(rest);
    parts
}

fn part_value(part: &[CssValue]) -> CssValue {
    match part {
        [single] => single.clone(),
        many => CssValue::List(many.to_vec()),
    }
}

/// The line a placement shorthand leaves out: the start line when that is a name, `auto`
/// otherwise.
fn omitted_line(start: &[CssValue]) -> CssValue {
    if is_line_name(start) {
        part_value(start)
    } else {
        CssValue::String("auto".to_string())
    }
}

/// `grid-row` and `grid-column`: `<grid-line> [ / <grid-line> ]?` (css-grid-2 §8.3).
fn grid_line_pair(input: &[CssValue], start_name: &str, end_name: &str, fix_list: &mut FixList) {
    let parts = split_on_solidus(input);
    let Some(start) = parts.first().filter(|part| !part.is_empty()) else {
        return;
    };
    fix_list.insert(start_name.to_string(), part_value(start));
    let end = match parts.get(1).filter(|part| !part.is_empty()) {
        Some(end) => part_value(end),
        None => omitted_line(start),
    };
    fix_list.insert(end_name.to_string(), end);
}

/// `grid-area`: `<grid-line> [ / <grid-line> ]{0,3}` (css-grid-2 §8.4). The values are
/// row-start, column-start, row-end, column-end in that order, and each one left out copies the
/// line it pairs with when that is a name, or falls back to `auto`.
fn grid_area(input: &[CssValue], fix_list: &mut FixList) {
    let parts = split_on_solidus(input);
    let Some(row_start) = parts.first().filter(|part| !part.is_empty()) else {
        return;
    };
    let column_start = parts.get(1).filter(|part| !part.is_empty());
    let row_end = parts.get(2).filter(|part| !part.is_empty());
    let column_end = parts.get(3).filter(|part| !part.is_empty());

    let column_start_value = column_start.map_or_else(|| omitted_line(row_start), |part| part_value(part));
    let row_end_value = row_end.map_or_else(|| omitted_line(row_start), |part| part_value(part));
    // The column end pairs with the column start, which may itself have been copied from the
    // row start - `grid-area: foo` is `foo` on all four sides.
    let column_end_value = column_end.map_or_else(
        || match column_start {
            Some(part) => omitted_line(part),
            None => omitted_line(row_start),
        },
        |part| part_value(part),
    );

    fix_list.insert("grid-row-start".to_string(), part_value(row_start));
    fix_list.insert("grid-column-start".to_string(), column_start_value);
    fix_list.insert("grid-row-end".to_string(), row_end_value);
    fix_list.insert("grid-column-end".to_string(), column_end_value);
}

/// Longhands that share a grammar piece, in the order the spec hands the pieces to them: the
/// first `<time>` of a transition is its duration and the second its delay
/// (css-transitions-1 §2.5), the first `<visual-box>` of a background layer is the origin and
/// the second the clip (css-backgrounds-3 §2.10.1).
const SHARED_PIECES: &[(&str, &[&str])] = &[
    ("transition", &["transition-duration", "transition-delay"]),
    ("animation", &["animation-duration", "animation-delay"]),
    ("background", &["background-origin", "background-clip"]),
    ("mask", &["mask-origin", "mask-clip"]),
];

/// Whether two grammar nodes have the same shape, multipliers and ranges aside.
///
/// This is what makes `<bg-image>` in a background layer the piece for `background-image`,
/// whose own grammar is `<bg-image>#`, and `<time [0s,∞]>#` (duration) claim a plain `<time>`.
fn same_shape(a: &SyntaxComponent, b: &SyntaxComponent) -> bool {
    use SyntaxComponent as C;
    match (a, b) {
        (
            C::Definition {
                datatype: x,
                quoted: qx,
                ..
            },
            C::Definition {
                datatype: y,
                quoted: qy,
                ..
            },
        ) => x == y && qx == qy,
        (C::Builtin { datatype: x, .. }, C::Builtin { datatype: y, .. }) => x == y,
        (C::GenericKeyword { keyword: x, .. }, C::GenericKeyword { keyword: y, .. }) => x == y,
        (C::Literal { literal: x, .. }, C::Literal { literal: y, .. }) => x == y,
        (C::Value { value: x, .. }, C::Value { value: y, .. }) => x == y,
        (C::Unit { unit: x, .. }, C::Unit { unit: y, .. }) => x == y,
        (
            C::Function {
                name: x, arguments: ax, ..
            },
            C::Function {
                name: y, arguments: ay, ..
            },
        ) => {
            x == y
                && match (ax, ay) {
                    (Some(a), Some(b)) => same_shape(a, b),
                    (None, None) => true,
                    _ => false,
                }
        }
        (C::Inherit { .. }, C::Inherit { .. })
        | (C::Initial { .. }, C::Initial { .. })
        | (C::Unset { .. }, C::Unset { .. }) => true,
        (
            C::Group {
                components: x,
                combinator: cx,
                ..
            },
            C::Group {
                components: y,
                combinator: cy,
                ..
            },
        ) => cx == cy && x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same_shape(a, b)),
        _ => false,
    }
}

/// Whether a longhand's grammar is a comma-separated list: `<bg-image>#`, or an alternation
/// with a list arm, `none | <single-transition-property>#`.
fn takes_comma_list(root: &SyntaxComponent) -> bool {
    let is_list = |c: &SyntaxComponent| {
        c.get_multipliers()
            .iter()
            .any(|m| matches!(m, SyntaxComponentMultiplier::CommaSeparatedRepeat(..)))
    };
    if is_list(root) {
        return true;
    }
    matches!(root, SyntaxComponent::Group { components, combinator: GroupCombinators::ExactlyOne, .. } if components.iter().any(is_list))
}

/// State of one [`CssDefinitions::search_shape`] walk.
struct ShapeSearch<'a> {
    /// Each longhand with its own grammar, as one node.
    roots: &'a [(String, SyntaxComponent)],
    /// The shared-piece group of this shorthand, if any, and how many pieces it has taken.
    shared: &'a [&'a str],
    shared_taken: usize,
    /// A piece several longhands claim with no spec order to settle it: the map is unusable.
    ambiguous: bool,
    /// A value-bearing leaf no longhand claims (`auto-flow` in `grid`): the grammar sets
    /// something the map cannot place, so a partial map would reset what the author wrote.
    unclaimed: bool,
    /// Value types being descended into, against cycles.
    stack: Vec<String>,
    out: Vec<Shorthand>,
}

impl CssDefinitions {
    /// Map `computed`'s longhands onto the pieces of `syntax` by shape; see [`same_shape`].
    ///
    /// The walk descends into value types (`<bg-layer>`). Resolution places a type's
    /// components under a wrapper group at the reference's position, so a path composed here
    /// is exactly the path the matcher walks through the resolved tree. A claimed piece is not
    /// descended into: the `<color>` inside a gradient must not be mistaken for
    /// `background-color`.
    /// Returns the pieces found and whether they account for every value the grammar can
    /// carry - the second is what makes a map that misses a longhand still safe to use.
    fn map_by_shape(&self, computed: &[String], syntax: &CssSyntaxTree, name: &str) -> (Vec<Shorthand>, bool) {
        let roots: Vec<(String, SyntaxComponent)> = computed
            .iter()
            .filter_map(|longhand| {
                let def = self.properties.get(longhand)?;
                let root = match def.syntax.components.as_slice() {
                    [single] => single.clone(),
                    many => SyntaxComponent::Group {
                        components: many.to_vec(),
                        combinator: Juxtaposition,
                        multipliers: vec![],
                    },
                };
                Some((longhand.clone(), root))
            })
            .collect();
        let shared: &[&str] = SHARED_PIECES
            .iter()
            .find(|(shorthand, _)| *shorthand == name)
            .map_or(&[], |(_, members)| members);
        let mut search = ShapeSearch {
            roots: &roots,
            shared,
            shared_taken: 0,
            ambiguous: false,
            unclaimed: false,
            stack: Vec::new(),
            out: Vec::new(),
        };
        // The root itself is never a piece; its children are, at path `[i]`.
        if let Some(root) = syntax.components.first() {
            self.search_children(root, &[], &mut search);
        }
        if search.ambiguous {
            // `grid-area: <grid-line> [ / <grid-line> ]{0,3}`: four longhands with the same
            // grammar, told apart by position and by rules the grammar does not carry. A
            // partial map would reset what it cannot place, which is worse than no map.
            return (Vec::new(), false);
        }
        (search.out, !search.unclaimed)
    }

    fn search_children(&self, node: &SyntaxComponent, path: &[usize], search: &mut ShapeSearch) {
        match node {
            SyntaxComponent::Group { components, .. } => {
                for (i, child) in components.iter().enumerate() {
                    let mut child_path = path.to_vec();
                    child_path.push(i);
                    self.search_shape(child, child_path, search);
                }
            }
            SyntaxComponent::Definition {
                datatype,
                quoted: false,
                ..
            } => {
                if search.stack.len() >= 8 || search.stack.contains(datatype) {
                    return;
                }
                let Some(def) = self.syntax.get(datatype) else {
                    return;
                };
                search.stack.push(datatype.clone());
                for (i, child) in def.syntax.components.iter().enumerate() {
                    let mut child_path = path.to_vec();
                    child_path.push(i);
                    self.search_shape(child, child_path, search);
                }
                search.stack.pop();
            }
            // A quoted reference no longhand is named after (`<'grid-template'>` in `grid`,
            // itself a shorthand), or a value type the table does not define: a value lives
            // here that nothing places.
            SyntaxComponent::Definition { .. } | SyntaxComponent::Builtin { .. } => search.unclaimed = true,
            SyntaxComponent::GenericKeyword { .. }
            | SyntaxComponent::Function { .. }
            | SyntaxComponent::Value { .. }
            | SyntaxComponent::Unit { .. } => search.unclaimed = true,
            // Separators and the CSS-wide keywords carry no value of their own.
            SyntaxComponent::Literal { .. }
            | SyntaxComponent::Inherit { .. }
            | SyntaxComponent::Initial { .. }
            | SyntaxComponent::Unset { .. } => {}
        }
    }

    fn search_shape(&self, node: &SyntaxComponent, path: Vec<usize>, search: &mut ShapeSearch) {
        if !search.shared.is_empty()
            && search
                .roots
                .iter()
                .any(|(longhand, root)| search.shared.contains(&longhand.as_str()) && same_shape(node, root))
        {
            let member = search.shared[search.shared_taken % search.shared.len()];
            search.shared_taken += 1;
            search.out.push(Shorthand {
                name: member.to_string(),
                components: path,
            });
            return;
        }

        let claimants: Vec<&str> = search
            .roots
            .iter()
            .filter(|(longhand, root)| {
                matches!(node, SyntaxComponent::Definition { datatype, quoted: true, .. } if datatype == longhand)
                    || same_shape(node, root)
            })
            .map(|(longhand, _)| longhand.as_str())
            .collect();
        match claimants.as_slice() {
            [] => self.search_children(node, &path, search),
            [one] => search.out.push(Shorthand {
                name: (*one).to_string(),
                components: path,
            }),
            many => {
                log::debug!("shorthand piece at {path:?} is claimed by {many:?}; the map is abandoned");
                search.ambiguous = true;
            }
        }
    }

    /// Whether `syntax` is a list of layers and, for the `[ <layer> , ]* <final-layer>` shape,
    /// the path of the comma that separates them.
    fn layer_shape(syntax: &CssSyntaxTree) -> (bool, Option<Vec<usize>>) {
        let Some(root) = syntax.components.first() else {
            return (false, None);
        };
        if root
            .get_multipliers()
            .iter()
            .any(|m| matches!(m, SyntaxComponentMultiplier::CommaSeparatedRepeat(..)))
        {
            return (true, None);
        }
        if let SyntaxComponent::Group {
            components,
            combinator: GroupCombinators::Juxtaposition,
            ..
        } = root
        {
            if let Some(SyntaxComponent::Group {
                components: repeated,
                multipliers,
                ..
            }) = components.first()
            {
                let repeats = multipliers.iter().any(|m| {
                    matches!(
                        m,
                        SyntaxComponentMultiplier::ZeroOrMore | SyntaxComponentMultiplier::OneOrMore
                    )
                });
                let comma = repeated
                    .iter()
                    .position(|c| matches!(c, SyntaxComponent::Literal { literal, .. } if literal == ","));
                if let (true, Some(comma)) = (repeats, comma) {
                    return (true, Some(vec![0, comma]));
                }
            }
        }
        (false, None)
    }

    pub fn index_shorthands(&mut self) {
        let mut shorthands = Vec::new();

        for prop in self.properties.values() {
            let syntax = self.resolve_shorthands(&prop.computed, &prop.syntax, &prop.name);

            if let Some(syntax) = syntax {
                shorthands.push((prop.name.clone(), syntax));
            }
        }

        for (name, syntax) in shorthands {
            let Some(prop) = self.properties.get_mut(&name) else {
                continue;
            };

            prop.shorthands = Some(syntax);
        }
    }

    #[must_use]
    pub fn resolve_shorthands(&self, computed: &[String], syntax: &CssSyntaxTree, name: &str) -> Option<Shorthands> {
        if computed.len() <= 1 || syntax.components.is_empty() {
            return None;
        }

        // `font` gets hand-built component paths: its longhand references (`<'font-size'>`,
        // `<'line-height'>`, ...) are inlined into their value grammars at definition-load time,
        // so the name-based property search below finds nothing and the whole shorthand would
        // silently expand to nothing - dropping e.g. the line-height in `font: 12px/1.5 serif`.
        if name == "font" {
            return font_shorthands(syntax, name);
        }

        // Placed by position, not by grammar: no map can describe them, and the fallbacks below
        // produce a wrong one rather than none. `expand_by_hand` handles these.
        if EXPANDED_BY_HAND.contains(&name) {
            return None;
        }

        let mut shorthands: Vec<Shorthand> = Vec::with_capacity(computed.len());

        if let Some(component) = syntax.components.first() {
            for m in component.multipliers() {
                match m {
                    SyntaxComponentMultiplier::Between(_, b)
                    | SyntaxComponentMultiplier::CommaSeparatedRepeat(_, b)
                        if *b == computed.len() =>
                    {
                        // QuadMulti's value->side mapping assumes the longhand list order
                        // [bottom, left, right, top] (what margin/padding use in the
                        // definitions data). Properties listing their longhands in natural
                        // TRBL order (border-color/style/width) must be reordered, or
                        // `border-color: a b c LEFT` lands its 4th value on the RIGHT.
                        for c in quad_side_order(computed) {
                            shorthands.push(Shorthand {
                                name: c,
                                components: vec![],
                            });
                        }

                        let multiplier = match computed.len() {
                            2 => Multiplier::DuoMulti,
                            4 => Multiplier::QuadMulti,
                            _ => Multiplier::NextProp,
                        };

                        return Some(Shorthands {
                            multiplier,
                            shorthands,
                            name: name.to_string(),
                            layered: false,
                        });
                    }

                    _ => {}
                }
            }

            // `border-radius` is `<lp>{1,4} [ / <lp>{1,4} ]?`, so - unlike margin/padding
            // whose `{1,4}` sits on the top-level component - the top-level component is a
            // Juxtaposition group whose FIRST child carries the `{1,4}` multiplier (the second
            // child is the optional `/ <vertical-radii>` part). Detect that shape and drive the
            // same {1,4}-value box expansion.
            //
            // The QuadMulti expansion is tuned for the TRBL box order (margin/padding scramble
            // their `computed` list to `[bottom, left, right, top]` so `get_names` lands each
            // value on the right side). Corner properties are listed naturally as
            // `[top-left, top-right, bottom-right, bottom-left]`, so feed the corners to the
            // resolver reordered to `[BR, BL, TR, TL]`; that makes `get_names` reproduce the CSS
            // corner rules (1 value -> all; 2 -> TL+BR / TR+BL; 3 -> TL / TR+BL / BR; 4 -> TL TR BR BL).
            if computed.len() == 4 {
                if let SyntaxComponent::Group { components: outer, .. } = component {
                    let nested_quad = outer.first().is_some_and(|first_child| {
                        first_child
                            .multipliers()
                            .iter()
                            .any(|m| matches!(m, SyntaxComponentMultiplier::Between(1, 4)))
                    });

                    if nested_quad {
                        // [BR, BL, TR, TL] - see comment above.
                        for i in [2usize, 3, 1, 0] {
                            shorthands.push(Shorthand {
                                name: computed[i].clone(),
                                // Descend into the outer group's first child (the `{1,4}` group)
                                // and its single value slot, so the resolver completes once per
                                // matched value the same way the top-level `{1,4}` case does.
                                components: vec![0, 0],
                            });
                        }

                        return Some(Shorthands {
                            multiplier: Multiplier::QuadMulti,
                            shorthands,
                            name: name.to_string(),
                            layered: false,
                        });
                    }
                }
            }
        }

        // Pieces mapped by shape, which subsumes the property-reference search below and also
        // reaches the type-named pieces of a layer (`<bg-image>` in `background`).
        let (by_shape, complete) = self.map_by_shape(computed, syntax, name);
        let (layered, comma_path) = Self::layer_shape(syntax);
        let mapped: std::collections::HashSet<&str> = by_shape.iter().map(|s| s.name.as_str()).collect();
        let full = !by_shape.is_empty() && computed.iter().all(|l| mapped.contains(l.as_str()));
        // A partial map is still a map when the grammar has no piece left over: the longhands
        // it misses cannot be set by this shorthand at all and are reset like any the
        // declaration left out. `text-decoration` lists four longhands and names three.
        if full || (!by_shape.is_empty() && complete) {
            let mut shorthands = by_shape;
            if let Some(comma_path) = comma_path {
                shorthands.push(Shorthand {
                    name: String::new(),
                    components: comma_path,
                });
            }
            return Some(Shorthands {
                multiplier: Multiplier::None,
                shorthands,
                name: name.to_string(),
                layered,
            });
        }

        let mut found_props = Vec::with_capacity(computed.len());
        for shorthand in computed {
            if let Some(shorthand) = syntax.has_property_syntax(shorthand) {
                found_props.push(shorthand);
            }
        }

        if found_props.len() == computed.len() {
            return Some(Shorthands {
                multiplier: Multiplier::None,
                shorthands: found_props,
                name: name.to_string(),
                layered: false,
            });
        }

        if let Some(SyntaxComponent::Group {
            components,
            // multipliers,
            ..
        }) = syntax.components.first()
        {
            if components.len() == computed.len() {
                for (i, property) in computed.iter().enumerate() {
                    shorthands.push(Shorthand {
                        name: property.clone(),
                        components: vec![i],
                    });
                }

                return Some(Shorthands {
                    multiplier: Multiplier::None,
                    shorthands,
                    name: name.to_string(),
                    layered: false,
                });
            }
        }

        // A property reference is written `<'name'>`, which compiles to a quoted `Definition` -
        // so both a value type and a property reference arrive here as one.
        if let [SyntaxComponent::Definition { datatype, .. }] = syntax.components.as_slice() {
            if let Some(d) = self.syntax.get(datatype) {
                if let Some(mut shorthands) = self.resolve_shorthands(computed, &d.syntax, name) {
                    shorthands.multiplier = Multiplier::None;

                    return Some(shorthands);
                }
            }

            if let Some(p) = self.properties.get(datatype) {
                if let Some(mut shorthands) = self.resolve_shorthands(computed, &p.syntax, name) {
                    shorthands.multiplier = Multiplier::None;

                    return Some(shorthands);
                }
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use crate::colors::RgbColor;
    use crate::matcher::property_definitions::get_css_definitions;
    use crate::matcher::shorthands::CssValue;
    use crate::matcher::shorthands::FixList;
    use crate::tokenizer::NumberKind;

    macro_rules! str {
        ($s:expr) => {
            CssValue::String($s.to_string())
        };
    }

    macro_rules! unit {
        ($v:expr, $u:expr) => {
            CssValue::Unit($v, $u.to_string())
        };
    }

    /// Expand a `font:` declaration (parsed from real CSS) and return the resulting
    /// (longhand, value) pairs.
    fn expand_font(decl: &str) -> Vec<(String, CssValue)> {
        use crate::Css3;
        use gosub_interface::css3::CssOrigin;
        use gosub_shared::config::ParserConfig;

        let definitions = get_css_definitions();
        let prop = definitions.find_property("font").unwrap();

        let css = format!("x {{ font: {decl}; }}");
        let config = ParserConfig {
            match_values: false,
            ignore_errors: true,
            ..Default::default()
        };
        let sheet = Css3::parse_str(&css, config, CssOrigin::Author, "font-test").expect("parse");
        let decl = &sheet.rules[0].declarations[0];
        let values = match &decl.value {
            CssValue::List(v) => v.clone(),
            other => vec![other.clone()],
        };

        let mut fix_list = FixList::new();
        assert!(
            prop.clone().matches_and_shorthands(&values, &mut fix_list),
            "font: declaration should match"
        );
        fix_list
            .list
            .iter()
            .map(|(name, decls)| (name.clone(), decls.last().unwrap().value.clone()))
            .collect()
    }

    fn value_of<'a>(expanded: &'a [(String, CssValue)], name: &str) -> Option<&'a CssValue> {
        expanded.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// Expand `prop: decl` the way `compute_properties` does: match, reset what the shorthand
    /// left out, expand nested shorthands. Returns the fix list's longhands and their values.
    fn expand(prop: &str, decl: &str) -> Vec<(String, CssValue)> {
        use crate::stylesheet::Specificity;
        use crate::Css3;
        use gosub_interface::css3::CssOrigin;
        use gosub_shared::config::ParserConfig;

        let definitions = get_css_definitions();
        let def = definitions.find_property(prop).expect("property is defined");
        let config = ParserConfig {
            match_values: false,
            ignore_errors: true,
            ..Default::default()
        };
        let sheet =
            Css3::parse_str(&format!("x {{ {prop}: {decl}; }}"), config, CssOrigin::Author, "t").expect("parse");
        let values = sheet.rules[0].declarations[0].value.to_slice().to_vec();

        let mut fix_list = FixList::new();
        fix_list.set_info(super::FixListInfo::new(
            CssOrigin::Author,
            false,
            String::new(),
            Specificity::new(0, 0, 0),
            0,
            1,
            None,
            false,
        ));
        assert!(
            def.matches_and_shorthands(&values, &mut fix_list),
            "{prop}: {decl} should match"
        );
        fix_list.reset_unmentioned(def, &values, definitions);
        fix_list.resolve_nested(definitions);
        fix_list
            .list
            .iter()
            .map(|(name, decls)| (name.clone(), decls.last().expect("a value").value.clone()))
            .collect()
    }

    /// A shorthand sets all of its longhands: the ones it does not mention get their initial
    /// value. Without this `border-color: red; border: 1px solid` kept the red, and
    /// `font-weight: bold; font: 12px serif` kept the bold.
    #[test]
    fn a_shorthand_resets_the_longhands_it_leaves_out() {
        let border = expand("border", "1px solid");
        for side in ["top", "right", "bottom", "left"] {
            assert_eq!(
                value_of(&border, &format!("border-{side}-color")),
                Some(&CssValue::String("currentcolor".into())),
                "border-{side}-color"
            );
            assert_eq!(
                value_of(&border, &format!("border-{side}-width")),
                Some(&CssValue::Unit(1.0, "px".into()))
            );
        }

        let font = expand("font", "12px serif");
        for longhand in [
            "font-weight",
            "font-style",
            "font-variant",
            "font-stretch",
            "line-height",
        ] {
            assert_eq!(
                value_of(&font, longhand),
                Some(&CssValue::String("normal".into())),
                "{longhand}"
            );
        }
        assert_eq!(value_of(&font, "font-size"), Some(&CssValue::Unit(12.0, "px".into())));
    }

    /// css-lists-3 §3.5: `none` in `list-style` is both the image and the marker type. The
    /// grammar gives it to the image, so the reset must not then hand the type its initial
    /// `disc` - that would put the bullets back on every `ul { list-style: none }`.
    #[test]
    fn list_style_none_removes_the_marker_too() {
        let none = expand("list-style", "none");
        assert_eq!(
            value_of(&none, "list-style-type"),
            Some(&CssValue::String("none".into()))
        );
        assert_eq!(
            value_of(&none, "list-style-position"),
            Some(&CssValue::String("outside".into()))
        );

        // A type given alongside `none` is kept; `none` is then the image alone.
        let square = expand("list-style", "none square");
        assert_eq!(
            value_of(&square, "list-style-type"),
            Some(&CssValue::String("square".into()))
        );
    }

    /// `flex` fills its omitted values from its own table (css-flexbox-1 §7.1.1), not from the
    /// longhand initials: `flex: 1` is `1 1 0`, which is what makes equal columns equal.
    #[test]
    fn flex_fills_its_own_defaults_not_the_initials() {
        let one = |n: f64| CssValue::Number(n, crate::tokenizer::NumberKind::Integer);

        let single = expand("flex", "1");
        assert_eq!(value_of(&single, "flex-grow"), Some(&one(1.0)));
        assert_eq!(value_of(&single, "flex-shrink"), Some(&one(1.0)));
        assert_eq!(value_of(&single, "flex-basis"), Some(&CssValue::Percentage(0.0)));

        let none = expand("flex", "none");
        assert_eq!(value_of(&none, "flex-grow"), Some(&one(0.0)));
        assert_eq!(value_of(&none, "flex-shrink"), Some(&one(0.0)));
        assert_eq!(value_of(&none, "flex-basis"), Some(&CssValue::String("auto".into())));

        let auto = expand("flex", "auto");
        assert_eq!(value_of(&auto, "flex-grow"), Some(&one(1.0)));
        assert_eq!(value_of(&auto, "flex-shrink"), Some(&one(1.0)));
        assert_eq!(value_of(&auto, "flex-basis"), Some(&CssValue::String("auto".into())));

        let two = expand("flex", "2 30%");
        assert_eq!(value_of(&two, "flex-grow"), Some(&one(2.0)));
        assert_eq!(value_of(&two, "flex-shrink"), Some(&one(1.0)));
        assert_eq!(value_of(&two, "flex-basis"), Some(&CssValue::Percentage(30.0)));
    }

    /// `background` is a list of layers named by value type, which the map reaches through
    /// `<bg-layer>`. Each longhand gets one value per layer, the initial value where a layer
    /// leaves it out, and one `<visual-box>` sets both origin and clip.
    #[test]
    fn background_expands_layer_by_layer() {
        let one = expand("background", "url(x.png) no-repeat center / cover fixed padding-box");
        let url = |name: &str| CssValue::Function("url".into(), vec![CssValue::String(name.into())]);
        assert_eq!(value_of(&one, "background-image"), Some(&url("x.png")));
        assert_eq!(
            value_of(&one, "background-repeat"),
            Some(&CssValue::String("no-repeat".into()))
        );
        assert_eq!(
            value_of(&one, "background-position"),
            Some(&CssValue::String("center".into()))
        );
        assert_eq!(
            value_of(&one, "background-size"),
            Some(&CssValue::String("cover".into()))
        );
        assert_eq!(
            value_of(&one, "background-attachment"),
            Some(&CssValue::String("fixed".into()))
        );
        assert_eq!(
            value_of(&one, "background-origin"),
            Some(&CssValue::String("padding-box".into()))
        );
        assert_eq!(
            value_of(&one, "background-clip"),
            Some(&CssValue::String("padding-box".into()))
        );
        assert_eq!(
            value_of(&one, "background-color"),
            Some(&CssValue::String("transparent".into())),
            "no colour in the value resets background-color"
        );

        let two = expand("background", "url(a.png) no-repeat, url(b.png) #123");
        assert_eq!(
            value_of(&two, "background-image"),
            Some(&CssValue::List(vec![url("a.png"), CssValue::Comma, url("b.png")]))
        );
        assert_eq!(
            value_of(&two, "background-repeat"),
            Some(&CssValue::List(vec![
                CssValue::String("no-repeat".into()),
                CssValue::Comma,
                CssValue::String("repeat".into()),
            ])),
            "the second layer's repeat is the initial value"
        );
        assert!(
            matches!(value_of(&two, "background-color"), Some(CssValue::Color(_))),
            "the colour of the final layer is the background-color"
        );
    }

    /// `transition` is `<single-transition>#`: two `<time>` pieces that the spec assigns as
    /// duration then delay, and a `||` group whose operand order must not change which
    /// keyword `ease` goes to.
    #[test]
    fn transition_expands_with_duration_before_delay() {
        let one = expand("transition", "ease all 300ms");
        assert_eq!(
            value_of(&one, "transition-property"),
            Some(&CssValue::String("all".into()))
        );
        assert_eq!(
            value_of(&one, "transition-timing-function"),
            Some(&CssValue::String("ease".into()))
        );
        assert_eq!(
            value_of(&one, "transition-duration"),
            Some(&CssValue::Unit(300.0, "ms".into()))
        );
        assert_eq!(
            value_of(&one, "transition-delay"),
            Some(&CssValue::Unit(0.0, "s".into()))
        );

        let two = expand("transition", "opacity .3s ease-in .1s, transform 1s");
        assert_eq!(
            value_of(&two, "transition-property"),
            Some(&CssValue::List(vec![
                CssValue::String("opacity".into()),
                CssValue::Comma,
                CssValue::String("transform".into()),
            ]))
        );
        assert_eq!(
            value_of(&two, "transition-delay"),
            Some(&CssValue::List(vec![
                CssValue::Unit(0.1, "s".into()),
                CssValue::Comma,
                CssValue::Unit(0.0, "s".into()),
            ]))
        );
        assert_eq!(
            value_of(&two, "transition-timing-function"),
            Some(&CssValue::List(vec![
                CssValue::String("ease-in".into()),
                CssValue::Comma,
                CssValue::String("ease".into()),
            ]))
        );
    }

    #[test]
    fn animation_and_text_decoration_expand() {
        let animation = expand("animation", "slide 1s ease-in 2s infinite");
        assert_eq!(
            value_of(&animation, "animation-name"),
            Some(&CssValue::String("slide".into()))
        );
        assert_eq!(
            value_of(&animation, "animation-duration"),
            Some(&CssValue::Unit(1.0, "s".into()))
        );
        assert_eq!(
            value_of(&animation, "animation-delay"),
            Some(&CssValue::Unit(2.0, "s".into()))
        );
        assert_eq!(
            value_of(&animation, "animation-iteration-count"),
            Some(&CssValue::String("infinite".into()))
        );
        assert!(
            matches!(value_of(&animation, "animation-fill-mode"), Some(CssValue::None)),
            "fill-mode is reset to its initial `none`"
        );

        // Three of its four longhands are in the grammar; the fourth is reset.
        let decoration = expand("text-decoration", "underline red");
        assert_eq!(
            value_of(&decoration, "text-decoration-line"),
            Some(&CssValue::String("underline".into()))
        );
        assert_eq!(
            value_of(&decoration, "text-decoration-color"),
            Some(&CssValue::String("red".into()))
        );
        assert_eq!(
            value_of(&decoration, "text-decoration-style"),
            Some(&CssValue::String("solid".into()))
        );
        assert_eq!(
            value_of(&decoration, "text-decoration-thickness"),
            Some(&CssValue::String("auto".into()))
        );
    }

    /// css-multicol-1 §3.2: `column-rule` is width, style and colour, like `border`. Upstream
    /// types it with the css-gaps-1 `<gap-rule-list>`, whose shape matches none of its
    /// longhands, so nothing expanded and `column-rule` never reset them.
    #[test]
    fn column_rule_expands_like_border() {
        let expanded = expand("column-rule", "2px dashed red");
        assert_eq!(
            value_of(&expanded, "column-rule-width"),
            Some(&CssValue::Unit(2.0, "px".to_string()))
        );
        assert_eq!(
            value_of(&expanded, "column-rule-style"),
            Some(&CssValue::String("dashed".into()))
        );
        assert_eq!(
            value_of(&expanded, "column-rule-color").map(ToString::to_string),
            Some("red".to_string())
        );
    }

    /// A `column-rule` that omits a component resets it to its initial value, as every
    /// shorthand does.
    #[test]
    fn column_rule_resets_what_it_omits() {
        let expanded = expand("column-rule", "dotted");
        assert_eq!(
            value_of(&expanded, "column-rule-style"),
            Some(&CssValue::String("dotted".into()))
        );
        assert_eq!(
            value_of(&expanded, "column-rule-width"),
            Some(&CssValue::String("medium".into()))
        );
    }

    /// css-masking-1 §5: `mask` is a layered shorthand like `background`. Its layer grammar
    /// names the box type `<geometry-box>` while the longhands take `<coord-box>`, two spellings
    /// of the same set from different drafts, and the shape mapper matched neither.
    #[test]
    fn mask_expands_layer_by_layer() {
        let expanded = expand("mask", "url(a.png) luminance, url(b.png)");
        // Both layers contribute an image, so the longhand is a two-item comma list.
        let images = value_of(&expanded, "mask-image").expect("mask-image is set");
        assert!(
            matches!(images, CssValue::List(items) if items.iter().filter(|v| matches!(v, CssValue::Comma)).count() == 1),
            "mask-image should be a two-layer list, got {images:?}"
        );
        // The second layer says nothing about the mode, so it takes the initial value.
        assert_eq!(
            value_of(&expanded, "mask-mode"),
            Some(&CssValue::List(vec![
                CssValue::String("luminance".into()),
                CssValue::Comma,
                CssValue::String("match-source".into()),
            ]))
        );
    }

    /// One box keyword in a mask layer sets both origin and clip, the rule `background` already
    /// follows (css-masking-1 §5.1).
    #[test]
    fn a_mask_box_sets_both_origin_and_clip() {
        let expanded = expand("mask", "url(a.png) padding-box");
        assert_eq!(
            value_of(&expanded, "mask-origin"),
            Some(&CssValue::String("padding-box".into()))
        );
        assert_eq!(
            value_of(&expanded, "mask-clip"),
            Some(&CssValue::String("padding-box".into()))
        );
    }

    /// css-grid-2 §8.4: `grid-area`'s four longhands are all `<grid-line>`, so the shape mapper
    /// cannot tell them apart and the shorthand used to expand to nothing. They are placed by
    /// position instead.
    #[test]
    fn grid_area_places_its_lines_by_position() {
        let expanded = expand("grid-area", "1 / 2 / 3 / 4");
        for (name, line) in [
            ("grid-row-start", 1.0),
            ("grid-column-start", 2.0),
            ("grid-row-end", 3.0),
            ("grid-column-end", 4.0),
        ] {
            assert_eq!(
                value_of(&expanded, name),
                Some(&CssValue::Number(line, NumberKind::Integer)),
                "{name}"
            );
        }
    }

    /// An omitted line copies the one it pairs with when that is a name, and is `auto`
    /// otherwise. `grid-area: foo` names all four sides.
    #[test]
    fn an_omitted_grid_line_copies_a_name_but_not_a_number() {
        let named = expand("grid-area", "foo");
        for name in ["grid-row-start", "grid-column-start", "grid-row-end", "grid-column-end"] {
            assert_eq!(value_of(&named, name), Some(&CssValue::String("foo".into())), "{name}");
        }

        let numbered = expand("grid-area", "1");
        assert_eq!(
            value_of(&numbered, "grid-row-start"),
            Some(&CssValue::Number(1.0, NumberKind::Integer))
        );
        for name in ["grid-column-start", "grid-row-end", "grid-column-end"] {
            assert_eq!(
                value_of(&numbered, name),
                Some(&CssValue::String("auto".into())),
                "{name}"
            );
        }
    }

    /// `grid-row` and `grid-column` are the two-line form of the same rule (css-grid-2 §8.3).
    #[test]
    fn grid_row_and_column_place_a_start_and_an_end() {
        let row = expand("grid-row", "span 2 / 4");
        assert_eq!(
            value_of(&row, "grid-row-start"),
            Some(&CssValue::List(vec![
                CssValue::String("span".into()),
                CssValue::Number(2.0, NumberKind::Integer)
            ]))
        );
        assert_eq!(
            value_of(&row, "grid-row-end"),
            Some(&CssValue::Number(4.0, NumberKind::Integer))
        );

        let column = expand("grid-column", "3");
        assert_eq!(
            value_of(&column, "grid-column-start"),
            Some(&CssValue::Number(3.0, NumberKind::Integer))
        );
        assert_eq!(
            value_of(&column, "grid-column-end"),
            Some(&CssValue::String("auto".into()))
        );
    }

    /// The point of expanding at all: a shorthand overrides a longhand that came before it.
    #[test]
    fn grid_area_overrides_an_earlier_longhand() {
        let expanded = expand("grid-area", "1 / 2");
        assert_eq!(
            value_of(&expanded, "grid-row-start"),
            Some(&CssValue::Number(1.0, NumberKind::Integer))
        );
        assert_eq!(
            value_of(&expanded, "grid-row-end"),
            Some(&CssValue::String("auto".into()))
        );
    }

    /// css-grid-2 §7.4: `grid` is the `grid-template` longhands plus the three that govern
    /// automatically generated tracks. Its grammar mixes references to other shorthands with
    /// `auto-flow`, a keyword that says which side of the slash is generated rather than
    /// supplying a value, so no shape map can describe it.
    #[test]
    fn grid_expands_its_template_form() {
        let expanded = expand("grid", "10px / 20%");
        assert_eq!(
            value_of(&expanded, "grid-template-rows"),
            Some(&CssValue::Unit(10.0, "px".to_string()))
        );
        assert_eq!(
            value_of(&expanded, "grid-template-columns"),
            Some(&CssValue::Percentage(20.0))
        );
        for (name, initial) in [
            ("grid-template-areas", "none"),
            ("grid-auto-rows", "auto"),
            ("grid-auto-columns", "auto"),
            ("grid-auto-flow", "row"),
        ] {
            assert_eq!(
                value_of(&expanded, name),
                Some(&CssValue::String(initial.into())),
                "{name}"
            );
        }
    }

    /// `auto-flow` on the left of the slash generates rows, so the size beside it is
    /// `grid-auto-rows` and the other side is the column template.
    #[test]
    fn grid_auto_flow_takes_the_side_it_is_on() {
        let rows = expand("grid", "auto-flow dense 40px / 1fr");
        assert_eq!(
            value_of(&rows, "grid-auto-flow"),
            Some(&CssValue::List(vec![
                CssValue::String("row".into()),
                CssValue::String("dense".into())
            ]))
        );
        assert_eq!(
            value_of(&rows, "grid-auto-rows"),
            Some(&CssValue::Unit(40.0, "px".to_string()))
        );
        assert_eq!(
            value_of(&rows, "grid-template-columns"),
            Some(&CssValue::Unit(1.0, "fr".to_string()))
        );
        assert_eq!(
            value_of(&rows, "grid-template-rows"),
            Some(&CssValue::String("none".into()))
        );

        let columns = expand("grid", "100px / auto-flow 40px");
        assert_eq!(
            value_of(&columns, "grid-auto-flow"),
            Some(&CssValue::String("column".into()))
        );
        assert_eq!(
            value_of(&columns, "grid-auto-columns"),
            Some(&CssValue::Unit(40.0, "px".to_string()))
        );
        assert_eq!(
            value_of(&columns, "grid-template-rows"),
            Some(&CssValue::Unit(100.0, "px".to_string()))
        );
    }

    /// An omitted automatic track size is `auto`.
    #[test]
    fn grid_fills_in_an_omitted_auto_track_size() {
        let expanded = expand("grid", "auto-flow / 1fr");
        assert_eq!(
            value_of(&expanded, "grid-auto-rows"),
            Some(&CssValue::String("auto".into()))
        );
        assert_eq!(
            value_of(&expanded, "grid-auto-flow"),
            Some(&CssValue::String("row".into()))
        );
    }

    /// `grid` does not touch the gutters. MDN still lists them as its longhands, from the draft
    /// where it did, and resetting them would undo a `column-gap` the author set elsewhere.
    #[test]
    fn grid_leaves_the_gutters_alone() {
        let expanded = expand("grid", "10px / 20%");
        for gutter in ["column-gap", "row-gap", "grid-column-gap", "grid-row-gap"] {
            assert_eq!(value_of(&expanded, gutter), None, "{gutter} must not be reset by grid");
        }
    }

    /// `grid-template` is a shorthand in its own right.
    #[test]
    fn grid_template_expands_rows_and_columns() {
        let expanded = expand("grid-template", "10px / 20%");
        assert_eq!(
            value_of(&expanded, "grid-template-rows"),
            Some(&CssValue::Unit(10.0, "px".to_string()))
        );
        assert_eq!(
            value_of(&expanded, "grid-template-areas"),
            Some(&CssValue::String("none".into()))
        );

        let none = expand("grid-template", "none");
        for name in ["grid-template-rows", "grid-template-columns", "grid-template-areas"] {
            assert_eq!(value_of(&none, name), Some(&CssValue::String("none".into())), "{name}");
        }
    }

    /// The row-of-names form: each string is a row of `grid-template-areas`, and
    /// `grid-template-rows` is rebuilt from the names and sizes around them. The names written
    /// after one row and before the next name the same line, so they merge into one group
    /// (css-grid-2 §7.3).
    #[test]
    fn grid_template_rebuilds_rows_from_area_strings() {
        let expanded = expand(
            "grid-template",
            "[header-top] \"a a a\" [header-bottom] [main-top] \"b b b\" 1fr [main-bottom] / auto 1fr auto",
        );
        let rows = value_of(&expanded, "grid-template-rows").expect("rows");
        assert_eq!(
            rows.to_string(),
            "[header-top] auto [header-bottom main-top] 1fr [main-bottom]"
        );
        assert_eq!(
            value_of(&expanded, "grid-template-columns").map(ToString::to_string),
            Some("auto 1fr auto".to_string())
        );
    }

    /// A row with no track size is `auto`, and an empty name group names nothing.
    #[test]
    fn grid_template_fills_in_auto_rows_and_drops_empty_name_groups() {
        let expanded = expand(
            "grid-template",
            "[] \"a a a\" [] [] \"b b b\" 1fr [] / [] auto 1fr [] auto []",
        );
        assert_eq!(
            value_of(&expanded, "grid-template-rows").map(ToString::to_string),
            Some("auto 1fr".to_string())
        );
    }

    /// css-backgrounds-4 §3.6: one `<bg-position>` covers both axes, so neither longhand has the
    /// shape of any piece of it and the shorthand expanded to nothing. The halves are worked out
    /// from the keywords instead, which may be written in either order.
    #[test]
    fn background_position_splits_into_its_two_axes() {
        let plain = expand("background-position", "10px 20px");
        assert_eq!(
            value_of(&plain, "background-position-x"),
            Some(&CssValue::Unit(10.0, "px".to_string()))
        );
        assert_eq!(
            value_of(&plain, "background-position-y"),
            Some(&CssValue::Unit(20.0, "px".to_string()))
        );

        // A single value centres the other axis.
        let single = expand("background-position", "10px");
        assert_eq!(
            value_of(&single, "background-position-y"),
            Some(&CssValue::String("center".into()))
        );

        // A lone vertical keyword sets the vertical axis, not the horizontal one.
        let vertical = expand("background-position", "top");
        assert_eq!(
            value_of(&vertical, "background-position-x"),
            Some(&CssValue::String("center".into()))
        );
        assert_eq!(
            value_of(&vertical, "background-position-y"),
            Some(&CssValue::String("top".into()))
        );

        // Keywords may be written the other way round.
        let swapped = expand("background-position", "top left");
        assert_eq!(
            value_of(&swapped, "background-position-x"),
            Some(&CssValue::String("left".into()))
        );
        assert_eq!(
            value_of(&swapped, "background-position-y"),
            Some(&CssValue::String("top".into()))
        );
    }

    /// The three- and four-value forms pair each keyword with the offset after it, and each pair
    /// names the axis it belongs to.
    #[test]
    fn background_position_pairs_each_keyword_with_its_offset() {
        let expanded = expand("background-position", "left 10px top 20px");
        assert_eq!(
            value_of(&expanded, "background-position-x").map(ToString::to_string),
            Some("left 10px".to_string())
        );
        assert_eq!(
            value_of(&expanded, "background-position-y").map(ToString::to_string),
            Some("top 20px".to_string())
        );

        let three = expand("background-position", "center right 7%");
        assert_eq!(
            value_of(&three, "background-position-x").map(ToString::to_string),
            Some("right 7%".to_string())
        );
        assert_eq!(
            value_of(&three, "background-position-y").map(ToString::to_string),
            Some("center".to_string())
        );
    }

    /// Every layer is split, and the halves are collected back into one comma list per longhand.
    #[test]
    fn background_position_splits_every_layer() {
        let expanded = expand("background-position", "left top, 10px 20px");
        assert_eq!(
            value_of(&expanded, "background-position-x").map(ToString::to_string),
            Some("left, 10px".to_string())
        );
    }

    /// A shorthand the resolver has no map for records nothing, and is then left alone rather
    /// than reset from nothing - resetting longhands it never set would erase what the author
    /// wrote. `-webkit-mask` is one: it is a layered shorthand whose layer grammar carries
    /// pieces its own longhand list does not name, and a partial map would reset the rest.
    #[test]
    fn a_shorthand_the_resolver_cannot_expand_is_not_reset() {
        assert!(expand("-webkit-mask", "url(a.png)").is_empty());
    }

    /// A declaration that fails as a whole must leave nothing behind. The resolver records a
    /// longhand as soon as its grammar piece completes, so without a rollback `border: 1px solid
    /// banana` set `border-width` and `border-style` even though the declaration was rejected.
    #[test]
    fn a_failed_shorthand_records_no_longhands() {
        use crate::Css3;
        use gosub_interface::css3::CssOrigin;
        use gosub_shared::config::ParserConfig;

        let definitions = get_css_definitions();
        let prop = definitions.find_property("border").expect("border is defined");
        let config = ParserConfig {
            match_values: false,
            ignore_errors: true,
            ..Default::default()
        };
        let sheet = Css3::parse_str("x { border: 1px solid banana; }", config, CssOrigin::Author, "t").expect("parse");
        let decl = &sheet.rules[0].declarations[0];
        let values = decl.value.to_slice();

        let mut fix_list = FixList::new();
        assert!(!prop.matches_and_shorthands(values, &mut fix_list));
        let recorded: Vec<&str> = fix_list.list.iter().map(|(name, _)| name.as_str()).collect();
        assert!(
            recorded.is_empty(),
            "a rejected declaration left {recorded:?} in the fix list"
        );
    }

    /// `font: <size>/<line-height> <family>` must expand line-height - it drives the line-box
    /// height of every WPT table test using the `font:` shorthand.
    #[test]
    fn font_shorthand_expands_size_line_height_family() {
        let expanded = expand_font("1.25em/1.2 serif");
        assert_eq!(
            value_of(&expanded, "font-size"),
            Some(&CssValue::Unit(1.25, "em".into()))
        );
        assert_eq!(
            value_of(&expanded, "line-height"),
            Some(&CssValue::Number(1.2, crate::tokenizer::NumberKind::Number))
        );
        assert_eq!(
            value_of(&expanded, "font-family"),
            Some(&CssValue::String("serif".into()))
        );
    }

    #[test]
    fn font_shorthand_expands_prelude_and_px_line_height() {
        let expanded = expand_font("italic bold 12px/18px serif");
        assert_eq!(
            value_of(&expanded, "font-style"),
            Some(&CssValue::String("italic".into()))
        );
        assert_eq!(
            value_of(&expanded, "font-weight"),
            Some(&CssValue::String("bold".into()))
        );
        assert_eq!(
            value_of(&expanded, "font-size"),
            Some(&CssValue::Unit(12.0, "px".into()))
        );
        assert_eq!(
            value_of(&expanded, "line-height"),
            Some(&CssValue::Unit(18.0, "px".into()))
        );
        assert_eq!(
            value_of(&expanded, "font-family"),
            Some(&CssValue::String("serif".into()))
        );
    }

    #[test]
    fn font_shorthand_without_line_height() {
        let expanded = expand_font("12px sans-serif");
        assert_eq!(
            value_of(&expanded, "font-size"),
            Some(&CssValue::Unit(12.0, "px".into()))
        );
        assert_eq!(value_of(&expanded, "line-height"), None);
        assert_eq!(
            value_of(&expanded, "font-family"),
            Some(&CssValue::String("sans-serif".into()))
        );
    }

    /// `border-color`'s longhands are listed in TRBL order in the definitions data, but
    /// QuadMulti maps values assuming [bottom, left, right, top] - without reordering,
    /// `border-color: a b c LEFT` landed its 4th value on the RIGHT
    /// (WPT border-conflict-element-001*).
    #[test]
    fn border_color_shorthand_assigns_sides_correctly() {
        let definitions = get_css_definitions();
        let prop = definitions.find_property("border-color").unwrap();

        let mut fix_list = FixList::new();
        assert!(prop.clone().matches_and_shorthands(
            &[str!("green"), str!("green"), str!("green"), str!("red")],
            &mut fix_list,
        ));

        let get = |name: &str| -> String {
            let (_, v) = fix_list.list.iter().find(|(k, _)| k == name).expect("longhand present");
            format!("{}", v.last().unwrap().value)
        };
        assert_eq!(get("border-top-color"), "green");
        assert_eq!(get("border-right-color"), "green");
        assert_eq!(get("border-bottom-color"), "green");
        assert_eq!(get("border-left-color"), "red");
    }

    #[test]
    fn margin() {
        let definitions = get_css_definitions();

        let prop = definitions.find_property("margin").unwrap();

        let mut fix_list = FixList::new();

        assert!(prop
            .clone()
            .matches_and_shorthands(&[unit!(1.0, "px"),], &mut fix_list,));

        assert_eq!(
            fix_list,
            FixList {
                list: vec![
                    ("margin-bottom".to_string(), vec![unit!(1.0, "px").into()]),
                    ("margin-left".to_string(), vec![unit!(1.0, "px").into()]),
                    ("margin-right".to_string(), vec![unit!(1.0, "px").into()]),
                    ("margin-top".to_string(), vec![unit!(1.0, "px").into()]),
                ],
                multipliers: vec![("margin".to_string(), 1),],
                touched: vec![
                    "margin-bottom".to_string(),
                    "margin-left".to_string(),
                    "margin-right".to_string(),
                    "margin-top".to_string(),
                ],
                layered: false,
                layer: 0,
                layer_values: vec![],
                current_info: None
            }
        );

        fix_list = FixList::new();

        assert!(prop
            .clone()
            .matches_and_shorthands(&[unit!(1.0, "px"), unit!(2.0, "px"),], &mut fix_list,));

        assert_eq!(
            fix_list,
            FixList {
                list: vec![
                    ("margin-bottom".to_string(), vec![unit!(1.0, "px").into()]),
                    ("margin-left".to_string(), vec![unit!(2.0, "px").into()]),
                    ("margin-right".to_string(), vec![unit!(2.0, "px").into()]),
                    ("margin-top".to_string(), vec![unit!(1.0, "px").into()]),
                ],
                multipliers: vec![("margin".to_string(), 2),],
                touched: vec![
                    "margin-bottom".to_string(),
                    "margin-left".to_string(),
                    "margin-right".to_string(),
                    "margin-top".to_string(),
                ],
                layered: false,
                layer: 0,
                layer_values: vec![],
                current_info: None
            }
        );

        fix_list = FixList::new();
        assert!(prop
            .clone()
            .matches_and_shorthands(&[unit!(1.0, "px"), unit!(2.0, "px"), unit!(3.0, "px"),], &mut fix_list,));

        assert_eq!(
            fix_list,
            FixList {
                list: vec![
                    ("margin-bottom".to_string(), vec![unit!(3.0, "px").into()]),
                    ("margin-left".to_string(), vec![unit!(2.0, "px").into()]),
                    ("margin-right".to_string(), vec![unit!(2.0, "px").into()]),
                    ("margin-top".to_string(), vec![unit!(1.0, "px").into()]),
                ],
                multipliers: vec![("margin".to_string(), 3),],
                touched: vec![
                    "margin-bottom".to_string(),
                    "margin-left".to_string(),
                    "margin-right".to_string(),
                    "margin-top".to_string(),
                ],
                layered: false,
                layer: 0,
                layer_values: vec![],
                current_info: None
            }
        );

        fix_list = FixList::new();
        assert!(prop.clone().matches_and_shorthands(
            &[unit!(1.0, "px"), unit!(2.0, "px"), unit!(3.0, "px"), unit!(4.0, "px"),],
            &mut fix_list,
        ));

        assert_eq!(
            fix_list,
            FixList {
                list: vec![
                    ("margin-bottom".to_string(), vec![unit!(3.0, "px").into()]),
                    ("margin-left".to_string(), vec![unit!(4.0, "px").into()]),
                    ("margin-right".to_string(), vec![unit!(2.0, "px").into()]),
                    ("margin-top".to_string(), vec![unit!(1.0, "px").into()]),
                ],
                multipliers: vec![("margin".to_string(), 4),],
                touched: vec![
                    "margin-bottom".to_string(),
                    "margin-left".to_string(),
                    "margin-right".to_string(),
                    "margin-top".to_string(),
                ],
                layered: false,
                layer: 0,
                layer_values: vec![],
                current_info: None
            }
        );
    }

    #[test]
    fn border() {
        let definitions = get_css_definitions();

        let prop = definitions.find_property("border").unwrap();

        let mut fix_list = FixList::new();

        assert!(prop.clone().matches_and_shorthands(
            &[
                unit!(1.0, "px"),
                str!("solid"),
                CssValue::Color(RgbColor::new(0.0, 0.0, 0.0, 0.0))
            ],
            &mut fix_list,
        ));

        fix_list.resolve_nested(definitions);

        fix_list = FixList::new();

        assert!(prop.clone().matches_and_shorthands(
            &[str!("solid"), CssValue::Color(RgbColor::new(0.0, 0.0, 0.0, 0.0))],
            &mut fix_list,
        ));

        fix_list = FixList::new();

        assert!(prop.clone().matches_and_shorthands(
            &[
                str!("solid"),
                CssValue::Color(RgbColor::new(0.0, 0.0, 0.0, 0.0)),
                unit!(1.0, "px")
            ],
            &mut fix_list,
        ));
    }

    #[test]
    fn border_radius() {
        let definitions = get_css_definitions();
        let prop = definitions.find_property("border-radius").unwrap();

        // Resolve the shorthand and return (top-left, top-right, bottom-right, bottom-left).
        let corners = |vals: &[CssValue]| -> (f64, f64, f64, f64) {
            let mut fl = FixList::new();
            assert!(prop.clone().matches_and_shorthands(vals, &mut fl), "should match");
            let get = |name: &str| -> f64 {
                let (_, v) = fl.list.iter().find(|(k, _)| k == name).expect("longhand present");
                match &v.last().unwrap().value {
                    CssValue::Unit(n, _) => *n,
                    other => panic!("unexpected value {other:?}"),
                }
            };
            (
                get("border-top-left-radius"),
                get("border-top-right-radius"),
                get("border-bottom-right-radius"),
                get("border-bottom-left-radius"),
            )
        };

        // 1 value: all four corners.
        assert_eq!(corners(&[unit!(6.0, "px")]), (6.0, 6.0, 6.0, 6.0));

        // 2 values: first is top-left & bottom-right, second is top-right & bottom-left.
        assert_eq!(corners(&[unit!(1.0, "px"), unit!(2.0, "px")]), (1.0, 2.0, 1.0, 2.0));

        // 3 values: top-left, (top-right & bottom-left), bottom-right.
        assert_eq!(
            corners(&[unit!(1.0, "px"), unit!(2.0, "px"), unit!(3.0, "px")]),
            (1.0, 2.0, 3.0, 2.0)
        );

        // 4 values: top-left, top-right, bottom-right, bottom-left.
        assert_eq!(
            corners(&[unit!(1.0, "px"), unit!(2.0, "px"), unit!(3.0, "px"), unit!(4.0, "px")]),
            (1.0, 2.0, 3.0, 4.0)
        );
    }
}
