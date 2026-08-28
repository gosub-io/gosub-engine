//! Broker side: the service process and the [`LocalStore`] that forwards to it.

use crate::storage::file_store::{partition_name, FileLocalStore};
use crate::storage::{LocalStore, PartitionKey, StorageArea};
use crate::storage_service::protocol::{AreaKey, FromStorage, Tag, ToStorage, STORAGE_ROLE};
use crate::zone::ZoneId;
use anyhow::{anyhow, Result};
use gosub_ipc::Endpoint;
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

const REPLY_TIMEOUT: Duration = Duration::from_secs(10);
const READY_TIMEOUT: Duration = Duration::from_secs(10);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// One link, request/reply, serialized by the mutex.
pub struct StorageProcess {
    link: Mutex<Endpoint>,
    next_tag: AtomicU64,
    child: Mutex<Option<gosub_sandbox::spawn::Child>>,
}

impl std::fmt::Debug for StorageProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageProcess").finish_non_exhaustive()
    }
}

impl StorageProcess {
    /// `dir` must exist: the service cannot create it.
    pub fn spawn(dir: &Path) -> Result<Self> {
        if crate::child_process::is_child_process() {
            anyhow::bail!(
                "this process was started as an engine child role but is running embedder startup, \
                 which means gosub_engine::child_process::dispatch() was not called at the top of \
                 main(); refusing to spawn further processes"
            );
        }
        let dir = std::fs::canonicalize(dir)?;
        let Some(dir_arg) = dir.to_str() else {
            anyhow::bail!("storage directory is not valid UTF-8: {}", dir.display());
        };
        let exe = std::env::current_exe()?;
        let (ours, theirs) = gosub_ipc::channel::Channel::pair()?;
        let child = gosub_sandbox::spawn::spawn(
            &exe,
            // `spawn` appends the primary link after these.
            &[crate::child_process::ROLE_FLAG, STORAGE_ROLE, dir_arg],
            theirs,
            gosub_sandbox::NamespaceIsolation::Full,
            gosub_sandbox::spawn::ContainerProfile {
                name: "gosub-storage",
                internet: false,
                fs_grant: Some((dir.as_path(), true)),
                data_limit: None,
                extra_fds: &[],
            },
        )?;
        if let Err(e) = gosub_sandbox::confine_spawned_child(&child) {
            log::warn!("could not confine the storage service: {e}");
        }
        let mut link = Endpoint::from_channel(ours)?;
        let _ = link.tx.set_write_timeout(Some(REPLY_TIMEOUT));
        let _ = link.rx.set_read_timeout(Some(READY_TIMEOUT));
        link.send(&ToStorage::Ping)?;
        match link.recv::<FromStorage>() {
            Ok(FromStorage::Pong) => {}
            other => {
                let mut child = child;
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!("the spawned process did not answer as a storage service: {other:?}");
            }
        }
        let _ = link.rx.set_read_timeout(Some(REPLY_TIMEOUT));
        Ok(Self {
            link: Mutex::new(link),
            next_tag: AtomicU64::new(1),
            child: Mutex::new(Some(child)),
        })
    }

    fn ask(&self, build: impl FnOnce(Tag) -> ToStorage) -> Result<FromStorage> {
        let tag = self.next_tag.fetch_add(1, Ordering::Relaxed);
        let mut link = self.link.lock();
        link.send(&build(tag))
            .map_err(|e| anyhow!("the storage service is unreachable: {e}"))?;
        let reply = link
            .recv::<FromStorage>()
            .map_err(|e| anyhow!("the storage service did not answer: {e}"))?;
        // One request in flight at a time: a mismatched tag is a broken child.
        let echoed = match &reply {
            FromStorage::Value { tag, .. }
            | FromStorage::Done { tag, .. }
            | FromStorage::Keys { tag, .. }
            | FromStorage::Len { tag, .. } => Some(*tag),
            FromStorage::Pong => None,
        };
        if echoed != Some(tag) {
            anyhow::bail!("the storage service answered out of order");
        }
        Ok(reply)
    }

    fn done(&self, build: impl FnOnce(Tag) -> ToStorage) -> Result<()> {
        match self.ask(build)? {
            FromStorage::Done { error: None, .. } => Ok(()),
            FromStorage::Done { error: Some(e), .. } => Err(anyhow!(e)),
            _ => Err(anyhow!("unexpected reply from the storage service")),
        }
    }

    pub fn shutdown(&self) {
        let _ = self.link.lock().send(&ToStorage::Shutdown);
        let Some(mut child) = self.child.lock().take() else {
            return;
        };
        let deadline = std::time::Instant::now() + SHUTDOWN_GRACE;
        while std::time::Instant::now() < deadline {
            match child.try_wait() {
                Ok(true) => return,
                Ok(false) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => break,
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl Drop for StorageProcess {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// A [`LocalStore`] served by the storage process. Starts it on first use;
/// falls back to a [`FileLocalStore`] on the same directory (with a warning)
/// when it cannot start.
#[derive(Debug)]
pub struct ServiceLocalStore {
    dir: PathBuf,
    backend: OnceLock<Backend>,
}

#[derive(Debug)]
enum Backend {
    Remote(Arc<StorageProcess>),
    Local(FileLocalStore),
}

impl ServiceLocalStore {
    /// Creates `dir` now; the service cannot.
    pub fn new(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            backend: OnceLock::new(),
        })
    }

    /// Whether the service process serves the areas (known after first use).
    pub fn is_remote(&self) -> bool {
        matches!(self.backend.get(), Some(Backend::Remote(_)))
    }

    fn backend(&self) -> &Backend {
        self.backend.get_or_init(|| match StorageProcess::spawn(&self.dir) {
            Ok(process) => {
                log::info!("localStorage is served by a separate, sandboxed storage process");
                Backend::Remote(Arc::new(process))
            }
            Err(e) => {
                log::warn!("the storage service could not start ({e}); localStorage stays in-process");
                Backend::Local(FileLocalStore::attach(&self.dir))
            }
        })
    }

    /// Areas handed out before fail on their next request.
    pub fn shutdown(&self) {
        if let Some(Backend::Remote(process)) = self.backend.get() {
            process.shutdown();
        }
    }
}

impl LocalStore for ServiceLocalStore {
    fn area(&self, zone: ZoneId, part: &PartitionKey, origin: &url::Origin) -> Result<Arc<dyn StorageArea>> {
        match self.backend() {
            Backend::Local(store) => store.area(zone, part, origin),
            Backend::Remote(process) => Ok(Arc::new(RemoteArea {
                process: Arc::clone(process),
                area: AreaKey {
                    zone: zone.to_string(),
                    partition: partition_name(part),
                    origin: origin.ascii_serialization(),
                },
            })),
        }
    }
}

struct RemoteArea {
    process: Arc<StorageProcess>,
    area: AreaKey,
}

impl StorageArea for RemoteArea {
    fn get_item(&self, key: &str) -> Option<String> {
        match self.process.ask(|tag| ToStorage::Get {
            tag,
            area: self.area.clone(),
            key: key.to_string(),
        }) {
            Ok(FromStorage::Value { value, .. }) => value,
            _ => None,
        }
    }

    fn set_item(&self, key: &str, value: &str) -> Result<()> {
        self.process.done(|tag| ToStorage::Set {
            tag,
            area: self.area.clone(),
            key: key.to_string(),
            value: value.to_string(),
        })
    }

    fn remove_item(&self, key: &str) -> Result<()> {
        self.process.done(|tag| ToStorage::Remove {
            tag,
            area: self.area.clone(),
            key: key.to_string(),
        })
    }

    fn clear(&self) -> Result<()> {
        self.process.done(|tag| ToStorage::Clear {
            tag,
            area: self.area.clone(),
        })
    }

    fn len(&self) -> usize {
        match self.process.ask(|tag| ToStorage::Len {
            tag,
            area: self.area.clone(),
        }) {
            Ok(FromStorage::Len { len, .. }) => len as usize,
            _ => 0,
        }
    }

    fn keys(&self) -> Vec<String> {
        match self.process.ask(|tag| ToStorage::Keys {
            tag,
            area: self.area.clone(),
        }) {
            Ok(FromStorage::Keys { keys, .. }) => keys,
            _ => Vec::new(),
        }
    }
}
