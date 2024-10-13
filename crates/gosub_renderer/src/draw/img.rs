use std::io::Cursor;
use std::sync::{Arc, LazyLock, Mutex};

use anyhow::anyhow;
use image::DynamicImage;
use log::error;

use crate::draw::img_cache::ImageCache;
use gosub_net::http::fetcher::Fetcher;
use gosub_render_backend::svg::SvgRenderer;
use gosub_render_backend::{Image as _, ImageBuffer, ImageCacheEntry, ImgCache, RenderBackend, SizeU32};
use gosub_shared::types::Result;

pub fn request_img<B: RenderBackend>(
    fetcher: Arc<Fetcher>,
    svg_renderer: Arc<Mutex<B::SVGRenderer>>,
    url: &str,
    size: Option<SizeU32>,
    img_cache: Arc<Mutex<ImageCache<B>>>,
    rerender: Arc<dyn Fn() + Send + Sync>,
) -> Result<ImageBuffer<B>> {
    let mut cache = img_cache.lock().map_err(|_| anyhow!("Could not lock img cache"))?;

    let img = cache.get(url);

    Ok(match img {
        ImageCacheEntry::Image(img) => img.clone(),
        ImageCacheEntry::Pending => ImageBuffer::Image(B::Image::from_img(image::DynamicImage::new_rgba8(0, 0))),
        ImageCacheEntry::None => {
            cache.add_pending(url.to_string());

            drop(cache);

            let url = url.to_string();

            gosub_shared::async_executor::spawn(async move {
                if let Ok(img) = load_img::<B>(&url, fetcher, svg_renderer, size).await {
                    let mut cache = match img_cache.lock() {
                        Ok(cache) => cache,
                        Err(e) => {
                            error!("Could not lock img cache: {}", e);
                            return;
                        }
                    };

                    cache.add(url.to_string(), img.clone(), size);

                    rerender();
                } else {
                    let mut cache = match img_cache.lock() {
                        Ok(cache) => cache,
                        Err(e) => {
                            error!("Could not lock img cache: {}", e);
                            return;
                        }
                    };

                    cache.add(
                        url.to_string(),
                        ImageBuffer::Image(B::Image::from_img(INVALID_IMG.clone())),
                        size,
                    );
                }
            });

            ImageBuffer::Image(B::Image::from_img(DynamicImage::new_rgba8(0, 0)))
        }
    })
}

async fn load_img<B: RenderBackend>(
    url: &str,
    fetcher: Arc<Fetcher>,
    svg_renderer: Arc<Mutex<B::SVGRenderer>>,
    size: Option<SizeU32>,
) -> Result<ImageBuffer<B>> {
    let res = fetcher.get(url).await?;
    if !res.is_ok() {
        return Err(anyhow!("Could not get url. Status code {}", res.status));
    }

    let img = res.body;

    let is_svg = img.starts_with(b"<?xml") || img.starts_with(b"<svg");

    Ok(if is_svg {
        let svg = String::from_utf8(img)?; //TODO: We need to handle non-utf8 SVGs here

        let svg = <B::SVGRenderer as SvgRenderer<B>>::parse_external(svg)?;

        let mut svg_renderer = svg_renderer
            .lock()
            .map_err(|_| anyhow!("Could not lock svg renderer"))?;

        if let Some(size) = size {
            svg_renderer.render_with_size(&svg, size)?
        } else {
            svg_renderer.render(&svg)?
        }
    } else {
        let format = image::guess_format(&img)?;
        let img = image::load(Cursor::new(img), format)?; //In that way we don't need to copy the image data

        let img = B::Image::from_img(img);

        ImageBuffer::Image(img)
    })
}

const INVALID_IMG_BYTES: &[u8] = include_bytes!("../../../../resources/test_img.png");

static INVALID_IMG: LazyLock<DynamicImage> =
    LazyLock::new(|| image::load_from_memory(INVALID_IMG_BYTES).unwrap_or(DynamicImage::new_rgba8(0, 0)));
