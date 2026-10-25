//! Web fonts kept back at load, their media conditions not holding: the context registers them
//! when a resize makes them apply, before the frame that lays text out in them.

use super::*;
use crate::engine::resource_pipeline::webfonts::DeferredFonts;
use crate::engine::settings_store;
use crate::html::DefaultRenderConfig;
use gosub_css3::system::Css3System;
use parking_lot::Mutex;

type Ctx = BrowsingContext<DefaultRenderConfig>;

fn viewport(width: u32) -> Viewport {
    Viewport {
        x: 0,
        y: 0,
        width,
        height: 600,
    }
}

/// A context showing a page with a face under `(min-width: 700px)` that its load kept back,
/// at `width`, and the families registered since. With `ready` the face's bytes are in, or
/// else still on their way.
fn page_at(width: u32, ready: bool) -> (Ctx, Arc<Mutex<Vec<String>>>) {
    let mut ctx: Ctx = BrowsingContext::new(settings_store::default_config());
    ctx.set_viewport(viewport(width));
    let html = "<!DOCTYPE html><html><head><style>\
        @media (min-width: 700px) { @font-face { font-family: Wide; src: url(https://fonts.test/w.ttf); } }\
        p { font-family: Wide, sans-serif; }</style></head><body><p>text</p></body></html>";
    let mut doc = gosub_html5::html_compile::<DefaultRenderConfig>(html);
    doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
    doc.url = Some(Url::parse("http://test.local/page.html").unwrap_or_else(|_| unreachable!()));
    ctx.set_document(Arc::new(doc), None);

    let registered = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&registered);
    ctx.set_deferred_fonts(DeferredFonts::for_test(
        &[("Wide", "https://fonts.test/w.ttf")],
        ready,
        Box::new(move |_, family| {
            seen.lock().push(family.to_string());
            Ok(())
        }),
    ));
    (ctx, registered)
}

#[test]
fn a_deferred_face_is_registered_when_a_resize_brings_it_in() {
    let (mut ctx, registered) = page_at(500, true);
    ctx.rebuild_pipeline_cache_if_needed();
    assert!(registered.lock().is_empty(), "a 500px page does not take the face");

    ctx.set_viewport(viewport(900));
    ctx.rebuild_pipeline_cache_if_needed();
    assert_eq!(*registered.lock(), ["Wide"], "registered for the frame at 900px");

    ctx.set_viewport(viewport(1000));
    ctx.rebuild_pipeline_cache_if_needed();
    assert_eq!(registered.lock().len(), 1, "once");
}

/// The load decides under the viewport it saw when the navigation started; the page may be
/// shown at another. A face kept back then but applying at the real viewport is registered for
/// the first frame, with no resize to prompt it.
#[test]
fn a_face_kept_back_at_load_is_registered_for_the_first_frame_if_it_applies() {
    let (mut ctx, registered) = page_at(900, true);
    ctx.rebuild_pipeline_cache_if_needed();
    assert_eq!(*registered.lock(), ["Wide"]);
}

/// The load does not wait for a face it kept back, so its bytes can land after the frame that
/// wanted them. The landing wakes the render loop and the page is laid out again in it.
#[test]
fn a_face_that_lands_after_the_frame_that_wanted_it_is_registered_on_landing() {
    let (mut ctx, registered) = page_at(900, false);
    ctx.rebuild_pipeline_cache_if_needed();
    assert!(registered.lock().is_empty(), "not in yet");
    assert!(!ctx.poll_deferred_fonts(), "nothing landed");

    ctx.deferred_fonts.land_for_test();
    assert!(ctx.poll_deferred_fonts(), "the landing wakes the frame loop");
    assert_eq!(*registered.lock(), ["Wide"]);
    assert!(!ctx.damage.is_none(), "and the page is to be laid out again");
    assert!(!ctx.poll_deferred_fonts(), "once");
}

/// A face that lands while its condition does not hold waits for a resize, as it would have
/// had it come in with the document.
#[test]
fn a_face_that_lands_while_it_does_not_apply_waits() {
    let (mut ctx, registered) = page_at(500, false);
    ctx.rebuild_pipeline_cache_if_needed();
    ctx.deferred_fonts.land_for_test();
    assert!(!ctx.poll_deferred_fonts(), "nothing to lay out again for");
    assert!(registered.lock().is_empty());

    ctx.set_viewport(viewport(900));
    ctx.rebuild_pipeline_cache_if_needed();
    assert_eq!(*registered.lock(), ["Wide"]);
}
