//! The storage service process: a `FileLocalStore` behind the filesystem
//! service filter, Landlock-scoped to its directory. Spawned by the engine,
//! not the zygote (which denies `openat`).

use crate::storage::file_store::FileLocalStore;
use crate::storage::StorageArea as _;
use crate::storage_service::protocol::{FromStorage, ToStorage};
use gosub_ipc::Endpoint;
use std::path::PathBuf;

/// `dir` exists: the broker created it.
pub fn serve(mut link: Endpoint, dir: PathBuf) -> i32 {
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
    while let Ok(msg) = link.recv::<ToStorage>() {
        let reply = match msg {
            ToStorage::Ping => FromStorage::Pong,
            ToStorage::Shutdown => break,
            ToStorage::Get { tag, area, key } => FromStorage::Value {
                tag,
                value: store.area_for(&area.zone, &area.partition, &area.origin).get_item(&key),
            },
            ToStorage::Set { tag, area, key, value } => FromStorage::Done {
                tag,
                error: store
                    .area_for(&area.zone, &area.partition, &area.origin)
                    .set_item(&key, &value)
                    .err()
                    .map(|e| e.to_string()),
            },
            ToStorage::Remove { tag, area, key } => FromStorage::Done {
                tag,
                error: store
                    .area_for(&area.zone, &area.partition, &area.origin)
                    .remove_item(&key)
                    .err()
                    .map(|e| e.to_string()),
            },
            ToStorage::Clear { tag, area } => FromStorage::Done {
                tag,
                error: store
                    .area_for(&area.zone, &area.partition, &area.origin)
                    .clear()
                    .err()
                    .map(|e| e.to_string()),
            },
            ToStorage::Keys { tag, area } => FromStorage::Keys {
                tag,
                keys: store.area_for(&area.zone, &area.partition, &area.origin).keys(),
            },
            ToStorage::Len { tag, area } => FromStorage::Len {
                tag,
                len: store.area_for(&area.zone, &area.partition, &area.origin).len() as u64,
            },
        };
        if link.send(&reply).is_err() {
            break;
        }
    }
    0
}
