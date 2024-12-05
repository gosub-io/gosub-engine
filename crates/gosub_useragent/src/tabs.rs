use gosub_shared::render_backend::layout::LayoutTree;
use gosub_shared::render_backend::{NodeDesc, WindowedEventLoop};
use gosub_shared::traits::config::ModuleConfiguration;
use gosub_shared::traits::draw::TreeDrawer;
use gosub_shared::types::Result;
use slotmap::{DefaultKey, SlotMap};
use std::sync::mpsc::Sender;
use url::Url;

pub struct Tabs<C: ModuleConfiguration> {
    #[allow(clippy::type_complexity)]
    pub tabs: SlotMap<DefaultKey, Tab<C>>,
    pub active: TabID,
}

impl<C: ModuleConfiguration> Default for Tabs<C> {
    fn default() -> Self {
        Self {
            tabs: SlotMap::new(),
            active: TabID::default(),
        }
    }
}

impl<C: ModuleConfiguration> Tabs<C> {
    pub fn new(initial: Tab<C>) -> Self {
        let mut tabs = SlotMap::new();
        let active = TabID(tabs.insert(initial));

        Self { tabs, active }
    }

    pub fn add_tab(&mut self, tab: Tab<C>) -> TabID {
        TabID(self.tabs.insert(tab))
    }

    pub fn remove_tab(&mut self, id: TabID) {
        self.tabs.remove(id.0);
    }

    pub fn activate_tab(&mut self, id: TabID) {
        self.active = id;
    }

    pub fn get_current_tab(&mut self) -> Option<&mut Tab<C>> {
        self.tabs.get_mut(self.active.0)
    }

    #[allow(unused)]
    pub(crate) async fn from_url(url: Url, layouter: C::Layouter, debug: bool) -> Result<Self> {
        let tab = Tab::from_url(url, layouter, debug).await?;
        Ok(Self::new(tab))
    }

    pub fn select_element(&mut self, id: <C::LayoutTree as LayoutTree<C>>::NodeId) {
        if let Some(tab) = self.get_current_tab() {
            tab.data.select_element(id);
        }
    }

    pub fn info(&mut self, id: <C::LayoutTree as LayoutTree<C>>::NodeId, sender: Sender<NodeDesc>) {
        if let Some(tab) = self.get_current_tab() {
            tab.data.info(id, sender);
        }
    }

    pub fn send_nodes(&mut self, sender: Sender<NodeDesc>) {
        if let Some(tab) = self.get_current_tab() {
            tab.data.send_nodes(sender);
        }
    }

    pub fn unselect_element(&mut self) {
        if let Some(tab) = self.get_current_tab() {
            tab.data.unselect_element();
        }
    }

    pub fn next_tab(&mut self) {
        if let Some(next_tab_id) = self.tabs.keys().skip_while(|key| *key != self.active.0).nth(1) {
            self.active = TabID(next_tab_id);
        }
    }

    pub fn previous_tab(&mut self) {
        if let Some(prev_tab_id) = self
            .tabs
            .keys()
            .collect::<Vec<_>>()
            .iter()
            .rev()
            .skip_while(|key| **key != self.active.0)
            .nth(1)
        {
            self.active = TabID(*prev_tab_id);
        }
    }

    pub fn activate_idx(&mut self, idx: usize) {
        if let Some(id) = self.tabs.keys().nth(idx) {
            self.active = TabID(id);
        }
    }
}

#[derive(Debug)]
pub struct Tab<C: ModuleConfiguration> {
    pub title: String,
    pub url: Url,
    pub data: C::TreeDrawer,
}

impl<C: ModuleConfiguration> Tab<C> {
    pub fn new(title: String, url: Url, data: C::TreeDrawer) -> Self {
        Self { title, url, data }
    }

    pub async fn from_url(url: Url, layouter: C::Layouter, debug: bool) -> Result<Self> {
        let data = C::TreeDrawer::from_url(url.clone(), layouter, debug).await?;

        Ok(Self {
            title: url.as_str().to_string(),
            url,
            data,
        })
    }

    pub fn reload(&mut self, el: impl WindowedEventLoop<C>) {
        self.data.reload(el);
    }

    pub fn reload_from(&mut self, rt: C::RenderTree) {
        self.data.reload_from(rt)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TabID(pub(crate) DefaultKey);
