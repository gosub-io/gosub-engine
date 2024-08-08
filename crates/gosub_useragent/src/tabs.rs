use slotmap::{DefaultKey, SlotMap};
use std::sync::mpsc::Sender;
use url::Url;

use gosub_render_backend::layout::{LayoutTree, Layouter};
use gosub_render_backend::{NodeDesc, RenderBackend};
use gosub_renderer::draw::SceneDrawer;
use gosub_shared::types::Result;

pub struct Tabs<D: SceneDrawer<B, L, LT>, B: RenderBackend, L: Layouter, LT: LayoutTree<L>> {
    pub tabs: SlotMap<DefaultKey, Tab<D, B, L, LT>>,
    pub active: TabID,
    _marker: std::marker::PhantomData<(B, L, LT)>,
}

impl<D: SceneDrawer<B, L, LT>, L: Layouter, LT: LayoutTree<L>, B: RenderBackend> Tabs<D, B, L, LT> {
    pub fn new(initial: Tab<D, B, L, LT>) -> Self {
        let mut tabs = SlotMap::new();
        let active = TabID(tabs.insert(initial));

        Self {
            tabs,
            active,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn add_tab(&mut self, tab: Tab<D, B, L, LT>) -> TabID {
        TabID(self.tabs.insert(tab))
    }

    pub fn remove_tab(&mut self, id: TabID) {
        self.tabs.remove(id.0);
    }

    pub fn activate_tab(&mut self, id: TabID) {
        self.active = id;
    }

    pub fn get_current_tab(&mut self) -> Option<&mut Tab<D, B, L, LT>> {
        self.tabs.get_mut(self.active.0)
    }

    pub(crate) fn from_url(url: Url, layouter: L, debug: bool) -> Result<Self> {
        let tab = Tab::from_url(url, layouter, debug)?;

        Ok(Self::new(tab))
    }

    pub fn select_element(&mut self, id: LT::NodeId) {
        if let Some(tab) = self.get_current_tab() {
            tab.data.select_element(id);
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
}

pub struct Tab<D: SceneDrawer<B, L, LT>, B: RenderBackend, L: Layouter, LT: LayoutTree<L>> {
    pub title: String,
    pub url: Url,
    pub data: D,
    _marker: std::marker::PhantomData<(B, L, LT)>,
}

impl<D: SceneDrawer<B, L, LT>, B: RenderBackend, L: Layouter, LT: LayoutTree<L>> Tab<D, B, L, LT> {
    pub fn new(title: String, url: Url, data: D) -> Self {
        Self {
            title,
            url,
            data,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn from_url(url: Url, layouter: L, debug: bool) -> Result<Self> {
        let data = D::from_url(url.clone(), layouter, debug)?;

        Ok(Self {
            title: url.as_str().to_string(),
            url,
            data,
            _marker: std::marker::PhantomData,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabID(pub(crate) DefaultKey);
