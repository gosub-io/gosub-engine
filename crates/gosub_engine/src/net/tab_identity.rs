//! What the I/O side knows about a tab, so it can attach cookies to a request
//! without the tab ever handling them.

use crate::cookies::CookieJarHandle;
use crate::engine::cookies::{hop_context, SameSiteContext};
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

/// How many of a tab's request references keep the document their loads are
/// for: the shown page's, a loading one's, and a renderer's brokered loaders.
const KEPT_DOCUMENTS: usize = 32;

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
    /// Who started each recent network navigation, newest last: the document
    /// whose link or form it was, or `None` for the user's own (address bar,
    /// reload, history). A navigation's hops are judged against it for
    /// `SameSite`, not against `top_level`, which is already the target.
    pub initiators: VecDeque<(NavigationId, Option<Url>)>,
    /// The document each recent reference's loads are made for, newest last,
    /// as the engine stamped it on them (their referrer). A page keeps loading
    /// after a navigation away from it has started, and `top_level` is then
    /// already the target: its late loads are judged against it here instead.
    pub documents: VecDeque<(RequestReference, Url)>,
}

impl TabIdentity {
    pub fn new(cookie_jar: CookieJarHandle) -> Self {
        Self {
            cookie_jar,
            top_level: None,
            navigations: VecDeque::new(),
            committed: None,
            initiators: VecDeque::new(),
            documents: VecDeque::new(),
        }
    }

    /// Whether a request under `navigation` is that navigation's own (one of
    /// its hops) rather than a load of the document it produced, and if so
    /// who started it. True from the start of a network navigation until its
    /// response is recorded, which drops its entry here before the document
    /// can ask for anything (see [`TabIdentityRegistry::record_navigation`]);
    /// a document loaded without the network never had an entry.
    fn navigating(&self, navigation: NavigationId) -> Option<Option<&Url>> {
        self.initiators
            .iter()
            .rev()
            .find(|(n, _)| *n == navigation)
            .map(|(_, initiator)| initiator.as_ref())
    }

    /// The `SameSite` context one hop of a request with `reference` is judged
    /// in. A navigation's hops are judged against the document that started
    /// it, so a link from another site is a cross-site navigation and gets no
    /// `Strict` cookies; the user's own is judged against its first URL, same-
    /// site until a redirect leaves. Anything else is a load of a document,
    /// judged against it as a subrequest (see [`Self::cookie_document`]).
    pub(crate) fn cookie_context(
        &self,
        reference: Option<RequestReference>,
        document: Option<&Url>,
        url: &Url,
        url_list: &[Url],
        method: &http::Method,
    ) -> SameSiteContext {
        match self.navigating_reference(reference) {
            Some(initiator) => {
                let site = initiator.unwrap_or_else(|| url_list.first().unwrap_or(url));
                hop_context(Some(site), url, url_list, method, true)
            }
            None => hop_context(self.cookie_document(reference, document), url, url_list, method, false),
        }
    }

    /// The top-level document a request with `reference` is made in, for the
    /// jar's third-party checks and as what a load is judged against: a
    /// navigation's own target (`top_level`); a load's `document`, the one the
    /// engine stamped on it, when the caller has the request; else the one
    /// recorded for its reference; else the tab's. Never the target of a
    /// navigation the page that asked is being left for.
    pub(crate) fn cookie_document<'a>(
        &'a self,
        reference: Option<RequestReference>,
        document: Option<&'a Url>,
    ) -> Option<&'a Url> {
        if self.navigating_reference(reference).is_some() {
            return self.top_level.as_ref();
        }
        document
            .or_else(|| reference.and_then(|r| self.document_of(r)))
            .or(self.top_level.as_ref())
    }

    /// What decides [`Self::cookie_context`] for `reference` besides the URL
    /// and method: two requests whose keys match get the same cookies.
    pub(crate) fn cookie_context_key(&self, reference: Option<RequestReference>) -> String {
        match self.navigating_reference(reference) {
            Some(initiator) => format!("navigation from {}", initiator.map_or("the user", |u| u.as_str())),
            None => format!(
                "in {}",
                self.cookie_document(reference, None).map_or("", |u| u.as_str())
            ),
        }
    }

    fn navigating_reference(&self, reference: Option<RequestReference>) -> Option<Option<&Url>> {
        match reference {
            Some(RequestReference::Navigation(id)) => self.navigating(id),
            _ => None,
        }
    }

    /// The document recorded for `reference`'s loads, if any.
    fn document_of(&self, reference: RequestReference) -> Option<&Url> {
        self.documents
            .iter()
            .rev()
            .find(|(r, _)| *r == reference)
            .map(|(_, document)| document)
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
        // Its response is in: from here a request under it is the document's.
        entry.initiators.retain(|(n, _)| *n != navigation);
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

    /// Record who started a network navigation, before its request is
    /// submitted: the document whose link or form it was, or `None` for the
    /// user's own.
    pub fn start_navigation(&self, tab_id: TabId, navigation: NavigationId, initiator: Option<Url>) {
        let Some(mut entry) = self.tabs.get_mut(&tab_id) else {
            return;
        };
        entry.initiators.push_back((navigation, initiator));
        while entry.initiators.len() > KEPT_NAVIGATIONS {
            entry.initiators.pop_front();
        }
    }

    /// Record the document a load under `reference` was made for, as the
    /// engine stamped it, before the load is sent: the fetcher's cookie hook
    /// sees only the reference.
    pub fn note_document(&self, tab_id: TabId, reference: RequestReference, document: Url) {
        let Some(mut entry) = self.tabs.get_mut(&tab_id) else {
            return;
        };
        if entry.document_of(reference) == Some(&document) {
            return;
        }
        entry.documents.retain(|(r, _)| *r != reference);
        entry.documents.push_back((reference, document));
        while entry.documents.len() > KEPT_DOCUMENTS {
            entry.documents.pop_front();
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

    /// A tab navigating to `target`, started by `initiator` (`None`: the user).
    fn navigating_to(target: &str, initiator: Option<&str>) -> (TabIdentityRegistry, TabId, NavigationId) {
        let reg = TabIdentityRegistry::new();
        let tab = TabId::new();
        let nav = NavigationId::new();
        reg.register(tab, jar());
        reg.set_top_level(tab, Url::parse(target).unwrap());
        reg.start_navigation(tab, nav, initiator.map(|u| Url::parse(u).unwrap()));
        (reg, tab, nav)
    }

    fn context(
        reg: &TabIdentityRegistry,
        tab: TabId,
        reference: RequestReference,
        url: &str,
        method: http::Method,
    ) -> SameSiteContext {
        reg.get(tab)
            .unwrap()
            .cookie_context(Some(reference), None, &Url::parse(url).unwrap(), &[], &method)
    }

    #[test]
    fn a_link_from_another_site_is_a_cross_site_navigation() {
        let (reg, tab, nav) = navigating_to("https://bank.test/", Some("https://evil.test/page"));
        let nav = RequestReference::Navigation(nav);
        assert_eq!(
            context(&reg, tab, nav, "https://bank.test/", http::Method::GET),
            SameSiteContext::CrossSiteNavigation
        );
        assert_eq!(
            context(&reg, tab, nav, "https://bank.test/", http::Method::POST),
            SameSiteContext::CrossSite
        );
    }

    #[test]
    fn a_link_within_the_site_and_the_users_own_navigation_are_same_site() {
        let (reg, tab, nav) = navigating_to("https://bank.test/a", Some("https://www.bank.test/"));
        let same = context(
            &reg,
            tab,
            RequestReference::Navigation(nav),
            "https://bank.test/a",
            http::Method::POST,
        );
        assert_eq!(same, SameSiteContext::SameSite);
        let (reg, tab, nav) = navigating_to("https://bank.test/", None);
        let own = context(
            &reg,
            tab,
            RequestReference::Navigation(nav),
            "https://bank.test/",
            http::Method::GET,
        );
        assert_eq!(own, SameSiteContext::SameSite);
    }

    #[test]
    fn the_users_own_navigation_is_cross_site_once_a_redirect_leaves() {
        let (reg, tab, nav) = navigating_to("https://start.test/", None);
        let hop = reg.get(tab).unwrap().cookie_context(
            Some(RequestReference::Navigation(nav)),
            None,
            &Url::parse("https://bank.test/").unwrap(),
            &[Url::parse("https://start.test/").unwrap()],
            &http::Method::GET,
        );
        assert_eq!(hop, SameSiteContext::CrossSiteNavigation);
    }

    /// The document's loads share the navigation's reference; once its
    /// response is in they are judged against the document as subrequests.
    #[test]
    fn the_documents_loads_are_subrequests_once_the_response_is_in() {
        let (reg, tab, nav) = navigating_to("https://bank.test/", Some("https://evil.test/"));
        reg.record_navigation(tab, nav, AddressSpace::Public);
        let reference = RequestReference::Navigation(nav);
        assert_eq!(
            context(&reg, tab, reference, "https://bank.test/style.css", http::Method::GET),
            SameSiteContext::SameSite
        );
        assert_eq!(
            context(&reg, tab, reference, "https://other.test/pixel.png", http::Method::GET),
            SameSiteContext::CrossSite
        );
    }

    /// A document loaded without the network (`LoadHtml`, an internal page)
    /// never started a navigation request: its loads are subrequests.
    #[test]
    fn a_local_documents_loads_are_subrequests() {
        let reg = TabIdentityRegistry::new();
        let tab = TabId::new();
        reg.register(tab, jar());
        reg.set_top_level(tab, Url::parse("https://local.test/").unwrap());
        let reference = RequestReference::Navigation(NavigationId::new());
        assert_eq!(
            context(&reg, tab, reference, "https://other.test/pixel.png", http::Method::GET),
            SameSiteContext::CrossSite
        );
    }

    /// A page being left still loads after the tab's top level moved to the
    /// target; its loads are judged against the page, recorded per reference.
    #[test]
    fn a_left_pages_loads_are_judged_against_the_page() {
        let (reg, tab, _) = navigating_to("https://bank.test/", None);
        let old = RequestReference::Document(7);
        // Unrecorded, a load falls back to the tab's top level.
        assert_eq!(
            context(&reg, tab, old, "https://bank.test/img.png", http::Method::GET),
            SameSiteContext::SameSite
        );
        reg.note_document(tab, old, Url::parse("https://evil.test/").unwrap());
        assert_eq!(
            context(&reg, tab, old, "https://bank.test/img.png", http::Method::GET),
            SameSiteContext::CrossSite
        );
        let identity = reg.get(tab).unwrap();
        assert_eq!(
            identity.cookie_document(Some(old), None).map(Url::as_str),
            Some("https://evil.test/")
        );
        // The navigation's own hops still take the target as their top level.
        let nav = identity.initiators[0].0;
        assert_eq!(
            identity
                .cookie_document(Some(RequestReference::Navigation(nav)), None)
                .map(Url::as_str),
            Some("https://bank.test/")
        );
    }

    /// A document a redirect moved is judged by where it ended up, as its
    /// loads say, not by the URL the navigation set out for.
    #[test]
    fn a_redirected_documents_loads_are_judged_against_where_it_ended_up() {
        let (reg, tab, nav) = navigating_to("https://start.test/", None);
        reg.record_navigation(tab, nav, AddressSpace::Public);
        let reference = RequestReference::Navigation(nav);
        reg.note_document(tab, reference, Url::parse("https://bank.test/home").unwrap());
        assert_eq!(
            context(&reg, tab, reference, "https://bank.test/style.css", http::Method::GET),
            SameSiteContext::SameSite
        );
        assert_eq!(
            context(&reg, tab, reference, "https://start.test/pixel.png", http::Method::GET),
            SameSiteContext::CrossSite
        );
    }

    #[test]
    fn documents_kept_are_bounded_and_newest_wins() {
        let reg = TabIdentityRegistry::new();
        let tab = TabId::new();
        reg.register(tab, jar());
        for n in 0..(KEPT_DOCUMENTS as u64 + 10) {
            reg.note_document(
                tab,
                RequestReference::Document(n),
                Url::parse("https://a.test/").unwrap(),
            );
        }
        reg.note_document(
            tab,
            RequestReference::Document(20),
            Url::parse("https://b.test/").unwrap(),
        );
        let identity = reg.get(tab).unwrap();
        assert_eq!(identity.documents.len(), KEPT_DOCUMENTS);
        assert_eq!(
            identity.document_of(RequestReference::Document(20)).map(Url::as_str),
            Some("https://b.test/")
        );
        assert_eq!(identity.document_of(RequestReference::Document(0)), None);
    }
}
