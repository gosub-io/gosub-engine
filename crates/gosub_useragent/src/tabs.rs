use slotmap::{DefaultKey, SlotMap};
use url::Url;

use gosub_render_backend::RenderBackend;
use gosub_renderer::draw::SceneDrawer;
use gosub_shared::types::Result;

pub struct Tabs<D: SceneDrawer<B>, B: RenderBackend> {
    pub tabs: SlotMap<DefaultKey, Tab<D, B>>,
    pub active: TabID,
    _marker: std::marker::PhantomData<B>,
}

impl<D: SceneDrawer<B>, B: RenderBackend> Tabs<D, B> {
    pub fn new(initial: Tab<D, B>) -> Self {
        let mut tabs = SlotMap::new();
        let active = TabID(tabs.insert(initial));

        Self {
            tabs,
            active,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn add_tab(&mut self, tab: Tab<D, B>) -> TabID {
        TabID(self.tabs.insert(tab))
    }

    pub fn remove_tab(&mut self, id: TabID) {
        self.tabs.remove(id.0);
    }

    pub fn activate_tab(&mut self, id: TabID) {
        self.active = id;
    }

    pub fn get_current_tab(&mut self) -> Option<&mut Tab<D, B>> {
        self.tabs.get_mut(self.active.0)
    }

    pub(crate) fn from_url(url: Url, debug: bool) -> Result<Self> {
        let tab = Tab::from_url(url, debug)?;

        Ok(Self::new(tab))
    }
}

pub struct Tab<D: SceneDrawer<B>, B: RenderBackend> {
    pub title: String,
    pub url: Url,
    pub data: D,
    _marker: std::marker::PhantomData<B>,
}

impl<D: SceneDrawer<B>, B: RenderBackend> Tab<D, B> {
    pub fn new(title: String, url: Url, data: D) -> Self {
        Self {
            title,
            url,
            data,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn from_url(url: Url, debug: bool) -> Result<Self> {
        let data = D::from_url(url.clone(), debug)?;

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
