use gosub_instance::{DebugEvent, EngineInstance, InstanceHandle, InstanceMessage};
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::instance::{Handles, InstanceId};
use gosub_shared::types::Result;
use slotmap::{DefaultKey, Key, KeyData, SlotMap};
use url::Url;

pub struct Tabs {
    #[allow(clippy::type_complexity)]
    pub tabs: SlotMap<DefaultKey, InstanceHandle>,
    pub active: InstanceId,
}

impl Default for Tabs {
    fn default() -> Self {
        Self {
            tabs: SlotMap::new(),
            active: InstanceId(DefaultKey::null().data().as_ffi()),
        }
    }
}

impl Tabs {
    pub fn new(initial: InstanceHandle) -> Self {
        let mut tabs = SlotMap::new();

        let active = kti(tabs.insert(initial));

        Self { tabs, active }
    }

    pub fn remove_tab(&mut self, id: InstanceId) -> Option<InstanceHandle> {
        self.tabs.remove(itk(id))
    }

    pub fn activate_tab(&mut self, id: InstanceId) {
        self.active = id;
    }

    pub fn get_current_tab(&mut self) -> Option<&mut InstanceHandle> {
        self.tabs.get_mut(itk(self.active))
    }

    #[allow(unused)]
    pub(crate) fn from_url<C: ModuleConfiguration>(
        url: Url,
        layouter: C::Layouter,
        handles: Handles<C>,
    ) -> Result<Self> {
        let mut tabs = SlotMap::new();

        let id = tabs.try_insert_with_key(|key| EngineInstance::new_on_thread(url, layouter, kti(key), handles))?;

        Ok(Self { tabs, active: kti(id) })
    }

    pub fn open<C: ModuleConfiguration>(&mut self, url: Url, layouter: C::Layouter, handles: Handles<C>) -> Result<()> {
        let id = self
            .tabs
            .try_insert_with_key(|key| EngineInstance::new_on_thread(url.clone(), layouter, kti(key), handles))?;

        self.active = kti(id);

        Ok(())
    }

    pub fn debug_event(&self, id: InstanceId, event: DebugEvent) -> Result<()> {
        if let Some(tab) = self.tabs.get(itk(id)) {
            tab.tx.blocking_send(InstanceMessage::Debug(event))?;
        }

        Ok(())
    }

    pub fn next_tab(&mut self) {
        let active = itk(self.active);
        if let Some(next_tab_id) = self.tabs.keys().skip_while(|key| *key != active).nth(1) {
            self.active = kti(next_tab_id);
        }
    }

    pub fn previous_tab(&mut self) {
        let active = itk(self.active);
        if let Some(prev_tab_id) = self
            .tabs
            .keys()
            .collect::<Vec<_>>()
            .iter()
            .rev()
            .skip_while(|key| **key != active)
            .nth(1)
        {
            self.active = kti(*prev_tab_id);
        }
    }

    pub fn activate_idx(&mut self, idx: usize) {
        if let Some(id) = self.tabs.keys().nth(idx) {
            self.active = kti(id);
        }
    }

    pub fn is_active(&self, id: InstanceId) -> bool {
        self.active == id
    }
}

/// DefaultKey to InstanceID
fn kti(key: DefaultKey) -> InstanceId {
    InstanceId(key.data().as_ffi())
}

/// InstanceID to DefaultKey
fn itk(id: InstanceId) -> DefaultKey {
    DefaultKey::from(KeyData::from_ffi(id.0))
}
