//! One allocation per distinct declared value, however many rules write it.
//!
//! A real-world stylesheet says the same thing over and over: on the 2.2 MB sheet the engine is
//! tested against, 37,706 declared values are only 4,689 distinct ones - `Zero` alone is written
//! 2,807 times. Each used to be its own allocation, and since a value is shared with every
//! element its rule matches (see [`crate::stylesheet::CssDeclaration::value`]), the duplication
//! was paid once per rule rather than once per page.
//!
//! The pool is filled while a sheet is parsed and dropped when the parse ends; what it handed
//! out stays alive in the sheet.
//!
//! # Identity, not equality
//!
//! The key is the value's exact representation. [`CssValue`]'s own `PartialEq` is not usable
//! here: for colours it asks whether two values *are the same colour*, comparing converted sRGB
//! with a tolerance, so `rgb(0 0 0)` and a colour a thousandth of a channel away compare equal.
//! Pooling on that would silently replace one with the other, and the survivor is what gets
//! serialised back to the page.

use std::borrow::Borrow;
use std::collections::hash_map::RandomState;
use std::collections::HashSet;
use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::Arc;

use crate::node::{Node, NodeType};
use crate::stylesheet::CssValue;

/// A pooled value, keyed by what it is, exactly.
struct ExactValue {
    hash: u64,
    value: Arc<CssValue>,
}

impl Hash for ExactValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}

impl PartialEq for ExactValue {
    fn eq(&self, other: &Self) -> bool {
        exactly_equal(&self.value, &other.value)
    }
}

impl Eq for ExactValue {}

/// What the pool is looked up by: a value not yet in an `Arc`, or the nodes it would be built
/// from. Each carries its hash, computed once.
trait PoolKey {
    fn hash(&self) -> u64;
    /// The value itself, when this key is one.
    fn value(&self) -> Option<&CssValue>;
    fn is(&self, pooled: &CssValue) -> bool;
}

impl PoolKey for ExactValue {
    fn hash(&self) -> u64 {
        self.hash
    }
    fn value(&self) -> Option<&CssValue> {
        Some(&self.value)
    }
    fn is(&self, pooled: &CssValue) -> bool {
        exactly_equal(&self.value, pooled)
    }
}

impl<'a> Borrow<dyn PoolKey + 'a> for ExactValue {
    fn borrow(&self) -> &(dyn PoolKey + 'a) {
        self
    }
}

impl PartialEq for dyn PoolKey + '_ {
    fn eq(&self, other: &Self) -> bool {
        match (self.value(), other.value()) {
            (_, Some(other_value)) => self.is(other_value),
            (Some(value), None) => other.is(value),
            (None, None) => false,
        }
    }
}

impl Eq for dyn PoolKey + '_ {}

impl Hash for dyn PoolKey + '_ {
    fn hash<H: Hasher>(&self, state: &mut H) {
        PoolKey::hash(self).hash(state);
    }
}

struct ValueKey<'a> {
    hash: u64,
    value: &'a CssValue,
}

impl PoolKey for ValueKey<'_> {
    fn hash(&self) -> u64 {
        self.hash
    }
    fn value(&self) -> Option<&CssValue> {
        Some(self.value)
    }
    fn is(&self, pooled: &CssValue) -> bool {
        exactly_equal(self.value, pooled)
    }
}

/// A component value read off its node, without building it.
enum NodeValue<'a> {
    /// A value with nothing on the heap, so it is built as is.
    Plain(CssValue),
    String(&'a str),
    Delim(&'a str),
    Unit(f64, &'a str),
}

impl<'a> NodeValue<'a> {
    /// What [`CssValue::parse_ast_node`] makes of `node`, or `None` for a node that builds more
    /// than this can see: functions, `calc()` and `url()`.
    fn of(node: &'a Node) -> Option<Self> {
        Some(match &node.node_type {
            NodeType::Ident { value } | NodeType::String { value } => NodeValue::String(value),
            NodeType::Number { value, kind } => NodeValue::Plain(CssValue::number(*value, *kind)),
            NodeType::Percentage { value } => NodeValue::Plain(CssValue::Percentage(*value)),
            // Folded to lowercase by `ValuePool::find` first.
            NodeType::Dimension { value, unit } => NodeValue::Unit(*value, unit),
            NodeType::Hash { value } => NodeValue::Plain(CssValue::hash_color(value)),
            NodeType::Operator { value, .. } => NodeValue::Delim(value),
            NodeType::Comma => NodeValue::Plain(CssValue::Comma),
            _ => return None,
        })
    }

    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            NodeValue::Plain(value) => hash_value(value, state),
            NodeValue::String(text) => hash_text(Tag::String, text, state),
            NodeValue::Delim(text) => hash_text(Tag::Delim, text, state),
            NodeValue::Unit(number, unit) => hash_unit(*number, unit, state),
        }
    }

    fn is(&self, pooled: &CssValue) -> bool {
        match (self, pooled) {
            (NodeValue::Plain(value), pooled) => exactly_equal(value, pooled),
            (NodeValue::String(a), CssValue::String(b)) | (NodeValue::Delim(a), CssValue::Delim(b)) => *a == b,
            (NodeValue::Unit(a, au), CssValue::Unit(b, bu)) => a.to_bits() == b.to_bits() && *au == bu,
            _ => false,
        }
    }
}

/// A declaration's value as its nodes, which build a list unless there is exactly one.
struct NodesKey<'a> {
    hash: u64,
    nodes: &'a [Node],
    /// What a lone keyword reads as instead, for a `display` alias.
    alias: Option<&'static str>,
}

impl NodesKey<'_> {
    fn single<'n>(&self, node: &'n Node) -> Option<NodeValue<'n>> {
        match self.alias {
            Some(alias) => Some(NodeValue::String(alias)),
            None => NodeValue::of(node),
        }
    }
}

impl PoolKey for NodesKey<'_> {
    fn hash(&self) -> u64 {
        self.hash
    }
    fn value(&self) -> Option<&CssValue> {
        None
    }
    fn is(&self, pooled: &CssValue) -> bool {
        match (self.nodes, pooled) {
            ([node], pooled) => self.single(node).is_some_and(|value| value.is(pooled)),
            (nodes, CssValue::List(values)) => {
                nodes.len() == values.len()
                    && nodes
                        .iter()
                        .zip(values)
                        .all(|(node, value)| NodeValue::of(node).is_some_and(|node| node.is(value)))
            }
            _ => false,
        }
    }
}

/// Which variant a value is. Explicit rather than `mem::discriminant`, so a value read off its
/// nodes hashes the same as the one built from them.
#[derive(Clone, Copy)]
enum Tag {
    None,
    Color,
    Zero,
    Number,
    Percentage,
    String,
    Delim,
    Unit,
    Function,
    Initial,
    Inherit,
    Comma,
    List,
}

fn tag(value: &CssValue) -> Tag {
    match value {
        CssValue::None => Tag::None,
        CssValue::Color(_) => Tag::Color,
        CssValue::Zero => Tag::Zero,
        CssValue::Number(..) => Tag::Number,
        CssValue::Percentage(_) => Tag::Percentage,
        CssValue::String(_) => Tag::String,
        CssValue::Delim(_) => Tag::Delim,
        CssValue::Unit(..) => Tag::Unit,
        CssValue::Function(..) => Tag::Function,
        CssValue::Initial => Tag::Initial,
        CssValue::Inherit => Tag::Inherit,
        CssValue::Comma => Tag::Comma,
        CssValue::List(_) => Tag::List,
    }
}

fn exactly_equal(a: &CssValue, b: &CssValue) -> bool {
    match (a, b) {
        (CssValue::None, CssValue::None)
        | (CssValue::Zero, CssValue::Zero)
        | (CssValue::Initial, CssValue::Initial)
        | (CssValue::Inherit, CssValue::Inherit)
        | (CssValue::Comma, CssValue::Comma) => true,
        (CssValue::Color(a), CssValue::Color(b)) => a.exact_parts() == b.exact_parts(),
        (CssValue::Number(a, ak), CssValue::Number(b, bk)) => a.to_bits() == b.to_bits() && ak == bk,
        (CssValue::Percentage(a), CssValue::Percentage(b)) => a.to_bits() == b.to_bits(),
        (CssValue::String(a), CssValue::String(b)) | (CssValue::Delim(a), CssValue::Delim(b)) => a == b,
        (CssValue::Unit(a, au), CssValue::Unit(b, bu)) => a.to_bits() == b.to_bits() && au == bu,
        (CssValue::Function(an, aa), CssValue::Function(bn, ba)) => {
            an == bn && aa.len() == ba.len() && aa.iter().zip(ba).all(|(a, b)| exactly_equal(a, b))
        }
        (CssValue::List(a), CssValue::List(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| exactly_equal(a, b))
        }
        _ => false,
    }
}

fn hash_text<H: Hasher>(tag: Tag, text: &str, state: &mut H) {
    (tag as u8).hash(state);
    text.hash(state);
}

fn hash_unit<H: Hasher>(number: f64, unit: &str, state: &mut H) {
    (Tag::Unit as u8).hash(state);
    number.to_bits().hash(state);
    unit.hash(state);
}

fn hash_value<H: Hasher>(value: &CssValue, state: &mut H) {
    match value {
        CssValue::String(text) => return hash_text(Tag::String, text, state),
        CssValue::Delim(text) => return hash_text(Tag::Delim, text, state),
        CssValue::Unit(number, unit) => return hash_unit(*number, unit, state),
        _ => {}
    }
    (tag(value) as u8).hash(state);
    match value {
        CssValue::None
        | CssValue::Zero
        | CssValue::Initial
        | CssValue::Inherit
        | CssValue::Comma
        | CssValue::String(_)
        | CssValue::Delim(_)
        | CssValue::Unit(..) => {}
        CssValue::Color(color) => color.exact_parts().hash(state),
        CssValue::Number(number, kind) => {
            number.to_bits().hash(state);
            kind.hash(state);
        }
        CssValue::Percentage(number) => number.to_bits().hash(state),
        CssValue::Function(name, args) => {
            name.hash(state);
            args.len().hash(state);
            for arg in args {
                hash_value(arg, state);
            }
        }
        CssValue::List(values) => {
            values.len().hash(state);
            for value in values {
                hash_value(value, state);
            }
        }
    }
}

/// The distinct values seen while parsing one stylesheet.
#[derive(Default)]
pub(crate) struct ValuePool {
    values: HashSet<ExactValue>,
    hasher: RandomState,
}

impl ValuePool {
    /// The shared form of `value`: the one already pooled if this value has been seen, otherwise
    /// this one, pooled.
    pub(crate) fn intern(&mut self, value: CssValue) -> Arc<CssValue> {
        let mut state = self.hasher.build_hasher();
        hash_value(&value, &mut state);
        let hash = state.finish();
        let key = ValueKey { hash, value: &value };
        if let Some(pooled) = self.values.get(&key as &dyn PoolKey) {
            return Arc::clone(&pooled.value);
        }
        let value = Arc::new(value);
        self.values.insert(ExactValue {
            hash,
            value: Arc::clone(&value),
        });
        value
    }

    /// The pooled value `nodes` would build, found without building it. `None` when it is not
    /// pooled yet, or holds a node only [`CssValue::parse_ast_node`] can convert.
    ///
    /// `alias` is what a lone keyword stands for instead, for a `display` alias. Units are folded
    /// to lowercase in place, as the conversion would.
    pub(crate) fn find(&self, nodes: &mut [Node], alias: Option<&'static str>) -> Option<Arc<CssValue>> {
        for node in nodes.iter_mut() {
            if let NodeType::Dimension { unit, .. } = &mut node.node_type {
                unit.make_ascii_lowercase();
            }
        }
        let nodes: &[Node] = nodes;
        let mut key = NodesKey { hash: 0, nodes, alias };
        let mut state = self.hasher.build_hasher();
        if let [node] = nodes {
            key.single(node)?.hash(&mut state);
        } else {
            (Tag::List as u8).hash(&mut state);
            nodes.len().hash(&mut state);
            for node in nodes.iter() {
                NodeValue::of(node)?.hash(&mut state);
            }
        }
        key.hash = state.finish();
        self.values
            .get(&key as &dyn PoolKey)
            .map(|pooled| Arc::clone(&pooled.value))
    }

    /// How many distinct values the pool holds, for tests and diagnostics.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.values.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colors::CssColor;

    #[test]
    fn the_same_value_written_twice_is_one_allocation() {
        let mut pool = ValuePool::default();
        let first = pool.intern(CssValue::String("none".into()));
        let second = pool.intern(CssValue::String("none".into()));
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn values_that_differ_stay_apart() {
        let mut pool = ValuePool::default();
        pool.intern(CssValue::Unit(1.0, "px".into()));
        pool.intern(CssValue::Unit(1.0, "em".into()));
        pool.intern(CssValue::Unit(2.0, "px".into()));
        assert_eq!(pool.len(), 3);
    }

    #[test]
    fn colours_are_pooled_by_what_they_hold_not_by_what_they_look_like() {
        let mut pool = ValuePool::default();
        // `PartialEq` calls these the same colour - the difference is far under half a channel -
        // but they are not the same value, and the one that survived would be the one written
        // back to the page.
        let one = pool.intern(CssValue::Color(CssColor::srgb(10.0, 20.0, 30.0, 255.0)));
        let other = pool.intern(CssValue::Color(CssColor::srgb(10.000_1, 20.0, 30.0, 255.0)));
        assert!(!Arc::ptr_eq(&one, &other));
        assert_eq!(pool.len(), 2);
    }

    fn node(node_type: NodeType) -> Node {
        Node::new(node_type, gosub_shared::byte_stream::Location::default())
    }

    /// What a declaration's nodes convert to, as the stylesheet builds it.
    fn build(nodes: Vec<Node>) -> CssValue {
        match <[Node; 1]>::try_from(nodes) {
            Ok([node]) => CssValue::parse_ast_node(node).expect("converts"),
            Err(nodes) => CssValue::List(
                nodes
                    .into_iter()
                    .map(|node| CssValue::parse_ast_node(node).expect("converts"))
                    .collect(),
            ),
        }
    }

    #[test]
    fn a_value_read_off_its_nodes_finds_the_one_built_from_them() {
        use crate::tokenizer::NumberKind;
        let ident = |text: &str| node(NodeType::Ident { value: text.into() });
        let number = |value: f64, kind| node(NodeType::Number { value, kind });
        let px = |value: f64, unit: &str| {
            node(NodeType::Dimension {
                value,
                unit: unit.into(),
            })
        };
        let operator = |text: &str| {
            node(NodeType::Operator {
                value: text.into(),
                space_before: true,
                space_after: true,
            })
        };
        let cases = vec![
            vec![ident("none")],
            vec![number(0.0, NumberKind::Integer)],
            vec![number(-0.0, NumberKind::Number)],
            vec![number(1.5, NumberKind::Number)],
            vec![node(NodeType::Percentage { value: 50.0 })],
            vec![px(1.0, "PX")],
            vec![node(NodeType::Hash { value: "fff".into() })],
            vec![node(NodeType::String { value: "a b".into() })],
            vec![operator("/")],
            vec![node(NodeType::Comma)],
            vec![
                px(1.0, "px"),
                ident("solid"),
                node(NodeType::Hash { value: "f00".into() }),
            ],
            vec![
                number(16.0, NumberKind::Integer),
                operator("/"),
                number(9.0, NumberKind::Integer),
            ],
            vec![],
        ];
        for mut nodes in cases {
            let mut pool = ValuePool::default();
            let pooled = pool.intern(build(nodes.clone()));
            let found = pool.find(&mut nodes, None);
            assert!(found.is_some_and(|found| Arc::ptr_eq(&found, &pooled)), "{nodes:?}");
        }
    }

    #[test]
    fn a_function_is_left_to_the_conversion() {
        let mut pool = ValuePool::default();
        let mut nodes = vec![node(NodeType::Function {
            name: "rgb".into(),
            arguments: vec![],
        })];
        pool.intern(build(nodes.clone()));
        assert!(pool.find(&mut nodes, None).is_none());
    }

    #[test]
    fn a_display_alias_is_looked_up_as_the_keyword_it_stands_for() {
        let mut pool = ValuePool::default();
        let mut nodes = vec![node(NodeType::Ident {
            value: "-webkit-box".into(),
        })];
        pool.intern(CssValue::String("-webkit-box".into()));
        assert!(pool.find(&mut nodes, Some("flex")).is_none());
        let flex = pool.intern(CssValue::String("flex".into()));
        assert!(pool
            .find(&mut nodes, Some("flex"))
            .is_some_and(|found| Arc::ptr_eq(&found, &flex)));
    }

    #[test]
    fn nesting_is_compared_all_the_way_down() {
        let mut pool = ValuePool::default();
        let list = |unit: f64| {
            CssValue::List(vec![
                CssValue::Unit(unit, "px".into()),
                CssValue::String("solid".into()),
            ])
        };
        let first = pool.intern(list(1.0));
        let same = pool.intern(list(1.0));
        let different = pool.intern(list(2.0));
        assert!(Arc::ptr_eq(&first, &same));
        assert!(!Arc::ptr_eq(&first, &different));
    }
}
