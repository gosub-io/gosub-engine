//! What the I/O side knows about a tab, so it can attach cookies to a request
//! without the tab ever handling them.

use crate::cookies::CookieJarHandle;
use crate::tab::TabId;
use dashmap::DashMap;
use url::Url;

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
        self.tabs.insert(
            tab_id,
            TabIdentity {
                cookie_jar,
                top_level: None,
            },
        );
    }

    /// Point a tab at the document it is now loading. Called before the
    /// navigation request is submitted, so that request is already attributed to
    /// its own URL.
    pub fn set_top_level(&self, tab_id: TabId, url: Url) {
        if let Some(mut entry) = self.tabs.get_mut(&tab_id) {
            entry.top_level = Some(url);
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
