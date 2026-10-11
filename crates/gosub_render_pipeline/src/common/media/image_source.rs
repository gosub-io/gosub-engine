//! Which URL an `<img>` shows: HTML's image source selection over `src`, `srcset`, `sizes` and
//! the `<source>` elements of a `<picture>` (HTML "update the source set" and "select an image
//! source").
//!
//! Layout fetches what this picks and the hit tests report the same URL, so all of them go
//! through [`select_image_source`]. The media environment it selects against (viewport, device
//! pixel ratio, what `<source media>` and `sizes` conditions match) is the CSS system's, the one
//! the page's own media queries are resolving against on this thread.

use super::decodes_image_type;
use gosub_interface::config::HasDocument;
use gosub_interface::css3::CssSystem;
use gosub_interface::document::Document;
use gosub_shared::node::NodeId;
use std::marker::PhantomData;

/// The document an `<img>` sits in, as far as choosing its source needs it.
pub trait ImageTree {
    type Id: Copy + PartialEq;
    fn tag_name(&self, id: Self::Id) -> Option<String>;
    fn attribute(&self, id: Self::Id, name: &str) -> Option<String>;
    fn parent(&self, id: Self::Id) -> Option<Self::Id>;
    fn children(&self, id: Self::Id) -> Vec<Self::Id>;
}

/// An engine document as an [`ImageTree`].
struct DocumentTree<'a, C: HasDocument>(&'a C::Document, PhantomData<C>);

impl<C: HasDocument> ImageTree for DocumentTree<'_, C> {
    type Id = NodeId;
    fn tag_name(&self, id: NodeId) -> Option<String> {
        self.0.tag_name(id).map(str::to_string)
    }
    fn attribute(&self, id: NodeId, name: &str) -> Option<String> {
        self.0.attribute(id, name).map(str::to_string)
    }
    fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.0.parent(id)
    }
    fn children(&self, id: NodeId) -> Vec<NodeId> {
        self.0.children(id).to_vec()
    }
}

/// [`select_image_source`] for the `<img>` `img` of an engine document.
pub fn select_in_document<C: HasDocument>(doc: &C::Document, img: NodeId) -> Option<SelectedImage> {
    select_image_source::<C::CssSystem, _>(&DocumentTree::<C>(doc, PhantomData), img)
}

/// What an `<img>` shows, before it is resolved against the document's base URL.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectedImage {
    pub url: String,
    /// Image pixels per CSS px: the natural size is the decoded size divided by this.
    pub density: f32,
    /// The element uses `srcset` or `<picture>`: Fetch's `imageset` initiator, which Mixed
    /// Content blocks where a plain `<img src>` is upgraded. Set by the element, not by where
    /// the chosen URL came from, so a picture's fallback `src` is one too.
    pub imageset: bool,
}

/// One candidate of a source set.
#[derive(Debug, Clone, PartialEq)]
struct Candidate {
    url: String,
    descriptor: Descriptor,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Descriptor {
    Density(f32),
    /// Image pixels wide; a density once the source size is known.
    Width(u32),
}

/// Choose the source of the `<img>` `img` in `tree`, against `S`'s media environment. `None`
/// when it has nothing to show: no candidate anywhere, or an empty `src` and no `srcset`.
pub fn select_image_source<S: CssSystem, T: ImageTree + ?Sized>(tree: &T, img: T::Id) -> Option<SelectedImage> {
    let srcset = tree.attribute(img, "srcset");
    let in_picture = tree.parent(img).filter(|&parent| {
        tree.tag_name(parent)
            .is_some_and(|tag| tag.eq_ignore_ascii_case("picture"))
    });
    let imageset = srcset.is_some() || in_picture.is_some();
    let select = |set: Vec<Candidate>, sizes: Option<String>| {
        choose::<S>(set, sizes.as_deref()).map(|(url, density)| SelectedImage { url, density, imageset })
    };

    // A `<source>` before the `<img>` is a candidate set; one after it is not.
    if let Some(picture) = in_picture {
        for child in tree.children(picture) {
            if child == img {
                break;
            }
            if !tree
                .tag_name(child)
                .is_some_and(|tag| tag.eq_ignore_ascii_case("source"))
            {
                continue;
            }
            let Some(set) = tree.attribute(child, "srcset").map(|srcset| parse_srcset(&srcset)) else {
                continue;
            };
            if set.is_empty() {
                continue;
            }
            // An unparseable list is `not all`.
            if let Some(media) = tree.attribute(child, "media") {
                if S::media_list_matches(&media) != Some(true) {
                    continue;
                }
            }
            if tree.attribute(child, "type").is_some_and(|ty| !decodes_image_type(&ty)) {
                continue;
            }
            return select(set, tree.attribute(child, "sizes"));
        }
    }

    let mut set = srcset.as_deref().map(parse_srcset).unwrap_or_default();
    // `src` is the 1x candidate, unless the set already has one or is sized by width.
    if let Some(src) = tree.attribute(img, "src").filter(|src| !src.is_empty()) {
        let covered = set
            .iter()
            .any(|c| matches!(c.descriptor, Descriptor::Width(_)) || c.descriptor == Descriptor::Density(1.0));
        if !covered {
            set.push(Candidate {
                url: src,
                descriptor: Descriptor::Density(1.0),
            });
        }
    }
    select(set, tree.attribute(img, "sizes"))
}

/// Pick from a source set: the lowest density that still covers the device pixel ratio, or the
/// densest there is. HTML leaves the choice to the user agent; this is what browsers do on a
/// first load.
fn choose<S: CssSystem>(set: Vec<Candidate>, sizes: Option<&str>) -> Option<(String, f32)> {
    let mut densities: Vec<(String, f32)> = Vec::with_capacity(set.len());
    for candidate in set {
        let density = match candidate.descriptor {
            Descriptor::Density(density) => density,
            Descriptor::Width(width) => width as f32 / source_size::<S>(sizes),
        };
        // HTML drops a candidate whose density repeats an earlier one.
        if density.is_finite() && !densities.iter().any(|(_, seen)| *seen == density) {
            densities.push((candidate.url, density));
        }
    }
    let (_, dpr) = S::media_viewport();
    let covering = densities
        .iter()
        .filter(|(_, density)| *density >= dpr)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    let best = covering.or_else(|| densities.iter().max_by(|a, b| a.1.total_cmp(&b.1)))?;
    Some(best.clone())
}

/// HTML "parse a sizes attribute": the size of the first entry whose condition matches, in px.
/// `100vw` when none does, or there is no attribute.
fn source_size<S: CssSystem>(sizes: Option<&str>) -> f32 {
    let (fallback, _) = S::media_viewport();
    let Some(sizes) = sizes else {
        return fallback;
    };
    for entry in split_top_level(sizes, ',') {
        let (condition, size) = split_last_component(entry.trim());
        // `auto` is for lazy images, which are not deferred here; it is skipped like any other
        // size that is not a length.
        let Some(px) = S::length_px(size) else {
            continue;
        };
        let condition = condition.trim();
        if condition.is_empty() || S::media_condition_matches(condition) == Some(true) {
            return px;
        }
    }
    fallback
}

/// `text` split at `separator` where it is not inside parentheses.
fn split_top_level(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, c) in text.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            c if c == separator && depth == 0 => {
                parts.push(&text[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

/// A `sizes` entry split into what comes before its last component value, and that value: a
/// whole function (`calc(100vw - 2em)`) or the last run without whitespace.
fn split_last_component(entry: &str) -> (&str, &str) {
    let start = if entry.ends_with(')') {
        let mut depth = 0usize;
        let mut open = None;
        for (i, c) in entry.char_indices().rev() {
            match c {
                ')' => depth += 1,
                '(' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        open = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        // Back over the function's name, which is part of the value.
        open.map(|open| {
            entry[..open]
                .char_indices()
                .rev()
                .take_while(|(_, c)| c.is_alphanumeric() || *c == '-' || *c == '_')
                .last()
                .map_or(open, |(i, _)| i)
        })
        .unwrap_or(0)
    } else {
        entry.rfind(char::is_whitespace).map_or(0, |i| i + 1)
    };
    (&entry[..start], &entry[start..])
}

/// HTML "parse a srcset attribute". Candidates with invalid descriptors are dropped.
fn parse_srcset(input: &str) -> Vec<Candidate> {
    let is_space = |c: char| matches!(c, ' ' | '\t' | '\n' | '\x0c' | '\r');
    let mut candidates = Vec::new();
    let mut rest = input;
    loop {
        rest = rest.trim_start_matches(|c: char| is_space(c) || c == ',');
        if rest.is_empty() {
            return candidates;
        }
        let url_end = rest.find(is_space).unwrap_or(rest.len());
        let (raw_url, after) = rest.split_at(url_end);
        rest = after;
        let mut descriptors = Vec::new();
        // A URL that ends in commas ends the candidate there; the commas are not part of it.
        let url = raw_url.trim_end_matches(',');
        if url.len() == raw_url.len() {
            rest = tokenize_descriptors(rest, &mut descriptors);
        }
        if url.is_empty() {
            continue;
        }
        if let Some(descriptor) = parse_descriptors(&descriptors) {
            candidates.push(Candidate {
                url: url.to_string(),
                descriptor,
            });
        }
    }
}

/// Collect one candidate's descriptors from `input`, returning what follows the candidate.
fn tokenize_descriptors<'a>(input: &'a str, out: &mut Vec<String>) -> &'a str {
    let is_space = |c: char| matches!(c, ' ' | '\t' | '\n' | '\x0c' | '\r');
    let mut current = String::new();
    let mut in_parens = false;
    for (i, c) in input.char_indices() {
        if in_parens {
            current.push(c);
            if c == ')' {
                in_parens = false;
            }
            continue;
        }
        match c {
            c if is_space(c) => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            ',' => {
                if !current.is_empty() {
                    out.push(current);
                }
                return &input[i + 1..];
            }
            '(' => {
                current.push(c);
                in_parens = true;
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    ""
}

/// The descriptor a candidate's descriptor tokens add up to; `None` when HTML calls them an
/// error. A height (`h`) is only allowed next to a width, and is otherwise unused.
fn parse_descriptors(tokens: &[String]) -> Option<Descriptor> {
    let mut width = None;
    let mut density = None;
    let mut height = None;
    for token in tokens {
        let (number, suffix) = token.split_at(token.char_indices().last()?.0);
        match suffix {
            "w" if width.is_none() && density.is_none() => {
                width = Some(non_negative_integer(number).filter(|&w| w > 0)?);
            }
            "x" if width.is_none() && density.is_none() && height.is_none() => {
                density = Some(floating_point(number).filter(|&d| d >= 0.0)?);
            }
            "h" if height.is_none() && density.is_none() => {
                height = Some(non_negative_integer(number).filter(|&h| h > 0)?);
            }
            _ => return None,
        }
    }
    match (width, density, height) {
        (Some(width), None, _) => Some(Descriptor::Width(width)),
        (None, _, Some(_)) => None,
        (None, density, None) => Some(Descriptor::Density(density.unwrap_or(1.0))),
        (Some(_), Some(_), _) => None,
    }
}

/// HTML's valid non-negative integer: ASCII digits only.
fn non_negative_integer(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// HTML's valid floating-point number: an optional `-`, digits with an optional fraction, an
/// optional exponent. Rust's own parser also takes `inf`, `+1` and `.5`, which HTML does not.
fn floating_point(text: &str) -> Option<f32> {
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(i) => (&unsigned[..i], Some(&unsigned[i + 1..])),
        None => (unsigned, None),
    };
    let (whole, fraction) = match mantissa.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (mantissa, None),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let valid = digits(whole)
        && fraction.is_none_or(digits)
        && exponent.is_none_or(|e| digits(e.strip_prefix(['-', '+']).unwrap_or(e)));
    if !valid {
        return None;
    }
    text.parse::<f32>().ok().filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gosub_css3::media_query::MediaEnvironment;
    use gosub_css3::system::Css3System;

    /// An element: its tag, attributes, parent and children.
    struct Node {
        tag: String,
        attrs: Vec<(String, String)>,
        parent: Option<usize>,
        children: Vec<usize>,
    }

    /// A document of elements.
    #[derive(Default)]
    struct Tree {
        nodes: Vec<Node>,
    }

    impl Tree {
        fn add(&mut self, parent: Option<usize>, tag: &str, attrs: &[(&str, &str)]) -> usize {
            let id = self.nodes.len();
            let attrs = attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            self.nodes.push(Node {
                tag: tag.to_string(),
                attrs,
                parent,
                children: Vec::new(),
            });
            if let Some(parent) = parent {
                self.nodes[parent].children.push(id);
            }
            id
        }
    }

    impl ImageTree for Tree {
        type Id = usize;
        fn tag_name(&self, id: usize) -> Option<String> {
            Some(self.nodes[id].tag.clone())
        }
        fn attribute(&self, id: usize, name: &str) -> Option<String> {
            self.nodes[id]
                .attrs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        }
        fn parent(&self, id: usize) -> Option<usize> {
            self.nodes[id].parent
        }
        fn children(&self, id: usize) -> Vec<usize> {
            self.nodes[id].children.clone()
        }
    }

    fn env(width: f32, dpr: f32) -> MediaEnvironment {
        MediaEnvironment {
            width,
            device_pixel_ratio: dpr,
            ..MediaEnvironment::default()
        }
    }

    /// The source `img` gets on a `width` px viewport at `dpr`.
    fn pick(tree: &Tree, img: usize, width: f32, dpr: f32) -> Option<SelectedImage> {
        gosub_css3::media_query::set_media_environment(env(width, dpr));
        select_image_source::<Css3System, _>(tree, img)
    }

    fn img(attrs: &[(&str, &str)], env: &MediaEnvironment) -> Option<SelectedImage> {
        let mut tree = Tree::default();
        let body = tree.add(None, "body", &[]);
        let img = tree.add(Some(body), "img", attrs);
        pick(&tree, img, env.width, env.device_pixel_ratio)
    }

    fn url(selected: Option<SelectedImage>) -> Option<String> {
        selected.map(|s| s.url)
    }

    #[test]
    fn a_plain_src_is_not_an_imageset() {
        let chosen = img(&[("src", "a.png")], &env(1000.0, 1.0)).expect("chosen");
        assert_eq!(chosen.url, "a.png");
        assert_eq!(chosen.density, 1.0);
        assert!(!chosen.imageset);
        assert_eq!(img(&[("src", "")], &env(1000.0, 1.0)), None);
        assert_eq!(img(&[], &env(1000.0, 1.0)), None);
    }

    #[test]
    fn density_descriptors_pick_the_lowest_that_covers_the_screen() {
        let attrs = [("src", "1x.png"), ("srcset", "2x.png 2x, 3x.png 3x")];
        assert_eq!(url(img(&attrs, &env(1000.0, 1.0))).as_deref(), Some("1x.png"));
        assert_eq!(url(img(&attrs, &env(1000.0, 1.5))).as_deref(), Some("2x.png"));
        assert_eq!(url(img(&attrs, &env(1000.0, 2.0))).as_deref(), Some("2x.png"));
        let densest = img(&attrs, &env(1000.0, 4.0)).expect("chosen");
        assert_eq!((densest.url.as_str(), densest.density), ("3x.png", 3.0));
        assert!(densest.imageset);
    }

    #[test]
    fn an_srcset_only_image_is_shown() {
        assert_eq!(
            url(img(&[("srcset", "a.png")], &env(1000.0, 1.0))).as_deref(),
            Some("a.png")
        );
    }

    #[test]
    fn width_descriptors_are_divided_by_the_source_size() {
        let attrs = [
            ("src", "fallback.png"),
            ("srcset", "small.png 400w, large.png 1600w"),
            ("sizes", "(max-width: 600px) 100vw, 400px"),
        ];
        // Wide: the slot is 400px, so small.png is exactly 1x.
        let wide = img(&attrs, &env(1000.0, 1.0)).expect("chosen");
        assert_eq!((wide.url.as_str(), wide.density), ("small.png", 1.0));
        // Narrow: the slot is the 500px viewport; small.png is 0.8x and does not cover it.
        assert_eq!(url(img(&attrs, &env(500.0, 1.0))).as_deref(), Some("large.png"));
        // Without `sizes` the slot is the whole viewport.
        let no_sizes = [("srcset", "small.png 400w, large.png 1600w")];
        assert_eq!(url(img(&no_sizes, &env(1000.0, 1.0))).as_deref(), Some("large.png"));
    }

    #[test]
    fn src_does_not_join_a_set_sized_by_width_or_with_its_own_1x() {
        let attrs = [("src", "src.png"), ("srcset", "w.png 2000w")];
        assert_eq!(url(img(&attrs, &env(1000.0, 1.0))).as_deref(), Some("w.png"));
        let attrs = [("src", "src.png"), ("srcset", "one.png 1x")];
        assert_eq!(url(img(&attrs, &env(1000.0, 1.0))).as_deref(), Some("one.png"));
    }

    #[test]
    fn srcset_parsing_follows_html() {
        let parsed = |s: &str| {
            parse_srcset(s)
                .into_iter()
                .map(|c| (c.url, c.descriptor))
                .collect::<Vec<_>>()
        };
        // A URL runs to whitespace, commas and all; only commas at its end end the candidate.
        assert_eq!(
            parsed("a,b.png 2x, c.png,d.png, e.png,"),
            vec![
                ("a,b.png".into(), Descriptor::Density(2.0)),
                ("c.png,d.png".into(), Descriptor::Density(1.0)),
                ("e.png".into(), Descriptor::Density(1.0)),
            ]
        );
        assert_eq!(
            parsed("  a.png  100w  "),
            vec![("a.png".into(), Descriptor::Width(100))]
        );
        assert_eq!(parsed("a.png 100w 50h"), vec![("a.png".into(), Descriptor::Width(100))]);
        // Errors drop the candidate, not the set.
        assert_eq!(
            parsed("bad.png 1x 2x, ok.png 2x"),
            vec![("ok.png".into(), Descriptor::Density(2.0))]
        );
        assert!(parsed("a.png 100w 1x").is_empty());
        assert!(parsed("a.png 50h").is_empty());
        assert!(parsed("a.png 0w").is_empty());
        assert!(parsed("a.png -1x").is_empty());
        assert!(parsed("a.png .5x").is_empty());
        assert!(parsed("a.png infx").is_empty());
        assert!(parsed("a.png 1.5wx").is_empty());
        assert!(parsed(" , ,").is_empty());
        assert!(parsed("a.png 2\u{e9}").is_empty());
    }

    #[test]
    fn sizes_take_the_first_matching_entry() {
        gosub_css3::media_query::set_media_environment(env(500.0, 1.0));
        assert_eq!(
            source_size::<Css3System>(Some("(max-width: 600px) 200px, 300px")),
            200.0
        );
        assert_eq!(
            source_size::<Css3System>(Some("(min-width: 600px) 200px, 300px")),
            300.0
        );
        assert_eq!(source_size::<Css3System>(Some("calc(100vw - 100px)")), 400.0);
        // An invalid entry is skipped, not fatal.
        assert_eq!(
            source_size::<Css3System>(Some("junk 1px, 50%, screen 10px, 120px")),
            120.0
        );
        assert_eq!(source_size::<Css3System>(Some("auto")), 500.0);
        assert_eq!(source_size::<Css3System>(None), 500.0);
    }

    #[test]
    fn a_picture_uses_its_first_matching_source() {
        let mut tree = Tree::default();
        let picture = tree.add(None, "picture", &[]);
        tree.add(
            Some(picture),
            "source",
            &[("srcset", "wide.png"), ("media", "(min-width: 800px)")],
        );
        tree.add(
            Some(picture),
            "source",
            &[("srcset", "photo.avif"), ("type", "image/avif")],
        );
        tree.add(
            Some(picture),
            "source",
            &[("srcset", "photo.webp"), ("type", "image/webp")],
        );
        let img = tree.add(Some(picture), "img", &[("src", "photo.jpg")]);
        tree.add(Some(picture), "source", &[("srcset", "after.png")]);

        let wide = pick(&tree, img, 1000.0, 1.0).expect("chosen");
        assert_eq!(wide.url, "wide.png");
        assert!(wide.imageset);
        // AVIF is not decoded here, so its source is passed over.
        assert_eq!(url(pick(&tree, img, 500.0, 1.0)).as_deref(), Some("photo.webp"));
    }

    #[test]
    fn a_picture_falls_back_to_its_img_and_that_is_still_an_imageset() {
        let mut tree = Tree::default();
        let picture = tree.add(None, "picture", &[]);
        tree.add(
            Some(picture),
            "source",
            &[("srcset", "never.png"), ("media", "not all")],
        );
        tree.add(
            Some(picture),
            "source",
            &[("srcset", "bad-media.png"), ("media", "(((")],
        );
        tree.add(Some(picture), "source", &[("src", "no-srcset.png")]);
        let img = tree.add(Some(picture), "img", &[("src", "photo.jpg")]);

        let chosen = pick(&tree, img, 1000.0, 1.0).expect("chosen");
        assert_eq!(chosen.url, "photo.jpg");
        assert!(chosen.imageset);
    }

    #[test]
    fn source_types_are_what_the_decoders_read() {
        assert!(decodes_image_type("image/png"));
        assert!(decodes_image_type("IMAGE/JPEG; q=1"));
        assert!(decodes_image_type("image/webp"));
        assert!(decodes_image_type("image/svg+xml"));
        assert!(decodes_image_type("image/apng"));
        assert!(!decodes_image_type("image/avif"));
        assert!(!decodes_image_type("image/jxl"));
        assert!(!decodes_image_type("image/tiff"));
        assert!(!decodes_image_type("text/html"));
    }
}
