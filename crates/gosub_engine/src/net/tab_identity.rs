//! What the I/O side knows about a tab, so it can attach cookies to a request
//! without the tab ever handling them.

use crate::cookies::CookieJarHandle;
use crate::engine::types::NavigationId;
use crate::net::req_ref_tracker::RequestReference;
use crate::net::ssrf::AddressSpace;
use crate::tab::TabId;
use dashmap::DashMap;
use std::collections::VecDeque;
use url::Url;

/// How many of a tab's navigations keep their address space: the committed one
/// plus those still loading, of which there is rarely more than one.
const KEPT_NAVIGATIONS: usize = 8;

/// The per-tab facts the I/O side needs to complete a request on its own.
#[derive(Clone, Debug)]
pub struct TabIdentity {
    /// The tab's effective jar, resolved once at tab creation from zone services
    /// and any per-tab override.
    pub cookie_jar: CookieJarHandle,
    /// The document the tab is currently loading or showing, used as the
    /// top-level URL for `SameSite` and partitioning decisions. `None` until the
    /// first navigation.
    pub top_level: Option<Url>,
    /// The address space each recent navigation's response came from, newest
    /// last. Recorded when the response arrives, keyed by the navigation and
    /// not by its URL: a navigation that never commits (a `204`, a download)
    /// gets an entry no displayed document's requests can name.
    pub navigations: VecDeque<(NavigationId, AddressSpace)>,
    /// The navigation whose document the tab shows, once one has committed.
    pub committed: Option<NavigationId>,
    /// What each recent navigation's `SameSite` context is judged from
    /// (RFC 6265bis §5.2), newest last: the page that started it (a link, a
    /// form), or for the user's own its destination. Kept per navigation, not
    /// as `top_level`, which moves on to the next navigation while a
    /// superseded one may still be following a redirect.
    pub navigation_sites: VecDeque<(NavigationId, Url)>,
}

impl TabIdentity {
    pub fn new(cookie_jar: CookieJarHandle) -> Self {
        Self {
            cookie_jar,
            top_level: None,
            navigations: VecDeque::new(),
            committed: None,
            navigation_sites: VecDeque::new(),
        }
    }

    /// The document `navigation`'s `SameSite` context is judged from (see
    /// [`Self::navigation_sites`]); `None` for one this tab no longer
    /// remembers, which then gets no cookies. Only the context: the
    /// third-party policy keeps `top_level`, as a navigation's own cookies are
    /// first-party.
    pub fn navigation_site(&self, navigation: NavigationId) -> Option<&Url> {
        self.navigation_sites
            .iter()
            .rev()
            .find(|(n, _)| *n == navigation)
            .map(|(_, site)| site)
    }

    /// The address space of the document a request with `reference` was made
    /// for. A navigation's own subresources name it; anything else the tab asks
    /// for (a renderer's loads, a favicon, a download) is for the document it
    /// shows. Public unless a response was recorded in a more private space: no
    /// record, no exemption from the private-network policy.
    pub fn document_space(&self, reference: Option<RequestReference>) -> AddressSpace {
        let navigation = match reference {
            Some(RequestReference::Navigation(id)) => Some(id),
            _ => self.committed,
        };
        navigation
            .and_then(|id| self.navigations.iter().rev().find(|(n, _)| *n == id))
            .map_or(AddressSpace::Public, |(_, space)| *space)
    }
}

/// Maps a tab to its cookie jar and current top-level document.
#[derive(Debug, Default)]
pub struct TabIdentityRegistry {
    tabs: DashMap<TabId, TabIdentity>,
}

impl TabIdentityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a tab's jar at creation. Its top-level URL is unset until it
    /// navigates.
    pub fn register(&self, tab_id: TabId, cookie_jar: CookieJarHandle) {
        self.tabs.insert(tab_id, TabIdentity::new(cookie_jar));
    }

    /// Point a tab at the document it is now loading. Called before the
    /// navigation request is submitted, so that request is already attributed to
    /// its own URL.
    pub fn set_top_level(&self, tab_id: TabId, url: Url) {
        if let Some(mut entry) = self.tabs.get_mut(&tab_id) {
            entry.top_level = Some(url);
        }
    }

    /// Record where a navigation's response came from. Before its document can
    /// load anything: the in-process pipeline parses, and fetches subresources,
    /// before the navigation commits.
    pub fn record_navigation(&self, tab_id: TabId, navigation: NavigationId, space: AddressSpace) {
        let Some(mut entry) = self.tabs.get_mut(&tab_id) else {
            return;
        };
        entry.navigations.retain(|(n, _)| *n != navigation);
        entry.navigations.push_back((navigation, space));
        while entry.navigations.len() > KEPT_NAVIGATIONS {
            let committed = entry.committed;
            match entry.navigations.iter().position(|(n, _)| Some(*n) != committed) {
                Some(oldest) => {
                    entry.navigations.remove(oldest);
                }
                None => break,
            }
        }
    }

    /// Record what `navigation` is judged from (see
    /// [`TabIdentity::navigation_sites`]): the page that started it, or the
    /// destination of the user's own. Keeps the most recent few.
    pub fn record_navigation_site(&self, tab_id: TabId, navigation: NavigationId, site: Url) {
        let Some(mut entry) = self.tabs.get_mut(&tab_id) else {
            return;
        };
        entry.navigation_sites.retain(|(n, _)| *n != navigation);
        entry.navigation_sites.push_back((navigation, site));
        while entry.navigation_sites.len() > KEPT_NAVIGATIONS {
            entry.navigation_sites.pop_front();
        }
    }

    /// The tab now shows the document `navigation` produced.
    pub fn commit_navigation(&self, tab_id: TabId, navigation: NavigationId) {
        if let Some(mut entry) = self.tabs.get_mut(&tab_id) {
            entry.committed = Some(navigation);
        }
    }

    pub fn get(&self, tab_id: TabId) -> Option<TabIdentity> {
        self.tabs.get(&tab_id).map(|e| e.clone())
    }

    /// Forget a closed tab. A fetch that outlives its tab then finds no identity
    /// and is sent without cookies, rather than borrowing a stale jar.
    pub fn remove(&self, tab_id: TabId) {
        self.tabs.remove(&tab_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cookies::DefaultCookieJar;

    fn jar() -> CookieJarHandle {
        DefaultCookieJar::new().into()
    }

    #[test]
    fn registers_and_resolves_a_tab() {
        let reg = TabIdentityRegistry::new();
        let tab = TabId::new();
        reg.register(tab, jar());

        let id = reg.get(tab).expect("registered tab resolves");
        assert!(id.top_level.is_none(), "no top-level before the first navigation");
    }

    /// Each navigation keeps what it is judged from - the page that started it, or the
    /// destination of the user's own - after the tab has moved on: a superseded navigation
    /// still following a redirect must not borrow the next one's site.
    #[test]
    fn a_navigation_keeps_the_site_it_is_judged_from() {
        let reg = TabIdentityRegistry::new();
        let tab = TabId::new();
        reg.register(tab, jar());
        let page = Url::parse("https://a.test/links").unwrap();
        let typed = Url::parse("https://b.test/").unwrap();
        let (linked, superseding) = (NavigationId::new(), NavigationId::new());

        reg.record_navigation_site(tab, linked, page.clone());
        reg.set_top_level(tab, typed.clone());
        reg.record_navigation_site(tab, superseding, typed.clone());
        let id = reg.get(tab).unwrap();
        assert_eq!(
            id.navigation_site(linked),
            Some(&page),
            "the superseded one keeps its page"
        );
        assert_eq!(id.navigation_site(superseding), Some(&typed));
        assert_eq!(id.navigation_site(NavigationId::new()), None, "an unknown one has none");

        for _ in 0..KEPT_NAVIGATIONS {
            reg.record_navigation_site(tab, NavigationId::new(), typed.clone());
        }
        assert_eq!(
            reg.get(tab).unwrap().navigation_site(linked),
            None,
            "only the recent few are kept"
        );
    }

    #[test]
    fn navigation_sets_the_top_level_document() {
        let reg = TabIdentityRegistry::new();
        let tab = TabId::new();
        reg.register(tab, jar());

        let url = Url::parse("https://example.com/page").unwrap();
        reg.set_top_level(tab, url.clone());

        assert_eq!(reg.get(tab).unwrap().top_level, Some(url));
    }

    /// Requests are judged by the navigation whose document made them: its own
    /// record while it loads, the committed one's for the tab's other loads.
    /// A navigation that never commits changes neither.
    #[test]
    fn a_document_is_placed_by_its_own_navigation() {
        use crate::engine::types::NavigationId;
        use crate::net::req_ref_tracker::RequestReference::{Document, Navigation};
        use crate::net::ssrf::AddressSpace::{Local, Public};

        let reg = TabIdentityRegistry::new();
        let tab = TabId::new();
        reg.register(tab, jar());
        let space = |r| reg.get(tab).unwrap().document_space(r);
        assert_eq!(space(None), Public, "nothing committed yet");

        let intranet = NavigationId::new();
        reg.record_navigation(tab, intranet, Local);
        assert_eq!(
            space(Some(Navigation(intranet))),
            Local,
            "its own loads, before the commit"
        );
        assert_eq!(space(Some(Document(1))), Public, "not yet the tab's document");
        reg.commit_navigation(tab, intranet);
        assert_eq!(space(Some(Document(1))), Local);
        assert_eq!(space(None), Local);

        // A rebound navigation that answers 204 from loopback, then one served
        // from a public address: neither has committed, so the tab is unchanged.
        let rebound = NavigationId::new();
        reg.record_navigation(tab, rebound, Local);
        let public = NavigationId::new();
        reg.record_navigation(tab, public, Public);
        assert_eq!(space(Some(Navigation(public))), Public);
        reg.commit_navigation(tab, public);
        assert_eq!(space(Some(Document(1))), Public);
        assert_eq!(
            space(Some(Navigation(intranet))),
            Local,
            "late loads keep their own document's"
        );
        assert_eq!(
            space(Some(Navigation(NavigationId::new()))),
            Public,
            "an unknown navigation"
        );
    }

    /// The record is bounded, and the committed navigation outlives the cap.
    #[test]
    fn the_committed_navigation_is_kept_past_the_cap() {
        use crate::engine::types::NavigationId;
        use crate::net::ssrf::AddressSpace::{Local, Public};

        let reg = TabIdentityRegistry::new();
        let tab = TabId::new();
        reg.register(tab, jar());
        let shown = NavigationId::new();
        reg.record_navigation(tab, shown, Local);
        reg.commit_navigation(tab, shown);
        for _ in 0..3 * KEPT_NAVIGATIONS {
            reg.record_navigation(tab, NavigationId::new(), Public);
        }
        let id = reg.get(tab).unwrap();
        assert_eq!(id.navigations.len(), KEPT_NAVIGATIONS);
        assert_eq!(id.document_space(None), Local);
    }

    #[test]
    fn an_unregistered_tab_resolves_to_nothing() {
        // The property the I/O side relies on: no identity means no cookies, not
        // some other tab's cookies.
        let reg = TabIdentityRegistry::new();
        assert!(reg.get(TabId::new()).is_none());

        let tab = TabId::new();
        reg.register(tab, jar());
        reg.remove(tab);
        assert!(reg.get(tab).is_none(), "a closed tab must not resolve");
    }

    #[test]
    fn tabs_keep_their_own_jars() {
        // Ephemeral and custom per-tab jars mean two tabs in one zone can hold
        // different jars; the registry must not collapse them.
        let reg = TabIdentityRegistry::new();
        let (a, b) = (TabId::new(), TabId::new());
        let (jar_a, jar_b) = (jar(), jar());
        reg.register(a, jar_a.clone());
        reg.register(b, jar_b.clone());

        assert!(CookieJarHandle::ptr_eq(&reg.get(a).unwrap().cookie_jar, &jar_a));
        assert!(CookieJarHandle::ptr_eq(&reg.get(b).unwrap().cookie_jar, &jar_b));
        assert!(!CookieJarHandle::ptr_eq(&reg.get(a).unwrap().cookie_jar, &jar_b));
    }
}
