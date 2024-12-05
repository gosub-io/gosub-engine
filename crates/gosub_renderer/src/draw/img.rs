use std::io::Cursor;
use std::sync::{Arc, LazyLock, Mutex};

use anyhow::anyhow;
use image::DynamicImage;

use crate::draw::img_cache::ImageCache;
use gosub_net::http::fetcher::Fetcher;
use gosub_shared::render_backend::svg::SvgRenderer;
use gosub_shared::render_backend::{
    Image as _, ImageBuffer, ImageCacheEntry, ImgCache, RenderBackend, SizeU32, WindowedEventLoop,
};
use gosub_shared::traits::config::HasDrawComponents;
use gosub_shared::types::Result;

pub fn request_img<C: HasDrawComponents>(
    fetcher: Arc<Fetcher>,
    svg_renderer: Arc<Mutex<<C::RenderBackend as RenderBackend>::SVGRenderer>>,
    url: &str,
    size: Option<SizeU32>,
    img_cache: &mut ImageCache<C::RenderBackend>,
    el: &impl WindowedEventLoop<C>,
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

            let mut el = el.clone();

            gosub_shared::async_executor::spawn(async move {
                if let Ok(img) = load_img::<C::RenderBackend>(&url, fetcher, svg_renderer, size).await {
                    el.add_img_cache(url.to_string(), img, size);
                } else {
                    el.add_img_cache(
                        url.to_string(),
                        ImageBuffer::Image(<C::RenderBackend as RenderBackend>::Image::from_img(
                            INVALID_IMG.clone(),
                        )),
                        size,
                    );
                }
            });

            ImageBuffer::Image(<C::RenderBackend as RenderBackend>::Image::from_img(
                DynamicImage::new_rgba8(0, 0),
            ))
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
