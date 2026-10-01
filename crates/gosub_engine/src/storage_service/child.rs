//! The storage service process: a `FileLocalStore` behind the filesystem
//! service filter, Landlock-scoped to its directory. Spawned by the engine,
//! not the zygote (which denies `openat`).

use crate::storage::file_store::FileLocalStore;
use crate::storage::StorageArea as _;
use crate::storage_service::protocol::{AreaKey, FromStorage, ToStorage};
use gosub_ipc::Endpoint;
use std::path::PathBuf;

/// `dir` exists: the broker created it.
pub fn serve(mut link: Endpoint, dir: PathBuf) -> i32 {
    gosub_sandbox::capture_process_title_region();
    gosub_sandbox::set_process_title("gosub-storage", "gosub: storage service");
    gosub_sandbox::lock_down_service(
        "storage",
        gosub_sandbox::ServiceCaps {
            filesystem: true,
            device: false,
        },
        &[(dir.as_path(), true)],
    );

    let store = FileLocalStore::attach(dir);
    // An area whose file cannot be loaded reads as empty (the read calls have
    // no error to give) and refuses every write, so its file is left alone.
    let area = |a: &AreaKey| {
        store
            .area_for(&a.zone, &a.partition, &a.origin)
            .inspect_err(|e| eprintln!("[storage] {e}"))
    };
    let done = |tag, result: anyhow::Result<()>| FromStorage::Done {
        tag,
        error: result.err().map(|e| e.to_string()),
    };
    while let Ok(msg) = link.recv::<ToStorage>() {
        let reply = match msg {
            ToStorage::Ping => FromStorage::Pong,
            ToStorage::Shutdown => break,
            ToStorage::Get { tag, area: a, key } => FromStorage::Value {
                tag,
                value: area(&a).ok().and_then(|area| area.get_item(&key)),
            },
            ToStorage::Set {
                tag,
                area: a,
                key,
                value,
            } => done(tag, area(&a).and_then(|area| area.set_item(&key, &value))),
            ToStorage::Remove { tag, area: a, key } => done(tag, area(&a).and_then(|area| area.remove_item(&key))),
            ToStorage::Clear { tag, area: a } => done(tag, area(&a).and_then(|area| area.clear())),
            ToStorage::Keys { tag, area: a } => FromStorage::Keys {
                tag,
                keys: area(&a).map(|area| area.keys()).unwrap_or_default(),
            },
            ToStorage::Len { tag, area: a } => FromStorage::Len {
                tag,
                len: area(&a).map(|area| area.len() as u64).unwrap_or(0),
            },
        };
        if link.send(&reply).is_err() {
            break;
        }
    }
    0
}
