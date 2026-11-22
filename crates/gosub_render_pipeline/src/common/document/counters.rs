//! CSS counters (css-lists-3 §4): the counters every generated box is in the scope of, and
//! their values.
//!
//! Counter values depend on everything before a box in tree order, so they are resolved for
//! the whole document in one walk rather than per box. The walk follows the spec's steps for
//! each element and pseudo-element: inherit (§4.4.1), then `counter-reset`, `counter-increment`
//! (with the implicit `list-item` increment of §4.6) and `counter-set`, then the counters
//! `content` uses. What a box does to its counters comes from a [`CounterTree`], which keeps
//! this module free of the style system.

use std::collections::HashMap;
use std::sync::Arc;

use gosub_shared::node::NodeId;

use super::pipeline_doc::PseudoKind;

/// The name of the counter list items increment by themselves (§4.6).
pub const LIST_ITEM: &str = "list-item";

/// An element or a pseudo-element of one: what creates, changes and reads counters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Item {
    Element(NodeId),
    Pseudo(NodeId, PseudoKind),
}

/// What one item does to counters.
#[derive(Debug, Default)]
pub struct Effects {
    /// Whether the item generates a box. One that does not cannot reset, increment or set a
    /// counter (§4.5), and a pseudo-element without one has no `content` to read them.
    pub generates_box: bool,
    /// Whether the walk goes on into the element's children. `display: contents` generates no
    /// box but its children do; `display: none` takes the whole subtree out.
    pub descend: bool,
    /// Whether the item is a list item, and so increments `list-item` unless it says otherwise.
    pub list_item: bool,
    /// `counter-reset`: name, reversed, starting value (`None` only for a reversed counter
    /// that was given none).
    pub reset: Vec<(Arc<str>, bool, Option<i64>)>,
    /// `counter-increment`: name and amount.
    pub increment: Vec<(Arc<str>, i64)>,
    /// `counter-set`: name and value.
    pub set: Vec<(Arc<str>, i64)>,
    /// The counters `counter()` and `counters()` in the item's `content` name.
    pub uses: Vec<Arc<str>>,
}

/// The document as the counter walk sees it.
pub trait CounterTree {
    /// The items under `element` in tree order: its `::marker`, `::before`, its flat-tree child
    /// elements, its `::after`. Called with the document node first.
    fn items(&self, element: NodeId) -> Vec<Item>;
    fn effects(&self, item: Item) -> Effects;
}

/// The counters one box is in the scope of, by name and value, outermost first.
type Values = Arc<[(Arc<str>, i64)]>;

/// The counters each pseudo-element with a box is in the scope of.
#[derive(Debug, Default)]
pub struct CounterTable {
    sets: HashMap<(NodeId, PseudoKind), Values>,
}

impl CounterTable {
    /// The values of every counter named `name` the pseudo-element is in the scope of,
    /// outermost first: what `counters()` writes, and the last one is what `counter()` writes.
    /// Empty for a pseudo-element the walk gave no box.
    pub fn values(&self, owner: NodeId, kind: PseudoKind, name: &str) -> Vec<i64> {
        self.sets
            .get(&(owner, kind))
            .map(|set| {
                set.iter()
                    .filter(|(counter, _)| &**counter == name)
                    .map(|&(_, value)| value)
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Resolve every counter in the document under `root`.
pub fn resolve(tree: &impl CounterTree, root: NodeId) -> CounterTable {
    let unknown = HashMap::new();
    let walk = Walk::run(tree, root, &unknown);
    if walk.tallies.is_empty() {
        return walk.table;
    }
    // A reversed counter without a starting value counts down to where its scope ends, which
    // is only known once the scope has been walked: work the start values out, then walk again.
    let initials = walk
        .tallies
        .into_iter()
        .map(|(id, tally)| (id, clamp(tally.num.saturating_add(tally.last_nonzero_negated))))
        .collect();
    Walk::run(tree, root, &initials).table
}

/// UAs may limit a counter's range, and then clamp to it (§4). This is the range of a 32-bit
/// integer, which is also what other engines use.
fn clamp(value: i64) -> i64 {
    value.clamp(i64::from(i32::MIN), i64::from(i32::MAX))
}

/// A counter: its name and creator identify it.
#[derive(Clone, Debug)]
struct Counter {
    name: Arc<str>,
    creator: Item,
    /// The element whose child the creator is (a pseudo-element's is its owner), to tell which
    /// counters were created by a previous sibling.
    creator_parent: NodeId,
    reversed: bool,
    /// `None` while a reversed counter's start value is still unknown, on the first walk.
    value: Option<i64>,
    /// A reversed counter that was given no starting value.
    implicit_start: bool,
}

type CounterId = (Arc<str>, Item);

/// The running sum of §4.4.2's start-value algorithm for one reversed counter.
#[derive(Default)]
struct Tally {
    num: i64,
    last_nonzero_negated: i64,
    /// A `counter-set` ends the algorithm's loop.
    done: bool,
}

/// One element whose items are being walked.
struct Frame {
    element: NodeId,
    /// The element's own counters, which its children inherit.
    set: Vec<Counter>,
    items: std::vec::IntoIter<Item>,
    /// The counters of the child element walked last: the next item's preceding sibling.
    previous: Option<Vec<Counter>>,
}

struct Walk<'a> {
    initials: &'a HashMap<CounterId, i64>,
    tallies: HashMap<CounterId, Tally>,
    table: CounterTable,
}

impl<'a> Walk<'a> {
    fn run(tree: &impl CounterTree, root: NodeId, initials: &'a HashMap<CounterId, i64>) -> Self {
        let mut walk = Walk {
            initials,
            tallies: HashMap::new(),
            table: CounterTable::default(),
        };
        // The counters of the item just before the current one in tree order.
        let mut last: Vec<Counter> = Vec::new();
        // An explicit stack: a page can nest elements deeper than the call stack goes.
        let mut stack = vec![Frame {
            element: root,
            set: Vec::new(),
            items: tree.items(root).into_iter(),
            previous: None,
        }];
        while let Some(frame) = stack.last_mut() {
            let Some(item) = frame.items.next() else {
                let Some(done) = stack.pop() else { break };
                if let Some(parent) = stack.last_mut() {
                    parent.previous = Some(done.set);
                }
                continue;
            };
            let parent = frame.element;
            // §4.4.1: the counters come from the preceding sibling, or else the parent, and
            // their values from the item just before this one in tree order.
            let source = frame.previous.as_ref().unwrap_or(&frame.set);
            let mut set: Vec<Counter> = last
                .iter()
                .filter(|counter| {
                    source
                        .iter()
                        .any(|other| other.name == counter.name && other.creator == counter.creator)
                })
                .cloned()
                .collect();

            let effects = tree.effects(item);
            if effects.generates_box {
                walk.apply(&mut set, item, parent, &effects);
            }
            last.clone_from(&set);

            match item {
                Item::Pseudo(owner, kind) => {
                    if effects.generates_box {
                        let values = set
                            .iter()
                            .map(|counter| (counter.name.clone(), counter.value.unwrap_or(0)))
                            .collect();
                        walk.table.sets.insert((owner, kind), values);
                    }
                    // Its values carry on in tree order through `last`, but it is nobody's
                    // preceding sibling: a counter it creates stays its own. That is what makes
                    // a reset on `::before` invisible to the element's children (WPT
                    // css-lists/counters-scope-003 and -004).
                }
                Item::Element(id) if effects.descend => stack.push(Frame {
                    element: id,
                    set,
                    items: tree.items(id).into_iter(),
                    previous: None,
                }),
                Item::Element(_) => frame.previous = Some(set),
            }
        }
        walk
    }

    /// Reset, increment and set the item's counters, then create the ones its `content` reads.
    fn apply(&mut self, set: &mut Vec<Counter>, item: Item, parent: NodeId, effects: &Effects) {
        // Only the last of several resets of one name counts (§4.1).
        for (i, (name, reversed, value)) in effects.reset.iter().enumerate() {
            if effects.reset[i + 1..].iter().any(|(later, _, _)| later == name) {
                continue;
            }
            let implicit_start = value.is_none();
            let value = match value {
                Some(value) => Some(clamp(*value)),
                None => self.initials.get(&(name.clone(), item)).copied(),
            };
            instantiate(set, name, item, parent, *reversed, value, implicit_start);
        }

        // Per counter: what this item adds to it and what it sets it to, for the tallies.
        let mut touched: Vec<(usize, i64, Option<i64>)> = Vec::new();
        let mut touch =
            |index: usize, increment: i64, value: Option<i64>| match touched.iter_mut().find(|(i, _, _)| *i == index) {
                Some(entry) => {
                    entry.1 = entry.1.saturating_add(increment);
                    if value.is_some() {
                        entry.2 = value;
                    }
                }
                None => touched.push((index, increment, value)),
            };

        // §4.6: a list item that does not name `list-item` itself increments it by one, or by
        // minus one when the counter is reversed.
        let implicit = effects.list_item && !effects.increment.iter().any(|(name, _)| &**name == LIST_ITEM);
        let list_item: Arc<str> = Arc::from(LIST_ITEM);
        let increments = effects
            .increment
            .iter()
            .map(|(name, by)| (name, Some(*by)))
            .chain(implicit.then_some((&list_item, None)));
        for (name, by) in increments {
            let index = innermost_or_new(set, name, item, parent);
            let counter = &mut set[index];
            let by = by.unwrap_or(if counter.reversed { -1 } else { 1 });
            counter.value = counter.value.map(|value| clamp(value.saturating_add(by)));
            touch(index, by, None);
        }
        for (name, value) in &effects.set {
            let index = innermost_or_new(set, name, item, parent);
            let value = clamp(*value);
            set[index].value = Some(value);
            touch(index, 0, Some(value));
        }
        for name in &effects.uses {
            innermost_or_new(set, name, item, parent);
        }

        for (index, increment, value) in touched {
            let counter = &set[index];
            if !counter.implicit_start {
                continue;
            }
            let tally = self.tallies.entry((counter.name.clone(), counter.creator)).or_default();
            if tally.done {
                continue;
            }
            let negated = increment.saturating_neg();
            if negated != 0 {
                tally.last_nonzero_negated = negated;
            }
            match value {
                Some(value) => {
                    tally.num = tally.num.saturating_add(value);
                    tally.done = true;
                }
                None => tally.num = tally.num.saturating_add(negated),
            }
        }
    }
}

/// The innermost counter named `name`, created with a value of 0 when there is none (§4.2).
fn innermost_or_new(set: &mut Vec<Counter>, name: &Arc<str>, item: Item, parent: NodeId) -> usize {
    if let Some(index) = set.iter().rposition(|counter| counter.name == *name) {
        return index;
    }
    instantiate(set, name, item, parent, false, Some(0), false);
    set.len() - 1
}

/// §4.4.2: a new counter replaces one of the same name made by the item itself or a previous
/// sibling, and nests inside one made by an ancestor.
fn instantiate(
    set: &mut Vec<Counter>,
    name: &Arc<str>,
    item: Item,
    parent: NodeId,
    reversed: bool,
    value: Option<i64>,
    implicit_start: bool,
) {
    if let Some(index) = set.iter().rposition(|counter| counter.name == *name) {
        let innermost = &set[index];
        if innermost.creator == item || innermost.creator_parent == parent {
            set.remove(index);
        }
    }
    set.push(Counter {
        name: name.clone(),
        creator: item,
        creator_parent: parent,
        reversed,
        value,
        implicit_start,
    });
}
