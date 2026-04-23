use crate::engine::types::PeekBuf;
use crate::net::emitter::NetObserver;
use crate::net::events::NetEvent;
use crate::net::fs_utils::temp_path_for;
use crate::net::types::NetError;
use crate::net::SharedBody;
use bytes::BytesMut;
use std::sync::Arc;
use std::{path::PathBuf, time::Instant};
use tokio::fs::OpenOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::task::JoinHandle;
use tokio::{
    io::AsyncRead,
    time::{sleep, timeout},
};
use tokio_util::sync::CancellationToken;
use url::Url;

/// Configuration for a single pump run.
///
/// - `idle`: Maximum allowed gap between successful reads. If no bytes are
///   read within this duration, the pump reports an idle timeout.
/// - `total_deadline`: Absolute wall-clock deadline for the *entire* transfer.
///   If `Some(instant)` and `instant` is reached, the pump reports a total timeout.
///
/// The pump checks `total_deadline` on each loop tick and via a dedicated
/// timer branch, so long stalls are still caught.
pub struct PumpCfg {
    /// Idle timeout
    pub idle: std::time::Duration,
    /// Total timeout
    pub total_deadline: Option<Instant>,
}

/// Destinations for a pump run.
///
/// At least one of `shared` or `file_dest` should be `Some(..)`. If both are
/// provided, the pump tees bytes to both concurrently. The optional `peek` is
/// emitted *before* streaming the tail.
///
/// - `shared`: Fan-out stream target; slow subscribers may be dropped by
///   [`SharedBody`] if their per-subscriber queue fills.
/// - `file_dest`: Final file path to write to. The pump writes to a temporary
///   path (`temp_path_for`) and renames on success (atomic on most platforms).
/// - `peek`: Initial bytes already read (“sniffed”) from the source that must
///   be replayed to targets to preserve the full body.
pub struct PumpTargets {
    // Shared body
    pub shared: Option<Arc<SharedBody>>,
    // Optional file destination
    pub file_dest: Option<PathBuf>,
    // peek buffer we need to send first
    pub peek_buf: PeekBuf,
}

/// Pumps bytes from an `AsyncRead` into one or both targets:
/// a fan-out [`SharedBody`] and/or a file on disk.
///
/// The pump enforces:
/// - **Idle timeout**: no bytes read within `idle` → `NetError::Timeout("Pump idle timeout")`.
/// - **Total deadline**: wall-clock deadline via `total_deadline` → `NetError::Timeout("Pump total timeout")`.
/// - **Cancellation**: cooperative cancellation via [`CancellationToken`] → `NetError::Cancelled("Pump cancelled")`.
///
/// If a file destination is provided, the pump writes to a **temporary path**
/// (via `temp_path_for`) and **atomically renames** to the final destination
/// *only if* the transfer finishes cleanly. On read errors, timeouts, or
/// cancellation, the temporary file is left in place (caller may clean up).
///
/// `peek` is emitted *first* to both targets (if present), then the streamed tail.
///
/// # Return
/// The task resolves to:
/// - `Ok(Some(final_path))` on a clean EOF and successful rename,
/// - `Ok(None)` if no file target was requested or the transfer did not finish cleanly,
/// - `Err(NetError)` only for early I/O failures before the main loop opens/writes the temp file.
///
/// # Example
/// ```ignore
/// # use std::{sync::Arc, time::Duration};
/// # use tokio::io::AsyncRead;
/// # use tokio_util::sync::CancellationToken;
/// # use url::Url;
/// # use gosub_engine_api::net::{SharedBody, types::NetError};
/// # use gosub_engine_api::net::events::NetObserver;
/// # async fn get_reader() -> impl AsyncRead + Unpin + Send + 'static { tokio::io::empty() }
/// # struct Obs; impl NetObserver for Obs {
/// #   fn on_event(&self, _e: gosub_engine_api::net::events::NetEvent) {}
/// # }
/// use gosub_engine_api::net::pump::{spawn_pump, PumpCfg, PumpTargets};
///
/// let reader = get_reader().await;
/// let shared = Arc::new(SharedBody::new(32));
/// let cancel = CancellationToken::new();
///
/// let cfg = PumpCfg {
///     idle: Duration::from_secs(10),
///     total_deadline: None,
/// };
///
/// let targets = PumpTargets {
///     shared: Some(shared.clone()),
///     file_dest: None,            // or Some("/path/to/file".into())
///     peek_buf: b"HTTP-HEAD".to_vec() // optional initial bytes to emit
/// };
///
/// let handle = spawn_pump(
///     reader,
///     targets,
///     cfg,
///     cancel.clone(),
///     Arc::new(Obs),
///     Url::parse("https://example.test").unwrap(),
/// );
///
/// let result = handle.await.unwrap();
/// assert!(result.is_none()); // no file target in this example
/// ```
pub fn spawn_pump<R>(
    // Reader we pump from
    mut reader: R,
    targets: PumpTargets,
    cfg: PumpCfg,
    cancel: CancellationToken,
    observer: Arc<dyn NetObserver>,
    url: Url,
) -> JoinHandle<Result<Option<PathBuf>, NetError>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let PumpTargets {
        shared,
        file_dest,
        peek_buf,
    } = targets;
    let idle = cfg.idle;
    let total_deadline = cfg.total_deadline;

    tokio::spawn(async move {
        // If we need to send to file, first open the file and write the peek data
        let mut writer = if let Some(dest) = &file_dest {
            let tmp_dest = match temp_path_for(dest) {
                Ok(p) => p,
                Err(e) => {
                    return Err(NetError::Io(Arc::new(e)));
                }
            };

            let mut f = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(tmp_dest.path())
                .await
                .map_err(|e| NetError::Io(Arc::new(e)))?;

            // Write peek data first
            if !peek_buf.is_empty() {
                f.write_all(&peek_buf).await.map_err(|e| NetError::Io(Arc::new(e)))?;
            }

            Some((tmp_dest, BufWriter::new(f)))
        } else {
            None
        };

        // Next, push the peek data to the shared first
        if let Some(s) = &shared {
            if !peek_buf.is_empty() {
                s.push(peek_buf.into_bytes());
            }
        }

        // Peek writes are done. Continue with the main loop that deals with the stream
        let mut buf = BytesMut::with_capacity(16 * 1024);
        let finish_ok = loop {
            let total_left = total_deadline.map(|dl| dl.saturating_duration_since(Instant::now()));

            let read_res = tokio::select! {
                _ = cancel.cancelled() => {
                    // Cancelled
                    if let Some(s) = &shared {
                        s.error(NetError::Cancelled("Pump cancelled".into()));
                    }
                    break false;
                }
                _ = async {
                    // Wait for total time to expire, if set
                    if let Some(rem) = total_left {
                        sleep(rem).await
                    } else {
                        futures::future::pending::<()>().await
                    }
                } => {
                    if let Some(s) = &shared {
                        s.error(NetError::Timeout("Pump total timeout".into()));
                    }
                    break false;
                }
                r = timeout(idle, reader.read_buf(&mut buf)) => r,
            };

            match read_res {
                Err(_) => {
                    // Error means timeout
                    if let Some(s) = &shared {
                        s.error(NetError::Timeout("Pump idle timeout".into()));
                        break false;
                    }
                }
                Ok(Ok(0)) => {
                    // zero bytes read means EOF
                    if !buf.is_empty() {
                        let chunk = buf.split().freeze();

                        // Write chunk to shared body
                        if let Some(s) = &shared {
                            s.push(chunk.clone());
                        }

                        // Write to file
                        if let Some((_tmp, w)) = &mut writer {
                            if let Err(e) = w.write_all(&chunk).await {
                                observer.on_event(NetEvent::Io {
                                    message: format!("Failed to write to file: {}", e),
                                });
                            }
                        }

                        // Finish the shared body
                        if let Some(s) = &shared {
                            s.finish();
                        }

                        // Finally, flush the file
                        if let Some((_tmp, w)) = &mut writer {
                            if let Err(e) = w.flush().await {
                                observer.on_event(NetEvent::Warning {
                                    url: url.clone(),
                                    message: format!("Failed to flush file: {}", e),
                                });
                            }
                        }
                    }
                    break true;
                }

                Ok(Ok(_)) => {
                    // Data received
                    let chunk = buf.split().freeze();
                    if !chunk.is_empty() {
                        // Send to shared body
                        if let Some(s) = &shared {
                            s.push(chunk.clone());
                        }

                        // Send to file
                        if let Some((_tmp, w)) = &mut writer {
                            if let Err(e) = w.write_all(&chunk).await {
                                observer.on_event(NetEvent::Warning {
                                    url: url.clone(),
                                    message: format!("Failed to write to file: {}", e),
                                });
                            }
                        }
                    }
                }
                Ok(Err(e)) => {
                    // Error reading, send error to shared body. Nothing to be done for the file
                    if let Some(s) = &shared {
                        s.error(NetError::Io(Arc::new(e)));
                    }
                    break false;
                }
            }
        };

        // If we wrote to a file, and finished ok, rename the temp file to the final destination
        if let Some((tmp, _w)) = writer {
            if finish_ok {
                if let Some(dest) = file_dest {
                    tokio::fs::rename(&tmp, &dest)
                        .await
                        .map_err(|e| NetError::Io(Arc::new(e)))?;

                    return Ok(Some(dest));
                }
            }
        }

        Ok(None)
    })
}
