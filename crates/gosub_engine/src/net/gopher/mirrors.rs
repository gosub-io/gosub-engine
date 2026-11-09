//! The mirrors menus name for their items (`+` lines, RFC 1436 §3.8), for a navigation to fall
//! back on when the item's server does not answer.
//!
//! Kept by zone and by the menu that named them: only a navigation that menu's page started,
//! in the zone that showed it, is offered an item's mirrors. One typed in, or from any other
//! page or zone, goes to the server its URL names and nowhere else. Kept in the process that
//! fetches, which renders the menu too: this one, or the network process, which also declares
//! them to the broker ([`Keeper`]) so the broker can hold the mirror's reply to them. Bounded:
//! the item used least recently is forgotten first.

use crate::net::protocols::DeclareMirrors;
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::Arc;
use url::Url;

/// Items whose mirrors are kept, across all zones and menus.
const KEPT_ITEMS: usize = 256;

/// Mirrors kept for one item; the rest of a menu's are only shown.
pub const MIRRORS_PER_ITEM: usize = 4;

struct Entry {
    zone: String,
    menu: String,
    item: String,
    mirrors: Vec<Url>,
}

/// The items' mirrors, least recently used first.
struct Kept(VecDeque<Entry>);

static KEPT: Mutex<Kept> = Mutex::new(Kept(VecDeque::new()));

/// Where the mirrors a menu names go: this process's store, for the zone, and the broker, when
/// the menu is served from the network process.
#[derive(Clone)]
pub struct Keeper {
    pub zone: String,
    pub declare: Option<Arc<dyn DeclareMirrors>>,
}

impl Keeper {
    /// Keep `mirror` for `item` on the menu at `menu`.
    pub fn keep(&self, menu: &Url, item: &Url, mirror: &Url) {
        remember(&self.zone, menu, item, mirror);
        if let Some(declare) = &self.declare {
            declare.mirror(menu, item, mirror);
        }
    }
}

/// Remember `mirror` for `item` on the menu at `menu`, shown in `zone`.
pub fn remember(zone: &str, menu: &Url, item: &Url, mirror: &Url) {
    KEPT.lock().remember(zone, menu, item, mirror);
}

/// The mirrors of `item` a navigation from the page at `menu` in `zone` may fall back on, in
/// the menu's order.
pub fn of(zone: &str, menu: &Url, item: &Url) -> Vec<Url> {
    KEPT.lock().of(zone, menu, item)
}

impl Kept {
    fn remember(&mut self, zone: &str, menu: &Url, item: &Url, mirror: &Url) {
        let (menu, item) = (key(menu), key(item));
        let found = self.take(zone, &menu, &item);
        let mut entry = found.unwrap_or_else(|| Entry {
            zone: zone.to_string(),
            menu,
            item,
            mirrors: Vec::new(),
        });
        if entry.mirrors.len() < MIRRORS_PER_ITEM && !entry.mirrors.contains(mirror) {
            entry.mirrors.push(mirror.clone());
        }
        if self.0.len() >= KEPT_ITEMS {
            self.0.pop_front();
        }
        self.0.push_back(entry);
    }

    fn of(&mut self, zone: &str, menu: &Url, item: &Url) -> Vec<Url> {
        let Some(entry) = self.take(zone, &key(menu), &key(item)) else {
            return Vec::new();
        };
        let mirrors = entry.mirrors.clone();
        self.0.push_back(entry);
        mirrors
    }

    fn take(&mut self, zone: &str, menu: &str, item: &str) -> Option<Entry> {
        let at = self
            .0
            .iter()
            .position(|e| e.zone == zone && e.menu == menu && e.item == item)?;
        self.0.remove(at)
    }
}

/// A URL as it is kept: without its fragment, which names no other resource.
fn key(url: &Url) -> String {
    let mut url = url.clone();
    url.set_fragment(None);
    url.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn mirrors_are_offered_only_from_their_menu_in_their_zone() {
        let (menu, item, mirror) = (
            url("gopher://menu.test/1/kept-by-menu"),
            url("gopher://primary.test/0/a"),
            url("gopher://mirror.test/0/a"),
        );
        remember("zone-a", &menu, &item, &mirror);
        assert_eq!(of("zone-a", &menu, &item), vec![mirror.clone()]);
        assert_eq!(
            of("zone-a", &url("gopher://menu.test/1/other"), &item),
            Vec::<Url>::new()
        );
        assert_eq!(of("zone-b", &menu, &item), Vec::<Url>::new());
        assert_eq!(
            of("zone-a", &url("gopher://menu.test/1/kept-by-menu#top"), &item),
            vec![mirror]
        );
    }

    #[test]
    fn an_item_keeps_a_few_mirrors_once_each() {
        let (menu, item) = (url("gopher://menu.test/1/few"), url("gopher://primary.test/0/few"));
        for n in 0..MIRRORS_PER_ITEM + 2 {
            let mirror = url(&format!("gopher://m{n}.test/0/few"));
            remember("zone-few", &menu, &item, &mirror);
            remember("zone-few", &menu, &item, &mirror);
        }
        let mirrors = of("zone-few", &menu, &item);
        assert_eq!(mirrors.len(), MIRRORS_PER_ITEM);
        assert_eq!(mirrors[0], url("gopher://m0.test/0/few"));
    }

    #[test]
    fn the_least_recently_used_item_is_forgotten_first() {
        let mut kept = Kept(VecDeque::new());
        let menu = url("gopher://menu.test/1/");
        let item = |n: usize| url(&format!("gopher://primary.test/0/{n}"));
        let mirror = url("gopher://mirror.test/0/");
        kept.remember("z", &menu, &item(0), &mirror);
        kept.remember("z", &menu, &item(1), &mirror);
        // Used again, so it outlives the second.
        kept.of("z", &menu, &item(0));
        for n in 2..=KEPT_ITEMS {
            kept.remember("z", &menu, &item(n), &mirror);
        }
        assert_eq!(kept.0.len(), KEPT_ITEMS);
        assert_eq!(kept.of("z", &menu, &item(0)), vec![mirror]);
        assert!(kept.of("z", &menu, &item(1)).is_empty());
    }
}
