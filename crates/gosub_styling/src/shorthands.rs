use std::collections::hash_map::Entry;

use gosub_css3::stylesheet::{CssOrigin, CssValue, Specificity};

use crate::property_definitions::CssDefinitions;
use crate::styling::{CssProperties, CssProperty, DeclarationProperty};
use crate::syntax::{SyntaxComponent, SyntaxComponentMultiplier};
use crate::syntax_matcher::CssSyntaxTree;

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
            SyntaxComponent::Property { property, .. } => prop == property,
            SyntaxComponent::Definition {
                datatype, quoted, ..
            } if *quoted => prop == datatype,
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

    pub fn multipliers(&self) -> &[SyntaxComponentMultiplier] {
        match self {
            SyntaxComponent::GenericKeyword { multipliers, .. } => multipliers,
            SyntaxComponent::Property { multipliers, .. } => multipliers,
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct FixList {
    list: Vec<(String, Vec<CssValue>)>,
    multipliers: Vec<(String, usize)>,
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
        ShorthandResolver {
            multiplier: self.multiplier,
            fix_list,
            shorthands: self.shorthands.iter().map(Shorthand::resolver).collect(),
            name: &self.name,
        }
    }
}

impl Shorthand {
    pub fn resolver(&self) -> ResolveShorthand {
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
    pub fn step(&'a mut self, idx: usize) -> Result<Option<Self>, CompleteStep<'a>> {
        let snapshot = Some(self.snapshot());

        let mut shorthands = Vec::with_capacity(self.shorthands.len());

        if matches!(
            self.multiplier,
            Multiplier::QuadMulti | Multiplier::DuoMulti | Multiplier::NextProp
        ) {
            let mut complete = Vec::with_capacity(self.shorthands.len());

            for shorthand in self.shorthands.iter() {
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
                let idx = self
                    .fix_list
                    .multipliers
                    .iter_mut()
                    .find(|m| m.0 == self.name);

                if let Some(idx) = idx {
                    let Some(items) = self.multiplier.get_names(complete, idx.1) else {
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

                let Some(items) = self.multiplier.get_names(complete, 0) else {
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

        for shorthand in self.shorthands.iter() {
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
        }))
    }

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
    pub fn new() -> Self {
        Self {
            list: Vec::new(),
            multipliers: Vec::new(),
        }
    }

    pub fn insert(&mut self, name: String, value: Vec<CssValue>) {
        for (k, v) in &mut self.list {
            if *k == name {
                *v = value;
                return;
            }
        }

        self.list.push((name, value));
    }

    pub fn resolve_nested(&mut self, definitions: &CssDefinitions) {
        let mut fix_list = FixList::new();

        let mut had_shorthands = false;

        for (name, value) in &self.list {
            let Some(prop) = definitions.find_property(name) else {
                continue;
            };

            if !prop.is_shorthand() {
                continue;
            }

            had_shorthands = true;

            prop.matches_and_shorthands(value, &mut fix_list);
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
            let Some(value) = value.first().cloned() else {
                continue;
            };

            let decl = DeclarationProperty {
                value,
                origin: CssOrigin::Author,
                important: false,
                location: "".to_string(),
                specificity: Specificity::new(0, 1, 0),
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
        for name in self.name.clone() {
            self.list.insert(name.to_string(), value.clone());
        }

        self.completed = true;
    }
}

impl CssDefinitions {
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

    pub fn resolve_shorthands(
        &self,
        computed: &[String],
        syntax: &CssSyntaxTree,
        name: &str,
    ) -> Option<Shorthands> {
        if computed.len() <= 1 || syntax.components.is_empty() {
            return None;
        }

        let mut shorthands: Vec<Shorthand> = Vec::with_capacity(computed.len());

        if let Some(component) = syntax.components.first() {
            for m in component.multipliers() {
                match m {
                    SyntaxComponentMultiplier::Between(_, b) => {
                        if *b == computed.len() {
                            for c in computed {
                                shorthands.push(Shorthand {
                                    name: c.clone(),
                                    components: vec![],
                                });
                            }

                            let multiplier;

                            if computed.len() == 2 {
                                multiplier = Multiplier::DuoMulti;
                            } else if computed.len() == 4 {
                                multiplier = Multiplier::QuadMulti;
                            } else {
                                multiplier = Multiplier::NextProp;
                            }

                            return Some(Shorthands {
                                multiplier,
                                shorthands,
                                name: name.to_string(),
                            });
                        }
                    }

                    SyntaxComponentMultiplier::CommaSeparatedRepeat(_, b) => {
                        if *b == computed.len() {
                            for c in computed {
                                shorthands.push(Shorthand {
                                    name: c.clone(),
                                    components: vec![],
                                });
                            }

                            let multiplier;

                            if computed.len() == 2 {
                                multiplier = Multiplier::DuoMulti;
                            } else if computed.len() == 4 {
                                multiplier = Multiplier::QuadMulti;
                            } else {
                                multiplier = Multiplier::NextProp;
                            }

                            return Some(Shorthands {
                                multiplier,
                                shorthands,
                                name: name.to_string(),
                            });
                        }
                    }

                    _ => {}
                }
            }
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
                });
            }
        }

        if syntax.components.len() == 1 {
            let component = syntax.components.first().unwrap();

            match component {
                SyntaxComponent::Definition { datatype, .. } => {
                    if let Some(d) = self.syntax.get(datatype) {
                        if let Some(mut shorthands) =
                            self.resolve_shorthands(computed, &d.syntax, name)
                        {
                            shorthands.multiplier = Multiplier::None;

                            return Some(shorthands);
                        }
                    }

                    if let Some(p) = self.properties.get(datatype) {
                        //currently properties get parsed as definitions
                        if let Some(mut shorthands) =
                            self.resolve_shorthands(computed, &p.syntax, name)
                        {
                            shorthands.multiplier = Multiplier::None;

                            return Some(shorthands);
                        }
                    }
                }

                SyntaxComponent::Property { property, .. } => {
                    if let Some(d) = self.properties.get(property) {
                        if let Some(mut shorthands) =
                            self.resolve_shorthands(computed, &d.syntax, name)
                        {
                            shorthands.multiplier = Multiplier::None;

                            return Some(shorthands);
                        }
                    }
                }

                _ => {}
            }
        }

        // if !found_props.is_empty() {
        //     println!("Found partial expanded properties for shorthand: {}", name);
        // } else {
        // println!("Missing properties for shorthand: {}", name);
        // }
        // let missing = computed
        //     .iter()
        //     .filter(|computed| !found_props.iter().any(|p| p.name == **computed))
        //     .collect::<Vec<_>>();
        //
        // println!("Missing properties: {:?}", missing);

        None
    }
}

#[cfg(test)]
mod tests {
    use gosub_css3::colors::RgbColor;
    use gosub_css3::stylesheet::CssValue;

    use crate::property_definitions::get_css_definitions;
    use crate::shorthands::FixList;

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
                    ("margin-bottom".to_string(), vec![unit!(1.0, "px")]),
                    ("margin-left".to_string(), vec![unit!(1.0, "px")]),
                    ("margin-right".to_string(), vec![unit!(1.0, "px")]),
                    ("margin-top".to_string(), vec![unit!(1.0, "px")]),
                ],
                multipliers: vec![("margin".to_string(), 1),],
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
                    ("margin-bottom".to_string(), vec![unit!(1.0, "px")]),
                    ("margin-left".to_string(), vec![unit!(2.0, "px")]),
                    ("margin-right".to_string(), vec![unit!(2.0, "px")]),
                    ("margin-top".to_string(), vec![unit!(1.0, "px")]),
                ],
                multipliers: vec![("margin".to_string(), 2),],
            }
        );

        fix_list = FixList::new();
        assert!(prop.clone().matches_and_shorthands(
            &[unit!(1.0, "px"), unit!(2.0, "px"), unit!(3.0, "px"),],
            &mut fix_list,
        ));

        assert_eq!(
            fix_list,
            FixList {
                list: vec![
                    ("margin-bottom".to_string(), vec![unit!(3.0, "px")]),
                    ("margin-left".to_string(), vec![unit!(2.0, "px")]),
                    ("margin-right".to_string(), vec![unit!(2.0, "px")]),
                    ("margin-top".to_string(), vec![unit!(1.0, "px")]),
                ],
                multipliers: vec![("margin".to_string(), 3),],
            }
        );

        fix_list = FixList::new();
        assert!(prop.clone().matches_and_shorthands(
            &[
                unit!(1.0, "px"),
                unit!(2.0, "px"),
                unit!(3.0, "px"),
                unit!(4.0, "px"),
            ],
            &mut fix_list,
        ));

        assert_eq!(
            fix_list,
            FixList {
                list: vec![
                    ("margin-bottom".to_string(), vec![unit!(3.0, "px")]),
                    ("margin-left".to_string(), vec![unit!(4.0, "px")]),
                    ("margin-right".to_string(), vec![unit!(2.0, "px")]),
                    ("margin-top".to_string(), vec![unit!(1.0, "px")]),
                ],
                multipliers: vec![("margin".to_string(), 4),],
            }
        );

        dbg!(fix_list);
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

        dbg!(&fix_list);

        fix_list.resolve_nested(definitions);

        dbg!(&fix_list);

        fix_list = FixList::new();

        assert!(prop.clone().matches_and_shorthands(
            &[
                str!("solid"),
                CssValue::Color(RgbColor::new(0.0, 0.0, 0.0, 0.0))
            ],
            &mut fix_list,
        ));

        dbg!(fix_list);

        fix_list = FixList::new();

        assert!(prop.clone().matches_and_shorthands(
            &[
                str!("solid"),
                CssValue::Color(RgbColor::new(0.0, 0.0, 0.0, 0.0)),
                unit!(1.0, "px")
            ],
            &mut fix_list,
        ));

        dbg!(fix_list);
    }
}
