//! The computed style of one element, as the render pipeline reads it.
//!
//! One typed field per property the pipeline consumes, and every field always holds a value -
//! the element's own computed value, the value it inherited, or the property's initial value.
//! There is no parent walk and no second initial-value table: whoever builds a `ComputedStyle`
//! resolves all three against the parent's struct once, and everything downstream is a field
//! read.
//!
//! The fields are grouped by what they inherit and what they belong to, and each group is its
//! own struct so a later step can put the untouched ones behind an `Arc` and share them between
//! siblings.
//!
//! Percentages are deliberately still here. A percentage needs a containing block, which style
//! resolution does not have, so [`LengthPercentage`] and [`LengthPercentageAuto`] carry it
//! through to layout and [`LengthPercentage::resolve`] settles it there.

use std::sync::Arc;

// ── Value types ──────────────────────────────────────────────────────────────

/// An sRGB colour, one byte per channel. `a == 255` is fully opaque.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    pub const TRANSPARENT: Color = Color { r: 0, g: 0, b: 0, a: 0 };

    #[must_use]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

/// A length that may still be a percentage of something layout knows and style does not.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LengthPercentage {
    Px(f32),
    Percent(f32),
}

impl LengthPercentage {
    pub const ZERO: LengthPercentage = LengthPercentage::Px(0.0);

    /// The length in px against a containing-block extent, which is what a percentage is of.
    #[must_use]
    pub fn resolve(self, basis: f32) -> f32 {
        match self {
            LengthPercentage::Px(px) => px,
            LengthPercentage::Percent(pct) => basis * pct / 100.0,
        }
    }

    /// The length in px, or `None` when it is a percentage and no basis was given.
    #[must_use]
    pub fn to_px(self) -> Option<f32> {
        match self {
            LengthPercentage::Px(px) => Some(px),
            LengthPercentage::Percent(_) => None,
        }
    }

    /// The bare number, whatever unit it was written in - a percentage comes back as `50`, not
    /// as a fraction of anything.
    ///
    /// This is what the old `get_style_f32` did, and the call sites that use it are the ones
    /// that silently treated a percentage as pixels. Kept so this step moves no pixel; each
    /// such site is a bug of its own.
    #[must_use]
    pub fn raw(self) -> f32 {
        match self {
            LengthPercentage::Px(v) | LengthPercentage::Percent(v) => v,
        }
    }
}

/// A length, a percentage, or `auto` - the value space of `width`, `margin-*` and the insets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LengthPercentageAuto {
    Auto,
    Px(f32),
    Percent(f32),
}

impl LengthPercentageAuto {
    pub const ZERO: LengthPercentageAuto = LengthPercentageAuto::Px(0.0);

    #[must_use]
    pub fn is_auto(self) -> bool {
        matches!(self, LengthPercentageAuto::Auto)
    }

    /// The length in px against a containing-block extent; `None` for `auto`.
    #[must_use]
    pub fn resolve(self, basis: f32) -> Option<f32> {
        match self {
            LengthPercentageAuto::Auto => None,
            LengthPercentageAuto::Px(px) => Some(px),
            LengthPercentageAuto::Percent(pct) => Some(basis * pct / 100.0),
        }
    }

    /// The length in px, or `None` for `auto` *and* for a percentage.
    #[must_use]
    pub fn to_px(self) -> Option<f32> {
        match self {
            LengthPercentageAuto::Px(px) => Some(px),
            LengthPercentageAuto::Auto | LengthPercentageAuto::Percent(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// Table interior, but participates in its parent's inline formatting context
    /// (CSS 2.1 §17.4).
    InlineTable,
    TableCaption,
    TableCell,
    TableFooterGroup,
    TableHeaderGroup,
    TableRow,
    TableRowGroup,
    /// `<col>` - defines column properties, generates no box of its own.
    TableColumn,
    /// `<colgroup>` - groups `<col>` elements, generates no box of its own.
    TableColumnGroup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    Static,
    Relative,
    Absolute,
    Fixed,
    Sticky,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Float {
    None,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clear {
    None,
    Left,
    Right,
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxSizing {
    ContentBox,
    BorderBox,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overflow {
    Visible,
    Hidden,
    Clip,
    Scroll,
    Auto,
}

impl Overflow {
    /// Whether this value makes the box establish a block formatting context, and so grow to
    /// contain its floats (CSS 2.1 §10.6.7).
    #[must_use]
    pub fn establishes_bfc(self) -> bool {
        !matches!(self, Overflow::Visible | Overflow::Clip)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

impl BorderStyle {
    /// Whether a border with this style paints anything and takes up width.
    #[must_use]
    pub fn is_visible(self) -> bool {
        !matches!(self, BorderStyle::None | BorderStyle::Hidden)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FontWeight {
    Normal,
    Bold,
    Bolder,
    Lighter,
    Number(f32),
}

impl FontWeight {
    /// The numeric weight, which is what a font system asks for. `bolder` and `lighter` are
    /// relative to the inherited weight per the spec; they are answered as plain bold and light
    /// here, as they always have been.
    #[must_use]
    pub fn to_number(self) -> f32 {
        match self {
            FontWeight::Normal => 400.0,
            FontWeight::Bold | FontWeight::Bolder => 700.0,
            FontWeight::Lighter => 300.0,
            FontWeight::Number(n) => n,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontStyle {
    Normal,
    Italic,
    Oblique,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Right,
    Center,
    Justify,
    Start,
    End,
    MatchParent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextWrap {
    Wrap,
    NoWrap,
    Balance,
    Pretty,
    Stable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WhiteSpace {
    Normal,
    Pre,
    NoWrap,
    PreWrap,
    PreLine,
    BreakSpaces,
}

impl WhiteSpace {
    /// Whether the value preserves the source's spaces and newlines as content.
    #[must_use]
    pub fn preserves_spaces(self) -> bool {
        matches!(self, WhiteSpace::Pre | WhiteSpace::PreWrap)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextTransform {
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptionSide {
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderCollapse {
    Separate,
    Collapse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableLayout {
    Auto,
    Fixed,
}

/// `vertical-align`, keyword form. `Other` is every value this engine does not act on - a
/// length, and the `inherit` the user-agent sheet puts on cells - which the cell-alignment
/// walk treats as "keep looking further up".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerticalAlign {
    Baseline,
    Sub,
    Super,
    TextTop,
    TextBottom,
    Middle,
    Top,
    Bottom,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlexDirection {
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlexWrap {
    NoWrap,
    Wrap,
    WrapReverse,
}

/// The shared value space of `align-items`, `align-self`, `align-content`, `justify-items`,
/// `justify-self` and `justify-content`. Which of them a value is legal on is the cascade's
/// business; what each consumer does with it is the consumer's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignValue {
    Normal,
    Auto,
    Stretch,
    Center,
    Start,
    End,
    FlexStart,
    FlexEnd,
    Baseline,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
    Legacy,
    /// A value none of the consumers act on, so they fall back to their own default.
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridAutoFlow {
    Row,
    Column,
    RowDense,
    ColumnDense,
}

/// `line-height`. A number is a multiplier of the element's own font-size and stays one, so it
/// inherits as written (css-inline-3 §2.3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineHeight {
    Normal,
    Px(f32),
    Number(f32),
}

impl LineHeight {
    /// The line height in px, or `None` for `normal` - which means the font metrics decide.
    #[must_use]
    pub fn to_px(self, font_size: f32) -> Option<f32> {
        match self {
            LineHeight::Normal => None,
            LineHeight::Px(px) => Some(px),
            LineHeight::Number(ratio) => Some(font_size * ratio),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZIndex {
    Auto,
    Index(f32),
}

/// `letter-spacing`. A percentage is of the font size and is resolved at used-value time
/// (css-text-4 §8.2), so it survives the computed stage.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LetterSpacing {
    Normal,
    Length(LengthPercentage),
}

/// `text-decoration-line`, as the two lines this engine paints.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextDecorationLine {
    pub underline: bool,
    pub line_through: bool,
}

impl TextDecorationLine {
    pub const NONE: TextDecorationLine = TextDecorationLine {
        underline: false,
        line_through: false,
    };
}

// ── Which properties the element's own cascade had a value for ───────────────

/// One property of a [`ComputedStyle`], for the `declared` set.
///
/// Every field of a `ComputedStyle` always holds a value, so "the author said nothing" is not
/// visible in the value itself - and a handful of readers need it: an element with no `display`
/// of its own falls back to what its tag name means, `z-index: auto` and an undeclared
/// `z-index` put the box in different stacking branches, and the cell-alignment walk keeps
/// climbing until it finds an ancestor that declared `vertical-align`.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prop {
    Color = 0,
    FontSize,
    FontFamily,
    FontStyle,
    FontWeight,
    LineHeight,
    TextAlign,
    TextTransform,
    TextDecorationLine,
    WhiteSpace,
    LetterSpacing,
    CaptionSide,
    BorderSpacingX,
    BorderSpacingY,
    BorderCollapse,

    Display,
    Position,
    Float,
    Clear,
    BoxSizing,
    OverflowX,
    OverflowY,
    ZIndex,
    Opacity,
    MixBlendMode,
    Resize,
    ScrollbarWidth,
    AspectRatio,
    TextWrap,
    TableLayout,
    VerticalAlign,

    Width,
    Height,
    MinWidth,
    MinHeight,
    MaxWidth,
    MaxHeight,

    MarginTop,
    MarginRight,
    MarginBottom,
    MarginLeft,

    PaddingTop,
    PaddingRight,
    PaddingBottom,
    PaddingLeft,

    BorderTopWidth,
    BorderRightWidth,
    BorderBottomWidth,
    BorderLeftWidth,
    BorderTopStyle,
    BorderRightStyle,
    BorderBottomStyle,
    BorderLeftStyle,
    BorderTopColor,
    BorderRightColor,
    BorderBottomColor,
    BorderLeftColor,
    BorderTopLeftRadius,
    BorderTopRightRadius,
    BorderBottomLeftRadius,
    BorderBottomRightRadius,

    OutlineWidth,
    OutlineStyle,
    OutlineColor,
    OutlineOffset,

    BackgroundColor,
    BackgroundImage,

    InsetBlockStart,
    InsetBlockEnd,
    InsetInlineStart,
    InsetInlineEnd,

    FlexBasis,
    FlexDirection,
    FlexGrow,
    FlexShrink,
    FlexWrap,
    Gap,
    AlignItems,
    AlignSelf,
    AlignContent,
    JustifyItems,
    JustifySelf,
    JustifyContent,

    GridRow,
    GridColumn,
    GridArea,
    GridTemplateRows,
    GridTemplateColumns,
    GridAutoRows,
    GridAutoColumns,
    GridTemplateAreas,
    GridAutoFlow,
}

/// How many `u64` words the `declared` set needs.
const DECLARED_WORDS: usize = 2;

/// The set of properties an element's own cascade produced a value for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeclaredSet([u64; DECLARED_WORDS]);

impl DeclaredSet {
    #[must_use]
    pub fn has(self, prop: Prop) -> bool {
        let bit = prop as usize;
        self.0.get(bit / 64).copied().unwrap_or(0) & (1u64 << (bit % 64)) != 0
    }

    pub fn set(&mut self, prop: Prop) {
        let bit = prop as usize;
        if let Some(word) = self.0.get_mut(bit / 64) {
            *word |= 1u64 << (bit % 64);
        }
    }
}

// ── Field groups ─────────────────────────────────────────────────────────────

/// The properties that inherit. An element that declares none of them has exactly its parent's.
#[derive(Clone, Debug, PartialEq)]
pub struct InheritedGroup {
    pub color: Color,
    /// Computed `font-size` in px.
    pub font_size: f32,
    /// The family list as written, comma-separated, for the font system to resolve.
    pub font_family: Arc<str>,
    pub font_style: FontStyle,
    pub font_weight: FontWeight,
    pub line_height: LineHeight,
    pub text_align: TextAlign,
    pub text_transform: TextTransform,
    pub text_decoration_line: TextDecorationLine,
    pub white_space: WhiteSpace,
    pub letter_spacing: LetterSpacing,
    pub caption_side: CaptionSide,
    /// Horizontal component of `border-spacing`, in px.
    pub border_spacing_x: f32,
    /// Vertical component of `border-spacing`, in px.
    pub border_spacing_y: f32,
    pub border_collapse: BorderCollapse,
    /// Whether this element or any ancestor declared a `font-size` at all.
    ///
    /// Only the monospace default-size quirk reads it: the 13px default applies when nothing
    /// up the chain ever said how big the text should be. It inherits like a property because
    /// that is the question it answers.
    pub font_size_declared_in_chain: bool,
}

/// The box's own nature: what kind of box it is, where it sits, how it composites.
#[derive(Clone, Debug, PartialEq)]
pub struct BoxGroup {
    pub display: Display,
    pub position: Position,
    pub float: Float,
    pub clear: Clear,
    pub box_sizing: BoxSizing,
    pub overflow_x: Overflow,
    pub overflow_y: Overflow,
    pub z_index: ZIndex,
    pub opacity: f32,
    /// Kept as text: the blend-mode set is the painter's, not style's.
    pub mix_blend_mode: Arc<str>,
    /// Kept as text: the resize set is the layouter's control machinery's.
    pub resize: Arc<str>,
    /// `None` unless a plain number was given, which is the only form taffy takes.
    pub scrollbar_width: Option<f32>,
    pub aspect_ratio: Option<f32>,
    pub text_wrap: TextWrap,
    pub table_layout: TableLayout,
    pub vertical_align: VerticalAlign,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SizeGroup {
    pub width: LengthPercentageAuto,
    pub height: LengthPercentageAuto,
    pub min_width: LengthPercentageAuto,
    pub min_height: LengthPercentageAuto,
    pub max_width: LengthPercentageAuto,
    pub max_height: LengthPercentageAuto,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MarginGroup {
    pub top: LengthPercentageAuto,
    pub right: LengthPercentageAuto,
    pub bottom: LengthPercentageAuto,
    pub left: LengthPercentageAuto,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaddingGroup {
    pub top: LengthPercentage,
    pub right: LengthPercentage,
    pub bottom: LengthPercentage,
    pub left: LengthPercentage,
}

/// Border widths are already zero where the matching style is `none` or `hidden`, so layout and
/// paint cannot disagree about the box.
#[derive(Clone, Debug, PartialEq)]
pub struct BorderGroup {
    pub top_width: f32,
    pub right_width: f32,
    pub bottom_width: f32,
    pub left_width: f32,
    pub top_style: BorderStyle,
    pub right_style: BorderStyle,
    pub bottom_style: BorderStyle,
    pub left_style: BorderStyle,
    pub top_color: Color,
    pub right_color: Color,
    pub bottom_color: Color,
    pub left_color: Color,
    pub top_left_radius: LengthPercentage,
    pub top_right_radius: LengthPercentage,
    pub bottom_left_radius: LengthPercentage,
    pub bottom_right_radius: LengthPercentage,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutlineGroup {
    pub width: f32,
    pub style: BorderStyle,
    pub color: Color,
    pub offset: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BackgroundGroup {
    pub color: Color,
    /// The `url()` target of the first image layer, unresolved against the document's base URL.
    pub image: Option<Arc<str>>,
}

/// The insets, named logically. The physical `top`/`right`/`bottom`/`left` map onto them
/// directly in the horizontal-tb, ltr writing mode this engine assumes.
#[derive(Clone, Debug, PartialEq)]
pub struct InsetGroup {
    pub block_start: LengthPercentageAuto,
    pub block_end: LengthPercentageAuto,
    pub inline_start: LengthPercentageAuto,
    pub inline_end: LengthPercentageAuto,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlexGroup {
    pub basis: LengthPercentageAuto,
    pub direction: FlexDirection,
    pub grow: f32,
    pub shrink: f32,
    pub wrap: FlexWrap,
    pub gap: LengthPercentage,
    pub align_items: AlignValue,
    pub align_self: AlignValue,
    pub align_content: AlignValue,
    pub justify_items: AlignValue,
    pub justify_self: AlignValue,
    pub justify_content: AlignValue,
}

/// Grid track lists and placements are kept as the CSS text they were written in: their value
/// space is open (`repeat(3, minmax(100px, 1fr))`) and the layouter has the parser for it.
#[derive(Clone, Debug, PartialEq)]
pub struct GridGroup {
    pub row: Arc<str>,
    pub column: Arc<str>,
    pub area: Arc<str>,
    pub template_rows: Arc<str>,
    pub template_columns: Arc<str>,
    pub auto_rows: Arc<str>,
    pub auto_columns: Arc<str>,
    /// One row per line, joined with `\n` - a character an area name cannot contain.
    pub template_areas: Arc<str>,
    pub auto_flow: GridAutoFlow,
}

// ── ComputedStyle ────────────────────────────────────────────────────────────

/// Everything the render pipeline reads about one element's style.
#[derive(Clone, Debug, PartialEq)]
pub struct ComputedStyle {
    pub inherited: InheritedGroup,
    pub box_group: BoxGroup,
    pub size: SizeGroup,
    pub margin: MarginGroup,
    pub padding: PaddingGroup,
    pub border: BorderGroup,
    pub outline: OutlineGroup,
    pub background: BackgroundGroup,
    pub inset: InsetGroup,
    pub flex: FlexGroup,
    pub grid: GridGroup,
    /// Which properties this element's own cascade produced a value for.
    pub declared: DeclaredSet,
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self::initial()
    }
}

impl ComputedStyle {
    /// Every property at its initial value and nothing declared.
    ///
    /// Built once and cloned: the string-valued fields are `Arc`s, so a clone bumps refcounts
    /// rather than allocating.
    #[must_use]
    pub fn initial() -> Self {
        static INITIAL: std::sync::OnceLock<ComputedStyle> = std::sync::OnceLock::new();
        INITIAL.get_or_init(Self::build_initial).clone()
    }

    /// The style of an element that declares nothing: the parent's inherited group, every other
    /// group at its initial value. The starting point for a real conversion, and the whole
    /// answer for a text node and for an anonymous box.
    #[must_use]
    pub fn inherit_from(parent: Option<&ComputedStyle>) -> Self {
        let mut style = Self::initial();
        if let Some(parent) = parent {
            style.inherited = parent.inherited.clone();
            // `currentColor` is the initial value of the border colours, and the colour it names
            // is this element's - which, with nothing declared, is the inherited one.
            let color = style.inherited.color;
            style.border.top_color = color;
            style.border.right_color = color;
            style.border.bottom_color = color;
            style.border.left_color = color;
        }
        style
    }

    /// The element's own `display`, or `None` when the cascade assigned it none.
    ///
    /// The difference matters: an element with no `display` of its own falls back to what its
    /// tag name means, since the user-agent sheet does not name one for everything.
    #[must_use]
    pub fn declared_display(&self) -> Option<Display> {
        self.has(Prop::Display).then_some(self.box_group.display)
    }

    /// Whether this element's own cascade produced a value for `prop`.
    #[must_use]
    pub fn has(&self, prop: Prop) -> bool {
        self.declared.has(prop)
    }

    fn build_initial() -> Self {
        ComputedStyle {
            inherited: InheritedGroup {
                color: Color::BLACK,
                font_size: 16.0,
                font_family: Arc::from("serif"),
                font_style: FontStyle::Normal,
                font_weight: FontWeight::Normal,
                line_height: LineHeight::Normal,
                text_align: TextAlign::Start,
                text_transform: TextTransform::None,
                text_decoration_line: TextDecorationLine::NONE,
                white_space: WhiteSpace::Normal,
                letter_spacing: LetterSpacing::Normal,
                caption_side: CaptionSide::Top,
                border_spacing_x: 0.0,
                border_spacing_y: 0.0,
                border_collapse: BorderCollapse::Separate,
                font_size_declared_in_chain: false,
            },
            box_group: BoxGroup {
                display: Display::Inline,
                position: Position::Static,
                float: Float::None,
                clear: Clear::None,
                box_sizing: BoxSizing::ContentBox,
                overflow_x: Overflow::Visible,
                overflow_y: Overflow::Visible,
                z_index: ZIndex::Auto,
                opacity: 1.0,
                mix_blend_mode: Arc::from("normal"),
                resize: Arc::from("none"),
                scrollbar_width: None,
                aspect_ratio: None,
                text_wrap: TextWrap::Wrap,
                table_layout: TableLayout::Auto,
                vertical_align: VerticalAlign::Baseline,
            },
            size: SizeGroup {
                width: LengthPercentageAuto::Auto,
                height: LengthPercentageAuto::Auto,
                // Not the spec's `auto`: this engine has always treated the initial minimum as
                // zero, and the table layouter reads it as a real length.
                min_width: LengthPercentageAuto::ZERO,
                min_height: LengthPercentageAuto::ZERO,
                max_width: LengthPercentageAuto::Auto,
                max_height: LengthPercentageAuto::Auto,
            },
            margin: MarginGroup {
                top: LengthPercentageAuto::ZERO,
                right: LengthPercentageAuto::ZERO,
                bottom: LengthPercentageAuto::ZERO,
                left: LengthPercentageAuto::ZERO,
            },
            padding: PaddingGroup {
                top: LengthPercentage::ZERO,
                right: LengthPercentage::ZERO,
                bottom: LengthPercentage::ZERO,
                left: LengthPercentage::ZERO,
            },
            border: BorderGroup {
                // The initial `border-*-width` is `medium`, but the initial `border-*-style` is
                // `none`, which zeroes it.
                top_width: 0.0,
                right_width: 0.0,
                bottom_width: 0.0,
                left_width: 0.0,
                top_style: BorderStyle::None,
                right_style: BorderStyle::None,
                bottom_style: BorderStyle::None,
                left_style: BorderStyle::None,
                top_color: Color::BLACK,
                right_color: Color::BLACK,
                bottom_color: Color::BLACK,
                left_color: Color::BLACK,
                top_left_radius: LengthPercentage::ZERO,
                top_right_radius: LengthPercentage::ZERO,
                bottom_left_radius: LengthPercentage::ZERO,
                bottom_right_radius: LengthPercentage::ZERO,
            },
            outline: OutlineGroup {
                width: 0.0,
                style: BorderStyle::None,
                color: Color::BLACK,
                offset: 0.0,
            },
            background: BackgroundGroup {
                color: Color::TRANSPARENT,
                image: None,
            },
            inset: InsetGroup {
                block_start: LengthPercentageAuto::Auto,
                block_end: LengthPercentageAuto::Auto,
                inline_start: LengthPercentageAuto::Auto,
                inline_end: LengthPercentageAuto::Auto,
            },
            flex: FlexGroup {
                basis: LengthPercentageAuto::Auto,
                direction: FlexDirection::Row,
                grow: 0.0,
                shrink: 1.0,
                wrap: FlexWrap::NoWrap,
                gap: LengthPercentage::ZERO,
                align_items: AlignValue::Normal,
                align_self: AlignValue::Auto,
                align_content: AlignValue::Normal,
                justify_items: AlignValue::Legacy,
                justify_self: AlignValue::Auto,
                justify_content: AlignValue::Normal,
            },
            grid: GridGroup {
                row: Arc::from("auto"),
                column: Arc::from("auto"),
                area: Arc::from("auto"),
                template_rows: Arc::from("none"),
                template_columns: Arc::from("none"),
                auto_rows: Arc::from("auto"),
                auto_columns: Arc::from("auto"),
                template_areas: Arc::from("none"),
                auto_flow: GridAutoFlow::Row,
            },
            declared: DeclaredSet::default(),
        }
    }
}
