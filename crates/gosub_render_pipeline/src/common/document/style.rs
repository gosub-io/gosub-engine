use parking_lot::Mutex;
use std::sync::OnceLock;

// ── String interner ──────────────────────────────────────────────────────────

static INTERNER: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

fn interner() -> &'static Mutex<Vec<String>> {
    INTERNER.get_or_init(|| Mutex::new(Vec::with_capacity(64)))
}

/// Intern a string and return its stable u32 id. O(n) scan — table stays tiny.
pub fn intern(s: &str) -> u32 {
    let mut table = interner().lock();
    if let Some(i) = table.iter().position(|x| x == s) {
        return i as u32;
    }
    let id = table.len() as u32;
    table.push(s.to_string());
    id
}

/// Look up a previously-interned string by id.
pub fn lookup(id: u32) -> String {
    interner().lock()[id as usize].clone()
}

// ── Sub-enums (unchanged from before) ────────────────────────────────────────

#[allow(unused)]
#[derive(Clone, Debug, PartialEq)]
pub enum Unit {
    Px,
    Em,
    Rem,
    Percent,
}

#[allow(unused)]
#[derive(Debug, Clone, PartialEq)]
pub enum BorderStyle {
    None,
    Hidden,
    Solid,
    Dashed,
    Dotted,
    Double,
    Groove,
    Ridge,
    Inset,
    Outset,
}

#[allow(unused)]
#[derive(Clone, Debug, PartialEq)]
pub enum Display {
    Block,
    Inline,
    InlineBlock,
    None,
    Flex,
    InlineFlex,
    Grid,
    InlineGrid,
    Table,
    TableCaption,
    TableCell,
    TableFooterGroup,
    TableHeaderGroup,
    TableRow,
    TableRowGroup,
}

#[allow(unused)]
#[derive(Debug, Clone, PartialEq)]
pub enum FontWeight {
    Normal,
    Bold,
    Bolder,
    Lighter,
    Number(f32),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TextAlign {
    Left,
    Right,
    Center,
    Justify,
    Start,
    End,
    MatchParent,
    Initial,
    Inherit,
    Revert,
    Unset,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TextWrap {
    Wrap,
    NoWrap,
    Balance,
    Pretty,
    Stable,
    Initial,
    Inherit,
    Revert,
    RevertLayer,
    Unset,
}

// ── Value — replaces StyleValue, ≤8 bytes, zero heap ─────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A length with a unit (px, em, rem, %).
    Unit(f32, Unit),
    /// RGBA colour (each channel 0-255; alpha 255 = fully opaque).
    Color(u8, u8, u8, u8),
    /// Unitless number (flex-grow, flex-shrink, aspect-ratio, …).
    Number(f32),
    /// Percentage value.
    Percentage(f32),
    /// A display mode.
    Display(Display),
    /// Font weight.
    FontWeight(FontWeight),
    /// Text-align value.
    TextAlign(TextAlign),
    /// Text-wrap value.
    TextWrap(TextWrap),
    /// Border style.
    BorderStyle(BorderStyle),
    /// An interned keyword string (font-family, position, flex-direction, …).
    Keyword(u32),
}

impl Value {
    /// Convenience: build a keyword Value by interning the string.
    pub fn keyword(s: &str) -> Self {
        Value::Keyword(intern(s))
    }

    pub fn to_css_string(&self) -> String {
        match self {
            Value::Unit(v, unit) => {
                let suffix = match unit {
                    Unit::Px => "px",
                    Unit::Em => "em",
                    Unit::Rem => "rem",
                    Unit::Percent => "%",
                };
                format!("{v}{suffix}")
            }
            Value::Color(r, g, b, 255) => format!("rgb({r}, {g}, {b})"),
            Value::Color(r, g, b, a) => {
                let af = *a as f32 / 255.0;
                format!("rgba({r}, {g}, {b}, {af:.4})")
            }
            Value::Number(v) => format!("{v}"),
            Value::Percentage(v) => format!("{v}%"),
            Value::Display(d) => match d {
                Display::Block => "block",
                Display::Inline => "inline",
                Display::InlineBlock => "inline-block",
                Display::None => "none",
                Display::Flex => "flex",
                Display::InlineFlex => "inline-flex",
                Display::Grid => "grid",
                Display::InlineGrid => "inline-grid",
                Display::Table => "table",
                Display::TableCaption => "table-caption",
                Display::TableCell => "table-cell",
                Display::TableFooterGroup => "table-footer-group",
                Display::TableHeaderGroup => "table-header-group",
                Display::TableRow => "table-row",
                Display::TableRowGroup => "table-row-group",
            }
            .to_string(),
            Value::FontWeight(fw) => match fw {
                FontWeight::Normal => "normal".to_string(),
                FontWeight::Bold => "bold".to_string(),
                FontWeight::Bolder => "bolder".to_string(),
                FontWeight::Lighter => "lighter".to_string(),
                FontWeight::Number(n) => format!("{n}"),
            },
            Value::TextAlign(ta) => match ta {
                TextAlign::Left => "left",
                TextAlign::Right => "right",
                TextAlign::Center => "center",
                TextAlign::Justify => "justify",
                TextAlign::Start => "start",
                TextAlign::End => "end",
                TextAlign::MatchParent => "match-parent",
                TextAlign::Initial => "initial",
                TextAlign::Inherit => "inherit",
                TextAlign::Revert => "revert",
                TextAlign::Unset => "unset",
            }
            .to_string(),
            Value::TextWrap(tw) => match tw {
                TextWrap::Wrap => "wrap",
                TextWrap::NoWrap => "nowrap",
                TextWrap::Balance => "balance",
                TextWrap::Pretty => "pretty",
                TextWrap::Stable => "stable",
                TextWrap::Initial => "initial",
                TextWrap::Inherit => "inherit",
                TextWrap::Revert => "revert",
                TextWrap::RevertLayer => "revert-layer",
                TextWrap::Unset => "unset",
            }
            .to_string(),
            Value::BorderStyle(bs) => match bs {
                BorderStyle::None => "none",
                BorderStyle::Hidden => "hidden",
                BorderStyle::Solid => "solid",
                BorderStyle::Dashed => "dashed",
                BorderStyle::Dotted => "dotted",
                BorderStyle::Double => "double",
                BorderStyle::Groove => "groove",
                BorderStyle::Ridge => "ridge",
                BorderStyle::Inset => "inset",
                BorderStyle::Outset => "outset",
            }
            .to_string(),
            Value::Keyword(id) => lookup(*id),
        }
    }
}

// ── StyleProperty ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum StyleProperty {
    Color,
    BackgroundColor,
    FontSize,
    FontWeight,
    Display,
    Width,
    Height,
    MarginTop,
    MarginRight,
    MarginBottom,
    MarginLeft,
    PaddingTop,
    PaddingRight,
    PaddingBottom,
    PaddingLeft,
    BorderBottomWidth,
    BorderTopWidth,
    BorderLeftWidth,
    BorderRightWidth,
    BorderBottomColor,
    BorderTopColor,
    BorderLeftColor,
    BorderRightColor,
    BorderTopStyle,
    BorderRightStyle,
    BorderBottomStyle,
    BorderLeftStyle,
    FontFamily,
    FlexBasis,
    FlexDirection,
    FlexGrow,
    FlexShrink,
    FlexWrap,
    ScrollbarWidth,
    Position,
    MinWidth,
    MinHeight,
    MaxWidth,
    MaxHeight,
    BorderBottomLeftRadius,
    BorderBottomRightRadius,
    BorderTopLeftRadius,
    BorderTopRightRadius,
    AspectRatio,
    Gap,
    AlignItems,
    AlignSelf,
    AlignContent,
    TextAlign,
    InsetBlockEnd,
    InsetBlockStart,
    InsetInlineEnd,
    InsetInlineStart,
    JustifyItems,
    JustifySelf,
    JustifyContent,
    OverflowX,
    OverflowY,
    BoxSizing,
    LineHeight,
    TextWrap,
    GridRow,
    GridColumn,
    GridAutoFlow,
    GridTemplateRows,
    GridTemplateColumns,
    GridAutoRows,
    GridAutoColumns,
    FontStyle,
    WhiteSpace,
    TextDecorationLine,
    BackgroundImage,
    Content,
    Opacity,
    TextTransform,
}

impl StyleProperty {
    /// Returns the stable u8 id for this property (index into PROPERTIES).
    pub fn id(&self) -> u8 {
        match self {
            StyleProperty::Color => 0,
            StyleProperty::BackgroundColor => 1,
            StyleProperty::FontSize => 2,
            StyleProperty::FontWeight => 3,
            StyleProperty::Display => 4,
            StyleProperty::Width => 5,
            StyleProperty::Height => 6,
            StyleProperty::MarginTop => 7,
            StyleProperty::MarginRight => 8,
            StyleProperty::MarginBottom => 9,
            StyleProperty::MarginLeft => 10,
            StyleProperty::PaddingTop => 11,
            StyleProperty::PaddingRight => 12,
            StyleProperty::PaddingBottom => 13,
            StyleProperty::PaddingLeft => 14,
            StyleProperty::BorderTopWidth => 15,
            StyleProperty::BorderRightWidth => 16,
            StyleProperty::BorderBottomWidth => 17,
            StyleProperty::BorderLeftWidth => 18,
            StyleProperty::BorderTopColor => 19,
            StyleProperty::BorderRightColor => 20,
            StyleProperty::BorderBottomColor => 21,
            StyleProperty::BorderLeftColor => 22,
            StyleProperty::BorderTopStyle => 23,
            StyleProperty::BorderRightStyle => 24,
            StyleProperty::BorderBottomStyle => 25,
            StyleProperty::BorderLeftStyle => 26,
            StyleProperty::FontFamily => 27,
            StyleProperty::FlexBasis => 28,
            StyleProperty::FlexDirection => 29,
            StyleProperty::FlexGrow => 30,
            StyleProperty::FlexShrink => 31,
            StyleProperty::FlexWrap => 32,
            StyleProperty::ScrollbarWidth => 33,
            StyleProperty::Position => 34,
            StyleProperty::MinWidth => 35,
            StyleProperty::MinHeight => 36,
            StyleProperty::MaxWidth => 37,
            StyleProperty::MaxHeight => 38,
            StyleProperty::BorderTopLeftRadius => 39,
            StyleProperty::BorderTopRightRadius => 40,
            StyleProperty::BorderBottomLeftRadius => 41,
            StyleProperty::BorderBottomRightRadius => 42,
            StyleProperty::AspectRatio => 43,
            StyleProperty::Gap => 44,
            StyleProperty::AlignItems => 45,
            StyleProperty::AlignSelf => 46,
            StyleProperty::AlignContent => 47,
            StyleProperty::TextAlign => 48,
            StyleProperty::InsetBlockEnd => 49,
            StyleProperty::InsetBlockStart => 50,
            StyleProperty::InsetInlineEnd => 51,
            StyleProperty::InsetInlineStart => 52,
            StyleProperty::JustifyItems => 53,
            StyleProperty::JustifySelf => 54,
            StyleProperty::JustifyContent => 55,
            StyleProperty::OverflowX => 56,
            StyleProperty::OverflowY => 57,
            StyleProperty::BoxSizing => 58,
            StyleProperty::LineHeight => 59,
            StyleProperty::TextWrap => 60,
            StyleProperty::GridRow => 61,
            StyleProperty::GridColumn => 62,
            StyleProperty::GridAutoFlow => 63,
            StyleProperty::GridTemplateRows => 64,
            StyleProperty::GridTemplateColumns => 65,
            StyleProperty::GridAutoRows => 66,
            StyleProperty::GridAutoColumns => 67,
            StyleProperty::FontStyle => 68,
            StyleProperty::WhiteSpace => 69,
            StyleProperty::TextDecorationLine => 70,
            StyleProperty::BackgroundImage => 71,
            StyleProperty::Content => 72,
            StyleProperty::Opacity => 73,
            StyleProperty::TextTransform => 74,
        }
    }

    /// Returns the metadata for this property (name, initial value, inherited flag).
    pub fn meta(&self) -> &'static PropertyMeta {
        property_meta(self.id())
    }

    pub fn css_name(&self) -> &'static str {
        self.meta().name
    }
}

// ── Property registry ─────────────────────────────────────────────────────────

pub struct PropertyMeta {
    pub name: &'static str,
    pub inherited: bool,
    /// CSS-spec initial value, expressed without heap allocation.
    /// Keyword values use a static str that is interned on first `initial_value()` call.
    pub initial_kind: InitialKind,
}

impl PropertyMeta {
    /// Returns the initial value as a `Value`, interning any keyword string on demand.
    pub fn initial_value(&self) -> Value {
        self.initial_kind.to_value()
    }
}

/// Like `Value` but keywords are `&'static str` so the registry compiles as a static.
#[derive(Clone)]
pub enum InitialKind {
    Unit(f32, Unit),
    Color(u8, u8, u8, u8),
    Number(f32),
    Display(Display),
    FontWeight(FontWeight),
    TextAlign(TextAlign),
    TextWrap(TextWrap),
    BorderStyle(BorderStyle),
    Keyword(&'static str),
}

impl InitialKind {
    pub fn to_value(&self) -> Value {
        match self {
            InitialKind::Unit(v, u) => Value::Unit(*v, u.clone()),
            InitialKind::Color(r, g, b, a) => Value::Color(*r, *g, *b, *a),
            InitialKind::Number(v) => Value::Number(*v),
            InitialKind::Display(d) => Value::Display(d.clone()),
            InitialKind::FontWeight(fw) => Value::FontWeight(fw.clone()),
            InitialKind::TextAlign(ta) => Value::TextAlign(ta.clone()),
            InitialKind::TextWrap(tw) => Value::TextWrap(tw.clone()),
            InitialKind::BorderStyle(bs) => Value::BorderStyle(bs.clone()),
            InitialKind::Keyword(s) => Value::Keyword(intern(s)),
        }
    }
}

pub fn property_meta(id: u8) -> &'static PropertyMeta {
    &PROPERTIES[id as usize]
}

// CSS-spec initial values and inherited flags for all 68 known properties.
// Order MUST match StyleProperty::id().
static PROPERTIES: &[PropertyMeta] = &[
    // 0  color — inherited; initial = black
    PropertyMeta {
        name: "color",
        inherited: true,
        initial_kind: InitialKind::Color(0, 0, 0, 255),
    },
    // 1  background-color — not inherited; initial = transparent
    PropertyMeta {
        name: "background-color",
        inherited: false,
        initial_kind: InitialKind::Color(0, 0, 0, 0),
    },
    // 2  font-size — inherited; initial = medium = 16px
    PropertyMeta {
        name: "font-size",
        inherited: true,
        initial_kind: InitialKind::Unit(16.0, Unit::Px),
    },
    // 3  font-weight — inherited; initial = normal (400)
    PropertyMeta {
        name: "font-weight",
        inherited: true,
        initial_kind: InitialKind::FontWeight(FontWeight::Normal),
    },
    // 4  display — not inherited; initial = inline (UA stylesheet makes block elements block)
    PropertyMeta {
        name: "display",
        inherited: false,
        initial_kind: InitialKind::Display(Display::Inline),
    },
    // 5  width — not inherited; initial = auto
    PropertyMeta {
        name: "width",
        inherited: false,
        initial_kind: InitialKind::Keyword("auto"),
    },
    // 6  height — not inherited; initial = auto
    PropertyMeta {
        name: "height",
        inherited: false,
        initial_kind: InitialKind::Keyword("auto"),
    },
    // 7  margin-top — not inherited; initial = 0
    PropertyMeta {
        name: "margin-top",
        inherited: false,
        initial_kind: InitialKind::Unit(0.0, Unit::Px),
    },
    // 8  margin-right
    PropertyMeta {
        name: "margin-right",
        inherited: false,
        initial_kind: InitialKind::Unit(0.0, Unit::Px),
    },
    // 9  margin-bottom
    PropertyMeta {
        name: "margin-bottom",
        inherited: false,
        initial_kind: InitialKind::Unit(0.0, Unit::Px),
    },
    // 10 margin-left
    PropertyMeta {
        name: "margin-left",
        inherited: false,
        initial_kind: InitialKind::Unit(0.0, Unit::Px),
    },
    // 11 padding-top — not inherited; initial = 0
    PropertyMeta {
        name: "padding-top",
        inherited: false,
        initial_kind: InitialKind::Unit(0.0, Unit::Px),
    },
    // 12 padding-right
    PropertyMeta {
        name: "padding-right",
        inherited: false,
        initial_kind: InitialKind::Unit(0.0, Unit::Px),
    },
    // 13 padding-bottom
    PropertyMeta {
        name: "padding-bottom",
        inherited: false,
        initial_kind: InitialKind::Unit(0.0, Unit::Px),
    },
    // 14 padding-left
    PropertyMeta {
        name: "padding-left",
        inherited: false,
        initial_kind: InitialKind::Unit(0.0, Unit::Px),
    },
    // 15 border-top-width — not inherited; initial = medium = 3px (but only applies when border-style != none)
    PropertyMeta {
        name: "border-top-width",
        inherited: false,
        initial_kind: InitialKind::Unit(0.0, Unit::Px),
    },
    // 16 border-right-width
    PropertyMeta {
        name: "border-right-width",
        inherited: false,
        initial_kind: InitialKind::Unit(0.0, Unit::Px),
    },
    // 17 border-bottom-width
    PropertyMeta {
        name: "border-bottom-width",
        inherited: false,
        initial_kind: InitialKind::Unit(0.0, Unit::Px),
    },
    // 18 border-left-width
    PropertyMeta {
        name: "border-left-width",
        inherited: false,
        initial_kind: InitialKind::Unit(0.0, Unit::Px),
    },
    // 19 border-top-color — not inherited; initial = currentColor (black)
    PropertyMeta {
        name: "border-top-color",
        inherited: false,
        initial_kind: InitialKind::Color(0, 0, 0, 255),
    },
    // 20 border-right-color
    PropertyMeta {
        name: "border-right-color",
        inherited: false,
        initial_kind: InitialKind::Color(0, 0, 0, 255),
    },
    // 21 border-bottom-color
    PropertyMeta {
        name: "border-bottom-color",
        inherited: false,
        initial_kind: InitialKind::Color(0, 0, 0, 255),
    },
    // 22 border-left-color
    PropertyMeta {
        name: "border-left-color",
        inherited: false,
        initial_kind: InitialKind::Color(0, 0, 0, 255),
    },
    // 23 border-top-style — not inherited; initial = none
    PropertyMeta {
        name: "border-top-style",
        inherited: false,
        initial_kind: InitialKind::BorderStyle(BorderStyle::None),
    },
    // 24 border-right-style
    PropertyMeta {
        name: "border-right-style",
        inherited: false,
        initial_kind: InitialKind::BorderStyle(BorderStyle::None),
    },
    // 25 border-bottom-style
    PropertyMeta {
        name: "border-bottom-style",
        inherited: false,
        initial_kind: InitialKind::BorderStyle(BorderStyle::None),
    },
    // 26 border-left-style
    PropertyMeta {
        name: "border-left-style",
        inherited: false,
        initial_kind: InitialKind::BorderStyle(BorderStyle::None),
    },
    // 27 font-family — inherited; initial = implementation-dependent
    PropertyMeta {
        name: "font-family",
        inherited: true,
        initial_kind: InitialKind::Keyword("serif"),
    },
    // 28 flex-basis — not inherited; initial = auto
    PropertyMeta {
        name: "flex-basis",
        inherited: false,
        initial_kind: InitialKind::Keyword("auto"),
    },
    // 29 flex-direction — not inherited; initial = row
    PropertyMeta {
        name: "flex-direction",
        inherited: false,
        initial_kind: InitialKind::Keyword("row"),
    },
    // 30 flex-grow — not inherited; initial = 0
    PropertyMeta {
        name: "flex-grow",
        inherited: false,
        initial_kind: InitialKind::Number(0.0),
    },
    // 31 flex-shrink — not inherited; initial = 1
    PropertyMeta {
        name: "flex-shrink",
        inherited: false,
        initial_kind: InitialKind::Number(1.0),
    },
    // 32 flex-wrap — not inherited; initial = nowrap
    PropertyMeta {
        name: "flex-wrap",
        inherited: false,
        initial_kind: InitialKind::Keyword("nowrap"),
    },
    // 33 scrollbar-width — not inherited; initial = auto
    PropertyMeta {
        name: "scrollbar-width",
        inherited: false,
        initial_kind: InitialKind::Keyword("auto"),
    },
    // 34 position — not inherited; initial = static
    PropertyMeta {
        name: "position",
        inherited: false,
        initial_kind: InitialKind::Keyword("static"),
    },
    // 35 min-width — not inherited; initial = 0
    PropertyMeta {
        name: "min-width",
        inherited: false,
        initial_kind: InitialKind::Unit(0.0, Unit::Px),
    },
    // 36 min-height
    PropertyMeta {
        name: "min-height",
        inherited: false,
        initial_kind: InitialKind::Unit(0.0, Unit::Px),
    },
    // 37 max-width — not inherited; initial = none
    PropertyMeta {
        name: "max-width",
        inherited: false,
        initial_kind: InitialKind::Keyword("none"),
    },
    // 38 max-height
    PropertyMeta {
        name: "max-height",
        inherited: false,
        initial_kind: InitialKind::Keyword("none"),
    },
    // 39 border-top-left-radius — not inherited; initial = 0
    PropertyMeta {
        name: "border-top-left-radius",
        inherited: false,
        initial_kind: InitialKind::Unit(0.0, Unit::Px),
    },
    // 40 border-top-right-radius
    PropertyMeta {
        name: "border-top-right-radius",
        inherited: false,
        initial_kind: InitialKind::Unit(0.0, Unit::Px),
    },
    // 41 border-bottom-left-radius
    PropertyMeta {
        name: "border-bottom-left-radius",
        inherited: false,
        initial_kind: InitialKind::Unit(0.0, Unit::Px),
    },
    // 42 border-bottom-right-radius
    PropertyMeta {
        name: "border-bottom-right-radius",
        inherited: false,
        initial_kind: InitialKind::Unit(0.0, Unit::Px),
    },
    // 43 aspect-ratio — not inherited; initial = auto
    PropertyMeta {
        name: "aspect-ratio",
        inherited: false,
        initial_kind: InitialKind::Keyword("auto"),
    },
    // 44 gap — not inherited; initial = normal
    PropertyMeta {
        name: "gap",
        inherited: false,
        initial_kind: InitialKind::Keyword("normal"),
    },
    // 45 align-items — not inherited; initial = normal
    PropertyMeta {
        name: "align-items",
        inherited: false,
        initial_kind: InitialKind::Keyword("normal"),
    },
    // 46 align-self — not inherited; initial = auto
    PropertyMeta {
        name: "align-self",
        inherited: false,
        initial_kind: InitialKind::Keyword("auto"),
    },
    // 47 align-content — not inherited; initial = normal
    PropertyMeta {
        name: "align-content",
        inherited: false,
        initial_kind: InitialKind::Keyword("normal"),
    },
    // 48 text-align — inherited; initial = start
    PropertyMeta {
        name: "text-align",
        inherited: true,
        initial_kind: InitialKind::TextAlign(TextAlign::Start),
    },
    // 49 inset-block-end — not inherited; initial = auto
    PropertyMeta {
        name: "inset-block-end",
        inherited: false,
        initial_kind: InitialKind::Keyword("auto"),
    },
    // 50 inset-block-start
    PropertyMeta {
        name: "inset-block-start",
        inherited: false,
        initial_kind: InitialKind::Keyword("auto"),
    },
    // 51 inset-inline-end
    PropertyMeta {
        name: "inset-inline-end",
        inherited: false,
        initial_kind: InitialKind::Keyword("auto"),
    },
    // 52 inset-inline-start
    PropertyMeta {
        name: "inset-inline-start",
        inherited: false,
        initial_kind: InitialKind::Keyword("auto"),
    },
    // 53 justify-items — not inherited; initial = legacy
    PropertyMeta {
        name: "justify-items",
        inherited: false,
        initial_kind: InitialKind::Keyword("legacy"),
    },
    // 54 justify-self — not inherited; initial = auto
    PropertyMeta {
        name: "justify-self",
        inherited: false,
        initial_kind: InitialKind::Keyword("auto"),
    },
    // 55 justify-content — not inherited; initial = normal
    PropertyMeta {
        name: "justify-content",
        inherited: false,
        initial_kind: InitialKind::Keyword("normal"),
    },
    // 56 overflow-x — not inherited; initial = visible
    PropertyMeta {
        name: "overflow-x",
        inherited: false,
        initial_kind: InitialKind::Keyword("visible"),
    },
    // 57 overflow-y
    PropertyMeta {
        name: "overflow-y",
        inherited: false,
        initial_kind: InitialKind::Keyword("visible"),
    },
    // 58 box-sizing — not inherited; initial = content-box
    PropertyMeta {
        name: "box-sizing",
        inherited: false,
        initial_kind: InitialKind::Keyword("content-box"),
    },
    // 59 line-height — inherited; initial = normal
    PropertyMeta {
        name: "line-height",
        inherited: true,
        initial_kind: InitialKind::Keyword("normal"),
    },
    // 60 text-wrap — not inherited; initial = wrap
    PropertyMeta {
        name: "text-wrap",
        inherited: false,
        initial_kind: InitialKind::TextWrap(TextWrap::Wrap),
    },
    // 61 grid-row — not inherited; initial = auto
    PropertyMeta {
        name: "grid-row",
        inherited: false,
        initial_kind: InitialKind::Keyword("auto"),
    },
    // 62 grid-column
    PropertyMeta {
        name: "grid-column",
        inherited: false,
        initial_kind: InitialKind::Keyword("auto"),
    },
    // 63 grid-auto-flow — not inherited; initial = row
    PropertyMeta {
        name: "grid-auto-flow",
        inherited: false,
        initial_kind: InitialKind::Keyword("row"),
    },
    // 64 grid-template-rows — not inherited; initial = none
    PropertyMeta {
        name: "grid-template-rows",
        inherited: false,
        initial_kind: InitialKind::Keyword("none"),
    },
    // 65 grid-template-columns
    PropertyMeta {
        name: "grid-template-columns",
        inherited: false,
        initial_kind: InitialKind::Keyword("none"),
    },
    // 66 grid-auto-rows — not inherited; initial = auto
    PropertyMeta {
        name: "grid-auto-rows",
        inherited: false,
        initial_kind: InitialKind::Keyword("auto"),
    },
    // 67 grid-auto-columns
    PropertyMeta {
        name: "grid-auto-columns",
        inherited: false,
        initial_kind: InitialKind::Keyword("auto"),
    },
    // 68 font-style — inherited; initial = normal
    PropertyMeta {
        name: "font-style",
        inherited: true,
        initial_kind: InitialKind::Keyword("normal"),
    },
    // 69 white-space — inherited; initial = normal
    PropertyMeta {
        name: "white-space",
        inherited: true,
        initial_kind: InitialKind::Keyword("normal"),
    },
    // 70 text-decoration-line — inherited so child text nodes pick up parent element's decoration
    PropertyMeta {
        name: "text-decoration-line",
        inherited: true,
        initial_kind: InitialKind::Keyword("none"),
    },
    // 71 background-image — not inherited; initial = none
    PropertyMeta {
        name: "background-image",
        inherited: false,
        initial_kind: InitialKind::Keyword("none"),
    },
    // 72 content — not inherited; initial = normal (applies to ::before/::after pseudo-elements)
    PropertyMeta {
        name: "content",
        inherited: false,
        initial_kind: InitialKind::Keyword("normal"),
    },
    // 73 opacity — not inherited; initial = 1.0
    PropertyMeta {
        name: "opacity",
        inherited: false,
        initial_kind: InitialKind::Number(1.0),
    },
    // 74 text-transform — inherited; initial = none
    PropertyMeta {
        name: "text-transform",
        inherited: true,
        initial_kind: InitialKind::Keyword("none"),
    },
];

// ── NodeStyle — replaces StylePropertyList ────────────────────────────────────

/// Per-node CSS properties. Only stores values explicitly set by stylesheet rules for
/// THIS node. Lookup for inherited properties must recurse to the parent via the document.
#[derive(Debug, Clone)]
pub struct NodeStyle {
    /// Sorted ascending by property id for O(log n) binary search.
    own: Vec<(u8, Value)>,
}

impl Default for NodeStyle {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeStyle {
    pub fn new() -> Self {
        Self { own: Vec::new() }
    }

    /// Returns this node's own value for `prop` without parent recursion.
    pub fn get_own(&self, prop: &StyleProperty) -> Option<&Value> {
        let id = prop.id();
        self.own.binary_search_by_key(&id, |e| e.0).ok().map(|i| &self.own[i].1)
    }

    /// Sets or replaces an own property (maintains sort order).
    pub fn set(&mut self, prop: StyleProperty, value: Value) {
        let id = prop.id();
        match self.own.binary_search_by_key(&id, |e| e.0) {
            Ok(i) => self.own[i].1 = value,
            Err(i) => self.own.insert(i, (id, value)),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (StyleProperty, &Value)> {
        self.own
            .iter()
            .filter_map(|(id, val)| from_id(*id).map(|prop| (prop, val)))
    }

    /// Returns all properties as (css-name, value-string) pairs, sorted by name.
    pub fn to_string_map(&self) -> Vec<(String, String)> {
        let mut pairs: Vec<(String, String)> = self
            .own
            .iter()
            .map(|(id, val)| {
                let name = property_meta(*id).name.to_string();
                (name, val.to_css_string())
            })
            .collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        pairs
    }
}

/// Convert a property id back to a StyleProperty variant.
fn from_id(id: u8) -> Option<StyleProperty> {
    match id {
        0 => Some(StyleProperty::Color),
        1 => Some(StyleProperty::BackgroundColor),
        2 => Some(StyleProperty::FontSize),
        3 => Some(StyleProperty::FontWeight),
        4 => Some(StyleProperty::Display),
        5 => Some(StyleProperty::Width),
        6 => Some(StyleProperty::Height),
        7 => Some(StyleProperty::MarginTop),
        8 => Some(StyleProperty::MarginRight),
        9 => Some(StyleProperty::MarginBottom),
        10 => Some(StyleProperty::MarginLeft),
        11 => Some(StyleProperty::PaddingTop),
        12 => Some(StyleProperty::PaddingRight),
        13 => Some(StyleProperty::PaddingBottom),
        14 => Some(StyleProperty::PaddingLeft),
        15 => Some(StyleProperty::BorderTopWidth),
        16 => Some(StyleProperty::BorderRightWidth),
        17 => Some(StyleProperty::BorderBottomWidth),
        18 => Some(StyleProperty::BorderLeftWidth),
        19 => Some(StyleProperty::BorderTopColor),
        20 => Some(StyleProperty::BorderRightColor),
        21 => Some(StyleProperty::BorderBottomColor),
        22 => Some(StyleProperty::BorderLeftColor),
        23 => Some(StyleProperty::BorderTopStyle),
        24 => Some(StyleProperty::BorderRightStyle),
        25 => Some(StyleProperty::BorderBottomStyle),
        26 => Some(StyleProperty::BorderLeftStyle),
        27 => Some(StyleProperty::FontFamily),
        28 => Some(StyleProperty::FlexBasis),
        29 => Some(StyleProperty::FlexDirection),
        30 => Some(StyleProperty::FlexGrow),
        31 => Some(StyleProperty::FlexShrink),
        32 => Some(StyleProperty::FlexWrap),
        33 => Some(StyleProperty::ScrollbarWidth),
        34 => Some(StyleProperty::Position),
        35 => Some(StyleProperty::MinWidth),
        36 => Some(StyleProperty::MinHeight),
        37 => Some(StyleProperty::MaxWidth),
        38 => Some(StyleProperty::MaxHeight),
        39 => Some(StyleProperty::BorderTopLeftRadius),
        40 => Some(StyleProperty::BorderTopRightRadius),
        41 => Some(StyleProperty::BorderBottomLeftRadius),
        42 => Some(StyleProperty::BorderBottomRightRadius),
        43 => Some(StyleProperty::AspectRatio),
        44 => Some(StyleProperty::Gap),
        45 => Some(StyleProperty::AlignItems),
        46 => Some(StyleProperty::AlignSelf),
        47 => Some(StyleProperty::AlignContent),
        48 => Some(StyleProperty::TextAlign),
        49 => Some(StyleProperty::InsetBlockEnd),
        50 => Some(StyleProperty::InsetBlockStart),
        51 => Some(StyleProperty::InsetInlineEnd),
        52 => Some(StyleProperty::InsetInlineStart),
        53 => Some(StyleProperty::JustifyItems),
        54 => Some(StyleProperty::JustifySelf),
        55 => Some(StyleProperty::JustifyContent),
        56 => Some(StyleProperty::OverflowX),
        57 => Some(StyleProperty::OverflowY),
        58 => Some(StyleProperty::BoxSizing),
        59 => Some(StyleProperty::LineHeight),
        60 => Some(StyleProperty::TextWrap),
        61 => Some(StyleProperty::GridRow),
        62 => Some(StyleProperty::GridColumn),
        63 => Some(StyleProperty::GridAutoFlow),
        64 => Some(StyleProperty::GridTemplateRows),
        65 => Some(StyleProperty::GridTemplateColumns),
        66 => Some(StyleProperty::GridAutoRows),
        67 => Some(StyleProperty::GridAutoColumns),
        68 => Some(StyleProperty::FontStyle),
        69 => Some(StyleProperty::WhiteSpace),
        70 => Some(StyleProperty::TextDecorationLine),
        71 => Some(StyleProperty::BackgroundImage),
        72 => Some(StyleProperty::Content),
        73 => Some(StyleProperty::Opacity),
        74 => Some(StyleProperty::TextTransform),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_style_set_get() {
        let mut style = NodeStyle::new();
        style.set(StyleProperty::Color, Value::Color(255, 0, 0, 255));
        assert_eq!(
            style.get_own(&StyleProperty::Color),
            Some(&Value::Color(255, 0, 0, 255))
        );
        assert_eq!(style.get_own(&StyleProperty::BackgroundColor), None);
    }

    #[test]
    fn test_node_style_sorted() {
        let mut style = NodeStyle::new();
        // Insert in reverse id order to test sorting
        style.set(StyleProperty::MarginBottom, Value::Unit(10.0, Unit::Px));
        style.set(StyleProperty::Color, Value::Color(0, 0, 0, 255));
        style.set(StyleProperty::MarginTop, Value::Unit(5.0, Unit::Px));
        // Should be sorted: Color(0), MarginTop(7), MarginBottom(9)
        assert!(style.own[0].0 < style.own[1].0);
        assert!(style.own[1].0 < style.own[2].0);
    }

    #[test]
    fn test_id_round_trip() {
        // Every StyleProperty should round-trip through id → from_id
        let props = [
            StyleProperty::Color,
            StyleProperty::BackgroundColor,
            StyleProperty::FontSize,
            StyleProperty::MarginTop,
            StyleProperty::Display,
            StyleProperty::FlexGrow,
        ];
        for prop in &props {
            let id = prop.id();
            assert_eq!(from_id(id).as_ref(), Some(prop), "round-trip failed for {prop:?}");
        }
    }

    #[test]
    fn test_properties_table_consistent() {
        // Every id() value must be a valid PROPERTIES index
        let props = [
            StyleProperty::Color,
            StyleProperty::BackgroundColor,
            StyleProperty::FontSize,
            StyleProperty::FontWeight,
            StyleProperty::Display,
            StyleProperty::Width,
            StyleProperty::GridAutoColumns,
        ];
        for prop in &props {
            let _ = prop.meta(); // must not panic
        }
    }
}
