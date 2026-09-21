//! Where a page's memory goes, as a table you can print.
//!
//! The sibling of [`crate::timing`], with one difference that shapes the whole module: timings
//! accumulate, memory is a snapshot. A report describes the engine at one moment, and the moment
//! is printed with it, because two snapshots taken at different points are not comparable.
//!
//! # Why not `size_of` times a count
//!
//! Most of what an element costs is not in the element. A `CssProperties` is a `Vec` and a boxed
//! slice; a `ComputedStyle` is eleven `Arc`s, most of them pointing at the parent's groups or at
//! one process-wide initial group. Multiplying a struct size by a node count answers a question
//! nobody asked, and walking every `Arc` from every element answers the opposite one: it reports
//! shared memory once per sharer, which is exactly the number the style system was rebuilt to
//! make false.
//!
//! So a walk carries a set of the allocations it has already seen, and counts each shared
//! allocation once, on the first element that reaches it. The report keeps those bytes in their
//! own column: `owned` is what this category would free if it went away, `shared` is what it
//! points at and is counted once across the whole snapshot.
//!
//! # Using it
//!
//! Implement [`HeapSize`] for a type by adding everything it owns beyond its own `size_of`:
//!
//! ```ignore
//! impl HeapSize for ElementData {
//!     fn heap_size(&self, walk: &mut Walk) {
//!         self.name.heap_size(walk);
//!         self.attributes.heap_size(walk);
//!     }
//! }
//! ```
//!
//! Then hand the collector's rows to [`record`] and print with [`dump`].

use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

/// A walk over a value graph, accumulating bytes and remembering which shared allocations it has
/// already counted.
///
/// Bytes land in `owned` or `shared` depending on whether the walk is currently inside an `Arc`
/// (or anything else reached through a pointer several owners share). That split is the whole
/// point: it is what tells you an element holds one style group of its own and points at ten.
#[derive(Default)]
pub struct Walk {
    /// Addresses of shared allocations already counted, so each is counted once per snapshot.
    seen: HashSet<usize>,
    /// How deep inside shared allocations the walk currently is. Non-zero means bytes are shared.
    shared_depth: u32,
    owned: usize,
    shared: usize,
}

impl Walk {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Bytes owned outright by whatever the walk is currently inside.
    pub fn bytes(&mut self, count: usize) {
        if self.shared_depth > 0 {
            self.shared += count;
        } else {
            self.owned += count;
        }
    }

    /// Walk into an allocation that may have several owners, counting it only the first time it
    /// is reached in this snapshot. Everything `body` adds is attributed to `shared`.
    ///
    /// `address` identifies the allocation: for an `Arc<T>`, `Arc::as_ptr`. Two live allocations
    /// never share an address, and a snapshot is taken while everything it walks is alive, so an
    /// address cannot be reused underneath the set.
    pub fn shared_once<F>(&mut self, address: usize, body: F)
    where
        F: FnOnce(&mut Self),
    {
        if !self.seen.insert(address) {
            return;
        }
        self.shared_depth += 1;
        body(self);
        self.shared_depth -= 1;
    }

    /// Bytes this walk found outside shared allocations.
    #[must_use]
    pub fn owned(&self) -> usize {
        self.owned
    }

    /// Bytes in shared allocations, each counted once.
    #[must_use]
    pub fn shared(&self) -> usize {
        self.shared
    }

    /// Start a fresh count, keeping the set of allocations already seen.
    ///
    /// This is what lets one snapshot report several categories that share with each other: the
    /// second category to reach a group does not count it again, so the rows add up to the
    /// snapshot's real total rather than to a sum with the shared parts counted twice.
    pub fn take_counts(&mut self) -> (usize, usize) {
        let counts = (self.owned, self.shared);
        self.owned = 0;
        self.shared = 0;
        counts
    }
}

/// Everything a value owns beyond its own `size_of`.
///
/// An implementation adds its heap allocations to the walk; it must not add `size_of::<Self>()`,
/// which the owner has already counted (inline in a struct, in a `Vec`'s capacity, or as the
/// pointee of an `Arc`).
pub trait HeapSize {
    fn heap_size(&self, walk: &mut Walk);
}

impl HeapSize for String {
    fn heap_size(&self, walk: &mut Walk) {
        walk.bytes(self.capacity());
    }
}

impl HeapSize for str {
    fn heap_size(&self, _walk: &mut Walk) {}
}

impl<T: HeapSize> HeapSize for Option<T> {
    fn heap_size(&self, walk: &mut Walk) {
        if let Some(value) = self {
            value.heap_size(walk);
        }
    }
}

impl<T: HeapSize> HeapSize for Vec<T> {
    fn heap_size(&self, walk: &mut Walk) {
        // The buffer is the capacity, not the length: a `Vec` that grew and shrank still holds it.
        walk.bytes(self.capacity() * size_of::<T>());
        for item in self {
            item.heap_size(walk);
        }
    }
}

impl<T: HeapSize> HeapSize for Box<[T]> {
    fn heap_size(&self, walk: &mut Walk) {
        walk.bytes(self.len() * size_of::<T>());
        for item in self.iter() {
            item.heap_size(walk);
        }
    }
}

impl<K: HeapSize, V: HeapSize> HeapSize for HashMap<K, V> {
    fn heap_size(&self, walk: &mut Walk) {
        // hashbrown allocates one control byte per bucket alongside the entries, and keeps the
        // table under 7/8 full. This is the shape of that, not a claim to match it exactly.
        let buckets = self.capacity().next_power_of_two().max(1);
        walk.bytes(buckets * (size_of::<(K, V)>() + 1));
        for (key, value) in self {
            key.heap_size(walk);
            value.heap_size(walk);
        }
    }
}

impl<T: HeapSize> HeapSize for HashSet<T> {
    fn heap_size(&self, walk: &mut Walk) {
        let buckets = self.capacity().next_power_of_two().max(1);
        walk.bytes(buckets * (size_of::<T>() + 1));
        for item in self {
            item.heap_size(walk);
        }
    }
}

impl<T: HeapSize> HeapSize for Arc<T> {
    fn heap_size(&self, walk: &mut Walk) {
        let address = Arc::as_ptr(self) as usize;
        walk.shared_once(address, |walk| {
            // The allocation holds the value and two reference counts.
            walk.bytes(size_of::<T>() + 2 * size_of::<usize>());
            T::heap_size(self, walk);
        });
    }
}

/// `Arc<str>` is not `Arc<T: Sized>`, so it needs its own implementation - and it is worth
/// having: the style groups hold their few strings this way precisely so that a thousand
/// elements naming the same font family hold one allocation between them.
impl HeapSize for Arc<str> {
    fn heap_size(&self, walk: &mut Walk) {
        let address = self.as_ptr() as usize;
        walk.shared_once(address, |walk| {
            walk.bytes(self.len() + 2 * size_of::<usize>());
        });
    }
}

/// Types with nothing on the heap. `impl_flat!(u32, NodeId, ...)`.
#[macro_export]
macro_rules! impl_flat_heap_size {
    ($($ty:ty),* $(,)?) => {
        $(impl $crate::memory::HeapSize for $ty {
            fn heap_size(&self, _walk: &mut $crate::memory::Walk) {}
        })*
    };
}

impl_flat_heap_size!(u8, u16, u32, u64, usize, i8, i16, i32, i64, isize, f32, f64, bool, char);
impl_flat_heap_size!(crate::node::NodeId, crate::byte_stream::Location);

/// One line of the report: a category of thing, how many there are, and what they cost.
#[derive(Clone, Debug)]
pub struct Row {
    /// What was counted, e.g. `"dom.nodes"`. Dotted, like a timing namespace.
    pub category: &'static str,
    /// How many of them.
    pub count: u64,
    /// `count * size_of::<T>()`: the structs themselves, wherever they live.
    pub inline: usize,
    /// Heap allocations these own outright.
    pub owned: usize,
    /// Heap allocations they share, counted once across the snapshot.
    pub shared: usize,
    /// Anything worth saying about how the number was reached.
    pub note: Option<String>,
}

impl Row {
    #[must_use]
    pub fn new(category: &'static str, count: u64, inline: usize, owned: usize, shared: usize) -> Self {
        Self {
            category,
            count,
            inline,
            owned,
            shared,
            note: None,
        }
    }

    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    #[must_use]
    pub fn total(&self) -> usize {
        self.inline + self.owned + self.shared
    }

    /// Bytes per counted thing, which is usually the number worth comparing across pages.
    #[must_use]
    pub fn per_item(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.total() as f64 / self.count as f64
        }
    }
}

#[derive(Default)]
struct Snapshot {
    /// What the engine was doing when this was taken.
    moment: String,
    rows: Vec<Row>,
}

fn snapshot() -> &'static Mutex<Snapshot> {
    static SNAPSHOT: OnceLock<Mutex<Snapshot>> = OnceLock::new();
    SNAPSHOT.get_or_init(|| Mutex::new(Snapshot::default()))
}

/// Throw away the previous snapshot and start a new one. `moment` says when it was taken - "after
/// first render", "after the capture" - and is printed in the table's header.
#[cfg(feature = "memory")]
pub fn begin(moment: impl Into<String>) {
    let mut snapshot = snapshot().lock();
    snapshot.moment = moment.into();
    snapshot.rows.clear();
}

#[cfg(not(feature = "memory"))]
pub fn begin(_moment: impl Into<String>) {}

/// Add a row to the current snapshot.
#[cfg(feature = "memory")]
pub fn record(row: Row) {
    snapshot().lock().rows.push(row);
}

#[cfg(not(feature = "memory"))]
pub fn record(_row: Row) {}

/// The rows of the current snapshot, for a caller that wants to render them itself.
#[cfg(feature = "memory")]
#[must_use]
pub fn rows() -> Vec<Row> {
    snapshot().lock().rows.clone()
}

#[cfg(not(feature = "memory"))]
#[must_use]
pub fn rows() -> Vec<Row> {
    Vec::new()
}

/// Human-readable bytes, matching the scale the timing table uses for durations.
#[must_use]
pub fn format_bytes(bytes: usize) -> String {
    const UNITS: [(&str, f64); 4] = [("GB", 1e9), ("MB", 1e6), ("kB", 1e3), ("B", 1.0)];
    let value = bytes as f64;
    for (unit, scale) in UNITS {
        if value >= scale {
            return if scale == 1.0 {
                format!("{bytes} B")
            } else {
                format!("{:.1} {unit}", value / scale)
            };
        }
    }
    "0 B".to_string()
}

/// Print the current snapshot.
#[cfg(feature = "memory")]
pub fn dump() {
    let snapshot = snapshot().lock();
    let moment = if snapshot.moment.is_empty() {
        "unspecified moment".to_string()
    } else {
        snapshot.moment.clone()
    };

    println!("\n=== Memory table (snapshot: {moment}) ===");
    println!(
        "{:<28} | {:>9} | {:>11} | {:>11} | {:>11} | {:>11} | {:>9}",
        "Category", "Count", "Inline", "Owned heap", "Shared", "Total", "Per item"
    );
    println!("{}", "-".repeat(110));

    let mut total = 0usize;
    for row in &snapshot.rows {
        total += row.total();
        println!(
            "{:<28} | {:>9} | {:>11} | {:>11} | {:>11} | {:>11} | {:>9}",
            row.category,
            row.count,
            format_bytes(row.inline),
            format_bytes(row.owned),
            format_bytes(row.shared),
            format_bytes(row.total()),
            format!("{:.0} B", row.per_item()),
        );
        if let Some(note) = &row.note {
            println!("{:<28} | {note}", "");
        }
    }

    println!("{}", "-".repeat(110));
    println!("{:<28} | {:>9} | {:>49} | {:>11}", "total", "", "", format_bytes(total));
    println!("\nShared allocations are counted once, on the first row that reaches them.");
    println!();
}

#[cfg(not(feature = "memory"))]
pub fn dump() {}

#[cfg(test)]
mod tests {
    use super::*;

    struct Group {
        values: Vec<u32>,
    }

    impl HeapSize for Group {
        fn heap_size(&self, walk: &mut Walk) {
            self.values.heap_size(walk);
        }
    }

    struct Element {
        own: Vec<u32>,
        group: Arc<Group>,
    }

    impl HeapSize for Element {
        fn heap_size(&self, walk: &mut Walk) {
            self.own.heap_size(walk);
            self.group.heap_size(walk);
        }
    }

    #[test]
    fn a_shared_group_is_counted_once_however_many_point_at_it() {
        let group = Arc::new(Group {
            values: Vec::with_capacity(10),
        });
        let elements: Vec<Element> = (0..100)
            .map(|_| Element {
                own: Vec::with_capacity(2),
                group: Arc::clone(&group),
            })
            .collect();

        let mut walk = Walk::new();
        for element in &elements {
            element.heap_size(&mut walk);
        }

        // 100 elements own two u32s each; the group is counted once, not a hundred times.
        assert_eq!(walk.owned(), 100 * 2 * size_of::<u32>());
        assert_eq!(
            walk.shared(),
            size_of::<Group>() + 2 * size_of::<usize>() + 10 * size_of::<u32>()
        );
    }

    #[test]
    fn bytes_inside_an_arc_are_shared_even_when_several_levels_down() {
        struct Outer {
            inner: Arc<Vec<String>>,
        }
        impl HeapSize for Outer {
            fn heap_size(&self, walk: &mut Walk) {
                self.inner.heap_size(walk);
            }
        }

        let outer = Outer {
            inner: Arc::new(vec![String::from("a string with a heap buffer")]),
        };
        let mut walk = Walk::new();
        outer.heap_size(&mut walk);

        assert_eq!(walk.owned(), 0, "nothing below an Arc is owned by the walker");
        assert!(walk.shared() > 27, "the String's buffer is counted, inside the Arc");
    }

    #[test]
    fn take_counts_resets_the_totals_but_not_what_has_been_seen() {
        let group = Arc::new(Group { values: Vec::new() });
        let mut walk = Walk::new();

        group.heap_size(&mut walk);
        let (owned, shared) = walk.take_counts();
        assert_eq!(owned, 0);
        assert!(shared > 0);

        // A second category reaching the same group adds nothing: the snapshot already has it.
        group.heap_size(&mut walk);
        assert_eq!(walk.take_counts(), (0, 0));
    }

    #[test]
    fn a_string_costs_its_capacity() {
        let mut text = String::with_capacity(64);
        text.push_str("short");
        let mut walk = Walk::new();
        text.heap_size(&mut walk);
        assert_eq!(walk.owned(), 64);
    }
}
