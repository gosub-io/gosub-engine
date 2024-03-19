use lazy_static::lazy_static;

use crate::css_values::CssValue;

// Values for this table is taken from https://www.w3.org/TR/CSS21/propidx.html
// Probably not the complete list, but it will do for now

pub struct PropertyTableEntry {
    pub(crate) name: &'static str,
    pub(crate) initial: CssValue,
    pub(crate) inheritable: bool,
}

lazy_static! {
    pub static ref PROPERTY_TABLE: Vec<PropertyTableEntry> = vec![
        PropertyTableEntry {
            name: "azimuth",
            initial: CssValue::String("center".into()),
            inheritable: true,
        },
        PropertyTableEntry {
            name: "background-attachment",
            initial: CssValue::String("scroll".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "background-color",
            initial: CssValue::String("transparent".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "background-image",
            initial: CssValue::String("none".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "background-position",
            initial: CssValue::String("0% 0%".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "background-repeat",
            initial: CssValue::String("repeat".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-collapse",
            initial: CssValue::String("separate".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-color",
            initial: CssValue::String("initial".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-spacing",
            initial: CssValue::Number(0_f32),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-style",
            initial: CssValue::String("none".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-top",
            initial: CssValue::String("initial".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-right",
            initial: CssValue::String("initial".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-bottom",
            initial: CssValue::String("initial".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-left",
            initial: CssValue::String("initial".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-top-color",
            initial: CssValue::String("initial".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-right-color",
            initial: CssValue::String("initial".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-bottom-color",
            initial: CssValue::String("initial".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-left-color",
            initial: CssValue::String("initial".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-top-style",
            initial: CssValue::String("none".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-right-style",
            initial: CssValue::String("none".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-bottom-style",
            initial: CssValue::String("none".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-left-style",
            initial: CssValue::String("none".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-top-width",
            initial: CssValue::String("medium".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-right-width",
            initial: CssValue::String("medium".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-bottom-width",
            initial: CssValue::String("medium".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-left-width",
            initial: CssValue::String("medium".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-width",
            initial: CssValue::String("initial".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "bottom",
            initial: CssValue::String("auto".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "caption-side",
            initial: CssValue::String("top".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "clear",
            initial: CssValue::String("none".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "clip",
            initial: CssValue::String("auto".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "color",
            initial: CssValue::String("initial".into()),
            inheritable: true,
        },
        PropertyTableEntry {
            name: "content",
            initial: CssValue::String("normal".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "counter-increment",
            initial: CssValue::String("none".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "counter-reset",
            initial: CssValue::String("none".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "cue",
            initial: CssValue::String("none".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "cue-after",
            initial: CssValue::String("none".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "cue-before",
            initial: CssValue::String("none".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "cursor",
            initial: CssValue::String("auto".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "direction",
            initial: CssValue::String("ltr".into()),
            inheritable: true,
        },
        PropertyTableEntry {
            name: "display",
            initial: CssValue::String("inline".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "elevation",
            initial: CssValue::String("level".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "empty-cells",
            initial: CssValue::String("show".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "float",
            initial: CssValue::String("none".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "font",
            initial: CssValue::String("initial".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "font-family",
            initial: CssValue::String("initial".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "font-size",
            initial: CssValue::String("medium".into()),
            inheritable: true,
        },
        PropertyTableEntry {
            name: "font-style",
            initial: CssValue::String("normal".into()),
            inheritable: true,
        },
        PropertyTableEntry {
            name: "font-variant",
            initial: CssValue::String("normal".into()),
            inheritable: true,
        },
        PropertyTableEntry {
            name: "font-weight",
            initial: CssValue::String("normal".into()),
            inheritable: true,
        },
        PropertyTableEntry {
            name: "height",
            initial: CssValue::String("auto".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "left",
            initial: CssValue::String("auto".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "letter-spacing",
            initial: CssValue::String("normal".into()),
            inheritable: true,
        },
        PropertyTableEntry {
            name: "line-height",
            initial: CssValue::String("normal".into()),
            inheritable: true,
        },
        PropertyTableEntry {
            name: "list-style",
            initial: CssValue::String("initial".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "list-style-image",
            initial: CssValue::String("none".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "list-style-position",
            initial: CssValue::String("outside".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "list-style-type",
            initial: CssValue::String("disc".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "margin",
            initial: CssValue::Number(0_f32),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "margin-top",
            initial: CssValue::Number(0_f32),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "margin-right",
            initial: CssValue::Number(0_f32),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "margin-bottom",
            initial: CssValue::Number(0_f32),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "margin-left",
            initial: CssValue::Number(0_f32),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "max-height",
            initial: CssValue::String("none".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "max-width",
            initial: CssValue::String("none".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "min-height",
            initial: CssValue::Number(0_f32),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "min-width",
            initial: CssValue::Number(0_f32),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "orphans",
            initial: CssValue::Number(2_f32),
            inheritable: true,
        },
        PropertyTableEntry {
            name: "outline",
            initial: CssValue::String("initial".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "outline-color",
            initial: CssValue::String("invert".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "outline-style",
            initial: CssValue::String("none".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "outline-width",
            initial: CssValue::String("medium".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "overflow",
            initial: CssValue::String("visible".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "padding",
            initial: CssValue::Number(0_f32),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "padding-top",
            initial: CssValue::Number(0_f32),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "padding-right",
            initial: CssValue::Number(0_f32),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "padding-bottom",
            initial: CssValue::Number(0_f32),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "padding-left",
            initial: CssValue::Number(0_f32),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "page-break-after",
            initial: CssValue::String("auto".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "page-break-before",
            initial: CssValue::String("auto".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "page-break-inside",
            initial: CssValue::String("auto".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "pause-after",
            initial: CssValue::Number(0_f32),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "pause-before",
            initial: CssValue::Number(0_f32),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "pitch",
            initial: CssValue::String("medium".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "pitch-range",
            initial: CssValue::Number(50_f32),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "play-during",
            initial: CssValue::String("auto".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "position",
            initial: CssValue::String("static".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "quotes",
            initial: CssValue::String("initial".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "richness",
            initial: CssValue::Number(50_f32),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "right",
            initial: CssValue::String("auto".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "speak",
            initial: CssValue::String("normal".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "speak-header",
            initial: CssValue::String("once".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "speak-numeral",
            initial: CssValue::String("continuous".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "speak-punctuation",
            initial: CssValue::String("none".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "speech-rate",
            initial: CssValue::String("medium".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "stress",
            initial: CssValue::Number(50_f32),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "table-layout",
            initial: CssValue::String("auto".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "text-align",
            initial: CssValue::String("initial".into()),
            inheritable: true,
        },
        PropertyTableEntry {
            name: "text-decoration",
            initial: CssValue::String("none".into()),
            inheritable: true,
        },
        PropertyTableEntry {
            name: "text-indent",
            initial: CssValue::Number(0_f32),
            inheritable: true,
        },
        PropertyTableEntry {
            name: "text-transform",
            initial: CssValue::String("none".into()),
            inheritable: true,
        },
        PropertyTableEntry {
            name: "top",
            initial: CssValue::String("auto".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "unicode-bidi",
            initial: CssValue::String("normal".into()),
            inheritable: true,
        },
        PropertyTableEntry {
            name: "vertical-align",
            initial: CssValue::String("baseline".into()),
            inheritable: true,
        },
        PropertyTableEntry {
            name: "visibility",
            initial: CssValue::String("visible".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "voice-family",
            initial: CssValue::String("initial".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "volume",
            initial: CssValue::String("medium".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "white-space",
            initial: CssValue::String("normal".into()),
            inheritable: true,
        },
        PropertyTableEntry {
            name: "widows",
            initial: CssValue::Number(2_f32),
            inheritable: true,
        },
        PropertyTableEntry {
            name: "width",
            initial: CssValue::String("auto".into()),
            inheritable: false,
        },
        PropertyTableEntry {
            name: "word-spacing",
            initial: CssValue::String("normal".into()),
            inheritable: true,
        },
        PropertyTableEntry {
            name: "z-index",
            initial: CssValue::String("auto".into()),
            inheritable: false,
        },
    ];
}
