use lazy_static::lazy_static;

// Values for this table is taken from https://www.w3.org/TR/CSS21/propidx.html
// Probably not the complete list, but it will do for now

struct PropertyTableEntry {
    name: &'static str,
    initial: &'static str,
    inheritable: bool,
}

lazy_static! {
    static ref PROPERTY_TABLE: &'static [PropertyTableEntry] = &[
        PropertyTableEntry {
            name: "azimuth",
            initial: "center",
            inheritable: true,
        },
        PropertyTableEntry {
            name: "background-attachment",
            initial: "scroll",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "background-color",
            initial: "transparent",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "background-image",
            initial: "none",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "background-position",
            initial: "0% 0%",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "background-repeat",
            initial: "repeat",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-collapse",
            initial: "separate",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-color",
            initial: "initial",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-spacing",
            initial: "0",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-style",
            initial: "none",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-top",
            initial: "initial",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-right",
            initial: "initial",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-bottom",
            initial: "initial",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-left",
            initial: "initial",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-top-color",
            initial: "initial",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-right-color",
            initial: "initial",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-bottom-color",
            initial: "initial",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-left-color",
            initial: "initial",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-top-style",
            initial: "none",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-right-style",
            initial: "none",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-bottom-style",
            initial: "none",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-left-style",
            initial: "none",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-top-width",
            initial: "medium",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-right-width",
            initial: "medium",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-bottom-width",
            initial: "medium",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-left-width",
            initial: "medium",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "border-width",
            initial: "initial",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "bottom",
            initial: "auto",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "caption-side",
            initial: "top",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "clear",
            initial: "none",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "clip",
            initial: "auto",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "color",
            initial: "initial",
            inheritable: true,
        },
        PropertyTableEntry {
            name: "content",
            initial: "normal",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "counter-increment",
            initial: "none",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "counter-reset",
            initial: "none",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "cue",
            initial: "none",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "cue-after",
            initial: "none",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "cue-before",
            initial: "none",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "cursor",
            initial: "auto",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "direction",
            initial: "ltr",
            inheritable: true,
        },
        PropertyTableEntry {
            name: "display",
            initial: "inline",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "elevation",
            initial: "level",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "empty-cells",
            initial: "show",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "float",
            initial: "none",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "font",
            initial: "initial",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "font-family",
            initial: "initial",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "font-size",
            initial: "medium",
            inheritable: true,
        },
        PropertyTableEntry {
            name: "font-style",
            initial: "normal",
            inheritable: true,
        },
        PropertyTableEntry {
            name: "font-variant",
            initial: "normal",
            inheritable: true,
        },
        PropertyTableEntry {
            name: "font-weight",
            initial: "normal",
            inheritable: true,
        },
        PropertyTableEntry {
            name: "height",
            initial: "auto",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "left",
            initial: "auto",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "letter-spacing",
            initial: "normal",
            inheritable: true,
        },
        PropertyTableEntry {
            name: "line-height",
            initial: "normal",
            inheritable: true,
        },
        PropertyTableEntry {
            name: "list-style",
            initial: "initial",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "list-style-image",
            initial: "none",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "list-style-position",
            initial: "outside",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "list-style-type",
            initial: "disc",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "margin",
            initial: "0",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "margin-top",
            initial: "0",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "margin-right",
            initial: "0",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "margin-bottom",
            initial: "0",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "margin-left",
            initial: "0",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "max-height",
            initial: "none",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "max-width",
            initial: "none",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "min-height",
            initial: "0",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "min-width",
            initial: "0",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "orphans",
            initial: "2",
            inheritable: true,
        },
        PropertyTableEntry {
            name: "outline",
            initial: "initial",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "outline-color",
            initial: "invert",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "outline-style",
            initial: "none",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "outline-width",
            initial: "medium",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "overflow",
            initial: "visible",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "padding",
            initial: "0",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "padding-top",
            initial: "0",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "padding-right",
            initial: "0",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "padding-bottom",
            initial: "0",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "padding-left",
            initial: "0",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "page-break-after",
            initial: "auto",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "page-break-before",
            initial: "auto",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "page-break-inside",
            initial: "auto",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "pause-after",
            initial: "0",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "pause-before",
            initial: "0",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "pitch",
            initial: "medium",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "pitch-range",
            initial: "50",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "play-during",
            initial: "auto",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "position",
            initial: "static",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "quotes",
            initial: "initial",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "richness",
            initial: "50",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "right",
            initial: "auto",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "speak",
            initial: "normal",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "speak-header",
            initial: "once",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "speak-numeral",
            initial: "continuous",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "speak-punctuation",
            initial: "none",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "speech-rate",
            initial: "medium",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "stress",
            initial: "50",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "table-layout",
            initial: "auto",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "text-align",
            initial: "initial",
            inheritable: true,
        },
        PropertyTableEntry {
            name: "text-decoration",
            initial: "none",
            inheritable: true,
        },
        PropertyTableEntry {
            name: "text-indent",
            initial: "0",
            inheritable: true,
        },
        PropertyTableEntry {
            name: "text-transform",
            initial: "none",
            inheritable: true,
        },
        PropertyTableEntry {
            name: "top",
            initial: "auto",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "unicode-bidi",
            initial: "normal",
            inheritable: true,
        },
        PropertyTableEntry {
            name: "vertical-align",
            initial: "baseline",
            inheritable: true,
        },
        PropertyTableEntry {
            name: "visibility",
            initial: "visible",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "voice-family",
            initial: "initial",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "volume",
            initial: "medium",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "white-space",
            initial: "normal",
            inheritable: true,
        },
        PropertyTableEntry {
            name: "widows",
            initial: "2",
            inheritable: true,
        },
        PropertyTableEntry {
            name: "width",
            initial: "auto",
            inheritable: false,
        },
        PropertyTableEntry {
            name: "word-spacing",
            initial: "normal",
            inheritable: true,
        },
        PropertyTableEntry {
            name: "z-index",
            initial: "auto",
            inheritable: false,
        },
    ];
}


#[allow(dead_code)]
fn get_initial_value(property: &str) -> Option<&'static str> {
    PROPERTY_TABLE.iter().find(|entry| entry.name == property).map(|entry| entry.initial)
}

#[allow(dead_code)]
fn is_inheritable(property: &str) -> bool {
    PROPERTY_TABLE.iter().find(|entry| entry.name == property).map(|entry| entry.inheritable).unwrap_or(false)
}