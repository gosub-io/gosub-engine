use std::fs;
use std::io::Cursor;

use url::Url;

use gosub_render_backend::svg::SvgRenderer;
use gosub_render_backend::{Image as _, ImageBuffer, RenderBackend};
use gosub_shared::types::Result;

pub fn request_img<B: RenderBackend>(
    svg_renderer: &mut B::SVGRenderer,
    url: &Url,
) -> Result<ImageBuffer<B>> {
    let img = if url.scheme() == "file" {
        let path = url.as_str().trim_start_matches("file://");

        println!("Loading image from: {:?}", path);

        fs::read(path)?
    } else {
        let res = gosub_net::http::ureq::get(url.as_str()).call()?;

        let mut img = Vec::with_capacity(
            res.header("Content-Length")
                .and_then(|x| x.parse::<usize>().ok())
                .unwrap_or(1024),
        );

        res.into_reader().read_to_end(&mut img)?;

        img
    };

    let is_svg = img.starts_with(b"<?xml") || img.starts_with(b"<svg");

    Ok(if is_svg {
        let svg = String::from_utf8(img)?; //TODO: We need to handle non-utf8 SVGs here

        let svg = <B::SVGRenderer as SvgRenderer<B>>::parse_external(svg)?;

        svg_renderer.render(&svg)?
    } else {
        let format = image::guess_format(&img)?;
        let img = image::load(Cursor::new(img), format)?; //In that way we don't need to copy the image data

        let img = B::Image::from_img(img);

        ImageBuffer::Image(img)
    })
}
