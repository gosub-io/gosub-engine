use crate::engine::types::{EventChannel, NavigationId};
use crate::events::{EngineEvent, NavigationEvent};
use crate::net::types::{BodyStream, FetchResultMeta};
use crate::tab::TabId;
use anyhow::anyhow;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;

#[allow(unused)]
pub fn start_download(
    tab_id: TabId,
    nav_id: NavigationId,
    meta: FetchResultMeta,
    mut stream: BodyStream,
    dest: PathBuf,
    event_tx: EventChannel,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let started = Instant::now();

        // tell the engine a download started
        let _ = event_tx.send(EngineEvent::Navigation {
            tab_id,
            event: NavigationEvent::Started {
                nav_id,
                url: meta.final_url.clone(),
            },
        });

        let mut file = match tokio::fs::File::create(&dest).await {
            Ok(f) => f,
            Err(e) => {
                let _ = event_tx.send(EngineEvent::Navigation {
                    tab_id,
                    event: NavigationEvent::Failed {
                        nav_id: Some(nav_id),
                        url: meta.final_url.clone(),
                        error: Arc::new(anyhow!(e)),
                    },
                });
                return;
            }
        };

        let mut buf = [0u8; 64 * 1024];
        let mut written: u64 = 0;

        loop {
            let n = match stream.read(&mut buf).await {
                Ok(0) => break, // Eof
                Ok(n) => n,
                Err(e) => {
                    let _ = event_tx.send(EngineEvent::Navigation {
                        tab_id,
                        event: NavigationEvent::Failed {
                            nav_id: Some(nav_id),
                            url: meta.final_url.clone(),
                            error: Arc::new(anyhow!(e)),
                        },
                    });

                    return;
                }
            };

            if let Err(e) = file.write_all(&buf[..n]).await {
                let _ = event_tx.send(EngineEvent::Navigation {
                    tab_id,
                    event: NavigationEvent::Failed {
                        nav_id: Some(nav_id),
                        url: meta.final_url.clone(),
                        error: Arc::new(anyhow!("Failed to write to file: {}", e)),
                    },
                });

                return;
            }

            written += n as u64;

            let _ = event_tx.send(EngineEvent::Navigation {
                tab_id,
                event: NavigationEvent::Progress {
                    nav_id,
                    received_bytes: written,
                    expected_length: meta.content_length,
                    elapsed: started.elapsed(),
                },
            });
        }

        // Ensure data hits disk
        let _ = file.flush().await;

        let _ = event_tx.send(EngineEvent::Navigation {
            tab_id,
            event: NavigationEvent::Finished {
                nav_id,
                url: meta.final_url.clone(),
            },
        });
    })
}
