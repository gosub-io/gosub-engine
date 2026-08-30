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

/// One link, request/reply, serialized by the mutex. A process that died
/// (its state is all on disk) is respawned on the next request, which is
/// then retried once.
pub struct StorageProcess {
    link: Mutex<Endpoint>,
    next_tag: AtomicU64,
    child: Mutex<Option<gosub_sandbox::spawn::Child>>,
    dir: PathBuf,
    last_respawn: Mutex<Option<std::time::Instant>>,
    closed: std::sync::atomic::AtomicBool,
}

const RESPAWN_COOLDOWN: Duration = Duration::from_secs(5);

impl std::fmt::Debug for StorageProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageProcess").finish_non_exhaustive()
    }
}

impl StorageProcess {
    /// `dir` must exist: the service cannot create it.
    pub fn spawn(dir: &Path) -> Result<Self> {
        let dir = std::fs::canonicalize(dir)?;
        let (link, child) = Self::launch(&dir)?;
        Ok(Self {
            link: Mutex::new(link),
            next_tag: AtomicU64::new(1),
            child: Mutex::new(Some(child)),
            dir,
            last_respawn: Mutex::new(None),
            closed: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// The service process's pid, while it has one.
    pub fn pid(&self) -> Option<u32> {
        self.child.lock().as_ref().map(gosub_sandbox::spawn::Child::id)
    }

    /// Re-exec this binary as the service on `dir` and confirm it answers.
    fn launch(dir: &Path) -> Result<(Endpoint, gosub_sandbox::spawn::Child)> {
        if crate::child_process::is_child_process() {
            anyhow::bail!(
                "this process was started as an engine child role but is running embedder startup, \
                 which means gosub_engine::child_process::dispatch() was not called at the top of \
                 main(); refusing to spawn further processes"
            );
        }
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
                fs_grant: Some((dir, true)),
                data_limit: None,
                extra_fds: &[],
                max_tasks: 64,
                // Whole-file rewrites of areas: a few of them at most.
                file_size_limit: Some(4 * crate::storage::file_store::MAX_AREA_BYTES as u64),
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
        Ok((link, child))
    }

    /// A dead service comes back for the next request, at most once per
    /// cooldown; `false` when it could not.
    fn respawn(&self, link: &mut Endpoint) -> bool {
        if self.closed.load(Ordering::Acquire) {
            return false;
        }
        let mut last = self.last_respawn.lock();
        if last.is_some_and(|at| at.elapsed() < RESPAWN_COOLDOWN) {
            return false;
        }
        *last = Some(std::time::Instant::now());
        log::warn!("the storage service died; respawning it");
        if let Some(mut child) = self.child.lock().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        match Self::launch(&self.dir) {
            Ok((fresh, child)) => {
                *link = fresh;
                *self.child.lock() = Some(child);
                log::info!("the storage service is back");
                true
            }
            Err(e) => {
                log::error!("the storage service could not be respawned ({e}); localStorage requests fail");
                false
            }
        }
    }

    fn ask(&self, build: impl FnOnce(Tag) -> ToStorage) -> Result<FromStorage> {
        let tag = self.next_tag.fetch_add(1, Ordering::Relaxed);
        let msg = build(tag);
        let mut link = self.link.lock();
        let exchange = |link: &mut Endpoint| -> Result<FromStorage> {
            link.send(&msg)
                .map_err(|e| anyhow!("the storage service is unreachable: {e}"))?;
            link.recv::<FromStorage>()
                .map_err(|e| anyhow!("the storage service did not answer: {e}"))
        };
        let reply = match exchange(&mut link) {
            Ok(reply) => reply,
            // Once more through a fresh process; a timeout of a live one is
            // not that (the link is intact and the process still there).
            Err(e) if !link.rx.peer_alive() && self.respawn(&mut link) => exchange(&mut link).map_err(|_| e)?,
            Err(e) => return Err(e),
        };
        // One request in flight at a time: a mismatched tag is a broken child.
        let echoed = match &reply {
            FromStorage::Value { tag, .. }
            | FromStorage::Done { tag, .. }
            | FromStorage::Keys { tag, .. }
            | FromStorage::Len { tag, .. }
            | FromStorage::Audit { tag, .. } => Some(*tag),
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

    /// The escape audit, run inside the service process.
    pub fn audit(&self) -> Result<gosub_sandbox::audit::AuditReport> {
        match self.ask(|tag| ToStorage::Audit { tag })? {
            FromStorage::Audit { report, .. } => Ok(report),
            _ => Err(anyhow!("unexpected reply to the audit")),
        }
    }

    pub fn shutdown(&self) {
        self.closed.store(true, Ordering::Release);
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

    /// The service process's pid, when one serves the areas.
    pub fn pid(&self) -> Option<u32> {
        match self.backend.get() {
            Some(Backend::Remote(process)) => process.pid(),
            _ => None,
        }
    }

    /// Areas handed out before fail on their next request.
    pub fn shutdown(&self) {
        if let Some(Backend::Remote(process)) = self.backend.get() {
            process.shutdown();
        }
    }
}

impl LocalStore for ServiceLocalStore {
    fn service_directory(&self) -> Option<PathBuf> {
        Some(self.dir.clone())
    }

    fn service_pid(&self) -> Option<u32> {
        self.pid()
    }

    fn escape_audit(&self) -> Option<gosub_sandbox::audit::AuditReport> {
        match self.backend.get() {
            Some(Backend::Remote(process)) => process.audit().ok(),
            _ => None,
        }
    }

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
