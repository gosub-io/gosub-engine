//! The mirrors pages served from the network process declared for their items, kept by the
//! broker to hold a later navigation from such a page to them (`NetProcess::plausible_reply`).
//!
//! A navigation over an engine protocol that serves mirrors (gopher) may end on another server
//! than its URL names: one the page the user came from named as a mirror of the item. The
//! network process says which one answered, and only it parsed the page, so the broker keeps
//! what the page declared ([`FromNet::Mirror`]) and accepts a final URL on another server only
//! when it is one of those. This binds the final URL to a mirror the page showed; it is no
//! defence against a compromised network process, which wrote that page and its declarations
//! itself. It guards against bugs, and makes the rule one that can be checked.
//!
//! Declarations are taken only for a request the broker opened for them ([`Declared::open`]),
//! and only for a page served under that request's own URL or one of the mirrors recorded for
//! it. They are kept by zone and by the page, without fragments. Bounded both ways: the oldest
//! open request and the item used least recently are forgotten first.
//!
//! [`FromNet::Mirror`]: crate::net::process::protocol::FromNet::Mirror

use crate::net::process::protocol::RequestTag;
use parking_lot::Mutex;
use std::collections::VecDeque;
use url::Url;

/// Requests whose pages may still declare mirrors. A streamed page declares them until its
/// last line, after its reply: the oldest open request gives way, not the one that ended.
const OPEN_REQUESTS: usize = 64;

/// Items whose mirrors are kept, across all zones and pages.
const KEPT_ITEMS: usize = 256;

/// Mirrors kept for one item.
const MIRRORS_PER_ITEM: usize = 4;

/// Longest URL a declaration may carry.
const MAX_URL: usize = 8 * 1024;

/// A request open for declarations: its zone, and the URLs its page may be served under.
struct Open {
    tag: RequestTag,
    zone: String,
    pages: Vec<String>,
}

struct Entry {
    zone: String,
    page: String,
    item: String,
    mirrors: Vec<String>,
}

#[derive(Default)]
struct Inner {
    open: VecDeque<Open>,
    kept: VecDeque<Entry>,
}

/// The broker's record of declared mirrors.
#[derive(Default)]
pub struct Declared(Mutex<Inner>);

impl Declared {
    /// Take declarations from the page request `tag` serves in `zone`, under `url` or one of
    /// `mirrors`. Called before the request goes out.
    pub fn open(&self, tag: RequestTag, zone: &str, url: &str, mirrors: &[String]) {
        let mut pages: Vec<String> = mirrors.to_vec();
        pages.extend(key(url));
        let mut inner = self.0.lock();
        if inner.open.len() >= OPEN_REQUESTS {
            inner.open.pop_front();
        }
        inner.open.push_back(Open {
            tag,
            zone: zone.to_string(),
            pages,
        });
    }

    /// The page request `tag` serves, under `page`, names `mirror` for `item`. Ignored for a
    /// request not open for it, a page it is not served under, and anything that is not a URL
    /// of the page's scheme.
    pub fn declare(&self, tag: RequestTag, page: &str, item: &str, mirror: &str) {
        let (Some(page), Some(item), Some(mirror)) = (key(page), key(item), key(mirror)) else {
            return;
        };
        let scheme = |url: &str| url.split_once(':').map(|(scheme, _)| scheme.to_string());
        if scheme(&item) != scheme(&page) || scheme(&mirror) != scheme(&page) {
            return;
        }
        let mut inner = self.0.lock();
        let Some(open) = inner.open.iter().find(|open| open.tag == tag) else {
            return;
        };
        if !open.pages.contains(&page) {
            return;
        }
        let zone = open.zone.clone();
        let found = take(&mut inner.kept, &zone, &page, &item);
        let mut entry = found.unwrap_or(Entry {
            zone,
            page,
            item,
            mirrors: Vec::new(),
        });
        if entry.mirrors.len() < MIRRORS_PER_ITEM && !entry.mirrors.contains(&mirror) {
            entry.mirrors.push(mirror);
        }
        if inner.kept.len() >= KEPT_ITEMS {
            inner.kept.pop_front();
        }
        inner.kept.push_back(entry);
    }

    /// The mirrors the page at `from` declared in `zone` for `item`, without fragments.
    pub fn mirrors(&self, zone: &str, from: &str, item: &str) -> Vec<String> {
        let (Some(from), Some(item)) = (key(from), key(item)) else {
            return Vec::new();
        };
        let mut inner = self.0.lock();
        let Some(entry) = take(&mut inner.kept, zone, &from, &item) else {
            return Vec::new();
        };
        let mirrors = entry.mirrors.clone();
        inner.kept.push_back(entry);
        mirrors
    }
}

fn take(kept: &mut VecDeque<Entry>, zone: &str, page: &str, item: &str) -> Option<Entry> {
    let at = kept
        .iter()
        .position(|e| e.zone == zone && e.page == page && e.item == item)?;
    kept.remove(at)
}

/// `url` as it is kept: parsed, without its fragment. `None` for what is no URL.
pub fn key(url: &str) -> Option<String> {
    if url.len() > MAX_URL {
        return None;
    }
    let mut url = Url::parse(url).ok()?;
    url.set_fragment(None);
    Some(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MENU: &str = "gopher://menu.test/1/";
    const ITEM: &str = "gopher://primary.test/0/a";
    const MIRROR: &str = "gopher://mirror.test/0/a";

    #[test]
    fn a_declared_mirror_is_kept_for_its_page_and_zone() {
        let declared = Declared::default();
        declared.open(1, "z", MENU, &[]);
        declared.declare(1, MENU, ITEM, MIRROR);
        assert_eq!(declared.mirrors("z", MENU, ITEM), [MIRROR]);
        assert_eq!(declared.mirrors("z", "gopher://menu.test/1/#top", ITEM), [MIRROR]);
        assert!(declared.mirrors("other", MENU, ITEM).is_empty(), "another zone");
        assert!(
            declared.mirrors("z", "gopher://menu.test/1/other", ITEM).is_empty(),
            "another page"
        );
        assert!(
            declared.mirrors("z", MENU, "gopher://primary.test/0/b").is_empty(),
            "another item"
        );
    }

    #[test]
    fn only_a_request_opened_for_it_declares_for_its_own_page() {
        let declared = Declared::default();
        declared.declare(1, MENU, ITEM, MIRROR);
        assert!(declared.mirrors("z", MENU, ITEM).is_empty(), "never opened");
        declared.open(2, "z", MENU, &[]);
        declared.declare(2, "gopher://elsewhere.test/1/", ITEM, MIRROR);
        assert!(
            declared.mirrors("z", "gopher://elsewhere.test/1/", ITEM).is_empty(),
            "not its page"
        );
        declared.declare(2, MENU, ITEM, "https://web.test/");
        declared.declare(2, MENU, "file:///etc/passwd", MIRROR);
        assert!(declared.mirrors("z", MENU, ITEM).is_empty(), "another scheme");
    }

    #[test]
    fn a_page_served_by_its_mirror_declares_under_the_mirror() {
        let declared = Declared::default();
        let served_by = "gopher://mirror.test/1/";
        declared.open(1, "z", MENU, &[served_by.to_string()]);
        declared.declare(1, served_by, ITEM, MIRROR);
        assert_eq!(declared.mirrors("z", served_by, ITEM), [MIRROR]);
    }

    #[test]
    fn an_item_keeps_a_few_mirrors_once_each() {
        let declared = Declared::default();
        declared.open(1, "z", MENU, &[]);
        for n in 0..MIRRORS_PER_ITEM + 2 {
            let mirror = format!("gopher://m{n}.test/0/a");
            declared.declare(1, MENU, ITEM, &mirror);
            declared.declare(1, MENU, ITEM, &mirror);
        }
        let mirrors = declared.mirrors("z", MENU, ITEM);
        assert_eq!(mirrors.len(), MIRRORS_PER_ITEM);
        assert_eq!(mirrors[0], "gopher://m0.test/0/a");
    }

    #[test]
    fn the_oldest_open_request_and_least_used_item_give_way() {
        let declared = Declared::default();
        declared.open(0, "z", MENU, &[]);
        for tag in 1..=OPEN_REQUESTS as u64 {
            declared.open(tag, "z", "gopher://other.test/1/", &[]);
        }
        declared.declare(0, MENU, ITEM, MIRROR);
        assert!(declared.mirrors("z", MENU, ITEM).is_empty(), "closed by the cap");

        declared.open(100, "z", MENU, &[]);
        let item = |n: usize| format!("gopher://primary.test/0/{n}");
        declared.declare(100, MENU, &item(0), MIRROR);
        declared.declare(100, MENU, &item(1), MIRROR);
        declared.mirrors("z", MENU, &item(0));
        for n in 2..=KEPT_ITEMS {
            declared.declare(100, MENU, &item(n), MIRROR);
        }
        assert_eq!(declared.mirrors("z", MENU, &item(0)), [MIRROR]);
        assert!(declared.mirrors("z", MENU, &item(1)).is_empty());
    }
}
