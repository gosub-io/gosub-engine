/// This file contains the shorthand properties and their expanded properties.
use std::collections::HashMap;
use lazy_static::lazy_static;

lazy_static! {
    static ref SHORTHAND_PROPERTIES: HashMap<&'static str, Vec<&'static str>> = {
        let mut m = HashMap::new();
        m.insert("background", vec!["background-color", "background-image", "background-repeat", "background-attachment", "background-position", "background-size"]);
        m.insert("border", vec!["border-width", "border-style", "border-color"]);
        m.insert("border-radius", vec!["border-top-left-radius", "border-top-right-radius", "border-bottom-right-radius", "border-bottom-left-radius"]);
        m.insert("border-width", vec!["border-top-width", "border-right-width", "border-bottom-width", "border-left-width"]);
        m.insert("border-style", vec!["border-top-style", "border-right-style", "border-bottom-style", "border-left-style"]);
        m.insert("border-color", vec!["border-top-color", "border-right-color", "border-bottom-color", "border-left-color"]);
        m.insert("margin", vec!["margin-top", "margin-right", "margin-bottom", "margin-left"]);
        m.insert("padding", vec!["padding-top", "padding-right", "padding-bottom", "padding-left"]);
        m.insert("font", vec!["font-style", "font-variant", "font-weight", "font-size", "line-height", "font-family"]);
        m.insert("list-style", vec!["list-style-type", "list-style-position", "list-style-image"]);
        m.insert("outline", vec!["outline-width", "outline-style", "outline-color"]);
        m.insert("overflow", vec!["overflow-x", "overflow-y"]);
        m.insert("text-decoration", vec!["text-decoration-line", "text-decoration-style", "text-decoration-color", "text-decoration-thickness", "text-underline-position"]);
        m.insert("transition", vec!["transition-property", "transition-duration", "transition-timing-function", "transition-delay"]);
        m.insert("border-block-end", vec!["border-block-end-width", "border-block-end-style", "border-block-end-color"]);
        m.insert("border-block-start", vec!["border-block-start-width", "border-block-start-style", "border-block-start-color"]);
        m.insert("border-bottom", vec!["border-bottom-width", "border-bottom-style", "border-bottom-color"]);
        m.insert("border-image", vec!["border-image-source", "border-image-slice", "border-image-width", "border-image-outset", "border-image-repeat"]);
        m.insert("border-inline-end", vec!["border-inline-end-width", "border-inline-end-style", "border-inline-end-color"]);
        m.insert("border-inline-start", vec!["border-inline-start-width", "border-inline-start-style", "border-inline-start-color"]);
        m.insert("border-left", vec!["border-left-width", "border-left-style", "border-left-color"]);
        m.insert("border-right", vec!["border-right-width", "border-right-style", "border-right-color"]);
        m.insert("border-top", vec!["border-top-width", "border-top-style", "border-top-color"]);
        m.insert("column-rule", vec!["column-rule-width", "column-rule-style", "column-rule-color"]);
        m.insert("columns", vec!["column-width", "column-count"]);
        m.insert("container", vec!["container-type", "container-name"]);
        m.insert("contain-intrinsic-size", vec!["width", "height"]);
        m.insert("flex", vec!["flex-grow", "flex-shrink", "flex-basis"]);
        m.insert("flex-flow", vec!["flex-direction", "flex-wrap"]);
        m.insert("font-synthesis", vec!["font-synthesis-weight", "font-synthesis-style", "font-synthesis-small-caps"]);
        m.insert("font-variant", vec!["font-variant-ligatures", "font-variant-caps", "font-variant-numeric", "font-variant-east-asian"]);
        m.insert("gap", vec!["row-gap", "column-gap"]);
        m.insert("grid", vec!["grid-template-rows", "grid-template-columns", "grid-template-areas", "grid-auto-rows", "grid-auto-columns", "grid-auto-flow"]);
        m.insert("grid-area", vec!["grid-row-start", "grid-column-start", "grid-row-end", "grid-column-end"]);
        m.insert("grid-column", vec!["grid-column-start", "grid-column-end"]);
        m.insert("grid-row", vec!["grid-row-start", "grid-row-end"]);
        m.insert("grid-template", vec!["grid-template-columns", "grid-template-rows", "grid-template-areas"]);
        m.insert("inset", vec!["top", "right", "bottom", "left"]);
        m.insert("mask", vec!["mask-image", "mask-mode", "mask-position", "mask-size", "mask-repeat", "mask-origin", "mask-clip"]);
        m.insert("mask-border", vec!["mask-border-source", "mask-border-slice", "mask-border-width", "mask-border-outset", "mask-border-repeat", "mask-border-mode"]);
        m.insert("offset", vec!["offset-position", "offset-path", "offset-distance", "offset-rotate", "offset-anchor"]);
        m.insert("place-content", vec!["align-content", "justify-content"]);
        m.insert("place-items", vec!["align-items", "justify-items"]);
        m.insert("place-self", vec!["align-self", "justify-self"]);
        m.insert("scroll-margin", vec!["scroll-margin-top", "scroll-margin-right", "scroll-margin-bottom", "scroll-margin-left"]);
        m.insert("scroll-padding", vec!["scroll-padding-top", "scroll-padding-right", "scroll-padding-bottom", "scroll-padding-left"]);
        m.insert("scroll-timeline", vec!["timeline-name", "timeline-axis", "timeline-range", "timeline-progress"]);
        m.insert("text-emphasis", vec!["text-emphasis-style", "text-emphasis-color"]);
        m
    };
}