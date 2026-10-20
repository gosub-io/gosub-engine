//! `page.first_paint` is marked on every render path, not only the display-list one the null
//! backend takes: a shell on Vello, Skia or Cairo never got one, so gosub://stats had none.

#![allow(clippy::unwrap_used, clippy::expect_used)] // a test that cannot set up must fail loudly

use std::sync::Arc;
use std::time::Duration;

use gosub_engine::cookies::DefaultCookieJar;
use gosub_engine::events::{EngineEvent, NavigationEvent, TabCommand};
use gosub_engine::storage::{InMemoryLocalStore, InMemorySessionStore, PartitionPolicy, StorageService};
use gosub_engine::zone::ZoneServices;
use gosub_engine::{DefaultRenderConfig, EngineConfig, GosubEngine};
use gosub_render_pipeline::render::backends::null::NullBackend;
use gosub_render_pipeline::render::{
    DefaultCompositor, ErasedSurface, ExternalHandle, PresentMode, RenderBackend, RenderContext, RgbaImage, SurfaceSize,
};
use gosub_shared::timing::{snapshot_stats_for, ScopeId};

fn services() -> ZoneServices {
    ZoneServices {
        storage: Arc::new(StorageService::new(
            Arc::new(InMemoryLocalStore::new()),
            Arc::new(InMemorySessionStore::new()),
        )),
        cookie_store: None,
        cookie_jar: Some(DefaultCookieJar::new().into()),
        partition_policy: PartitionPolicy::None,
        places: None,
    }
}

/// The null backend dressed up as a GPU one, so the worker takes the scene path Vello takes
/// (one viewport render into a texture) without needing a GPU.
struct GpuScene(NullBackend);

impl RenderBackend for GpuScene {
    fn name(&self) -> &'static str {
        "GpuScene"
    }
    fn create_surface(&self, size: SurfaceSize, present: PresentMode) -> anyhow::Result<Box<dyn ErasedSurface + Send>> {
        self.0.create_surface(size, present)
    }
    fn render(&self, context: &mut dyn RenderContext, surface: &mut dyn ErasedSurface) -> anyhow::Result<()> {
        self.0.render(context, surface)
    }
    fn snapshot(&self, surface: &mut dyn ErasedSurface, max_dim: u32) -> anyhow::Result<RgbaImage> {
        self.0.snapshot(surface, max_dim)
    }
    fn external_handle(&self, surface: &mut dyn ErasedSurface) -> anyhow::Result<ExternalHandle> {
        self.0.external_handle(surface)
    }
    fn renders_to_gpu_texture(&self) -> bool {
        true
    }
}

/// Loads a page on `backend` and returns (page.dom_complete, page.first_paint) in µs, once
/// the first paint is in or the wait runs out.
async fn marks_on<B: RenderBackend + Send + Sync + 'static>(backend: B) -> (Option<u64>, Option<u64>) {
    let mut engine: GosubEngine<DefaultRenderConfig<B>> = GosubEngine::new(
        Some(EngineConfig::default()),
        Arc::new(backend),
        Arc::new(DefaultCompositor::default()),
    );
    let task = tokio::spawn(engine.start().expect("engine start"));
    let mut events = engine.subscribe_events();
    let mut zone = engine.zone_builder().services(services()).create().expect("zone");
    let tab = zone.tab_builder().create().await.expect("tab");

    tab.send(TabCommand::SetViewport {
        x: 0,
        y: 0,
        width: 800,
        height: 600,
    })
    .await
    .unwrap();
    tab.send(TabCommand::LoadHtml {
        html: "<html><body><p>hello</p></body></html>".to_string(),
        base_url: "https://example.test/page".to_string(),
    })
    .await
    .unwrap();
    tab.send(TabCommand::ResumeDrawing { fps: 60 }).await.unwrap();

    let nav_id = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match events.recv().await {
                Ok(EngineEvent::Navigation {
                    tab_id,
                    event: NavigationEvent::Started { nav_id, .. },
                }) if tab_id == tab.tab_id => return Some(nav_id),
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return None,
            }
        }
    })
    .await
    .ok()
    .flatten()
    .expect("the navigation starts");

    let scope = ScopeId(nav_id.0);
    let mark = |name: &str| {
        snapshot_stats_for(scope)
            .into_iter()
            .find(|s| s.namespace == name)
            .map(|s| s.total_us)
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while mark("page.first_paint").is_none() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let marks = (mark("page.dom_complete"), mark("page.first_paint"));
    let _ = engine.shutdown().await;
    task.abort();
    marks
}

fn assert_first_paint((dom_complete, first_paint): (Option<u64>, Option<u64>)) {
    let first_paint = first_paint.expect("page.first_paint is marked");
    let dom_complete = dom_complete.expect("page.dom_complete is marked");
    assert!(
        first_paint >= dom_complete,
        "the first paint is a frame of the new page, so it comes after the commit: \
         first paint {first_paint}us, dom complete {dom_complete}us"
    );
}

#[tokio::test]
async fn the_display_list_path_marks_first_paint() {
    assert_first_paint(marks_on(NullBackend::new()).await);
}

#[tokio::test]
async fn the_gpu_scene_path_marks_first_paint() {
    assert_first_paint(marks_on(GpuScene(NullBackend::new())).await);
}
