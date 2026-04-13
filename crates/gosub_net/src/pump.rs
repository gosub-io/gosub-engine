use crate::emitter::NetObserver;
use crate::events::NetEvent;
use crate::fs_utils::temp_path_for;
use crate::net_types::NetError;
use crate::shared_body::SharedBody;
use crate::types::PeekBuf;
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
pub struct PumpCfg {
    /// Idle timeout
    pub idle: std::time::Duration,
    /// Total timeout
    pub total_deadline: Option<Instant>,
}

/// Destinations for a pump run.
pub struct PumpTargets {
    /// Shared body
    pub shared: Option<Arc<SharedBody>>,
    /// Optional file destination
    pub file_dest: Option<PathBuf>,
    /// peek buffer we need to send first
    pub peek_buf: PeekBuf,
}

/// Pumps bytes from an `AsyncRead` into one or both targets:
/// a fan-out `SharedBody` and/or a file on disk.
pub fn spawn_pump<R>(
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

            if !peek_buf.is_empty() {
                f.write_all(&peek_buf).await.map_err(|e| NetError::Io(Arc::new(e)))?;
            }

            Some((tmp_dest, BufWriter::new(f)))
        } else {
            None
        };

        if let Some(s) = &shared {
            if !peek_buf.is_empty() {
                s.push(peek_buf.into_bytes());
            }
        }

        let mut buf = BytesMut::with_capacity(16 * 1024);
        let finish_ok = loop {
            let total_left = total_deadline.map(|dl| dl.saturating_duration_since(Instant::now()));

            let read_res = tokio::select! {
                _ = cancel.cancelled() => {
                    if let Some(s) = &shared {
                        s.error(NetError::Cancelled("Pump cancelled".into()));
                    }
                    break false;
                }
                _ = async {
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
                    if let Some(s) = &shared {
                        s.error(NetError::Timeout("Pump idle timeout".into()));
                        break false;
                    }
                }
                Ok(Ok(0)) => {
                    if !buf.is_empty() {
                        let chunk = buf.split().freeze();

                        if let Some(s) = &shared {
                            s.push(chunk.clone());
                        }

                        if let Some((_tmp, w)) = &mut writer {
                            if let Err(e) = w.write_all(&chunk).await {
                                observer.on_event(NetEvent::Io {
                                    message: format!("Failed to write to file: {}", e),
                                });
                            }
                        }

                        if let Some(s) = &shared {
                            s.finish();
                        }

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
                    let chunk = buf.split().freeze();
                    if !chunk.is_empty() {
                        if let Some(s) = &shared {
                            s.push(chunk.clone());
                        }

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
                    if let Some(s) = &shared {
                        s.error(NetError::Io(Arc::new(e)));
                    }
                    break false;
                }
            }
        };

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
