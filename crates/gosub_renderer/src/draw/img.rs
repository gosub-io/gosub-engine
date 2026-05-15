use parking_lot::Mutex;
use std::io::Cursor;
use std::sync::{Arc, LazyLock};

use crate::draw::img_cache::ImageCache;
use anyhow::anyhow;
use gosub_interface::config::HasDrawComponents;
use gosub_interface::eventloop::EventLoopHandle;
use gosub_interface::render_backend::{Image as _, ImageBuffer, ImageCacheEntry, ImgCache, RenderBackend, SizeU32};
use gosub_interface::svg::SvgRenderer;
use gosub_net::net::simple::sync_get;
use gosub_shared::types::Result;
use image::DynamicImage;
use url::{ParseError, Url};

pub fn request_img<C: HasDrawComponents>(
    base_url: &Url,
    svg_renderer: Arc<Mutex<<C::RenderBackend as RenderBackend>::SVGRenderer>>,
    url: &str,
    size: Option<SizeU32>,
    img_cache: &mut ImageCache<C::RenderBackend>,
    el: &impl EventLoopHandle<C>,
) -> Result<ImageBuffer<C::RenderBackend>> {
    let img = img_cache.get(url);

    Ok(match img {
        ImageCacheEntry::Image(img) => img.clone(),
        ImageCacheEntry::Pending => ImageBuffer::Image(<C::RenderBackend as RenderBackend>::Image::from_img(
            image::DynamicImage::new_rgba8(0, 0),
        )),
        ImageCacheEntry::None => {
            img_cache.add_pending(url.to_string());

            let url = url.to_string();
            let base_url = base_url.clone();
            let el = el.clone();

            std::thread::spawn(move || {
                let resolved = resolve_url(&base_url, &url);
                match resolved {
                    Ok(resolved_url) => {
                        if let Ok(img) = load_img::<C::RenderBackend>(&resolved_url, svg_renderer, size) {
                            el.add_img_cache(resolved_url, img, size);
                        } else {
                            el.add_img_cache(
                                Url::parse(&url).unwrap_or(base_url),
                                ImageBuffer::Image(<C::RenderBackend as RenderBackend>::Image::from_img(
                                    INVALID_IMG.clone(),
                                )),
                                size,
                            );
                        }
                    }
                    Err(_) => {
                        el.add_img_cache(
                            base_url,
                            ImageBuffer::Image(<C::RenderBackend as RenderBackend>::Image::from_img(
                                INVALID_IMG.clone(),
                            )),
                            size,
                        );
                    }
                }
            });

            ImageBuffer::Image(<C::RenderBackend as RenderBackend>::Image::from_img(
                DynamicImage::new_rgba8(0, 0),
            ))
        }
    })
}

fn resolve_url(base: &Url, url: &str) -> Result<Url> {
    match Url::parse(url) {
        Ok(u) => Ok(u),
        Err(ParseError::RelativeUrlWithoutBase) => Ok(base.join(url)?),
        Err(e) => Err(e.into()),
    }
}

fn load_img<B: RenderBackend>(
    url: &Url,
    svg_renderer: Arc<Mutex<B::SVGRenderer>>,
    size: Option<SizeU32>,
) -> Result<ImageBuffer<B>> {
    let img = sync_get(url)?;
    if img.is_empty() {
        return Err(anyhow!("Empty response for {url}"));
    }

    let img = img.to_vec();
    let is_svg = img.starts_with(b"<?xml") || img.starts_with(b"<svg");

    Ok(if is_svg {
        let svg = String::from_utf8(img)?;
        let svg = <B::SVGRenderer as SvgRenderer<B>>::parse_external(svg)?;
        let mut svg_renderer = svg_renderer.lock();
        if let Some(size) = size {
            svg_renderer.render_with_size(&svg, size)?
        } else {
            svg_renderer.render(&svg)?
        }
    } else {
        let format = image::guess_format(&img)?;
        let img = image::load(Cursor::new(img), format)?;
        ImageBuffer::Image(B::Image::from_img(img))
    })
}

const INVALID_IMG_BYTES: &[u8] = include_bytes!("../../resources/test_img.png");

static INVALID_IMG: LazyLock<DynamicImage> =
    LazyLock::new(|| image::load_from_memory(INVALID_IMG_BYTES).unwrap_or(DynamicImage::new_rgba8(0, 0)));
