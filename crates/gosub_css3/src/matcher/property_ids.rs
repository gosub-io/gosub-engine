//! Property ids: one small integer per CSS property, generated from the property
//! definitions this crate embeds.
//!
//! The cascade is keyed by these rather than by property names. A name is a string to
//! hash, a string to allocate and a string to compare; an id is an array index, and a
//! dense one, so a property map is a slot per id rather than a hash table.
//!
//! Custom properties (`--*`) get no id. They are unbounded in number and cascade in a
//! pass of their own, so they stay in a map keyed by their full name.
//!
//! The tables here hold what can be read straight out of the definition data. Anything
//! that needs the resolved value grammar - whether a property takes a colour, the range
//! its computed value is clamped to - stays on [`crate::matcher::property_definitions::PropertyDefinition`],
//! which is reachable by id through
//! [`CssDefinitions::definition`](crate::matcher::property_definitions::CssDefinitions::definition).
//!
//! GENERATED FILE - do not edit. Regenerate after changing the definition JSON with:
//!
//! ```text
//! cargo run -p generate_definitions -- --property-ids
//! ```
//!
//! That mode reads the checked-in `resources/definitions/definitions_properties.json`
//! and touches the network for nothing. `property_ids_match_the_definitions` in this
//! crate fails if the two ever drift apart.

/// How many longhand properties there are.
pub const LONGHAND_COUNT: usize = 567;

/// How many shorthand properties there are.
pub const SHORTHAND_COUNT: usize = 98;

/// How many properties have an id, longhands and shorthands together. Every
/// [`PropertyId::index`] is below this, so an array of this length is a slot per
/// property.
pub const PROPERTY_COUNT: usize = LONGHAND_COUNT + SHORTHAND_COUNT;

/// A property that is not a shorthand: it holds a value of its own.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LonghandId {
    /// `-moz-appearance`
    MozAppearance = 0,
    /// `-moz-binding`
    MozBinding = 1,
    /// `-moz-border-bottom-colors`
    MozBorderBottomColors = 2,
    /// `-moz-border-left-colors`
    MozBorderLeftColors = 3,
    /// `-moz-border-right-colors`
    MozBorderRightColors = 4,
    /// `-moz-border-top-colors`
    MozBorderTopColors = 5,
    /// `-moz-context-properties`
    MozContextProperties = 6,
    /// `-moz-float-edge`
    MozFloatEdge = 7,
    /// `-moz-force-broken-image-icon`
    MozForceBrokenImageIcon = 8,
    /// `-moz-orient`
    MozOrient = 9,
    /// `-moz-outline-radius-bottomleft`
    MozOutlineRadiusBottomleft = 10,
    /// `-moz-outline-radius-bottomright`
    MozOutlineRadiusBottomright = 11,
    /// `-moz-outline-radius-topleft`
    MozOutlineRadiusTopleft = 12,
    /// `-moz-outline-radius-topright`
    MozOutlineRadiusTopright = 13,
    /// `-moz-stack-sizing`
    MozStackSizing = 14,
    /// `-moz-text-blink`
    MozTextBlink = 15,
    /// `-moz-user-focus`
    MozUserFocus = 16,
    /// `-moz-user-input`
    MozUserInput = 17,
    /// `-moz-user-modify`
    MozUserModify = 18,
    /// `-moz-window-dragging`
    MozWindowDragging = 19,
    /// `-moz-window-shadow`
    MozWindowShadow = 20,
    /// `-ms-accelerator`
    MsAccelerator = 21,
    /// `-ms-block-progression`
    MsBlockProgression = 22,
    /// `-ms-content-zoom-chaining`
    MsContentZoomChaining = 23,
    /// `-ms-content-zoom-limit-max`
    MsContentZoomLimitMax = 24,
    /// `-ms-content-zoom-limit-min`
    MsContentZoomLimitMin = 25,
    /// `-ms-content-zoom-snap-points`
    MsContentZoomSnapPoints = 26,
    /// `-ms-content-zoom-snap-type`
    MsContentZoomSnapType = 27,
    /// `-ms-content-zooming`
    MsContentZooming = 28,
    /// `-ms-filter`
    MsFilter = 29,
    /// `-ms-flow-from`
    MsFlowFrom = 30,
    /// `-ms-flow-into`
    MsFlowInto = 31,
    /// `-ms-grid-columns`
    MsGridColumns = 32,
    /// `-ms-grid-rows`
    MsGridRows = 33,
    /// `-ms-high-contrast-adjust`
    MsHighContrastAdjust = 34,
    /// `-ms-hyphenate-limit-chars`
    MsHyphenateLimitChars = 35,
    /// `-ms-hyphenate-limit-lines`
    MsHyphenateLimitLines = 36,
    /// `-ms-hyphenate-limit-zone`
    MsHyphenateLimitZone = 37,
    /// `-ms-ime-align`
    MsImeAlign = 38,
    /// `-ms-overflow-style`
    MsOverflowStyle = 39,
    /// `-ms-scroll-chaining`
    MsScrollChaining = 40,
    /// `-ms-scroll-limit-x-max`
    MsScrollLimitXMax = 41,
    /// `-ms-scroll-limit-x-min`
    MsScrollLimitXMin = 42,
    /// `-ms-scroll-limit-y-max`
    MsScrollLimitYMax = 43,
    /// `-ms-scroll-limit-y-min`
    MsScrollLimitYMin = 44,
    /// `-ms-scroll-rails`
    MsScrollRails = 45,
    /// `-ms-scroll-snap-points-x`
    MsScrollSnapPointsX = 46,
    /// `-ms-scroll-snap-points-y`
    MsScrollSnapPointsY = 47,
    /// `-ms-scroll-snap-type`
    MsScrollSnapType = 48,
    /// `-ms-scroll-translation`
    MsScrollTranslation = 49,
    /// `-ms-scrollbar-3dlight-color`
    MsScrollbar3dlightColor = 50,
    /// `-ms-scrollbar-arrow-color`
    MsScrollbarArrowColor = 51,
    /// `-ms-scrollbar-base-color`
    MsScrollbarBaseColor = 52,
    /// `-ms-scrollbar-darkshadow-color`
    MsScrollbarDarkshadowColor = 53,
    /// `-ms-scrollbar-face-color`
    MsScrollbarFaceColor = 54,
    /// `-ms-scrollbar-highlight-color`
    MsScrollbarHighlightColor = 55,
    /// `-ms-scrollbar-shadow-color`
    MsScrollbarShadowColor = 56,
    /// `-ms-scrollbar-track-color`
    MsScrollbarTrackColor = 57,
    /// `-ms-text-autospace`
    MsTextAutospace = 58,
    /// `-ms-touch-select`
    MsTouchSelect = 59,
    /// `-ms-user-select`
    MsUserSelect = 60,
    /// `-ms-wrap-flow`
    MsWrapFlow = 61,
    /// `-ms-wrap-margin`
    MsWrapMargin = 62,
    /// `-ms-wrap-through`
    MsWrapThrough = 63,
    /// `-webkit-appearance`
    WebkitAppearance = 64,
    /// `-webkit-border-after-color`
    WebkitBorderAfterColor = 65,
    /// `-webkit-border-after-style`
    WebkitBorderAfterStyle = 66,
    /// `-webkit-border-after-width`
    WebkitBorderAfterWidth = 67,
    /// `-webkit-border-before-color`
    WebkitBorderBeforeColor = 68,
    /// `-webkit-border-before-style`
    WebkitBorderBeforeStyle = 69,
    /// `-webkit-border-before-width`
    WebkitBorderBeforeWidth = 70,
    /// `-webkit-border-end-color`
    WebkitBorderEndColor = 71,
    /// `-webkit-border-end-style`
    WebkitBorderEndStyle = 72,
    /// `-webkit-border-end-width`
    WebkitBorderEndWidth = 73,
    /// `-webkit-border-start-color`
    WebkitBorderStartColor = 74,
    /// `-webkit-border-start-style`
    WebkitBorderStartStyle = 75,
    /// `-webkit-border-start-width`
    WebkitBorderStartWidth = 76,
    /// `-webkit-box-reflect`
    WebkitBoxReflect = 77,
    /// `-webkit-line-clamp`
    WebkitLineClamp = 78,
    /// `-webkit-mask-attachment`
    WebkitMaskAttachment = 79,
    /// `-webkit-mask-clip`
    WebkitMaskClip = 80,
    /// `-webkit-mask-composite`
    WebkitMaskComposite = 81,
    /// `-webkit-mask-image`
    WebkitMaskImage = 82,
    /// `-webkit-mask-origin`
    WebkitMaskOrigin = 83,
    /// `-webkit-mask-position`
    WebkitMaskPosition = 84,
    /// `-webkit-mask-position-x`
    WebkitMaskPositionX = 85,
    /// `-webkit-mask-position-y`
    WebkitMaskPositionY = 86,
    /// `-webkit-mask-repeat`
    WebkitMaskRepeat = 87,
    /// `-webkit-mask-repeat-x`
    WebkitMaskRepeatX = 88,
    /// `-webkit-mask-repeat-y`
    WebkitMaskRepeatY = 89,
    /// `-webkit-mask-size`
    WebkitMaskSize = 90,
    /// `-webkit-overflow-scrolling`
    WebkitOverflowScrolling = 91,
    /// `-webkit-tap-highlight-color`
    WebkitTapHighlightColor = 92,
    /// `-webkit-text-fill-color`
    WebkitTextFillColor = 93,
    /// `-webkit-text-stroke-color`
    WebkitTextStrokeColor = 94,
    /// `-webkit-text-stroke-width`
    WebkitTextStrokeWidth = 95,
    /// `-webkit-touch-callout`
    WebkitTouchCallout = 96,
    /// `-webkit-user-modify`
    WebkitUserModify = 97,
    /// `-webkit-user-select`
    WebkitUserSelect = 98,
    /// `accent-color`
    AccentColor = 99,
    /// `align-content`
    AlignContent = 100,
    /// `align-items`
    AlignItems = 101,
    /// `align-self`
    AlignSelf = 102,
    /// `align-tracks`
    AlignTracks = 103,
    /// `alignment-baseline`
    AlignmentBaseline = 104,
    /// `all`
    All = 105,
    /// `anchor-name`
    AnchorName = 106,
    /// `anchor-scope`
    AnchorScope = 107,
    /// `animation-composition`
    AnimationComposition = 108,
    /// `animation-delay`
    AnimationDelay = 109,
    /// `animation-direction`
    AnimationDirection = 110,
    /// `animation-duration`
    AnimationDuration = 111,
    /// `animation-fill-mode`
    AnimationFillMode = 112,
    /// `animation-iteration-count`
    AnimationIterationCount = 113,
    /// `animation-name`
    AnimationName = 114,
    /// `animation-play-state`
    AnimationPlayState = 115,
    /// `animation-range-end`
    AnimationRangeEnd = 116,
    /// `animation-range-start`
    AnimationRangeStart = 117,
    /// `animation-timeline`
    AnimationTimeline = 118,
    /// `animation-timing-function`
    AnimationTimingFunction = 119,
    /// `animation-trigger`
    AnimationTrigger = 120,
    /// `appearance`
    Appearance = 121,
    /// `aspect-ratio`
    AspectRatio = 122,
    /// `backdrop-filter`
    BackdropFilter = 123,
    /// `backface-visibility`
    BackfaceVisibility = 124,
    /// `background-attachment`
    BackgroundAttachment = 125,
    /// `background-blend-mode`
    BackgroundBlendMode = 126,
    /// `background-clip`
    BackgroundClip = 127,
    /// `background-color`
    BackgroundColor = 128,
    /// `background-image`
    BackgroundImage = 129,
    /// `background-origin`
    BackgroundOrigin = 130,
    /// `background-position-x`
    BackgroundPositionX = 131,
    /// `background-position-y`
    BackgroundPositionY = 132,
    /// `background-repeat`
    BackgroundRepeat = 133,
    /// `background-size`
    BackgroundSize = 134,
    /// `baseline-shift`
    BaselineShift = 135,
    /// `baseline-source`
    BaselineSource = 136,
    /// `block-size`
    BlockSize = 137,
    /// `border-block-end-color`
    BorderBlockEndColor = 138,
    /// `border-block-end-style`
    BorderBlockEndStyle = 139,
    /// `border-block-end-width`
    BorderBlockEndWidth = 140,
    /// `border-block-start-color`
    BorderBlockStartColor = 141,
    /// `border-block-start-style`
    BorderBlockStartStyle = 142,
    /// `border-block-start-width`
    BorderBlockStartWidth = 143,
    /// `border-bottom-color`
    BorderBottomColor = 144,
    /// `border-bottom-left-radius`
    BorderBottomLeftRadius = 145,
    /// `border-bottom-right-radius`
    BorderBottomRightRadius = 146,
    /// `border-bottom-style`
    BorderBottomStyle = 147,
    /// `border-bottom-width`
    BorderBottomWidth = 148,
    /// `border-collapse`
    BorderCollapse = 149,
    /// `border-end-end-radius`
    BorderEndEndRadius = 150,
    /// `border-end-start-radius`
    BorderEndStartRadius = 151,
    /// `border-image-outset`
    BorderImageOutset = 152,
    /// `border-image-repeat`
    BorderImageRepeat = 153,
    /// `border-image-slice`
    BorderImageSlice = 154,
    /// `border-image-source`
    BorderImageSource = 155,
    /// `border-image-width`
    BorderImageWidth = 156,
    /// `border-inline-end-color`
    BorderInlineEndColor = 157,
    /// `border-inline-end-style`
    BorderInlineEndStyle = 158,
    /// `border-inline-end-width`
    BorderInlineEndWidth = 159,
    /// `border-inline-start-color`
    BorderInlineStartColor = 160,
    /// `border-inline-start-style`
    BorderInlineStartStyle = 161,
    /// `border-inline-start-width`
    BorderInlineStartWidth = 162,
    /// `border-left-color`
    BorderLeftColor = 163,
    /// `border-left-style`
    BorderLeftStyle = 164,
    /// `border-left-width`
    BorderLeftWidth = 165,
    /// `border-right-color`
    BorderRightColor = 166,
    /// `border-right-style`
    BorderRightStyle = 167,
    /// `border-right-width`
    BorderRightWidth = 168,
    /// `border-shape`
    BorderShape = 169,
    /// `border-spacing`
    BorderSpacing = 170,
    /// `border-start-end-radius`
    BorderStartEndRadius = 171,
    /// `border-start-start-radius`
    BorderStartStartRadius = 172,
    /// `border-top-color`
    BorderTopColor = 173,
    /// `border-top-left-radius`
    BorderTopLeftRadius = 174,
    /// `border-top-right-radius`
    BorderTopRightRadius = 175,
    /// `border-top-style`
    BorderTopStyle = 176,
    /// `border-top-width`
    BorderTopWidth = 177,
    /// `bottom`
    Bottom = 178,
    /// `box-align`
    BoxAlign = 179,
    /// `box-decoration-break`
    BoxDecorationBreak = 180,
    /// `box-direction`
    BoxDirection = 181,
    /// `box-flex`
    BoxFlex = 182,
    /// `box-flex-group`
    BoxFlexGroup = 183,
    /// `box-lines`
    BoxLines = 184,
    /// `box-ordinal-group`
    BoxOrdinalGroup = 185,
    /// `box-orient`
    BoxOrient = 186,
    /// `box-pack`
    BoxPack = 187,
    /// `box-shadow`
    BoxShadow = 188,
    /// `box-sizing`
    BoxSizing = 189,
    /// `break-after`
    BreakAfter = 190,
    /// `break-before`
    BreakBefore = 191,
    /// `break-inside`
    BreakInside = 192,
    /// `caption-side`
    CaptionSide = 193,
    /// `caret-animation`
    CaretAnimation = 194,
    /// `caret-color`
    CaretColor = 195,
    /// `caret-shape`
    CaretShape = 196,
    /// `clear`
    Clear = 197,
    /// `clip`
    Clip = 198,
    /// `clip-path`
    ClipPath = 199,
    /// `clip-rule`
    ClipRule = 200,
    /// `color`
    Color = 201,
    /// `color-interpolation-filters`
    ColorInterpolationFilters = 202,
    /// `color-scheme`
    ColorScheme = 203,
    /// `column-count`
    ColumnCount = 204,
    /// `column-fill`
    ColumnFill = 205,
    /// `column-gap`
    ColumnGap = 206,
    /// `column-height`
    ColumnHeight = 207,
    /// `column-rule-color`
    ColumnRuleColor = 208,
    /// `column-rule-style`
    ColumnRuleStyle = 209,
    /// `column-rule-width`
    ColumnRuleWidth = 210,
    /// `column-span`
    ColumnSpan = 211,
    /// `column-width`
    ColumnWidth = 212,
    /// `column-wrap`
    ColumnWrap = 213,
    /// `contain`
    Contain = 214,
    /// `contain-intrinsic-block-size`
    ContainIntrinsicBlockSize = 215,
    /// `contain-intrinsic-height`
    ContainIntrinsicHeight = 216,
    /// `contain-intrinsic-inline-size`
    ContainIntrinsicInlineSize = 217,
    /// `contain-intrinsic-width`
    ContainIntrinsicWidth = 218,
    /// `container-name`
    ContainerName = 219,
    /// `container-type`
    ContainerType = 220,
    /// `content`
    Content = 221,
    /// `content-visibility`
    ContentVisibility = 222,
    /// `corner-bottom-left-shape`
    CornerBottomLeftShape = 223,
    /// `corner-bottom-right-shape`
    CornerBottomRightShape = 224,
    /// `corner-end-end-shape`
    CornerEndEndShape = 225,
    /// `corner-end-start-shape`
    CornerEndStartShape = 226,
    /// `corner-start-end-shape`
    CornerStartEndShape = 227,
    /// `corner-start-start-shape`
    CornerStartStartShape = 228,
    /// `corner-top-left-shape`
    CornerTopLeftShape = 229,
    /// `corner-top-right-shape`
    CornerTopRightShape = 230,
    /// `counter-increment`
    CounterIncrement = 231,
    /// `counter-reset`
    CounterReset = 232,
    /// `counter-set`
    CounterSet = 233,
    /// `cursor`
    Cursor = 234,
    /// `cx`
    Cx = 235,
    /// `cy`
    Cy = 236,
    /// `d`
    D = 237,
    /// `direction`
    Direction = 238,
    /// `display`
    Display = 239,
    /// `dominant-baseline`
    DominantBaseline = 240,
    /// `dynamic-range-limit`
    DynamicRangeLimit = 241,
    /// `empty-cells`
    EmptyCells = 242,
    /// `field-sizing`
    FieldSizing = 243,
    /// `fill`
    Fill = 244,
    /// `fill-opacity`
    FillOpacity = 245,
    /// `fill-rule`
    FillRule = 246,
    /// `filter`
    Filter = 247,
    /// `flex-basis`
    FlexBasis = 248,
    /// `flex-direction`
    FlexDirection = 249,
    /// `flex-grow`
    FlexGrow = 250,
    /// `flex-shrink`
    FlexShrink = 251,
    /// `flex-wrap`
    FlexWrap = 252,
    /// `float`
    Float = 253,
    /// `flood-color`
    FloodColor = 254,
    /// `flood-opacity`
    FloodOpacity = 255,
    /// `font-family`
    FontFamily = 256,
    /// `font-feature-settings`
    FontFeatureSettings = 257,
    /// `font-kerning`
    FontKerning = 258,
    /// `font-language-override`
    FontLanguageOverride = 259,
    /// `font-optical-sizing`
    FontOpticalSizing = 260,
    /// `font-palette`
    FontPalette = 261,
    /// `font-size`
    FontSize = 262,
    /// `font-size-adjust`
    FontSizeAdjust = 263,
    /// `font-smooth`
    FontSmooth = 264,
    /// `font-stretch`
    FontStretch = 265,
    /// `font-style`
    FontStyle = 266,
    /// `font-synthesis`
    FontSynthesis = 267,
    /// `font-synthesis-position`
    FontSynthesisPosition = 268,
    /// `font-synthesis-small-caps`
    FontSynthesisSmallCaps = 269,
    /// `font-synthesis-style`
    FontSynthesisStyle = 270,
    /// `font-synthesis-weight`
    FontSynthesisWeight = 271,
    /// `font-variant`
    FontVariant = 272,
    /// `font-variant-alternates`
    FontVariantAlternates = 273,
    /// `font-variant-caps`
    FontVariantCaps = 274,
    /// `font-variant-east-asian`
    FontVariantEastAsian = 275,
    /// `font-variant-emoji`
    FontVariantEmoji = 276,
    /// `font-variant-ligatures`
    FontVariantLigatures = 277,
    /// `font-variant-numeric`
    FontVariantNumeric = 278,
    /// `font-variant-position`
    FontVariantPosition = 279,
    /// `font-variation-settings`
    FontVariationSettings = 280,
    /// `font-weight`
    FontWeight = 281,
    /// `font-width`
    FontWidth = 282,
    /// `forced-color-adjust`
    ForcedColorAdjust = 283,
    /// `frame-sizing`
    FrameSizing = 284,
    /// `grid-auto-columns`
    GridAutoColumns = 285,
    /// `grid-auto-flow`
    GridAutoFlow = 286,
    /// `grid-auto-rows`
    GridAutoRows = 287,
    /// `grid-column-end`
    GridColumnEnd = 288,
    /// `grid-column-gap`
    GridColumnGap = 289,
    /// `grid-column-start`
    GridColumnStart = 290,
    /// `grid-row-end`
    GridRowEnd = 291,
    /// `grid-row-gap`
    GridRowGap = 292,
    /// `grid-row-start`
    GridRowStart = 293,
    /// `grid-template-areas`
    GridTemplateAreas = 294,
    /// `grid-template-columns`
    GridTemplateColumns = 295,
    /// `grid-template-rows`
    GridTemplateRows = 296,
    /// `hanging-punctuation`
    HangingPunctuation = 297,
    /// `height`
    Height = 298,
    /// `hyphenate-character`
    HyphenateCharacter = 299,
    /// `hyphenate-limit-chars`
    HyphenateLimitChars = 300,
    /// `hyphens`
    Hyphens = 301,
    /// `image-orientation`
    ImageOrientation = 302,
    /// `image-rendering`
    ImageRendering = 303,
    /// `image-resolution`
    ImageResolution = 304,
    /// `ime-mode`
    ImeMode = 305,
    /// `initial-letter`
    InitialLetter = 306,
    /// `initial-letter-align`
    InitialLetterAlign = 307,
    /// `inline-size`
    InlineSize = 308,
    /// `inset-block-end`
    InsetBlockEnd = 309,
    /// `inset-block-start`
    InsetBlockStart = 310,
    /// `inset-inline-end`
    InsetInlineEnd = 311,
    /// `inset-inline-start`
    InsetInlineStart = 312,
    /// `interactivity`
    Interactivity = 313,
    /// `interest-delay-end`
    InterestDelayEnd = 314,
    /// `interest-delay-start`
    InterestDelayStart = 315,
    /// `interpolate-size`
    InterpolateSize = 316,
    /// `isolation`
    Isolation = 317,
    /// `justify-content`
    JustifyContent = 318,
    /// `justify-items`
    JustifyItems = 319,
    /// `justify-self`
    JustifySelf = 320,
    /// `justify-tracks`
    JustifyTracks = 321,
    /// `left`
    Left = 322,
    /// `letter-spacing`
    LetterSpacing = 323,
    /// `lighting-color`
    LightingColor = 324,
    /// `line-break`
    LineBreak = 325,
    /// `line-clamp`
    LineClamp = 326,
    /// `line-height`
    LineHeight = 327,
    /// `line-height-step`
    LineHeightStep = 328,
    /// `list-style-image`
    ListStyleImage = 329,
    /// `list-style-position`
    ListStylePosition = 330,
    /// `list-style-type`
    ListStyleType = 331,
    /// `margin-block-end`
    MarginBlockEnd = 332,
    /// `margin-block-start`
    MarginBlockStart = 333,
    /// `margin-bottom`
    MarginBottom = 334,
    /// `margin-inline-end`
    MarginInlineEnd = 335,
    /// `margin-inline-start`
    MarginInlineStart = 336,
    /// `margin-left`
    MarginLeft = 337,
    /// `margin-right`
    MarginRight = 338,
    /// `margin-top`
    MarginTop = 339,
    /// `margin-trim`
    MarginTrim = 340,
    /// `marker`
    Marker = 341,
    /// `marker-end`
    MarkerEnd = 342,
    /// `marker-mid`
    MarkerMid = 343,
    /// `marker-start`
    MarkerStart = 344,
    /// `mask-border-mode`
    MaskBorderMode = 345,
    /// `mask-border-outset`
    MaskBorderOutset = 346,
    /// `mask-border-repeat`
    MaskBorderRepeat = 347,
    /// `mask-border-slice`
    MaskBorderSlice = 348,
    /// `mask-border-source`
    MaskBorderSource = 349,
    /// `mask-border-width`
    MaskBorderWidth = 350,
    /// `mask-clip`
    MaskClip = 351,
    /// `mask-composite`
    MaskComposite = 352,
    /// `mask-image`
    MaskImage = 353,
    /// `mask-mode`
    MaskMode = 354,
    /// `mask-origin`
    MaskOrigin = 355,
    /// `mask-position`
    MaskPosition = 356,
    /// `mask-repeat`
    MaskRepeat = 357,
    /// `mask-size`
    MaskSize = 358,
    /// `mask-type`
    MaskType = 359,
    /// `masonry-auto-flow`
    MasonryAutoFlow = 360,
    /// `math-depth`
    MathDepth = 361,
    /// `math-shift`
    MathShift = 362,
    /// `math-style`
    MathStyle = 363,
    /// `max-block-size`
    MaxBlockSize = 364,
    /// `max-height`
    MaxHeight = 365,
    /// `max-inline-size`
    MaxInlineSize = 366,
    /// `max-lines`
    MaxLines = 367,
    /// `max-width`
    MaxWidth = 368,
    /// `min-block-size`
    MinBlockSize = 369,
    /// `min-height`
    MinHeight = 370,
    /// `min-inline-size`
    MinInlineSize = 371,
    /// `min-width`
    MinWidth = 372,
    /// `mix-blend-mode`
    MixBlendMode = 373,
    /// `object-fit`
    ObjectFit = 374,
    /// `object-position`
    ObjectPosition = 375,
    /// `object-view-box`
    ObjectViewBox = 376,
    /// `offset-anchor`
    OffsetAnchor = 377,
    /// `offset-distance`
    OffsetDistance = 378,
    /// `offset-path`
    OffsetPath = 379,
    /// `offset-position`
    OffsetPosition = 380,
    /// `offset-rotate`
    OffsetRotate = 381,
    /// `opacity`
    Opacity = 382,
    /// `order`
    Order = 383,
    /// `orphans`
    Orphans = 384,
    /// `outline-color`
    OutlineColor = 385,
    /// `outline-offset`
    OutlineOffset = 386,
    /// `outline-style`
    OutlineStyle = 387,
    /// `outline-width`
    OutlineWidth = 388,
    /// `overflow-anchor`
    OverflowAnchor = 389,
    /// `overflow-block`
    OverflowBlock = 390,
    /// `overflow-clip-box`
    OverflowClipBox = 391,
    /// `overflow-clip-margin`
    OverflowClipMargin = 392,
    /// `overflow-inline`
    OverflowInline = 393,
    /// `overflow-wrap`
    OverflowWrap = 394,
    /// `overflow-x`
    OverflowX = 395,
    /// `overflow-y`
    OverflowY = 396,
    /// `overlay`
    Overlay = 397,
    /// `overscroll-behavior-block`
    OverscrollBehaviorBlock = 398,
    /// `overscroll-behavior-inline`
    OverscrollBehaviorInline = 399,
    /// `overscroll-behavior-x`
    OverscrollBehaviorX = 400,
    /// `overscroll-behavior-y`
    OverscrollBehaviorY = 401,
    /// `padding-block-end`
    PaddingBlockEnd = 402,
    /// `padding-block-start`
    PaddingBlockStart = 403,
    /// `padding-bottom`
    PaddingBottom = 404,
    /// `padding-inline-end`
    PaddingInlineEnd = 405,
    /// `padding-inline-start`
    PaddingInlineStart = 406,
    /// `padding-left`
    PaddingLeft = 407,
    /// `padding-right`
    PaddingRight = 408,
    /// `padding-top`
    PaddingTop = 409,
    /// `page`
    Page = 410,
    /// `page-break-after`
    PageBreakAfter = 411,
    /// `page-break-before`
    PageBreakBefore = 412,
    /// `page-break-inside`
    PageBreakInside = 413,
    /// `paint-order`
    PaintOrder = 414,
    /// `perspective`
    Perspective = 415,
    /// `perspective-origin`
    PerspectiveOrigin = 416,
    /// `pointer-events`
    PointerEvents = 417,
    /// `position`
    Position = 418,
    /// `position-anchor`
    PositionAnchor = 419,
    /// `position-area`
    PositionArea = 420,
    /// `position-try-fallbacks`
    PositionTryFallbacks = 421,
    /// `position-try-order`
    PositionTryOrder = 422,
    /// `position-visibility`
    PositionVisibility = 423,
    /// `print-color-adjust`
    PrintColorAdjust = 424,
    /// `quotes`
    Quotes = 425,
    /// `r`
    R = 426,
    /// `reading-flow`
    ReadingFlow = 427,
    /// `reading-order`
    ReadingOrder = 428,
    /// `resize`
    Resize = 429,
    /// `right`
    Right = 430,
    /// `rotate`
    Rotate = 431,
    /// `row-gap`
    RowGap = 432,
    /// `ruby-align`
    RubyAlign = 433,
    /// `ruby-merge`
    RubyMerge = 434,
    /// `ruby-overhang`
    RubyOverhang = 435,
    /// `ruby-position`
    RubyPosition = 436,
    /// `rx`
    Rx = 437,
    /// `ry`
    Ry = 438,
    /// `scale`
    Scale = 439,
    /// `scroll-behavior`
    ScrollBehavior = 440,
    /// `scroll-initial-target`
    ScrollInitialTarget = 441,
    /// `scroll-margin-block-end`
    ScrollMarginBlockEnd = 442,
    /// `scroll-margin-block-start`
    ScrollMarginBlockStart = 443,
    /// `scroll-margin-bottom`
    ScrollMarginBottom = 444,
    /// `scroll-margin-inline-end`
    ScrollMarginInlineEnd = 445,
    /// `scroll-margin-inline-start`
    ScrollMarginInlineStart = 446,
    /// `scroll-margin-left`
    ScrollMarginLeft = 447,
    /// `scroll-margin-right`
    ScrollMarginRight = 448,
    /// `scroll-margin-top`
    ScrollMarginTop = 449,
    /// `scroll-marker-group`
    ScrollMarkerGroup = 450,
    /// `scroll-padding-block-end`
    ScrollPaddingBlockEnd = 451,
    /// `scroll-padding-block-start`
    ScrollPaddingBlockStart = 452,
    /// `scroll-padding-bottom`
    ScrollPaddingBottom = 453,
    /// `scroll-padding-inline-end`
    ScrollPaddingInlineEnd = 454,
    /// `scroll-padding-inline-start`
    ScrollPaddingInlineStart = 455,
    /// `scroll-padding-left`
    ScrollPaddingLeft = 456,
    /// `scroll-padding-right`
    ScrollPaddingRight = 457,
    /// `scroll-padding-top`
    ScrollPaddingTop = 458,
    /// `scroll-snap-align`
    ScrollSnapAlign = 459,
    /// `scroll-snap-coordinate`
    ScrollSnapCoordinate = 460,
    /// `scroll-snap-destination`
    ScrollSnapDestination = 461,
    /// `scroll-snap-points-x`
    ScrollSnapPointsX = 462,
    /// `scroll-snap-points-y`
    ScrollSnapPointsY = 463,
    /// `scroll-snap-stop`
    ScrollSnapStop = 464,
    /// `scroll-snap-type`
    ScrollSnapType = 465,
    /// `scroll-snap-type-x`
    ScrollSnapTypeX = 466,
    /// `scroll-snap-type-y`
    ScrollSnapTypeY = 467,
    /// `scroll-target-group`
    ScrollTargetGroup = 468,
    /// `scroll-timeline-axis`
    ScrollTimelineAxis = 469,
    /// `scroll-timeline-name`
    ScrollTimelineName = 470,
    /// `scrollbar-color`
    ScrollbarColor = 471,
    /// `scrollbar-gutter`
    ScrollbarGutter = 472,
    /// `scrollbar-width`
    ScrollbarWidth = 473,
    /// `shape-image-threshold`
    ShapeImageThreshold = 474,
    /// `shape-margin`
    ShapeMargin = 475,
    /// `shape-outside`
    ShapeOutside = 476,
    /// `shape-rendering`
    ShapeRendering = 477,
    /// `speak-as`
    SpeakAs = 478,
    /// `stop-color`
    StopColor = 479,
    /// `stop-opacity`
    StopOpacity = 480,
    /// `stroke`
    Stroke = 481,
    /// `stroke-color`
    StrokeColor = 482,
    /// `stroke-dasharray`
    StrokeDasharray = 483,
    /// `stroke-dashoffset`
    StrokeDashoffset = 484,
    /// `stroke-linecap`
    StrokeLinecap = 485,
    /// `stroke-linejoin`
    StrokeLinejoin = 486,
    /// `stroke-miterlimit`
    StrokeMiterlimit = 487,
    /// `stroke-opacity`
    StrokeOpacity = 488,
    /// `stroke-width`
    StrokeWidth = 489,
    /// `tab-size`
    TabSize = 490,
    /// `table-layout`
    TableLayout = 491,
    /// `text-align`
    TextAlign = 492,
    /// `text-align-last`
    TextAlignLast = 493,
    /// `text-anchor`
    TextAnchor = 494,
    /// `text-autospace`
    TextAutospace = 495,
    /// `text-box`
    TextBox = 496,
    /// `text-box-edge`
    TextBoxEdge = 497,
    /// `text-box-trim`
    TextBoxTrim = 498,
    /// `text-combine-upright`
    TextCombineUpright = 499,
    /// `text-decoration-color`
    TextDecorationColor = 500,
    /// `text-decoration-inset`
    TextDecorationInset = 501,
    /// `text-decoration-line`
    TextDecorationLine = 502,
    /// `text-decoration-skip`
    TextDecorationSkip = 503,
    /// `text-decoration-skip-ink`
    TextDecorationSkipInk = 504,
    /// `text-decoration-style`
    TextDecorationStyle = 505,
    /// `text-decoration-thickness`
    TextDecorationThickness = 506,
    /// `text-emphasis-color`
    TextEmphasisColor = 507,
    /// `text-emphasis-position`
    TextEmphasisPosition = 508,
    /// `text-emphasis-style`
    TextEmphasisStyle = 509,
    /// `text-indent`
    TextIndent = 510,
    /// `text-justify`
    TextJustify = 511,
    /// `text-orientation`
    TextOrientation = 512,
    /// `text-overflow`
    TextOverflow = 513,
    /// `text-rendering`
    TextRendering = 514,
    /// `text-shadow`
    TextShadow = 515,
    /// `text-size-adjust`
    TextSizeAdjust = 516,
    /// `text-spacing-trim`
    TextSpacingTrim = 517,
    /// `text-transform`
    TextTransform = 518,
    /// `text-underline-offset`
    TextUnderlineOffset = 519,
    /// `text-underline-position`
    TextUnderlinePosition = 520,
    /// `text-wrap-mode`
    TextWrapMode = 521,
    /// `text-wrap-style`
    TextWrapStyle = 522,
    /// `timeline-scope`
    TimelineScope = 523,
    /// `timeline-trigger-activation-range-end`
    TimelineTriggerActivationRangeEnd = 524,
    /// `timeline-trigger-activation-range-start`
    TimelineTriggerActivationRangeStart = 525,
    /// `timeline-trigger-active-range-end`
    TimelineTriggerActiveRangeEnd = 526,
    /// `timeline-trigger-active-range-start`
    TimelineTriggerActiveRangeStart = 527,
    /// `timeline-trigger-name`
    TimelineTriggerName = 528,
    /// `timeline-trigger-source`
    TimelineTriggerSource = 529,
    /// `top`
    Top = 530,
    /// `touch-action`
    TouchAction = 531,
    /// `transform`
    Transform = 532,
    /// `transform-box`
    TransformBox = 533,
    /// `transform-origin`
    TransformOrigin = 534,
    /// `transform-style`
    TransformStyle = 535,
    /// `transition-behavior`
    TransitionBehavior = 536,
    /// `transition-delay`
    TransitionDelay = 537,
    /// `transition-duration`
    TransitionDuration = 538,
    /// `transition-property`
    TransitionProperty = 539,
    /// `transition-timing-function`
    TransitionTimingFunction = 540,
    /// `translate`
    Translate = 541,
    /// `trigger-scope`
    TriggerScope = 542,
    /// `unicode-bidi`
    UnicodeBidi = 543,
    /// `user-select`
    UserSelect = 544,
    /// `vector-effect`
    VectorEffect = 545,
    /// `vertical-align`
    VerticalAlign = 546,
    /// `view-timeline-axis`
    ViewTimelineAxis = 547,
    /// `view-timeline-inset`
    ViewTimelineInset = 548,
    /// `view-timeline-name`
    ViewTimelineName = 549,
    /// `view-transition-class`
    ViewTransitionClass = 550,
    /// `view-transition-name`
    ViewTransitionName = 551,
    /// `view-transition-scope`
    ViewTransitionScope = 552,
    /// `visibility`
    Visibility = 553,
    /// `white-space`
    WhiteSpace = 554,
    /// `white-space-collapse`
    WhiteSpaceCollapse = 555,
    /// `widows`
    Widows = 556,
    /// `width`
    Width = 557,
    /// `will-change`
    WillChange = 558,
    /// `word-break`
    WordBreak = 559,
    /// `word-spacing`
    WordSpacing = 560,
    /// `word-wrap`
    WordWrap = 561,
    /// `writing-mode`
    WritingMode = 562,
    /// `x`
    X = 563,
    /// `y`
    Y = 564,
    /// `z-index`
    ZIndex = 565,
    /// `zoom`
    Zoom = 566,
}

/// A property whose value is distributed over other properties; see
/// [`ShorthandId::longhands`].
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShorthandId {
    /// `-moz-outline-radius`
    MozOutlineRadius = 0,
    /// `-ms-content-zoom-limit`
    MsContentZoomLimit = 1,
    /// `-ms-content-zoom-snap`
    MsContentZoomSnap = 2,
    /// `-ms-scroll-limit`
    MsScrollLimit = 3,
    /// `-ms-scroll-snap-x`
    MsScrollSnapX = 4,
    /// `-ms-scroll-snap-y`
    MsScrollSnapY = 5,
    /// `-webkit-border-after`
    WebkitBorderAfter = 6,
    /// `-webkit-border-before`
    WebkitBorderBefore = 7,
    /// `-webkit-border-end`
    WebkitBorderEnd = 8,
    /// `-webkit-border-start`
    WebkitBorderStart = 9,
    /// `-webkit-mask`
    WebkitMask = 10,
    /// `-webkit-text-stroke`
    WebkitTextStroke = 11,
    /// `animation`
    Animation = 12,
    /// `animation-range`
    AnimationRange = 13,
    /// `background`
    Background = 14,
    /// `background-position`
    BackgroundPosition = 15,
    /// `border`
    Border = 16,
    /// `border-block`
    BorderBlock = 17,
    /// `border-block-color`
    BorderBlockColor = 18,
    /// `border-block-end`
    BorderBlockEnd = 19,
    /// `border-block-start`
    BorderBlockStart = 20,
    /// `border-block-style`
    BorderBlockStyle = 21,
    /// `border-block-width`
    BorderBlockWidth = 22,
    /// `border-bottom`
    BorderBottom = 23,
    /// `border-color`
    BorderColor = 24,
    /// `border-image`
    BorderImage = 25,
    /// `border-inline`
    BorderInline = 26,
    /// `border-inline-color`
    BorderInlineColor = 27,
    /// `border-inline-end`
    BorderInlineEnd = 28,
    /// `border-inline-start`
    BorderInlineStart = 29,
    /// `border-inline-style`
    BorderInlineStyle = 30,
    /// `border-inline-width`
    BorderInlineWidth = 31,
    /// `border-left`
    BorderLeft = 32,
    /// `border-radius`
    BorderRadius = 33,
    /// `border-right`
    BorderRight = 34,
    /// `border-style`
    BorderStyle = 35,
    /// `border-top`
    BorderTop = 36,
    /// `border-width`
    BorderWidth = 37,
    /// `caret`
    Caret = 38,
    /// `column-rule`
    ColumnRule = 39,
    /// `columns`
    Columns = 40,
    /// `contain-intrinsic-size`
    ContainIntrinsicSize = 41,
    /// `container`
    Container = 42,
    /// `corner-block-end-shape`
    CornerBlockEndShape = 43,
    /// `corner-block-start-shape`
    CornerBlockStartShape = 44,
    /// `corner-bottom-shape`
    CornerBottomShape = 45,
    /// `corner-inline-end-shape`
    CornerInlineEndShape = 46,
    /// `corner-inline-start-shape`
    CornerInlineStartShape = 47,
    /// `corner-left-shape`
    CornerLeftShape = 48,
    /// `corner-right-shape`
    CornerRightShape = 49,
    /// `corner-shape`
    CornerShape = 50,
    /// `corner-top-shape`
    CornerTopShape = 51,
    /// `flex`
    Flex = 52,
    /// `flex-flow`
    FlexFlow = 53,
    /// `font`
    Font = 54,
    /// `gap`
    Gap = 55,
    /// `grid`
    Grid = 56,
    /// `grid-area`
    GridArea = 57,
    /// `grid-column`
    GridColumn = 58,
    /// `grid-gap`
    GridGap = 59,
    /// `grid-row`
    GridRow = 60,
    /// `grid-template`
    GridTemplate = 61,
    /// `inset`
    Inset = 62,
    /// `inset-block`
    InsetBlock = 63,
    /// `inset-inline`
    InsetInline = 64,
    /// `interest-delay`
    InterestDelay = 65,
    /// `list-style`
    ListStyle = 66,
    /// `margin`
    Margin = 67,
    /// `margin-block`
    MarginBlock = 68,
    /// `margin-inline`
    MarginInline = 69,
    /// `mask`
    Mask = 70,
    /// `mask-border`
    MaskBorder = 71,
    /// `offset`
    Offset = 72,
    /// `outline`
    Outline = 73,
    /// `overflow`
    Overflow = 74,
    /// `overscroll-behavior`
    OverscrollBehavior = 75,
    /// `padding`
    Padding = 76,
    /// `padding-block`
    PaddingBlock = 77,
    /// `padding-inline`
    PaddingInline = 78,
    /// `place-content`
    PlaceContent = 79,
    /// `place-items`
    PlaceItems = 80,
    /// `place-self`
    PlaceSelf = 81,
    /// `position-try`
    PositionTry = 82,
    /// `scroll-margin`
    ScrollMargin = 83,
    /// `scroll-margin-block`
    ScrollMarginBlock = 84,
    /// `scroll-margin-inline`
    ScrollMarginInline = 85,
    /// `scroll-padding`
    ScrollPadding = 86,
    /// `scroll-padding-block`
    ScrollPaddingBlock = 87,
    /// `scroll-padding-inline`
    ScrollPaddingInline = 88,
    /// `scroll-timeline`
    ScrollTimeline = 89,
    /// `text-decoration`
    TextDecoration = 90,
    /// `text-emphasis`
    TextEmphasis = 91,
    /// `text-wrap`
    TextWrap = 92,
    /// `timeline-trigger`
    TimelineTrigger = 93,
    /// `timeline-trigger-activation-range`
    TimelineTriggerActivationRange = 94,
    /// `timeline-trigger-active-range`
    TimelineTriggerActiveRange = 95,
    /// `transition`
    Transition = 96,
    /// `view-timeline`
    ViewTimeline = 97,
}

/// A property this engine knows, as a longhand or a shorthand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PropertyId {
    Longhand(LonghandId),
    Shorthand(ShorthandId),
}

/// What the definition data says about one longhand.
struct LonghandInfo {
    name: &'static str,
    inherited: bool,
    /// The initial value as written in the definition data, before it is parsed.
    initial: Option<&'static str>,
    percentage_is_number: bool,
}

/// What the definition data says about one shorthand.
struct ShorthandInfo {
    name: &'static str,
    inherited: bool,
    initial: Option<&'static str>,
    percentage_is_number: bool,
    /// The properties the shorthand sets, as its `computed` list names them. A few
    /// of these are shorthands in their own right (`border` sets `border-color`).
    longhands: &'static [PropertyId],
}

#[rustfmt::skip]
static LONGHANDS: [LonghandInfo; LONGHAND_COUNT] = [
    LonghandInfo { name: "-moz-appearance", inherited: false, initial: Some("noneButOverriddenInUserAgentCSS"), percentage_is_number: false },
    LonghandInfo { name: "-moz-binding", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-moz-border-bottom-colors", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-moz-border-left-colors", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-moz-border-right-colors", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-moz-border-top-colors", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-moz-context-properties", inherited: true, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-moz-float-edge", inherited: false, initial: Some("content-box"), percentage_is_number: false },
    LonghandInfo { name: "-moz-force-broken-image-icon", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "-moz-orient", inherited: false, initial: Some("inline"), percentage_is_number: false },
    LonghandInfo { name: "-moz-outline-radius-bottomleft", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "-moz-outline-radius-bottomright", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "-moz-outline-radius-topleft", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "-moz-outline-radius-topright", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "-moz-stack-sizing", inherited: true, initial: Some("stretch-to-fit"), percentage_is_number: false },
    LonghandInfo { name: "-moz-text-blink", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-moz-user-focus", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-moz-user-input", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "-moz-user-modify", inherited: true, initial: Some("read-only"), percentage_is_number: false },
    LonghandInfo { name: "-moz-window-dragging", inherited: false, initial: Some("drag"), percentage_is_number: false },
    LonghandInfo { name: "-moz-window-shadow", inherited: false, initial: Some("default"), percentage_is_number: false },
    LonghandInfo { name: "-ms-accelerator", inherited: false, initial: Some("false"), percentage_is_number: false },
    LonghandInfo { name: "-ms-block-progression", inherited: false, initial: Some("tb"), percentage_is_number: false },
    LonghandInfo { name: "-ms-content-zoom-chaining", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-ms-content-zoom-limit-max", inherited: false, initial: Some("400%"), percentage_is_number: false },
    LonghandInfo { name: "-ms-content-zoom-limit-min", inherited: false, initial: Some("100%"), percentage_is_number: false },
    LonghandInfo { name: "-ms-content-zoom-snap-points", inherited: false, initial: Some("snapInterval(0%, 100%)"), percentage_is_number: false },
    LonghandInfo { name: "-ms-content-zoom-snap-type", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-ms-content-zooming", inherited: false, initial: Some("zoomForTheTopLevelNoneForTheRest"), percentage_is_number: false },
    LonghandInfo { name: "-ms-filter", inherited: false, initial: Some("\"\""), percentage_is_number: false },
    LonghandInfo { name: "-ms-flow-from", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-ms-flow-into", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-ms-grid-columns", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-ms-grid-rows", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-ms-high-contrast-adjust", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "-ms-hyphenate-limit-chars", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "-ms-hyphenate-limit-lines", inherited: true, initial: Some("no-limit"), percentage_is_number: false },
    LonghandInfo { name: "-ms-hyphenate-limit-zone", inherited: true, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "-ms-ime-align", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "-ms-overflow-style", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "-ms-scroll-chaining", inherited: false, initial: Some("chained"), percentage_is_number: false },
    LonghandInfo { name: "-ms-scroll-limit-x-max", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "-ms-scroll-limit-x-min", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "-ms-scroll-limit-y-max", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "-ms-scroll-limit-y-min", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "-ms-scroll-rails", inherited: false, initial: Some("railed"), percentage_is_number: false },
    LonghandInfo { name: "-ms-scroll-snap-points-x", inherited: false, initial: Some("snapInterval(0px, 100%)"), percentage_is_number: false },
    LonghandInfo { name: "-ms-scroll-snap-points-y", inherited: false, initial: Some("snapInterval(0px, 100%)"), percentage_is_number: false },
    LonghandInfo { name: "-ms-scroll-snap-type", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-ms-scroll-translation", inherited: true, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-ms-scrollbar-3dlight-color", inherited: true, initial: Some("dependsOnUserAgent"), percentage_is_number: false },
    LonghandInfo { name: "-ms-scrollbar-arrow-color", inherited: true, initial: Some("ButtonText"), percentage_is_number: false },
    LonghandInfo { name: "-ms-scrollbar-base-color", inherited: true, initial: Some("dependsOnUserAgent"), percentage_is_number: false },
    LonghandInfo { name: "-ms-scrollbar-darkshadow-color", inherited: true, initial: Some("ThreeDDarkShadow"), percentage_is_number: false },
    LonghandInfo { name: "-ms-scrollbar-face-color", inherited: true, initial: Some("ThreeDFace"), percentage_is_number: false },
    LonghandInfo { name: "-ms-scrollbar-highlight-color", inherited: true, initial: Some("ThreeDHighlight"), percentage_is_number: false },
    LonghandInfo { name: "-ms-scrollbar-shadow-color", inherited: true, initial: Some("ThreeDDarkShadow"), percentage_is_number: false },
    LonghandInfo { name: "-ms-scrollbar-track-color", inherited: true, initial: Some("Scrollbar"), percentage_is_number: false },
    LonghandInfo { name: "-ms-text-autospace", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-ms-touch-select", inherited: true, initial: Some("grippers"), percentage_is_number: false },
    LonghandInfo { name: "-ms-user-select", inherited: false, initial: Some("text"), percentage_is_number: false },
    LonghandInfo { name: "-ms-wrap-flow", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "-ms-wrap-margin", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "-ms-wrap-through", inherited: false, initial: Some("wrap"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-appearance", inherited: false, initial: Some("noneButOverriddenInUserAgentCSS"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-border-after-color", inherited: false, initial: Some("currentcolor"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-border-after-style", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-border-after-width", inherited: false, initial: Some("medium"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-border-before-color", inherited: false, initial: Some("currentcolor"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-border-before-style", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-border-before-width", inherited: false, initial: Some("medium"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-border-end-color", inherited: false, initial: Some("currentcolor"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-border-end-style", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-border-end-width", inherited: false, initial: Some("medium"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-border-start-color", inherited: false, initial: Some("currentcolor"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-border-start-style", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-border-start-width", inherited: false, initial: Some("medium"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-box-reflect", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-line-clamp", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-mask-attachment", inherited: false, initial: Some("scroll"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-mask-clip", inherited: false, initial: Some("border"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-mask-composite", inherited: false, initial: Some("source-over"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-mask-image", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-mask-origin", inherited: false, initial: Some("padding"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-mask-position", inherited: false, initial: Some("0% 0%"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-mask-position-x", inherited: false, initial: Some("0%"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-mask-position-y", inherited: false, initial: Some("0%"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-mask-repeat", inherited: false, initial: Some("repeat"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-mask-repeat-x", inherited: false, initial: Some("repeat"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-mask-repeat-y", inherited: false, initial: Some("repeat"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-mask-size", inherited: false, initial: Some("auto auto"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-overflow-scrolling", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-tap-highlight-color", inherited: true, initial: Some("black"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-text-fill-color", inherited: true, initial: Some("currentcolor"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-text-stroke-color", inherited: true, initial: Some("currentcolor"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-text-stroke-width", inherited: true, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-touch-callout", inherited: true, initial: Some("default"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-user-modify", inherited: true, initial: Some("read-only"), percentage_is_number: false },
    LonghandInfo { name: "-webkit-user-select", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "accent-color", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "align-content", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "align-items", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "align-self", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "align-tracks", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "alignment-baseline", inherited: false, initial: Some("baseline"), percentage_is_number: false },
    LonghandInfo { name: "all", inherited: false, initial: Some("noPracticalInitialValue"), percentage_is_number: false },
    LonghandInfo { name: "anchor-name", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "anchor-scope", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "animation-composition", inherited: false, initial: Some("replace"), percentage_is_number: false },
    LonghandInfo { name: "animation-delay", inherited: false, initial: Some("0s"), percentage_is_number: false },
    LonghandInfo { name: "animation-direction", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "animation-duration", inherited: false, initial: Some("0s"), percentage_is_number: false },
    LonghandInfo { name: "animation-fill-mode", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "animation-iteration-count", inherited: false, initial: Some("1"), percentage_is_number: false },
    LonghandInfo { name: "animation-name", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "animation-play-state", inherited: false, initial: Some("running"), percentage_is_number: false },
    LonghandInfo { name: "animation-range-end", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "animation-range-start", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "animation-timeline", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "animation-timing-function", inherited: false, initial: Some("ease"), percentage_is_number: false },
    LonghandInfo { name: "animation-trigger", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "appearance", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "aspect-ratio", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "backdrop-filter", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "backface-visibility", inherited: false, initial: Some("visible"), percentage_is_number: false },
    LonghandInfo { name: "background-attachment", inherited: false, initial: Some("scroll"), percentage_is_number: false },
    LonghandInfo { name: "background-blend-mode", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "background-clip", inherited: false, initial: Some("border-box"), percentage_is_number: false },
    LonghandInfo { name: "background-color", inherited: false, initial: Some("transparent"), percentage_is_number: false },
    LonghandInfo { name: "background-image", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "background-origin", inherited: false, initial: Some("padding-box"), percentage_is_number: false },
    LonghandInfo { name: "background-position-x", inherited: false, initial: Some("0%"), percentage_is_number: false },
    LonghandInfo { name: "background-position-y", inherited: false, initial: Some("0%"), percentage_is_number: false },
    LonghandInfo { name: "background-repeat", inherited: false, initial: Some("repeat"), percentage_is_number: false },
    LonghandInfo { name: "background-size", inherited: false, initial: Some("auto auto"), percentage_is_number: false },
    LonghandInfo { name: "baseline-shift", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "baseline-source", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "block-size", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "border-block-end-color", inherited: false, initial: Some("currentcolor"), percentage_is_number: false },
    LonghandInfo { name: "border-block-end-style", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "border-block-end-width", inherited: false, initial: Some("medium"), percentage_is_number: false },
    LonghandInfo { name: "border-block-start-color", inherited: false, initial: Some("currentcolor"), percentage_is_number: false },
    LonghandInfo { name: "border-block-start-style", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "border-block-start-width", inherited: false, initial: Some("medium"), percentage_is_number: false },
    LonghandInfo { name: "border-bottom-color", inherited: false, initial: Some("currentcolor"), percentage_is_number: false },
    LonghandInfo { name: "border-bottom-left-radius", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "border-bottom-right-radius", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "border-bottom-style", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "border-bottom-width", inherited: false, initial: Some("medium"), percentage_is_number: false },
    LonghandInfo { name: "border-collapse", inherited: true, initial: Some("separate"), percentage_is_number: false },
    LonghandInfo { name: "border-end-end-radius", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "border-end-start-radius", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "border-image-outset", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "border-image-repeat", inherited: false, initial: Some("stretch"), percentage_is_number: false },
    LonghandInfo { name: "border-image-slice", inherited: false, initial: Some("100%"), percentage_is_number: false },
    LonghandInfo { name: "border-image-source", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "border-image-width", inherited: false, initial: Some("1"), percentage_is_number: false },
    LonghandInfo { name: "border-inline-end-color", inherited: false, initial: Some("currentcolor"), percentage_is_number: false },
    LonghandInfo { name: "border-inline-end-style", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "border-inline-end-width", inherited: false, initial: Some("medium"), percentage_is_number: false },
    LonghandInfo { name: "border-inline-start-color", inherited: false, initial: Some("currentcolor"), percentage_is_number: false },
    LonghandInfo { name: "border-inline-start-style", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "border-inline-start-width", inherited: false, initial: Some("medium"), percentage_is_number: false },
    LonghandInfo { name: "border-left-color", inherited: false, initial: Some("currentcolor"), percentage_is_number: false },
    LonghandInfo { name: "border-left-style", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "border-left-width", inherited: false, initial: Some("medium"), percentage_is_number: false },
    LonghandInfo { name: "border-right-color", inherited: false, initial: Some("currentcolor"), percentage_is_number: false },
    LonghandInfo { name: "border-right-style", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "border-right-width", inherited: false, initial: Some("medium"), percentage_is_number: false },
    LonghandInfo { name: "border-shape", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "border-spacing", inherited: true, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "border-start-end-radius", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "border-start-start-radius", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "border-top-color", inherited: false, initial: Some("currentcolor"), percentage_is_number: false },
    LonghandInfo { name: "border-top-left-radius", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "border-top-right-radius", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "border-top-style", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "border-top-width", inherited: false, initial: Some("medium"), percentage_is_number: false },
    LonghandInfo { name: "bottom", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "box-align", inherited: false, initial: Some("stretch"), percentage_is_number: false },
    LonghandInfo { name: "box-decoration-break", inherited: false, initial: Some("slice"), percentage_is_number: false },
    LonghandInfo { name: "box-direction", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "box-flex", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "box-flex-group", inherited: false, initial: Some("1"), percentage_is_number: false },
    LonghandInfo { name: "box-lines", inherited: false, initial: Some("single"), percentage_is_number: false },
    LonghandInfo { name: "box-ordinal-group", inherited: false, initial: Some("1"), percentage_is_number: false },
    LonghandInfo { name: "box-orient", inherited: false, initial: Some("inline-axis"), percentage_is_number: false },
    LonghandInfo { name: "box-pack", inherited: false, initial: Some("start"), percentage_is_number: false },
    LonghandInfo { name: "box-shadow", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "box-sizing", inherited: false, initial: Some("content-box"), percentage_is_number: false },
    LonghandInfo { name: "break-after", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "break-before", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "break-inside", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "caption-side", inherited: true, initial: Some("top"), percentage_is_number: false },
    LonghandInfo { name: "caret-animation", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "caret-color", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "caret-shape", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "clear", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "clip", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "clip-path", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "clip-rule", inherited: true, initial: Some("nonzero"), percentage_is_number: false },
    LonghandInfo { name: "color", inherited: true, initial: Some("canvastext"), percentage_is_number: false },
    LonghandInfo { name: "color-interpolation-filters", inherited: true, initial: Some("linearRGB"), percentage_is_number: false },
    LonghandInfo { name: "color-scheme", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "column-count", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "column-fill", inherited: false, initial: Some("balance"), percentage_is_number: false },
    LonghandInfo { name: "column-gap", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "column-height", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "column-rule-color", inherited: false, initial: Some("currentcolor"), percentage_is_number: false },
    LonghandInfo { name: "column-rule-style", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "column-rule-width", inherited: false, initial: Some("medium"), percentage_is_number: false },
    LonghandInfo { name: "column-span", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "column-width", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "column-wrap", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "contain", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "contain-intrinsic-block-size", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "contain-intrinsic-height", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "contain-intrinsic-inline-size", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "contain-intrinsic-width", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "container-name", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "container-type", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "content", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "content-visibility", inherited: false, initial: Some("visible"), percentage_is_number: false },
    LonghandInfo { name: "corner-bottom-left-shape", inherited: false, initial: Some("round"), percentage_is_number: false },
    LonghandInfo { name: "corner-bottom-right-shape", inherited: false, initial: Some("round"), percentage_is_number: false },
    LonghandInfo { name: "corner-end-end-shape", inherited: false, initial: Some("round"), percentage_is_number: false },
    LonghandInfo { name: "corner-end-start-shape", inherited: false, initial: Some("round"), percentage_is_number: false },
    LonghandInfo { name: "corner-start-end-shape", inherited: false, initial: Some("round"), percentage_is_number: false },
    LonghandInfo { name: "corner-start-start-shape", inherited: false, initial: Some("round"), percentage_is_number: false },
    LonghandInfo { name: "corner-top-left-shape", inherited: false, initial: Some("round"), percentage_is_number: false },
    LonghandInfo { name: "corner-top-right-shape", inherited: false, initial: Some("round"), percentage_is_number: false },
    LonghandInfo { name: "counter-increment", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "counter-reset", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "counter-set", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "cursor", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "cx", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "cy", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "d", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "direction", inherited: true, initial: Some("ltr"), percentage_is_number: false },
    LonghandInfo { name: "display", inherited: false, initial: Some("inline"), percentage_is_number: false },
    LonghandInfo { name: "dominant-baseline", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "dynamic-range-limit", inherited: true, initial: Some("no-limit"), percentage_is_number: false },
    LonghandInfo { name: "empty-cells", inherited: true, initial: Some("show"), percentage_is_number: false },
    LonghandInfo { name: "field-sizing", inherited: false, initial: Some("fixed"), percentage_is_number: false },
    LonghandInfo { name: "fill", inherited: true, initial: Some("black"), percentage_is_number: false },
    LonghandInfo { name: "fill-opacity", inherited: true, initial: Some("1"), percentage_is_number: true },
    LonghandInfo { name: "fill-rule", inherited: true, initial: Some("nonzero"), percentage_is_number: false },
    LonghandInfo { name: "filter", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "flex-basis", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "flex-direction", inherited: false, initial: Some("row"), percentage_is_number: false },
    LonghandInfo { name: "flex-grow", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "flex-shrink", inherited: false, initial: Some("1"), percentage_is_number: false },
    LonghandInfo { name: "flex-wrap", inherited: false, initial: Some("nowrap"), percentage_is_number: false },
    LonghandInfo { name: "float", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "flood-color", inherited: false, initial: Some("black"), percentage_is_number: false },
    LonghandInfo { name: "flood-opacity", inherited: false, initial: Some("black"), percentage_is_number: true },
    LonghandInfo { name: "font-family", inherited: true, initial: Some("dependsOnUserAgent"), percentage_is_number: false },
    LonghandInfo { name: "font-feature-settings", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "font-kerning", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "font-language-override", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "font-optical-sizing", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "font-palette", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "font-size", inherited: true, initial: Some("medium"), percentage_is_number: false },
    LonghandInfo { name: "font-size-adjust", inherited: true, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "font-smooth", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "font-stretch", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "font-style", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "font-synthesis", inherited: true, initial: Some("weight style small-caps position "), percentage_is_number: false },
    LonghandInfo { name: "font-synthesis-position", inherited: true, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "font-synthesis-small-caps", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "font-synthesis-style", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "font-synthesis-weight", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "font-variant", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "font-variant-alternates", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "font-variant-caps", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "font-variant-east-asian", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "font-variant-emoji", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "font-variant-ligatures", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "font-variant-numeric", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "font-variant-position", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "font-variation-settings", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "font-weight", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "font-width", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "forced-color-adjust", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "frame-sizing", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "grid-auto-columns", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "grid-auto-flow", inherited: false, initial: Some("row"), percentage_is_number: false },
    LonghandInfo { name: "grid-auto-rows", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "grid-column-end", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "grid-column-gap", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "grid-column-start", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "grid-row-end", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "grid-row-gap", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "grid-row-start", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "grid-template-areas", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "grid-template-columns", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "grid-template-rows", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "hanging-punctuation", inherited: true, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "height", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "hyphenate-character", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "hyphenate-limit-chars", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "hyphens", inherited: true, initial: Some("manual"), percentage_is_number: false },
    LonghandInfo { name: "image-orientation", inherited: true, initial: Some("from-image"), percentage_is_number: false },
    LonghandInfo { name: "image-rendering", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "image-resolution", inherited: true, initial: Some("1dppx"), percentage_is_number: false },
    LonghandInfo { name: "ime-mode", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "initial-letter", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "initial-letter-align", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "inline-size", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "inset-block-end", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "inset-block-start", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "inset-inline-end", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "inset-inline-start", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "interactivity", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "interest-delay-end", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "interest-delay-start", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "interpolate-size", inherited: true, initial: Some("numeric-only"), percentage_is_number: false },
    LonghandInfo { name: "isolation", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "justify-content", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "justify-items", inherited: false, initial: Some("legacy"), percentage_is_number: false },
    LonghandInfo { name: "justify-self", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "justify-tracks", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "left", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "letter-spacing", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "lighting-color", inherited: false, initial: Some("white"), percentage_is_number: false },
    LonghandInfo { name: "line-break", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "line-clamp", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "line-height", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "line-height-step", inherited: true, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "list-style-image", inherited: true, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "list-style-position", inherited: true, initial: Some("outside"), percentage_is_number: false },
    LonghandInfo { name: "list-style-type", inherited: true, initial: Some("disc"), percentage_is_number: false },
    LonghandInfo { name: "margin-block-end", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "margin-block-start", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "margin-bottom", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "margin-inline-end", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "margin-inline-start", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "margin-left", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "margin-right", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "margin-top", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "margin-trim", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "marker", inherited: true, initial: None, percentage_is_number: false },
    LonghandInfo { name: "marker-end", inherited: true, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "marker-mid", inherited: true, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "marker-start", inherited: true, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "mask-border-mode", inherited: false, initial: Some("alpha"), percentage_is_number: false },
    LonghandInfo { name: "mask-border-outset", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "mask-border-repeat", inherited: false, initial: Some("stretch"), percentage_is_number: false },
    LonghandInfo { name: "mask-border-slice", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "mask-border-source", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "mask-border-width", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "mask-clip", inherited: false, initial: Some("border-box"), percentage_is_number: false },
    LonghandInfo { name: "mask-composite", inherited: false, initial: Some("add"), percentage_is_number: false },
    LonghandInfo { name: "mask-image", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "mask-mode", inherited: false, initial: Some("match-source"), percentage_is_number: false },
    LonghandInfo { name: "mask-origin", inherited: false, initial: Some("border-box"), percentage_is_number: false },
    LonghandInfo { name: "mask-position", inherited: false, initial: Some("0% 0%"), percentage_is_number: false },
    LonghandInfo { name: "mask-repeat", inherited: false, initial: Some("repeat"), percentage_is_number: false },
    LonghandInfo { name: "mask-size", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "mask-type", inherited: false, initial: Some("luminance"), percentage_is_number: false },
    LonghandInfo { name: "masonry-auto-flow", inherited: false, initial: Some("pack"), percentage_is_number: false },
    LonghandInfo { name: "math-depth", inherited: true, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "math-shift", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "math-style", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "max-block-size", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "max-height", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "max-inline-size", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "max-lines", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "max-width", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "min-block-size", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "min-height", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "min-inline-size", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "min-width", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "mix-blend-mode", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "object-fit", inherited: false, initial: Some("fill"), percentage_is_number: false },
    LonghandInfo { name: "object-position", inherited: false, initial: Some("50% 50%"), percentage_is_number: false },
    LonghandInfo { name: "object-view-box", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "offset-anchor", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "offset-distance", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "offset-path", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "offset-position", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "offset-rotate", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "opacity", inherited: false, initial: Some("1"), percentage_is_number: true },
    LonghandInfo { name: "order", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "orphans", inherited: true, initial: Some("2"), percentage_is_number: false },
    LonghandInfo { name: "outline-color", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "outline-offset", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "outline-style", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "outline-width", inherited: false, initial: Some("medium"), percentage_is_number: false },
    LonghandInfo { name: "overflow-anchor", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "overflow-block", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "overflow-clip-box", inherited: false, initial: Some("padding-box"), percentage_is_number: false },
    LonghandInfo { name: "overflow-clip-margin", inherited: false, initial: Some("0px"), percentage_is_number: false },
    LonghandInfo { name: "overflow-inline", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "overflow-wrap", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "overflow-x", inherited: false, initial: Some("visible"), percentage_is_number: false },
    LonghandInfo { name: "overflow-y", inherited: false, initial: Some("visible"), percentage_is_number: false },
    LonghandInfo { name: "overlay", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "overscroll-behavior-block", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "overscroll-behavior-inline", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "overscroll-behavior-x", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "overscroll-behavior-y", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "padding-block-end", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "padding-block-start", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "padding-bottom", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "padding-inline-end", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "padding-inline-start", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "padding-left", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "padding-right", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "padding-top", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "page", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "page-break-after", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "page-break-before", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "page-break-inside", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "paint-order", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "perspective", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "perspective-origin", inherited: false, initial: Some("50% 50%"), percentage_is_number: false },
    LonghandInfo { name: "pointer-events", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "position", inherited: false, initial: Some("static"), percentage_is_number: false },
    LonghandInfo { name: "position-anchor", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "position-area", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "position-try-fallbacks", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "position-try-order", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "position-visibility", inherited: false, initial: Some("anchors-visible"), percentage_is_number: false },
    LonghandInfo { name: "print-color-adjust", inherited: true, initial: Some("economy"), percentage_is_number: false },
    LonghandInfo { name: "quotes", inherited: true, initial: Some("dependsOnUserAgent"), percentage_is_number: false },
    LonghandInfo { name: "r", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "reading-flow", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "reading-order", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "resize", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "right", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "rotate", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "row-gap", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "ruby-align", inherited: true, initial: Some("space-around"), percentage_is_number: false },
    LonghandInfo { name: "ruby-merge", inherited: true, initial: Some("separate"), percentage_is_number: false },
    LonghandInfo { name: "ruby-overhang", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "ruby-position", inherited: true, initial: Some("alternate"), percentage_is_number: false },
    LonghandInfo { name: "rx", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "ry", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "scale", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "scroll-behavior", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "scroll-initial-target", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "scroll-margin-block-end", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "scroll-margin-block-start", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "scroll-margin-bottom", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "scroll-margin-inline-end", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "scroll-margin-inline-start", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "scroll-margin-left", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "scroll-margin-right", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "scroll-margin-top", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "scroll-marker-group", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "scroll-padding-block-end", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "scroll-padding-block-start", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "scroll-padding-bottom", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "scroll-padding-inline-end", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "scroll-padding-inline-start", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "scroll-padding-left", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "scroll-padding-right", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "scroll-padding-top", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "scroll-snap-align", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "scroll-snap-coordinate", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "scroll-snap-destination", inherited: false, initial: Some("0px 0px"), percentage_is_number: false },
    LonghandInfo { name: "scroll-snap-points-x", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "scroll-snap-points-y", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "scroll-snap-stop", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "scroll-snap-type", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "scroll-snap-type-x", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "scroll-snap-type-y", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "scroll-target-group", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "scroll-timeline-axis", inherited: false, initial: Some("block"), percentage_is_number: false },
    LonghandInfo { name: "scroll-timeline-name", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "scrollbar-color", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "scrollbar-gutter", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "scrollbar-width", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "shape-image-threshold", inherited: false, initial: Some("0.0"), percentage_is_number: true },
    LonghandInfo { name: "shape-margin", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "shape-outside", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "shape-rendering", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "speak-as", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "stop-color", inherited: false, initial: Some("black"), percentage_is_number: false },
    LonghandInfo { name: "stop-opacity", inherited: false, initial: Some("black"), percentage_is_number: false },
    LonghandInfo { name: "stroke", inherited: true, initial: None, percentage_is_number: false },
    LonghandInfo { name: "stroke-color", inherited: true, initial: Some("transparent"), percentage_is_number: false },
    LonghandInfo { name: "stroke-dasharray", inherited: true, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "stroke-dashoffset", inherited: true, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "stroke-linecap", inherited: true, initial: Some("butt"), percentage_is_number: false },
    LonghandInfo { name: "stroke-linejoin", inherited: true, initial: Some("miter"), percentage_is_number: false },
    LonghandInfo { name: "stroke-miterlimit", inherited: true, initial: Some("4"), percentage_is_number: false },
    LonghandInfo { name: "stroke-opacity", inherited: true, initial: Some("1"), percentage_is_number: true },
    LonghandInfo { name: "stroke-width", inherited: true, initial: Some("1px"), percentage_is_number: false },
    LonghandInfo { name: "tab-size", inherited: true, initial: Some("8"), percentage_is_number: false },
    LonghandInfo { name: "table-layout", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "text-align", inherited: true, initial: Some("startOrNamelessValueIfLTRRightIfRTL"), percentage_is_number: false },
    LonghandInfo { name: "text-align-last", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "text-anchor", inherited: true, initial: Some("start"), percentage_is_number: false },
    LonghandInfo { name: "text-autospace", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "text-box", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "text-box-edge", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "text-box-trim", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "text-combine-upright", inherited: true, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "text-decoration-color", inherited: false, initial: Some("currentcolor"), percentage_is_number: false },
    LonghandInfo { name: "text-decoration-inset", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "text-decoration-line", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "text-decoration-skip", inherited: true, initial: Some("objects"), percentage_is_number: false },
    LonghandInfo { name: "text-decoration-skip-ink", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "text-decoration-style", inherited: false, initial: Some("solid"), percentage_is_number: false },
    LonghandInfo { name: "text-decoration-thickness", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "text-emphasis-color", inherited: true, initial: Some("currentcolor"), percentage_is_number: false },
    LonghandInfo { name: "text-emphasis-position", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "text-emphasis-style", inherited: true, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "text-indent", inherited: true, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "text-justify", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "text-orientation", inherited: true, initial: Some("mixed"), percentage_is_number: false },
    LonghandInfo { name: "text-overflow", inherited: false, initial: Some("clip"), percentage_is_number: false },
    LonghandInfo { name: "text-rendering", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "text-shadow", inherited: true, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "text-size-adjust", inherited: true, initial: Some("autoForSmartphoneBrowsersSupportingInflation"), percentage_is_number: false },
    LonghandInfo { name: "text-spacing-trim", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "text-transform", inherited: true, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "text-underline-offset", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "text-underline-position", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "text-wrap-mode", inherited: true, initial: Some("wrap"), percentage_is_number: false },
    LonghandInfo { name: "text-wrap-style", inherited: true, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "timeline-scope", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "timeline-trigger-activation-range-end", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "timeline-trigger-activation-range-start", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "timeline-trigger-active-range-end", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "timeline-trigger-active-range-start", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "timeline-trigger-name", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "timeline-trigger-source", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "top", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "touch-action", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "transform", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "transform-box", inherited: false, initial: Some("view-box"), percentage_is_number: false },
    LonghandInfo { name: "transform-origin", inherited: false, initial: Some("50% 50% 0"), percentage_is_number: false },
    LonghandInfo { name: "transform-style", inherited: false, initial: Some("flat"), percentage_is_number: false },
    LonghandInfo { name: "transition-behavior", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "transition-delay", inherited: false, initial: Some("0s"), percentage_is_number: false },
    LonghandInfo { name: "transition-duration", inherited: false, initial: Some("0s"), percentage_is_number: false },
    LonghandInfo { name: "transition-property", inherited: false, initial: Some("all"), percentage_is_number: false },
    LonghandInfo { name: "transition-timing-function", inherited: false, initial: Some("ease"), percentage_is_number: false },
    LonghandInfo { name: "translate", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "trigger-scope", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "unicode-bidi", inherited: false, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "user-select", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "vector-effect", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "vertical-align", inherited: false, initial: Some("baseline"), percentage_is_number: false },
    LonghandInfo { name: "view-timeline-axis", inherited: false, initial: Some("block"), percentage_is_number: false },
    LonghandInfo { name: "view-timeline-inset", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "view-timeline-name", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "view-transition-class", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "view-transition-name", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "view-transition-scope", inherited: false, initial: Some("none"), percentage_is_number: false },
    LonghandInfo { name: "visibility", inherited: true, initial: Some("visible"), percentage_is_number: false },
    LonghandInfo { name: "white-space", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "white-space-collapse", inherited: true, initial: Some("collapse"), percentage_is_number: false },
    LonghandInfo { name: "widows", inherited: true, initial: Some("2"), percentage_is_number: false },
    LonghandInfo { name: "width", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "will-change", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "word-break", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "word-spacing", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "word-wrap", inherited: true, initial: Some("normal"), percentage_is_number: false },
    LonghandInfo { name: "writing-mode", inherited: true, initial: Some("horizontal-tb"), percentage_is_number: false },
    LonghandInfo { name: "x", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "y", inherited: false, initial: Some("0"), percentage_is_number: false },
    LonghandInfo { name: "z-index", inherited: false, initial: Some("auto"), percentage_is_number: false },
    LonghandInfo { name: "zoom", inherited: false, initial: Some("1"), percentage_is_number: false },
];

/// Every [`LonghandId`], in id order - which is name order.
#[rustfmt::skip]
pub static ALL_LONGHAND_IDS: [LonghandId; LONGHAND_COUNT] = [
    LonghandId::MozAppearance, LonghandId::MozBinding, LonghandId::MozBorderBottomColors,
    LonghandId::MozBorderLeftColors, LonghandId::MozBorderRightColors, LonghandId::MozBorderTopColors,
    LonghandId::MozContextProperties, LonghandId::MozFloatEdge, LonghandId::MozForceBrokenImageIcon,
    LonghandId::MozOrient, LonghandId::MozOutlineRadiusBottomleft, LonghandId::MozOutlineRadiusBottomright,
    LonghandId::MozOutlineRadiusTopleft, LonghandId::MozOutlineRadiusTopright, LonghandId::MozStackSizing,
    LonghandId::MozTextBlink, LonghandId::MozUserFocus, LonghandId::MozUserInput, LonghandId::MozUserModify,
    LonghandId::MozWindowDragging, LonghandId::MozWindowShadow, LonghandId::MsAccelerator,
    LonghandId::MsBlockProgression, LonghandId::MsContentZoomChaining, LonghandId::MsContentZoomLimitMax,
    LonghandId::MsContentZoomLimitMin, LonghandId::MsContentZoomSnapPoints, LonghandId::MsContentZoomSnapType,
    LonghandId::MsContentZooming, LonghandId::MsFilter, LonghandId::MsFlowFrom, LonghandId::MsFlowInto,
    LonghandId::MsGridColumns, LonghandId::MsGridRows, LonghandId::MsHighContrastAdjust,
    LonghandId::MsHyphenateLimitChars, LonghandId::MsHyphenateLimitLines, LonghandId::MsHyphenateLimitZone,
    LonghandId::MsImeAlign, LonghandId::MsOverflowStyle, LonghandId::MsScrollChaining,
    LonghandId::MsScrollLimitXMax, LonghandId::MsScrollLimitXMin, LonghandId::MsScrollLimitYMax,
    LonghandId::MsScrollLimitYMin, LonghandId::MsScrollRails, LonghandId::MsScrollSnapPointsX,
    LonghandId::MsScrollSnapPointsY, LonghandId::MsScrollSnapType, LonghandId::MsScrollTranslation,
    LonghandId::MsScrollbar3dlightColor, LonghandId::MsScrollbarArrowColor, LonghandId::MsScrollbarBaseColor,
    LonghandId::MsScrollbarDarkshadowColor, LonghandId::MsScrollbarFaceColor, LonghandId::MsScrollbarHighlightColor,
    LonghandId::MsScrollbarShadowColor, LonghandId::MsScrollbarTrackColor, LonghandId::MsTextAutospace,
    LonghandId::MsTouchSelect, LonghandId::MsUserSelect, LonghandId::MsWrapFlow, LonghandId::MsWrapMargin,
    LonghandId::MsWrapThrough, LonghandId::WebkitAppearance, LonghandId::WebkitBorderAfterColor,
    LonghandId::WebkitBorderAfterStyle, LonghandId::WebkitBorderAfterWidth, LonghandId::WebkitBorderBeforeColor,
    LonghandId::WebkitBorderBeforeStyle, LonghandId::WebkitBorderBeforeWidth, LonghandId::WebkitBorderEndColor,
    LonghandId::WebkitBorderEndStyle, LonghandId::WebkitBorderEndWidth, LonghandId::WebkitBorderStartColor,
    LonghandId::WebkitBorderStartStyle, LonghandId::WebkitBorderStartWidth, LonghandId::WebkitBoxReflect,
    LonghandId::WebkitLineClamp, LonghandId::WebkitMaskAttachment, LonghandId::WebkitMaskClip,
    LonghandId::WebkitMaskComposite, LonghandId::WebkitMaskImage, LonghandId::WebkitMaskOrigin,
    LonghandId::WebkitMaskPosition, LonghandId::WebkitMaskPositionX, LonghandId::WebkitMaskPositionY,
    LonghandId::WebkitMaskRepeat, LonghandId::WebkitMaskRepeatX, LonghandId::WebkitMaskRepeatY,
    LonghandId::WebkitMaskSize, LonghandId::WebkitOverflowScrolling, LonghandId::WebkitTapHighlightColor,
    LonghandId::WebkitTextFillColor, LonghandId::WebkitTextStrokeColor, LonghandId::WebkitTextStrokeWidth,
    LonghandId::WebkitTouchCallout, LonghandId::WebkitUserModify, LonghandId::WebkitUserSelect,
    LonghandId::AccentColor, LonghandId::AlignContent, LonghandId::AlignItems, LonghandId::AlignSelf,
    LonghandId::AlignTracks, LonghandId::AlignmentBaseline, LonghandId::All, LonghandId::AnchorName,
    LonghandId::AnchorScope, LonghandId::AnimationComposition, LonghandId::AnimationDelay,
    LonghandId::AnimationDirection, LonghandId::AnimationDuration, LonghandId::AnimationFillMode,
    LonghandId::AnimationIterationCount, LonghandId::AnimationName, LonghandId::AnimationPlayState,
    LonghandId::AnimationRangeEnd, LonghandId::AnimationRangeStart, LonghandId::AnimationTimeline,
    LonghandId::AnimationTimingFunction, LonghandId::AnimationTrigger, LonghandId::Appearance,
    LonghandId::AspectRatio, LonghandId::BackdropFilter, LonghandId::BackfaceVisibility,
    LonghandId::BackgroundAttachment, LonghandId::BackgroundBlendMode, LonghandId::BackgroundClip,
    LonghandId::BackgroundColor, LonghandId::BackgroundImage, LonghandId::BackgroundOrigin,
    LonghandId::BackgroundPositionX, LonghandId::BackgroundPositionY, LonghandId::BackgroundRepeat,
    LonghandId::BackgroundSize, LonghandId::BaselineShift, LonghandId::BaselineSource, LonghandId::BlockSize,
    LonghandId::BorderBlockEndColor, LonghandId::BorderBlockEndStyle, LonghandId::BorderBlockEndWidth,
    LonghandId::BorderBlockStartColor, LonghandId::BorderBlockStartStyle, LonghandId::BorderBlockStartWidth,
    LonghandId::BorderBottomColor, LonghandId::BorderBottomLeftRadius, LonghandId::BorderBottomRightRadius,
    LonghandId::BorderBottomStyle, LonghandId::BorderBottomWidth, LonghandId::BorderCollapse,
    LonghandId::BorderEndEndRadius, LonghandId::BorderEndStartRadius, LonghandId::BorderImageOutset,
    LonghandId::BorderImageRepeat, LonghandId::BorderImageSlice, LonghandId::BorderImageSource,
    LonghandId::BorderImageWidth, LonghandId::BorderInlineEndColor, LonghandId::BorderInlineEndStyle,
    LonghandId::BorderInlineEndWidth, LonghandId::BorderInlineStartColor, LonghandId::BorderInlineStartStyle,
    LonghandId::BorderInlineStartWidth, LonghandId::BorderLeftColor, LonghandId::BorderLeftStyle,
    LonghandId::BorderLeftWidth, LonghandId::BorderRightColor, LonghandId::BorderRightStyle,
    LonghandId::BorderRightWidth, LonghandId::BorderShape, LonghandId::BorderSpacing,
    LonghandId::BorderStartEndRadius, LonghandId::BorderStartStartRadius, LonghandId::BorderTopColor,
    LonghandId::BorderTopLeftRadius, LonghandId::BorderTopRightRadius, LonghandId::BorderTopStyle,
    LonghandId::BorderTopWidth, LonghandId::Bottom, LonghandId::BoxAlign, LonghandId::BoxDecorationBreak,
    LonghandId::BoxDirection, LonghandId::BoxFlex, LonghandId::BoxFlexGroup, LonghandId::BoxLines,
    LonghandId::BoxOrdinalGroup, LonghandId::BoxOrient, LonghandId::BoxPack, LonghandId::BoxShadow,
    LonghandId::BoxSizing, LonghandId::BreakAfter, LonghandId::BreakBefore, LonghandId::BreakInside,
    LonghandId::CaptionSide, LonghandId::CaretAnimation, LonghandId::CaretColor, LonghandId::CaretShape,
    LonghandId::Clear, LonghandId::Clip, LonghandId::ClipPath, LonghandId::ClipRule, LonghandId::Color,
    LonghandId::ColorInterpolationFilters, LonghandId::ColorScheme, LonghandId::ColumnCount, LonghandId::ColumnFill,
    LonghandId::ColumnGap, LonghandId::ColumnHeight, LonghandId::ColumnRuleColor, LonghandId::ColumnRuleStyle,
    LonghandId::ColumnRuleWidth, LonghandId::ColumnSpan, LonghandId::ColumnWidth, LonghandId::ColumnWrap,
    LonghandId::Contain, LonghandId::ContainIntrinsicBlockSize, LonghandId::ContainIntrinsicHeight,
    LonghandId::ContainIntrinsicInlineSize, LonghandId::ContainIntrinsicWidth, LonghandId::ContainerName,
    LonghandId::ContainerType, LonghandId::Content, LonghandId::ContentVisibility,
    LonghandId::CornerBottomLeftShape, LonghandId::CornerBottomRightShape, LonghandId::CornerEndEndShape,
    LonghandId::CornerEndStartShape, LonghandId::CornerStartEndShape, LonghandId::CornerStartStartShape,
    LonghandId::CornerTopLeftShape, LonghandId::CornerTopRightShape, LonghandId::CounterIncrement,
    LonghandId::CounterReset, LonghandId::CounterSet, LonghandId::Cursor, LonghandId::Cx, LonghandId::Cy,
    LonghandId::D, LonghandId::Direction, LonghandId::Display, LonghandId::DominantBaseline,
    LonghandId::DynamicRangeLimit, LonghandId::EmptyCells, LonghandId::FieldSizing, LonghandId::Fill,
    LonghandId::FillOpacity, LonghandId::FillRule, LonghandId::Filter, LonghandId::FlexBasis,
    LonghandId::FlexDirection, LonghandId::FlexGrow, LonghandId::FlexShrink, LonghandId::FlexWrap,
    LonghandId::Float, LonghandId::FloodColor, LonghandId::FloodOpacity, LonghandId::FontFamily,
    LonghandId::FontFeatureSettings, LonghandId::FontKerning, LonghandId::FontLanguageOverride,
    LonghandId::FontOpticalSizing, LonghandId::FontPalette, LonghandId::FontSize, LonghandId::FontSizeAdjust,
    LonghandId::FontSmooth, LonghandId::FontStretch, LonghandId::FontStyle, LonghandId::FontSynthesis,
    LonghandId::FontSynthesisPosition, LonghandId::FontSynthesisSmallCaps, LonghandId::FontSynthesisStyle,
    LonghandId::FontSynthesisWeight, LonghandId::FontVariant, LonghandId::FontVariantAlternates,
    LonghandId::FontVariantCaps, LonghandId::FontVariantEastAsian, LonghandId::FontVariantEmoji,
    LonghandId::FontVariantLigatures, LonghandId::FontVariantNumeric, LonghandId::FontVariantPosition,
    LonghandId::FontVariationSettings, LonghandId::FontWeight, LonghandId::FontWidth, LonghandId::ForcedColorAdjust,
    LonghandId::FrameSizing, LonghandId::GridAutoColumns, LonghandId::GridAutoFlow, LonghandId::GridAutoRows,
    LonghandId::GridColumnEnd, LonghandId::GridColumnGap, LonghandId::GridColumnStart, LonghandId::GridRowEnd,
    LonghandId::GridRowGap, LonghandId::GridRowStart, LonghandId::GridTemplateAreas,
    LonghandId::GridTemplateColumns, LonghandId::GridTemplateRows, LonghandId::HangingPunctuation,
    LonghandId::Height, LonghandId::HyphenateCharacter, LonghandId::HyphenateLimitChars, LonghandId::Hyphens,
    LonghandId::ImageOrientation, LonghandId::ImageRendering, LonghandId::ImageResolution, LonghandId::ImeMode,
    LonghandId::InitialLetter, LonghandId::InitialLetterAlign, LonghandId::InlineSize, LonghandId::InsetBlockEnd,
    LonghandId::InsetBlockStart, LonghandId::InsetInlineEnd, LonghandId::InsetInlineStart,
    LonghandId::Interactivity, LonghandId::InterestDelayEnd, LonghandId::InterestDelayStart,
    LonghandId::InterpolateSize, LonghandId::Isolation, LonghandId::JustifyContent, LonghandId::JustifyItems,
    LonghandId::JustifySelf, LonghandId::JustifyTracks, LonghandId::Left, LonghandId::LetterSpacing,
    LonghandId::LightingColor, LonghandId::LineBreak, LonghandId::LineClamp, LonghandId::LineHeight,
    LonghandId::LineHeightStep, LonghandId::ListStyleImage, LonghandId::ListStylePosition,
    LonghandId::ListStyleType, LonghandId::MarginBlockEnd, LonghandId::MarginBlockStart, LonghandId::MarginBottom,
    LonghandId::MarginInlineEnd, LonghandId::MarginInlineStart, LonghandId::MarginLeft, LonghandId::MarginRight,
    LonghandId::MarginTop, LonghandId::MarginTrim, LonghandId::Marker, LonghandId::MarkerEnd, LonghandId::MarkerMid,
    LonghandId::MarkerStart, LonghandId::MaskBorderMode, LonghandId::MaskBorderOutset, LonghandId::MaskBorderRepeat,
    LonghandId::MaskBorderSlice, LonghandId::MaskBorderSource, LonghandId::MaskBorderWidth, LonghandId::MaskClip,
    LonghandId::MaskComposite, LonghandId::MaskImage, LonghandId::MaskMode, LonghandId::MaskOrigin,
    LonghandId::MaskPosition, LonghandId::MaskRepeat, LonghandId::MaskSize, LonghandId::MaskType,
    LonghandId::MasonryAutoFlow, LonghandId::MathDepth, LonghandId::MathShift, LonghandId::MathStyle,
    LonghandId::MaxBlockSize, LonghandId::MaxHeight, LonghandId::MaxInlineSize, LonghandId::MaxLines,
    LonghandId::MaxWidth, LonghandId::MinBlockSize, LonghandId::MinHeight, LonghandId::MinInlineSize,
    LonghandId::MinWidth, LonghandId::MixBlendMode, LonghandId::ObjectFit, LonghandId::ObjectPosition,
    LonghandId::ObjectViewBox, LonghandId::OffsetAnchor, LonghandId::OffsetDistance, LonghandId::OffsetPath,
    LonghandId::OffsetPosition, LonghandId::OffsetRotate, LonghandId::Opacity, LonghandId::Order,
    LonghandId::Orphans, LonghandId::OutlineColor, LonghandId::OutlineOffset, LonghandId::OutlineStyle,
    LonghandId::OutlineWidth, LonghandId::OverflowAnchor, LonghandId::OverflowBlock, LonghandId::OverflowClipBox,
    LonghandId::OverflowClipMargin, LonghandId::OverflowInline, LonghandId::OverflowWrap, LonghandId::OverflowX,
    LonghandId::OverflowY, LonghandId::Overlay, LonghandId::OverscrollBehaviorBlock,
    LonghandId::OverscrollBehaviorInline, LonghandId::OverscrollBehaviorX, LonghandId::OverscrollBehaviorY,
    LonghandId::PaddingBlockEnd, LonghandId::PaddingBlockStart, LonghandId::PaddingBottom,
    LonghandId::PaddingInlineEnd, LonghandId::PaddingInlineStart, LonghandId::PaddingLeft, LonghandId::PaddingRight,
    LonghandId::PaddingTop, LonghandId::Page, LonghandId::PageBreakAfter, LonghandId::PageBreakBefore,
    LonghandId::PageBreakInside, LonghandId::PaintOrder, LonghandId::Perspective, LonghandId::PerspectiveOrigin,
    LonghandId::PointerEvents, LonghandId::Position, LonghandId::PositionAnchor, LonghandId::PositionArea,
    LonghandId::PositionTryFallbacks, LonghandId::PositionTryOrder, LonghandId::PositionVisibility,
    LonghandId::PrintColorAdjust, LonghandId::Quotes, LonghandId::R, LonghandId::ReadingFlow,
    LonghandId::ReadingOrder, LonghandId::Resize, LonghandId::Right, LonghandId::Rotate, LonghandId::RowGap,
    LonghandId::RubyAlign, LonghandId::RubyMerge, LonghandId::RubyOverhang, LonghandId::RubyPosition,
    LonghandId::Rx, LonghandId::Ry, LonghandId::Scale, LonghandId::ScrollBehavior, LonghandId::ScrollInitialTarget,
    LonghandId::ScrollMarginBlockEnd, LonghandId::ScrollMarginBlockStart, LonghandId::ScrollMarginBottom,
    LonghandId::ScrollMarginInlineEnd, LonghandId::ScrollMarginInlineStart, LonghandId::ScrollMarginLeft,
    LonghandId::ScrollMarginRight, LonghandId::ScrollMarginTop, LonghandId::ScrollMarkerGroup,
    LonghandId::ScrollPaddingBlockEnd, LonghandId::ScrollPaddingBlockStart, LonghandId::ScrollPaddingBottom,
    LonghandId::ScrollPaddingInlineEnd, LonghandId::ScrollPaddingInlineStart, LonghandId::ScrollPaddingLeft,
    LonghandId::ScrollPaddingRight, LonghandId::ScrollPaddingTop, LonghandId::ScrollSnapAlign,
    LonghandId::ScrollSnapCoordinate, LonghandId::ScrollSnapDestination, LonghandId::ScrollSnapPointsX,
    LonghandId::ScrollSnapPointsY, LonghandId::ScrollSnapStop, LonghandId::ScrollSnapType,
    LonghandId::ScrollSnapTypeX, LonghandId::ScrollSnapTypeY, LonghandId::ScrollTargetGroup,
    LonghandId::ScrollTimelineAxis, LonghandId::ScrollTimelineName, LonghandId::ScrollbarColor,
    LonghandId::ScrollbarGutter, LonghandId::ScrollbarWidth, LonghandId::ShapeImageThreshold,
    LonghandId::ShapeMargin, LonghandId::ShapeOutside, LonghandId::ShapeRendering, LonghandId::SpeakAs,
    LonghandId::StopColor, LonghandId::StopOpacity, LonghandId::Stroke, LonghandId::StrokeColor,
    LonghandId::StrokeDasharray, LonghandId::StrokeDashoffset, LonghandId::StrokeLinecap,
    LonghandId::StrokeLinejoin, LonghandId::StrokeMiterlimit, LonghandId::StrokeOpacity, LonghandId::StrokeWidth,
    LonghandId::TabSize, LonghandId::TableLayout, LonghandId::TextAlign, LonghandId::TextAlignLast,
    LonghandId::TextAnchor, LonghandId::TextAutospace, LonghandId::TextBox, LonghandId::TextBoxEdge,
    LonghandId::TextBoxTrim, LonghandId::TextCombineUpright, LonghandId::TextDecorationColor,
    LonghandId::TextDecorationInset, LonghandId::TextDecorationLine, LonghandId::TextDecorationSkip,
    LonghandId::TextDecorationSkipInk, LonghandId::TextDecorationStyle, LonghandId::TextDecorationThickness,
    LonghandId::TextEmphasisColor, LonghandId::TextEmphasisPosition, LonghandId::TextEmphasisStyle,
    LonghandId::TextIndent, LonghandId::TextJustify, LonghandId::TextOrientation, LonghandId::TextOverflow,
    LonghandId::TextRendering, LonghandId::TextShadow, LonghandId::TextSizeAdjust, LonghandId::TextSpacingTrim,
    LonghandId::TextTransform, LonghandId::TextUnderlineOffset, LonghandId::TextUnderlinePosition,
    LonghandId::TextWrapMode, LonghandId::TextWrapStyle, LonghandId::TimelineScope,
    LonghandId::TimelineTriggerActivationRangeEnd, LonghandId::TimelineTriggerActivationRangeStart,
    LonghandId::TimelineTriggerActiveRangeEnd, LonghandId::TimelineTriggerActiveRangeStart,
    LonghandId::TimelineTriggerName, LonghandId::TimelineTriggerSource, LonghandId::Top, LonghandId::TouchAction,
    LonghandId::Transform, LonghandId::TransformBox, LonghandId::TransformOrigin, LonghandId::TransformStyle,
    LonghandId::TransitionBehavior, LonghandId::TransitionDelay, LonghandId::TransitionDuration,
    LonghandId::TransitionProperty, LonghandId::TransitionTimingFunction, LonghandId::Translate,
    LonghandId::TriggerScope, LonghandId::UnicodeBidi, LonghandId::UserSelect, LonghandId::VectorEffect,
    LonghandId::VerticalAlign, LonghandId::ViewTimelineAxis, LonghandId::ViewTimelineInset,
    LonghandId::ViewTimelineName, LonghandId::ViewTransitionClass, LonghandId::ViewTransitionName,
    LonghandId::ViewTransitionScope, LonghandId::Visibility, LonghandId::WhiteSpace, LonghandId::WhiteSpaceCollapse,
    LonghandId::Widows, LonghandId::Width, LonghandId::WillChange, LonghandId::WordBreak, LonghandId::WordSpacing,
    LonghandId::WordWrap, LonghandId::WritingMode, LonghandId::X, LonghandId::Y, LonghandId::ZIndex,
    LonghandId::Zoom,
];

#[rustfmt::skip]
static SHORTHANDS: [ShorthandInfo; SHORTHAND_COUNT] = [
    ShorthandInfo { name: "-moz-outline-radius", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::MozOutlineRadiusTopleft), PropertyId::Longhand(LonghandId::MozOutlineRadiusTopright), PropertyId::Longhand(LonghandId::MozOutlineRadiusBottomright), PropertyId::Longhand(LonghandId::MozOutlineRadiusBottomleft)] },
    ShorthandInfo { name: "-ms-content-zoom-limit", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::MsContentZoomLimitMax), PropertyId::Longhand(LonghandId::MsContentZoomLimitMin)] },
    ShorthandInfo { name: "-ms-content-zoom-snap", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::MsContentZoomSnapType), PropertyId::Longhand(LonghandId::MsContentZoomSnapPoints)] },
    ShorthandInfo { name: "-ms-scroll-limit", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::MsScrollLimitXMin), PropertyId::Longhand(LonghandId::MsScrollLimitYMin), PropertyId::Longhand(LonghandId::MsScrollLimitXMax), PropertyId::Longhand(LonghandId::MsScrollLimitYMax)] },
    ShorthandInfo { name: "-ms-scroll-snap-x", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::MsScrollSnapType), PropertyId::Longhand(LonghandId::MsScrollSnapPointsX)] },
    ShorthandInfo { name: "-ms-scroll-snap-y", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::MsScrollSnapType), PropertyId::Longhand(LonghandId::MsScrollSnapPointsY)] },
    ShorthandInfo { name: "-webkit-border-after", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BorderBlockEndWidth), PropertyId::Longhand(LonghandId::BorderBlockEndStyle), PropertyId::Longhand(LonghandId::BorderBlockEndColor)] },
    ShorthandInfo { name: "-webkit-border-before", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BorderBlockStartWidth), PropertyId::Longhand(LonghandId::BorderBlockStartStyle), PropertyId::Longhand(LonghandId::BorderBlockStartColor)] },
    ShorthandInfo { name: "-webkit-border-end", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BorderInlineEndWidth), PropertyId::Longhand(LonghandId::BorderInlineEndStyle), PropertyId::Longhand(LonghandId::BorderInlineEndColor)] },
    ShorthandInfo { name: "-webkit-border-start", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BorderInlineStartWidth), PropertyId::Longhand(LonghandId::BorderInlineStartStyle), PropertyId::Longhand(LonghandId::BorderInlineStartColor)] },
    ShorthandInfo { name: "-webkit-mask", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::WebkitMaskImage), PropertyId::Longhand(LonghandId::WebkitMaskRepeat), PropertyId::Longhand(LonghandId::WebkitMaskAttachment), PropertyId::Longhand(LonghandId::WebkitMaskPosition), PropertyId::Longhand(LonghandId::WebkitMaskOrigin), PropertyId::Longhand(LonghandId::WebkitMaskClip)] },
    ShorthandInfo { name: "-webkit-text-stroke", inherited: true, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::WebkitTextStrokeWidth), PropertyId::Longhand(LonghandId::WebkitTextStrokeColor)] },
    ShorthandInfo { name: "animation", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::AnimationName), PropertyId::Longhand(LonghandId::AnimationDuration), PropertyId::Longhand(LonghandId::AnimationTimingFunction), PropertyId::Longhand(LonghandId::AnimationDelay), PropertyId::Longhand(LonghandId::AnimationDirection), PropertyId::Longhand(LonghandId::AnimationIterationCount), PropertyId::Longhand(LonghandId::AnimationFillMode), PropertyId::Longhand(LonghandId::AnimationPlayState), PropertyId::Longhand(LonghandId::AnimationTimeline)] },
    ShorthandInfo { name: "animation-range", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::AnimationRangeStart), PropertyId::Longhand(LonghandId::AnimationRangeEnd)] },
    ShorthandInfo { name: "background", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BackgroundImage), PropertyId::Shorthand(ShorthandId::BackgroundPosition), PropertyId::Longhand(LonghandId::BackgroundSize), PropertyId::Longhand(LonghandId::BackgroundRepeat), PropertyId::Longhand(LonghandId::BackgroundOrigin), PropertyId::Longhand(LonghandId::BackgroundClip), PropertyId::Longhand(LonghandId::BackgroundAttachment), PropertyId::Longhand(LonghandId::BackgroundColor)] },
    ShorthandInfo { name: "background-position", inherited: false, initial: Some("0% 0%"), percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BackgroundPositionX), PropertyId::Longhand(LonghandId::BackgroundPositionY)] },
    ShorthandInfo { name: "border", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Shorthand(ShorthandId::BorderWidth), PropertyId::Shorthand(ShorthandId::BorderStyle), PropertyId::Shorthand(ShorthandId::BorderColor)] },
    ShorthandInfo { name: "border-block", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Shorthand(ShorthandId::BorderBlockWidth), PropertyId::Shorthand(ShorthandId::BorderBlockStyle), PropertyId::Shorthand(ShorthandId::BorderBlockColor)] },
    ShorthandInfo { name: "border-block-color", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BorderBlockStartColor), PropertyId::Longhand(LonghandId::BorderBlockEndColor)] },
    ShorthandInfo { name: "border-block-end", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BorderBlockEndWidth), PropertyId::Longhand(LonghandId::BorderBlockEndStyle), PropertyId::Longhand(LonghandId::BorderBlockEndColor)] },
    ShorthandInfo { name: "border-block-start", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BorderBlockStartWidth), PropertyId::Longhand(LonghandId::BorderBlockStartStyle), PropertyId::Longhand(LonghandId::BorderBlockStartColor)] },
    ShorthandInfo { name: "border-block-style", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BorderBlockStartStyle), PropertyId::Longhand(LonghandId::BorderBlockEndStyle)] },
    ShorthandInfo { name: "border-block-width", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BorderBlockStartWidth), PropertyId::Longhand(LonghandId::BorderBlockEndWidth)] },
    ShorthandInfo { name: "border-bottom", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BorderBottomWidth), PropertyId::Longhand(LonghandId::BorderBottomStyle), PropertyId::Longhand(LonghandId::BorderBottomColor)] },
    ShorthandInfo { name: "border-color", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BorderTopColor), PropertyId::Longhand(LonghandId::BorderRightColor), PropertyId::Longhand(LonghandId::BorderBottomColor), PropertyId::Longhand(LonghandId::BorderLeftColor)] },
    ShorthandInfo { name: "border-image", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BorderImageSource), PropertyId::Longhand(LonghandId::BorderImageSlice), PropertyId::Longhand(LonghandId::BorderImageWidth), PropertyId::Longhand(LonghandId::BorderImageOutset), PropertyId::Longhand(LonghandId::BorderImageRepeat)] },
    ShorthandInfo { name: "border-inline", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Shorthand(ShorthandId::BorderInlineWidth), PropertyId::Shorthand(ShorthandId::BorderInlineStyle), PropertyId::Shorthand(ShorthandId::BorderInlineColor)] },
    ShorthandInfo { name: "border-inline-color", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BorderInlineStartColor), PropertyId::Longhand(LonghandId::BorderInlineEndColor)] },
    ShorthandInfo { name: "border-inline-end", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BorderInlineEndWidth), PropertyId::Longhand(LonghandId::BorderInlineEndStyle), PropertyId::Longhand(LonghandId::BorderInlineEndColor)] },
    ShorthandInfo { name: "border-inline-start", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BorderInlineStartWidth), PropertyId::Longhand(LonghandId::BorderInlineStartStyle), PropertyId::Longhand(LonghandId::BorderInlineStartColor)] },
    ShorthandInfo { name: "border-inline-style", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BorderInlineStartStyle), PropertyId::Longhand(LonghandId::BorderInlineEndStyle)] },
    ShorthandInfo { name: "border-inline-width", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BorderInlineStartWidth), PropertyId::Longhand(LonghandId::BorderInlineEndWidth)] },
    ShorthandInfo { name: "border-left", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BorderLeftWidth), PropertyId::Longhand(LonghandId::BorderLeftStyle), PropertyId::Longhand(LonghandId::BorderLeftColor)] },
    ShorthandInfo { name: "border-radius", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BorderTopLeftRadius), PropertyId::Longhand(LonghandId::BorderTopRightRadius), PropertyId::Longhand(LonghandId::BorderBottomRightRadius), PropertyId::Longhand(LonghandId::BorderBottomLeftRadius)] },
    ShorthandInfo { name: "border-right", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BorderRightWidth), PropertyId::Longhand(LonghandId::BorderRightStyle), PropertyId::Longhand(LonghandId::BorderRightColor)] },
    ShorthandInfo { name: "border-style", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BorderTopStyle), PropertyId::Longhand(LonghandId::BorderRightStyle), PropertyId::Longhand(LonghandId::BorderBottomStyle), PropertyId::Longhand(LonghandId::BorderLeftStyle)] },
    ShorthandInfo { name: "border-top", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BorderTopWidth), PropertyId::Longhand(LonghandId::BorderTopStyle), PropertyId::Longhand(LonghandId::BorderTopColor)] },
    ShorthandInfo { name: "border-width", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::BorderTopWidth), PropertyId::Longhand(LonghandId::BorderRightWidth), PropertyId::Longhand(LonghandId::BorderBottomWidth), PropertyId::Longhand(LonghandId::BorderLeftWidth)] },
    ShorthandInfo { name: "caret", inherited: true, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::CaretColor), PropertyId::Longhand(LonghandId::CaretAnimation), PropertyId::Longhand(LonghandId::CaretShape)] },
    ShorthandInfo { name: "column-rule", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::ColumnRuleWidth), PropertyId::Longhand(LonghandId::ColumnRuleStyle), PropertyId::Longhand(LonghandId::ColumnRuleColor)] },
    ShorthandInfo { name: "columns", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::ColumnWidth), PropertyId::Longhand(LonghandId::ColumnCount), PropertyId::Longhand(LonghandId::ColumnHeight)] },
    ShorthandInfo { name: "contain-intrinsic-size", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::ContainIntrinsicWidth), PropertyId::Longhand(LonghandId::ContainIntrinsicHeight)] },
    ShorthandInfo { name: "container", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::ContainerName), PropertyId::Longhand(LonghandId::ContainerType)] },
    ShorthandInfo { name: "corner-block-end-shape", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::CornerEndStartShape), PropertyId::Longhand(LonghandId::CornerEndEndShape)] },
    ShorthandInfo { name: "corner-block-start-shape", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::CornerStartStartShape), PropertyId::Longhand(LonghandId::CornerStartEndShape)] },
    ShorthandInfo { name: "corner-bottom-shape", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::CornerBottomLeftShape), PropertyId::Longhand(LonghandId::CornerBottomRightShape)] },
    ShorthandInfo { name: "corner-inline-end-shape", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::CornerStartEndShape), PropertyId::Longhand(LonghandId::CornerEndEndShape)] },
    ShorthandInfo { name: "corner-inline-start-shape", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::CornerStartStartShape), PropertyId::Longhand(LonghandId::CornerStartEndShape)] },
    ShorthandInfo { name: "corner-left-shape", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::CornerTopLeftShape), PropertyId::Longhand(LonghandId::CornerBottomLeftShape)] },
    ShorthandInfo { name: "corner-right-shape", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::CornerTopRightShape), PropertyId::Longhand(LonghandId::CornerBottomRightShape)] },
    ShorthandInfo { name: "corner-shape", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::CornerTopLeftShape), PropertyId::Longhand(LonghandId::CornerTopRightShape), PropertyId::Longhand(LonghandId::CornerBottomLeftShape), PropertyId::Longhand(LonghandId::CornerBottomRightShape)] },
    ShorthandInfo { name: "corner-top-shape", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::CornerTopLeftShape), PropertyId::Longhand(LonghandId::CornerTopRightShape)] },
    ShorthandInfo { name: "flex", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::FlexGrow), PropertyId::Longhand(LonghandId::FlexShrink), PropertyId::Longhand(LonghandId::FlexBasis)] },
    ShorthandInfo { name: "flex-flow", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::FlexDirection), PropertyId::Longhand(LonghandId::FlexWrap)] },
    ShorthandInfo { name: "font", inherited: true, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::FontStyle), PropertyId::Longhand(LonghandId::FontVariant), PropertyId::Longhand(LonghandId::FontWeight), PropertyId::Longhand(LonghandId::FontStretch), PropertyId::Longhand(LonghandId::FontSize), PropertyId::Longhand(LonghandId::LineHeight), PropertyId::Longhand(LonghandId::FontFamily)] },
    ShorthandInfo { name: "gap", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::RowGap), PropertyId::Longhand(LonghandId::ColumnGap)] },
    ShorthandInfo { name: "grid", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::GridTemplateRows), PropertyId::Longhand(LonghandId::GridTemplateColumns), PropertyId::Longhand(LonghandId::GridTemplateAreas), PropertyId::Longhand(LonghandId::GridAutoRows), PropertyId::Longhand(LonghandId::GridAutoColumns), PropertyId::Longhand(LonghandId::GridAutoFlow)] },
    ShorthandInfo { name: "grid-area", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::GridRowStart), PropertyId::Longhand(LonghandId::GridColumnStart), PropertyId::Longhand(LonghandId::GridRowEnd), PropertyId::Longhand(LonghandId::GridColumnEnd)] },
    ShorthandInfo { name: "grid-column", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::GridColumnStart), PropertyId::Longhand(LonghandId::GridColumnEnd)] },
    ShorthandInfo { name: "grid-gap", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::GridRowGap), PropertyId::Longhand(LonghandId::GridColumnGap)] },
    ShorthandInfo { name: "grid-row", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::GridRowStart), PropertyId::Longhand(LonghandId::GridRowEnd)] },
    ShorthandInfo { name: "grid-template", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::GridTemplateColumns), PropertyId::Longhand(LonghandId::GridTemplateRows), PropertyId::Longhand(LonghandId::GridTemplateAreas)] },
    ShorthandInfo { name: "inset", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::Top), PropertyId::Longhand(LonghandId::Bottom), PropertyId::Longhand(LonghandId::Left), PropertyId::Longhand(LonghandId::Right)] },
    ShorthandInfo { name: "inset-block", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::InsetBlockStart), PropertyId::Longhand(LonghandId::InsetBlockEnd)] },
    ShorthandInfo { name: "inset-inline", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::InsetInlineStart), PropertyId::Longhand(LonghandId::InsetInlineEnd)] },
    ShorthandInfo { name: "interest-delay", inherited: true, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::InterestDelayStart), PropertyId::Longhand(LonghandId::InterestDelayEnd)] },
    ShorthandInfo { name: "list-style", inherited: true, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::ListStyleImage), PropertyId::Longhand(LonghandId::ListStylePosition), PropertyId::Longhand(LonghandId::ListStyleType)] },
    ShorthandInfo { name: "margin", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::MarginBottom), PropertyId::Longhand(LonghandId::MarginLeft), PropertyId::Longhand(LonghandId::MarginRight), PropertyId::Longhand(LonghandId::MarginTop)] },
    ShorthandInfo { name: "margin-block", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::MarginBlockStart), PropertyId::Longhand(LonghandId::MarginBlockEnd)] },
    ShorthandInfo { name: "margin-inline", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::MarginInlineStart), PropertyId::Longhand(LonghandId::MarginInlineEnd)] },
    ShorthandInfo { name: "mask", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::MaskImage), PropertyId::Longhand(LonghandId::MaskMode), PropertyId::Longhand(LonghandId::MaskRepeat), PropertyId::Longhand(LonghandId::MaskPosition), PropertyId::Longhand(LonghandId::MaskClip), PropertyId::Longhand(LonghandId::MaskOrigin), PropertyId::Longhand(LonghandId::MaskSize), PropertyId::Longhand(LonghandId::MaskComposite)] },
    ShorthandInfo { name: "mask-border", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::MaskBorderMode), PropertyId::Longhand(LonghandId::MaskBorderOutset), PropertyId::Longhand(LonghandId::MaskBorderRepeat), PropertyId::Longhand(LonghandId::MaskBorderSlice), PropertyId::Longhand(LonghandId::MaskBorderSource), PropertyId::Longhand(LonghandId::MaskBorderWidth)] },
    ShorthandInfo { name: "offset", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::OffsetPosition), PropertyId::Longhand(LonghandId::OffsetPath), PropertyId::Longhand(LonghandId::OffsetDistance), PropertyId::Longhand(LonghandId::OffsetAnchor), PropertyId::Longhand(LonghandId::OffsetRotate)] },
    ShorthandInfo { name: "outline", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::OutlineWidth), PropertyId::Longhand(LonghandId::OutlineStyle), PropertyId::Longhand(LonghandId::OutlineColor)] },
    ShorthandInfo { name: "overflow", inherited: false, initial: Some("visible"), percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::OverflowX), PropertyId::Longhand(LonghandId::OverflowY)] },
    ShorthandInfo { name: "overscroll-behavior", inherited: false, initial: Some("auto"), percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::OverscrollBehaviorX), PropertyId::Longhand(LonghandId::OverscrollBehaviorY)] },
    ShorthandInfo { name: "padding", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::PaddingBottom), PropertyId::Longhand(LonghandId::PaddingLeft), PropertyId::Longhand(LonghandId::PaddingRight), PropertyId::Longhand(LonghandId::PaddingTop)] },
    ShorthandInfo { name: "padding-block", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::PaddingBlockStart), PropertyId::Longhand(LonghandId::PaddingBlockEnd)] },
    ShorthandInfo { name: "padding-inline", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::PaddingInlineStart), PropertyId::Longhand(LonghandId::PaddingInlineEnd)] },
    ShorthandInfo { name: "place-content", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::AlignContent), PropertyId::Longhand(LonghandId::JustifyContent)] },
    ShorthandInfo { name: "place-items", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::AlignItems), PropertyId::Longhand(LonghandId::JustifyItems)] },
    ShorthandInfo { name: "place-self", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::AlignSelf), PropertyId::Longhand(LonghandId::JustifySelf)] },
    ShorthandInfo { name: "position-try", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::PositionTryFallbacks), PropertyId::Longhand(LonghandId::PositionTryOrder)] },
    ShorthandInfo { name: "scroll-margin", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::ScrollMarginBottom), PropertyId::Longhand(LonghandId::ScrollMarginLeft), PropertyId::Longhand(LonghandId::ScrollMarginRight), PropertyId::Longhand(LonghandId::ScrollMarginTop)] },
    ShorthandInfo { name: "scroll-margin-block", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::ScrollMarginBlockStart), PropertyId::Longhand(LonghandId::ScrollMarginBlockEnd)] },
    ShorthandInfo { name: "scroll-margin-inline", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::ScrollMarginInlineStart), PropertyId::Longhand(LonghandId::ScrollMarginInlineEnd)] },
    ShorthandInfo { name: "scroll-padding", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::ScrollPaddingBottom), PropertyId::Longhand(LonghandId::ScrollPaddingLeft), PropertyId::Longhand(LonghandId::ScrollPaddingRight), PropertyId::Longhand(LonghandId::ScrollPaddingTop)] },
    ShorthandInfo { name: "scroll-padding-block", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::ScrollPaddingBlockStart), PropertyId::Longhand(LonghandId::ScrollPaddingBlockEnd)] },
    ShorthandInfo { name: "scroll-padding-inline", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::ScrollPaddingInlineStart), PropertyId::Longhand(LonghandId::ScrollPaddingInlineEnd)] },
    ShorthandInfo { name: "scroll-timeline", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::ScrollTimelineName), PropertyId::Longhand(LonghandId::ScrollTimelineAxis)] },
    ShorthandInfo { name: "text-decoration", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::TextDecorationLine), PropertyId::Longhand(LonghandId::TextDecorationStyle), PropertyId::Longhand(LonghandId::TextDecorationColor), PropertyId::Longhand(LonghandId::TextDecorationThickness)] },
    ShorthandInfo { name: "text-emphasis", inherited: true, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::TextEmphasisStyle), PropertyId::Longhand(LonghandId::TextEmphasisColor)] },
    ShorthandInfo { name: "text-wrap", inherited: true, initial: Some("wrap"), percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::TextWrapMode), PropertyId::Longhand(LonghandId::TextWrapStyle)] },
    ShorthandInfo { name: "timeline-trigger", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::TimelineTriggerName), PropertyId::Longhand(LonghandId::TimelineTriggerSource), PropertyId::Shorthand(ShorthandId::TimelineTriggerActivationRange), PropertyId::Shorthand(ShorthandId::TimelineTriggerActiveRange)] },
    ShorthandInfo { name: "timeline-trigger-activation-range", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::TimelineTriggerActivationRangeStart), PropertyId::Longhand(LonghandId::TimelineTriggerActivationRangeEnd)] },
    ShorthandInfo { name: "timeline-trigger-active-range", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::TimelineTriggerActiveRangeStart), PropertyId::Longhand(LonghandId::TimelineTriggerActiveRangeEnd)] },
    ShorthandInfo { name: "transition", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::TransitionDelay), PropertyId::Longhand(LonghandId::TransitionDuration), PropertyId::Longhand(LonghandId::TransitionProperty), PropertyId::Longhand(LonghandId::TransitionTimingFunction), PropertyId::Longhand(LonghandId::TransitionBehavior)] },
    ShorthandInfo { name: "view-timeline", inherited: false, initial: None, percentage_is_number: false, longhands: &[PropertyId::Longhand(LonghandId::ViewTimelineName), PropertyId::Longhand(LonghandId::ViewTimelineAxis)] },
];

/// Every [`ShorthandId`], in id order - which is name order.
#[rustfmt::skip]
pub static ALL_SHORTHAND_IDS: [ShorthandId; SHORTHAND_COUNT] = [
    ShorthandId::MozOutlineRadius, ShorthandId::MsContentZoomLimit, ShorthandId::MsContentZoomSnap,
    ShorthandId::MsScrollLimit, ShorthandId::MsScrollSnapX, ShorthandId::MsScrollSnapY,
    ShorthandId::WebkitBorderAfter, ShorthandId::WebkitBorderBefore, ShorthandId::WebkitBorderEnd,
    ShorthandId::WebkitBorderStart, ShorthandId::WebkitMask, ShorthandId::WebkitTextStroke, ShorthandId::Animation,
    ShorthandId::AnimationRange, ShorthandId::Background, ShorthandId::BackgroundPosition, ShorthandId::Border,
    ShorthandId::BorderBlock, ShorthandId::BorderBlockColor, ShorthandId::BorderBlockEnd,
    ShorthandId::BorderBlockStart, ShorthandId::BorderBlockStyle, ShorthandId::BorderBlockWidth,
    ShorthandId::BorderBottom, ShorthandId::BorderColor, ShorthandId::BorderImage, ShorthandId::BorderInline,
    ShorthandId::BorderInlineColor, ShorthandId::BorderInlineEnd, ShorthandId::BorderInlineStart,
    ShorthandId::BorderInlineStyle, ShorthandId::BorderInlineWidth, ShorthandId::BorderLeft,
    ShorthandId::BorderRadius, ShorthandId::BorderRight, ShorthandId::BorderStyle, ShorthandId::BorderTop,
    ShorthandId::BorderWidth, ShorthandId::Caret, ShorthandId::ColumnRule, ShorthandId::Columns,
    ShorthandId::ContainIntrinsicSize, ShorthandId::Container, ShorthandId::CornerBlockEndShape,
    ShorthandId::CornerBlockStartShape, ShorthandId::CornerBottomShape, ShorthandId::CornerInlineEndShape,
    ShorthandId::CornerInlineStartShape, ShorthandId::CornerLeftShape, ShorthandId::CornerRightShape,
    ShorthandId::CornerShape, ShorthandId::CornerTopShape, ShorthandId::Flex, ShorthandId::FlexFlow,
    ShorthandId::Font, ShorthandId::Gap, ShorthandId::Grid, ShorthandId::GridArea, ShorthandId::GridColumn,
    ShorthandId::GridGap, ShorthandId::GridRow, ShorthandId::GridTemplate, ShorthandId::Inset,
    ShorthandId::InsetBlock, ShorthandId::InsetInline, ShorthandId::InterestDelay, ShorthandId::ListStyle,
    ShorthandId::Margin, ShorthandId::MarginBlock, ShorthandId::MarginInline, ShorthandId::Mask,
    ShorthandId::MaskBorder, ShorthandId::Offset, ShorthandId::Outline, ShorthandId::Overflow,
    ShorthandId::OverscrollBehavior, ShorthandId::Padding, ShorthandId::PaddingBlock, ShorthandId::PaddingInline,
    ShorthandId::PlaceContent, ShorthandId::PlaceItems, ShorthandId::PlaceSelf, ShorthandId::PositionTry,
    ShorthandId::ScrollMargin, ShorthandId::ScrollMarginBlock, ShorthandId::ScrollMarginInline,
    ShorthandId::ScrollPadding, ShorthandId::ScrollPaddingBlock, ShorthandId::ScrollPaddingInline,
    ShorthandId::ScrollTimeline, ShorthandId::TextDecoration, ShorthandId::TextEmphasis, ShorthandId::TextWrap,
    ShorthandId::TimelineTrigger, ShorthandId::TimelineTriggerActivationRange,
    ShorthandId::TimelineTriggerActiveRange, ShorthandId::Transition, ShorthandId::ViewTimeline,
];

/// Every property name with its id, sorted by name so [`PropertyId::from_name`] can
/// binary-search it.
#[rustfmt::skip]
static BY_NAME: [(&str, PropertyId); PROPERTY_COUNT] = [
    ("-moz-appearance", PropertyId::Longhand(LonghandId::MozAppearance)),
    ("-moz-binding", PropertyId::Longhand(LonghandId::MozBinding)),
    ("-moz-border-bottom-colors", PropertyId::Longhand(LonghandId::MozBorderBottomColors)),
    ("-moz-border-left-colors", PropertyId::Longhand(LonghandId::MozBorderLeftColors)),
    ("-moz-border-right-colors", PropertyId::Longhand(LonghandId::MozBorderRightColors)),
    ("-moz-border-top-colors", PropertyId::Longhand(LonghandId::MozBorderTopColors)),
    ("-moz-context-properties", PropertyId::Longhand(LonghandId::MozContextProperties)),
    ("-moz-float-edge", PropertyId::Longhand(LonghandId::MozFloatEdge)),
    ("-moz-force-broken-image-icon", PropertyId::Longhand(LonghandId::MozForceBrokenImageIcon)),
    ("-moz-orient", PropertyId::Longhand(LonghandId::MozOrient)),
    ("-moz-outline-radius", PropertyId::Shorthand(ShorthandId::MozOutlineRadius)),
    ("-moz-outline-radius-bottomleft", PropertyId::Longhand(LonghandId::MozOutlineRadiusBottomleft)),
    ("-moz-outline-radius-bottomright", PropertyId::Longhand(LonghandId::MozOutlineRadiusBottomright)),
    ("-moz-outline-radius-topleft", PropertyId::Longhand(LonghandId::MozOutlineRadiusTopleft)),
    ("-moz-outline-radius-topright", PropertyId::Longhand(LonghandId::MozOutlineRadiusTopright)),
    ("-moz-stack-sizing", PropertyId::Longhand(LonghandId::MozStackSizing)),
    ("-moz-text-blink", PropertyId::Longhand(LonghandId::MozTextBlink)),
    ("-moz-user-focus", PropertyId::Longhand(LonghandId::MozUserFocus)),
    ("-moz-user-input", PropertyId::Longhand(LonghandId::MozUserInput)),
    ("-moz-user-modify", PropertyId::Longhand(LonghandId::MozUserModify)),
    ("-moz-window-dragging", PropertyId::Longhand(LonghandId::MozWindowDragging)),
    ("-moz-window-shadow", PropertyId::Longhand(LonghandId::MozWindowShadow)),
    ("-ms-accelerator", PropertyId::Longhand(LonghandId::MsAccelerator)),
    ("-ms-block-progression", PropertyId::Longhand(LonghandId::MsBlockProgression)),
    ("-ms-content-zoom-chaining", PropertyId::Longhand(LonghandId::MsContentZoomChaining)),
    ("-ms-content-zoom-limit", PropertyId::Shorthand(ShorthandId::MsContentZoomLimit)),
    ("-ms-content-zoom-limit-max", PropertyId::Longhand(LonghandId::MsContentZoomLimitMax)),
    ("-ms-content-zoom-limit-min", PropertyId::Longhand(LonghandId::MsContentZoomLimitMin)),
    ("-ms-content-zoom-snap", PropertyId::Shorthand(ShorthandId::MsContentZoomSnap)),
    ("-ms-content-zoom-snap-points", PropertyId::Longhand(LonghandId::MsContentZoomSnapPoints)),
    ("-ms-content-zoom-snap-type", PropertyId::Longhand(LonghandId::MsContentZoomSnapType)),
    ("-ms-content-zooming", PropertyId::Longhand(LonghandId::MsContentZooming)),
    ("-ms-filter", PropertyId::Longhand(LonghandId::MsFilter)),
    ("-ms-flow-from", PropertyId::Longhand(LonghandId::MsFlowFrom)),
    ("-ms-flow-into", PropertyId::Longhand(LonghandId::MsFlowInto)),
    ("-ms-grid-columns", PropertyId::Longhand(LonghandId::MsGridColumns)),
    ("-ms-grid-rows", PropertyId::Longhand(LonghandId::MsGridRows)),
    ("-ms-high-contrast-adjust", PropertyId::Longhand(LonghandId::MsHighContrastAdjust)),
    ("-ms-hyphenate-limit-chars", PropertyId::Longhand(LonghandId::MsHyphenateLimitChars)),
    ("-ms-hyphenate-limit-lines", PropertyId::Longhand(LonghandId::MsHyphenateLimitLines)),
    ("-ms-hyphenate-limit-zone", PropertyId::Longhand(LonghandId::MsHyphenateLimitZone)),
    ("-ms-ime-align", PropertyId::Longhand(LonghandId::MsImeAlign)),
    ("-ms-overflow-style", PropertyId::Longhand(LonghandId::MsOverflowStyle)),
    ("-ms-scroll-chaining", PropertyId::Longhand(LonghandId::MsScrollChaining)),
    ("-ms-scroll-limit", PropertyId::Shorthand(ShorthandId::MsScrollLimit)),
    ("-ms-scroll-limit-x-max", PropertyId::Longhand(LonghandId::MsScrollLimitXMax)),
    ("-ms-scroll-limit-x-min", PropertyId::Longhand(LonghandId::MsScrollLimitXMin)),
    ("-ms-scroll-limit-y-max", PropertyId::Longhand(LonghandId::MsScrollLimitYMax)),
    ("-ms-scroll-limit-y-min", PropertyId::Longhand(LonghandId::MsScrollLimitYMin)),
    ("-ms-scroll-rails", PropertyId::Longhand(LonghandId::MsScrollRails)),
    ("-ms-scroll-snap-points-x", PropertyId::Longhand(LonghandId::MsScrollSnapPointsX)),
    ("-ms-scroll-snap-points-y", PropertyId::Longhand(LonghandId::MsScrollSnapPointsY)),
    ("-ms-scroll-snap-type", PropertyId::Longhand(LonghandId::MsScrollSnapType)),
    ("-ms-scroll-snap-x", PropertyId::Shorthand(ShorthandId::MsScrollSnapX)),
    ("-ms-scroll-snap-y", PropertyId::Shorthand(ShorthandId::MsScrollSnapY)),
    ("-ms-scroll-translation", PropertyId::Longhand(LonghandId::MsScrollTranslation)),
    ("-ms-scrollbar-3dlight-color", PropertyId::Longhand(LonghandId::MsScrollbar3dlightColor)),
    ("-ms-scrollbar-arrow-color", PropertyId::Longhand(LonghandId::MsScrollbarArrowColor)),
    ("-ms-scrollbar-base-color", PropertyId::Longhand(LonghandId::MsScrollbarBaseColor)),
    ("-ms-scrollbar-darkshadow-color", PropertyId::Longhand(LonghandId::MsScrollbarDarkshadowColor)),
    ("-ms-scrollbar-face-color", PropertyId::Longhand(LonghandId::MsScrollbarFaceColor)),
    ("-ms-scrollbar-highlight-color", PropertyId::Longhand(LonghandId::MsScrollbarHighlightColor)),
    ("-ms-scrollbar-shadow-color", PropertyId::Longhand(LonghandId::MsScrollbarShadowColor)),
    ("-ms-scrollbar-track-color", PropertyId::Longhand(LonghandId::MsScrollbarTrackColor)),
    ("-ms-text-autospace", PropertyId::Longhand(LonghandId::MsTextAutospace)),
    ("-ms-touch-select", PropertyId::Longhand(LonghandId::MsTouchSelect)),
    ("-ms-user-select", PropertyId::Longhand(LonghandId::MsUserSelect)),
    ("-ms-wrap-flow", PropertyId::Longhand(LonghandId::MsWrapFlow)),
    ("-ms-wrap-margin", PropertyId::Longhand(LonghandId::MsWrapMargin)),
    ("-ms-wrap-through", PropertyId::Longhand(LonghandId::MsWrapThrough)),
    ("-webkit-appearance", PropertyId::Longhand(LonghandId::WebkitAppearance)),
    ("-webkit-border-after", PropertyId::Shorthand(ShorthandId::WebkitBorderAfter)),
    ("-webkit-border-after-color", PropertyId::Longhand(LonghandId::WebkitBorderAfterColor)),
    ("-webkit-border-after-style", PropertyId::Longhand(LonghandId::WebkitBorderAfterStyle)),
    ("-webkit-border-after-width", PropertyId::Longhand(LonghandId::WebkitBorderAfterWidth)),
    ("-webkit-border-before", PropertyId::Shorthand(ShorthandId::WebkitBorderBefore)),
    ("-webkit-border-before-color", PropertyId::Longhand(LonghandId::WebkitBorderBeforeColor)),
    ("-webkit-border-before-style", PropertyId::Longhand(LonghandId::WebkitBorderBeforeStyle)),
    ("-webkit-border-before-width", PropertyId::Longhand(LonghandId::WebkitBorderBeforeWidth)),
    ("-webkit-border-end", PropertyId::Shorthand(ShorthandId::WebkitBorderEnd)),
    ("-webkit-border-end-color", PropertyId::Longhand(LonghandId::WebkitBorderEndColor)),
    ("-webkit-border-end-style", PropertyId::Longhand(LonghandId::WebkitBorderEndStyle)),
    ("-webkit-border-end-width", PropertyId::Longhand(LonghandId::WebkitBorderEndWidth)),
    ("-webkit-border-start", PropertyId::Shorthand(ShorthandId::WebkitBorderStart)),
    ("-webkit-border-start-color", PropertyId::Longhand(LonghandId::WebkitBorderStartColor)),
    ("-webkit-border-start-style", PropertyId::Longhand(LonghandId::WebkitBorderStartStyle)),
    ("-webkit-border-start-width", PropertyId::Longhand(LonghandId::WebkitBorderStartWidth)),
    ("-webkit-box-reflect", PropertyId::Longhand(LonghandId::WebkitBoxReflect)),
    ("-webkit-line-clamp", PropertyId::Longhand(LonghandId::WebkitLineClamp)),
    ("-webkit-mask", PropertyId::Shorthand(ShorthandId::WebkitMask)),
    ("-webkit-mask-attachment", PropertyId::Longhand(LonghandId::WebkitMaskAttachment)),
    ("-webkit-mask-clip", PropertyId::Longhand(LonghandId::WebkitMaskClip)),
    ("-webkit-mask-composite", PropertyId::Longhand(LonghandId::WebkitMaskComposite)),
    ("-webkit-mask-image", PropertyId::Longhand(LonghandId::WebkitMaskImage)),
    ("-webkit-mask-origin", PropertyId::Longhand(LonghandId::WebkitMaskOrigin)),
    ("-webkit-mask-position", PropertyId::Longhand(LonghandId::WebkitMaskPosition)),
    ("-webkit-mask-position-x", PropertyId::Longhand(LonghandId::WebkitMaskPositionX)),
    ("-webkit-mask-position-y", PropertyId::Longhand(LonghandId::WebkitMaskPositionY)),
    ("-webkit-mask-repeat", PropertyId::Longhand(LonghandId::WebkitMaskRepeat)),
    ("-webkit-mask-repeat-x", PropertyId::Longhand(LonghandId::WebkitMaskRepeatX)),
    ("-webkit-mask-repeat-y", PropertyId::Longhand(LonghandId::WebkitMaskRepeatY)),
    ("-webkit-mask-size", PropertyId::Longhand(LonghandId::WebkitMaskSize)),
    ("-webkit-overflow-scrolling", PropertyId::Longhand(LonghandId::WebkitOverflowScrolling)),
    ("-webkit-tap-highlight-color", PropertyId::Longhand(LonghandId::WebkitTapHighlightColor)),
    ("-webkit-text-fill-color", PropertyId::Longhand(LonghandId::WebkitTextFillColor)),
    ("-webkit-text-stroke", PropertyId::Shorthand(ShorthandId::WebkitTextStroke)),
    ("-webkit-text-stroke-color", PropertyId::Longhand(LonghandId::WebkitTextStrokeColor)),
    ("-webkit-text-stroke-width", PropertyId::Longhand(LonghandId::WebkitTextStrokeWidth)),
    ("-webkit-touch-callout", PropertyId::Longhand(LonghandId::WebkitTouchCallout)),
    ("-webkit-user-modify", PropertyId::Longhand(LonghandId::WebkitUserModify)),
    ("-webkit-user-select", PropertyId::Longhand(LonghandId::WebkitUserSelect)),
    ("accent-color", PropertyId::Longhand(LonghandId::AccentColor)),
    ("align-content", PropertyId::Longhand(LonghandId::AlignContent)),
    ("align-items", PropertyId::Longhand(LonghandId::AlignItems)),
    ("align-self", PropertyId::Longhand(LonghandId::AlignSelf)),
    ("align-tracks", PropertyId::Longhand(LonghandId::AlignTracks)),
    ("alignment-baseline", PropertyId::Longhand(LonghandId::AlignmentBaseline)),
    ("all", PropertyId::Longhand(LonghandId::All)),
    ("anchor-name", PropertyId::Longhand(LonghandId::AnchorName)),
    ("anchor-scope", PropertyId::Longhand(LonghandId::AnchorScope)),
    ("animation", PropertyId::Shorthand(ShorthandId::Animation)),
    ("animation-composition", PropertyId::Longhand(LonghandId::AnimationComposition)),
    ("animation-delay", PropertyId::Longhand(LonghandId::AnimationDelay)),
    ("animation-direction", PropertyId::Longhand(LonghandId::AnimationDirection)),
    ("animation-duration", PropertyId::Longhand(LonghandId::AnimationDuration)),
    ("animation-fill-mode", PropertyId::Longhand(LonghandId::AnimationFillMode)),
    ("animation-iteration-count", PropertyId::Longhand(LonghandId::AnimationIterationCount)),
    ("animation-name", PropertyId::Longhand(LonghandId::AnimationName)),
    ("animation-play-state", PropertyId::Longhand(LonghandId::AnimationPlayState)),
    ("animation-range", PropertyId::Shorthand(ShorthandId::AnimationRange)),
    ("animation-range-end", PropertyId::Longhand(LonghandId::AnimationRangeEnd)),
    ("animation-range-start", PropertyId::Longhand(LonghandId::AnimationRangeStart)),
    ("animation-timeline", PropertyId::Longhand(LonghandId::AnimationTimeline)),
    ("animation-timing-function", PropertyId::Longhand(LonghandId::AnimationTimingFunction)),
    ("animation-trigger", PropertyId::Longhand(LonghandId::AnimationTrigger)),
    ("appearance", PropertyId::Longhand(LonghandId::Appearance)),
    ("aspect-ratio", PropertyId::Longhand(LonghandId::AspectRatio)),
    ("backdrop-filter", PropertyId::Longhand(LonghandId::BackdropFilter)),
    ("backface-visibility", PropertyId::Longhand(LonghandId::BackfaceVisibility)),
    ("background", PropertyId::Shorthand(ShorthandId::Background)),
    ("background-attachment", PropertyId::Longhand(LonghandId::BackgroundAttachment)),
    ("background-blend-mode", PropertyId::Longhand(LonghandId::BackgroundBlendMode)),
    ("background-clip", PropertyId::Longhand(LonghandId::BackgroundClip)),
    ("background-color", PropertyId::Longhand(LonghandId::BackgroundColor)),
    ("background-image", PropertyId::Longhand(LonghandId::BackgroundImage)),
    ("background-origin", PropertyId::Longhand(LonghandId::BackgroundOrigin)),
    ("background-position", PropertyId::Shorthand(ShorthandId::BackgroundPosition)),
    ("background-position-x", PropertyId::Longhand(LonghandId::BackgroundPositionX)),
    ("background-position-y", PropertyId::Longhand(LonghandId::BackgroundPositionY)),
    ("background-repeat", PropertyId::Longhand(LonghandId::BackgroundRepeat)),
    ("background-size", PropertyId::Longhand(LonghandId::BackgroundSize)),
    ("baseline-shift", PropertyId::Longhand(LonghandId::BaselineShift)),
    ("baseline-source", PropertyId::Longhand(LonghandId::BaselineSource)),
    ("block-size", PropertyId::Longhand(LonghandId::BlockSize)),
    ("border", PropertyId::Shorthand(ShorthandId::Border)),
    ("border-block", PropertyId::Shorthand(ShorthandId::BorderBlock)),
    ("border-block-color", PropertyId::Shorthand(ShorthandId::BorderBlockColor)),
    ("border-block-end", PropertyId::Shorthand(ShorthandId::BorderBlockEnd)),
    ("border-block-end-color", PropertyId::Longhand(LonghandId::BorderBlockEndColor)),
    ("border-block-end-style", PropertyId::Longhand(LonghandId::BorderBlockEndStyle)),
    ("border-block-end-width", PropertyId::Longhand(LonghandId::BorderBlockEndWidth)),
    ("border-block-start", PropertyId::Shorthand(ShorthandId::BorderBlockStart)),
    ("border-block-start-color", PropertyId::Longhand(LonghandId::BorderBlockStartColor)),
    ("border-block-start-style", PropertyId::Longhand(LonghandId::BorderBlockStartStyle)),
    ("border-block-start-width", PropertyId::Longhand(LonghandId::BorderBlockStartWidth)),
    ("border-block-style", PropertyId::Shorthand(ShorthandId::BorderBlockStyle)),
    ("border-block-width", PropertyId::Shorthand(ShorthandId::BorderBlockWidth)),
    ("border-bottom", PropertyId::Shorthand(ShorthandId::BorderBottom)),
    ("border-bottom-color", PropertyId::Longhand(LonghandId::BorderBottomColor)),
    ("border-bottom-left-radius", PropertyId::Longhand(LonghandId::BorderBottomLeftRadius)),
    ("border-bottom-right-radius", PropertyId::Longhand(LonghandId::BorderBottomRightRadius)),
    ("border-bottom-style", PropertyId::Longhand(LonghandId::BorderBottomStyle)),
    ("border-bottom-width", PropertyId::Longhand(LonghandId::BorderBottomWidth)),
    ("border-collapse", PropertyId::Longhand(LonghandId::BorderCollapse)),
    ("border-color", PropertyId::Shorthand(ShorthandId::BorderColor)),
    ("border-end-end-radius", PropertyId::Longhand(LonghandId::BorderEndEndRadius)),
    ("border-end-start-radius", PropertyId::Longhand(LonghandId::BorderEndStartRadius)),
    ("border-image", PropertyId::Shorthand(ShorthandId::BorderImage)),
    ("border-image-outset", PropertyId::Longhand(LonghandId::BorderImageOutset)),
    ("border-image-repeat", PropertyId::Longhand(LonghandId::BorderImageRepeat)),
    ("border-image-slice", PropertyId::Longhand(LonghandId::BorderImageSlice)),
    ("border-image-source", PropertyId::Longhand(LonghandId::BorderImageSource)),
    ("border-image-width", PropertyId::Longhand(LonghandId::BorderImageWidth)),
    ("border-inline", PropertyId::Shorthand(ShorthandId::BorderInline)),
    ("border-inline-color", PropertyId::Shorthand(ShorthandId::BorderInlineColor)),
    ("border-inline-end", PropertyId::Shorthand(ShorthandId::BorderInlineEnd)),
    ("border-inline-end-color", PropertyId::Longhand(LonghandId::BorderInlineEndColor)),
    ("border-inline-end-style", PropertyId::Longhand(LonghandId::BorderInlineEndStyle)),
    ("border-inline-end-width", PropertyId::Longhand(LonghandId::BorderInlineEndWidth)),
    ("border-inline-start", PropertyId::Shorthand(ShorthandId::BorderInlineStart)),
    ("border-inline-start-color", PropertyId::Longhand(LonghandId::BorderInlineStartColor)),
    ("border-inline-start-style", PropertyId::Longhand(LonghandId::BorderInlineStartStyle)),
    ("border-inline-start-width", PropertyId::Longhand(LonghandId::BorderInlineStartWidth)),
    ("border-inline-style", PropertyId::Shorthand(ShorthandId::BorderInlineStyle)),
    ("border-inline-width", PropertyId::Shorthand(ShorthandId::BorderInlineWidth)),
    ("border-left", PropertyId::Shorthand(ShorthandId::BorderLeft)),
    ("border-left-color", PropertyId::Longhand(LonghandId::BorderLeftColor)),
    ("border-left-style", PropertyId::Longhand(LonghandId::BorderLeftStyle)),
    ("border-left-width", PropertyId::Longhand(LonghandId::BorderLeftWidth)),
    ("border-radius", PropertyId::Shorthand(ShorthandId::BorderRadius)),
    ("border-right", PropertyId::Shorthand(ShorthandId::BorderRight)),
    ("border-right-color", PropertyId::Longhand(LonghandId::BorderRightColor)),
    ("border-right-style", PropertyId::Longhand(LonghandId::BorderRightStyle)),
    ("border-right-width", PropertyId::Longhand(LonghandId::BorderRightWidth)),
    ("border-shape", PropertyId::Longhand(LonghandId::BorderShape)),
    ("border-spacing", PropertyId::Longhand(LonghandId::BorderSpacing)),
    ("border-start-end-radius", PropertyId::Longhand(LonghandId::BorderStartEndRadius)),
    ("border-start-start-radius", PropertyId::Longhand(LonghandId::BorderStartStartRadius)),
    ("border-style", PropertyId::Shorthand(ShorthandId::BorderStyle)),
    ("border-top", PropertyId::Shorthand(ShorthandId::BorderTop)),
    ("border-top-color", PropertyId::Longhand(LonghandId::BorderTopColor)),
    ("border-top-left-radius", PropertyId::Longhand(LonghandId::BorderTopLeftRadius)),
    ("border-top-right-radius", PropertyId::Longhand(LonghandId::BorderTopRightRadius)),
    ("border-top-style", PropertyId::Longhand(LonghandId::BorderTopStyle)),
    ("border-top-width", PropertyId::Longhand(LonghandId::BorderTopWidth)),
    ("border-width", PropertyId::Shorthand(ShorthandId::BorderWidth)),
    ("bottom", PropertyId::Longhand(LonghandId::Bottom)),
    ("box-align", PropertyId::Longhand(LonghandId::BoxAlign)),
    ("box-decoration-break", PropertyId::Longhand(LonghandId::BoxDecorationBreak)),
    ("box-direction", PropertyId::Longhand(LonghandId::BoxDirection)),
    ("box-flex", PropertyId::Longhand(LonghandId::BoxFlex)),
    ("box-flex-group", PropertyId::Longhand(LonghandId::BoxFlexGroup)),
    ("box-lines", PropertyId::Longhand(LonghandId::BoxLines)),
    ("box-ordinal-group", PropertyId::Longhand(LonghandId::BoxOrdinalGroup)),
    ("box-orient", PropertyId::Longhand(LonghandId::BoxOrient)),
    ("box-pack", PropertyId::Longhand(LonghandId::BoxPack)),
    ("box-shadow", PropertyId::Longhand(LonghandId::BoxShadow)),
    ("box-sizing", PropertyId::Longhand(LonghandId::BoxSizing)),
    ("break-after", PropertyId::Longhand(LonghandId::BreakAfter)),
    ("break-before", PropertyId::Longhand(LonghandId::BreakBefore)),
    ("break-inside", PropertyId::Longhand(LonghandId::BreakInside)),
    ("caption-side", PropertyId::Longhand(LonghandId::CaptionSide)),
    ("caret", PropertyId::Shorthand(ShorthandId::Caret)),
    ("caret-animation", PropertyId::Longhand(LonghandId::CaretAnimation)),
    ("caret-color", PropertyId::Longhand(LonghandId::CaretColor)),
    ("caret-shape", PropertyId::Longhand(LonghandId::CaretShape)),
    ("clear", PropertyId::Longhand(LonghandId::Clear)),
    ("clip", PropertyId::Longhand(LonghandId::Clip)),
    ("clip-path", PropertyId::Longhand(LonghandId::ClipPath)),
    ("clip-rule", PropertyId::Longhand(LonghandId::ClipRule)),
    ("color", PropertyId::Longhand(LonghandId::Color)),
    ("color-interpolation-filters", PropertyId::Longhand(LonghandId::ColorInterpolationFilters)),
    ("color-scheme", PropertyId::Longhand(LonghandId::ColorScheme)),
    ("column-count", PropertyId::Longhand(LonghandId::ColumnCount)),
    ("column-fill", PropertyId::Longhand(LonghandId::ColumnFill)),
    ("column-gap", PropertyId::Longhand(LonghandId::ColumnGap)),
    ("column-height", PropertyId::Longhand(LonghandId::ColumnHeight)),
    ("column-rule", PropertyId::Shorthand(ShorthandId::ColumnRule)),
    ("column-rule-color", PropertyId::Longhand(LonghandId::ColumnRuleColor)),
    ("column-rule-style", PropertyId::Longhand(LonghandId::ColumnRuleStyle)),
    ("column-rule-width", PropertyId::Longhand(LonghandId::ColumnRuleWidth)),
    ("column-span", PropertyId::Longhand(LonghandId::ColumnSpan)),
    ("column-width", PropertyId::Longhand(LonghandId::ColumnWidth)),
    ("column-wrap", PropertyId::Longhand(LonghandId::ColumnWrap)),
    ("columns", PropertyId::Shorthand(ShorthandId::Columns)),
    ("contain", PropertyId::Longhand(LonghandId::Contain)),
    ("contain-intrinsic-block-size", PropertyId::Longhand(LonghandId::ContainIntrinsicBlockSize)),
    ("contain-intrinsic-height", PropertyId::Longhand(LonghandId::ContainIntrinsicHeight)),
    ("contain-intrinsic-inline-size", PropertyId::Longhand(LonghandId::ContainIntrinsicInlineSize)),
    ("contain-intrinsic-size", PropertyId::Shorthand(ShorthandId::ContainIntrinsicSize)),
    ("contain-intrinsic-width", PropertyId::Longhand(LonghandId::ContainIntrinsicWidth)),
    ("container", PropertyId::Shorthand(ShorthandId::Container)),
    ("container-name", PropertyId::Longhand(LonghandId::ContainerName)),
    ("container-type", PropertyId::Longhand(LonghandId::ContainerType)),
    ("content", PropertyId::Longhand(LonghandId::Content)),
    ("content-visibility", PropertyId::Longhand(LonghandId::ContentVisibility)),
    ("corner-block-end-shape", PropertyId::Shorthand(ShorthandId::CornerBlockEndShape)),
    ("corner-block-start-shape", PropertyId::Shorthand(ShorthandId::CornerBlockStartShape)),
    ("corner-bottom-left-shape", PropertyId::Longhand(LonghandId::CornerBottomLeftShape)),
    ("corner-bottom-right-shape", PropertyId::Longhand(LonghandId::CornerBottomRightShape)),
    ("corner-bottom-shape", PropertyId::Shorthand(ShorthandId::CornerBottomShape)),
    ("corner-end-end-shape", PropertyId::Longhand(LonghandId::CornerEndEndShape)),
    ("corner-end-start-shape", PropertyId::Longhand(LonghandId::CornerEndStartShape)),
    ("corner-inline-end-shape", PropertyId::Shorthand(ShorthandId::CornerInlineEndShape)),
    ("corner-inline-start-shape", PropertyId::Shorthand(ShorthandId::CornerInlineStartShape)),
    ("corner-left-shape", PropertyId::Shorthand(ShorthandId::CornerLeftShape)),
    ("corner-right-shape", PropertyId::Shorthand(ShorthandId::CornerRightShape)),
    ("corner-shape", PropertyId::Shorthand(ShorthandId::CornerShape)),
    ("corner-start-end-shape", PropertyId::Longhand(LonghandId::CornerStartEndShape)),
    ("corner-start-start-shape", PropertyId::Longhand(LonghandId::CornerStartStartShape)),
    ("corner-top-left-shape", PropertyId::Longhand(LonghandId::CornerTopLeftShape)),
    ("corner-top-right-shape", PropertyId::Longhand(LonghandId::CornerTopRightShape)),
    ("corner-top-shape", PropertyId::Shorthand(ShorthandId::CornerTopShape)),
    ("counter-increment", PropertyId::Longhand(LonghandId::CounterIncrement)),
    ("counter-reset", PropertyId::Longhand(LonghandId::CounterReset)),
    ("counter-set", PropertyId::Longhand(LonghandId::CounterSet)),
    ("cursor", PropertyId::Longhand(LonghandId::Cursor)),
    ("cx", PropertyId::Longhand(LonghandId::Cx)),
    ("cy", PropertyId::Longhand(LonghandId::Cy)),
    ("d", PropertyId::Longhand(LonghandId::D)),
    ("direction", PropertyId::Longhand(LonghandId::Direction)),
    ("display", PropertyId::Longhand(LonghandId::Display)),
    ("dominant-baseline", PropertyId::Longhand(LonghandId::DominantBaseline)),
    ("dynamic-range-limit", PropertyId::Longhand(LonghandId::DynamicRangeLimit)),
    ("empty-cells", PropertyId::Longhand(LonghandId::EmptyCells)),
    ("field-sizing", PropertyId::Longhand(LonghandId::FieldSizing)),
    ("fill", PropertyId::Longhand(LonghandId::Fill)),
    ("fill-opacity", PropertyId::Longhand(LonghandId::FillOpacity)),
    ("fill-rule", PropertyId::Longhand(LonghandId::FillRule)),
    ("filter", PropertyId::Longhand(LonghandId::Filter)),
    ("flex", PropertyId::Shorthand(ShorthandId::Flex)),
    ("flex-basis", PropertyId::Longhand(LonghandId::FlexBasis)),
    ("flex-direction", PropertyId::Longhand(LonghandId::FlexDirection)),
    ("flex-flow", PropertyId::Shorthand(ShorthandId::FlexFlow)),
    ("flex-grow", PropertyId::Longhand(LonghandId::FlexGrow)),
    ("flex-shrink", PropertyId::Longhand(LonghandId::FlexShrink)),
    ("flex-wrap", PropertyId::Longhand(LonghandId::FlexWrap)),
    ("float", PropertyId::Longhand(LonghandId::Float)),
    ("flood-color", PropertyId::Longhand(LonghandId::FloodColor)),
    ("flood-opacity", PropertyId::Longhand(LonghandId::FloodOpacity)),
    ("font", PropertyId::Shorthand(ShorthandId::Font)),
    ("font-family", PropertyId::Longhand(LonghandId::FontFamily)),
    ("font-feature-settings", PropertyId::Longhand(LonghandId::FontFeatureSettings)),
    ("font-kerning", PropertyId::Longhand(LonghandId::FontKerning)),
    ("font-language-override", PropertyId::Longhand(LonghandId::FontLanguageOverride)),
    ("font-optical-sizing", PropertyId::Longhand(LonghandId::FontOpticalSizing)),
    ("font-palette", PropertyId::Longhand(LonghandId::FontPalette)),
    ("font-size", PropertyId::Longhand(LonghandId::FontSize)),
    ("font-size-adjust", PropertyId::Longhand(LonghandId::FontSizeAdjust)),
    ("font-smooth", PropertyId::Longhand(LonghandId::FontSmooth)),
    ("font-stretch", PropertyId::Longhand(LonghandId::FontStretch)),
    ("font-style", PropertyId::Longhand(LonghandId::FontStyle)),
    ("font-synthesis", PropertyId::Longhand(LonghandId::FontSynthesis)),
    ("font-synthesis-position", PropertyId::Longhand(LonghandId::FontSynthesisPosition)),
    ("font-synthesis-small-caps", PropertyId::Longhand(LonghandId::FontSynthesisSmallCaps)),
    ("font-synthesis-style", PropertyId::Longhand(LonghandId::FontSynthesisStyle)),
    ("font-synthesis-weight", PropertyId::Longhand(LonghandId::FontSynthesisWeight)),
    ("font-variant", PropertyId::Longhand(LonghandId::FontVariant)),
    ("font-variant-alternates", PropertyId::Longhand(LonghandId::FontVariantAlternates)),
    ("font-variant-caps", PropertyId::Longhand(LonghandId::FontVariantCaps)),
    ("font-variant-east-asian", PropertyId::Longhand(LonghandId::FontVariantEastAsian)),
    ("font-variant-emoji", PropertyId::Longhand(LonghandId::FontVariantEmoji)),
    ("font-variant-ligatures", PropertyId::Longhand(LonghandId::FontVariantLigatures)),
    ("font-variant-numeric", PropertyId::Longhand(LonghandId::FontVariantNumeric)),
    ("font-variant-position", PropertyId::Longhand(LonghandId::FontVariantPosition)),
    ("font-variation-settings", PropertyId::Longhand(LonghandId::FontVariationSettings)),
    ("font-weight", PropertyId::Longhand(LonghandId::FontWeight)),
    ("font-width", PropertyId::Longhand(LonghandId::FontWidth)),
    ("forced-color-adjust", PropertyId::Longhand(LonghandId::ForcedColorAdjust)),
    ("frame-sizing", PropertyId::Longhand(LonghandId::FrameSizing)),
    ("gap", PropertyId::Shorthand(ShorthandId::Gap)),
    ("grid", PropertyId::Shorthand(ShorthandId::Grid)),
    ("grid-area", PropertyId::Shorthand(ShorthandId::GridArea)),
    ("grid-auto-columns", PropertyId::Longhand(LonghandId::GridAutoColumns)),
    ("grid-auto-flow", PropertyId::Longhand(LonghandId::GridAutoFlow)),
    ("grid-auto-rows", PropertyId::Longhand(LonghandId::GridAutoRows)),
    ("grid-column", PropertyId::Shorthand(ShorthandId::GridColumn)),
    ("grid-column-end", PropertyId::Longhand(LonghandId::GridColumnEnd)),
    ("grid-column-gap", PropertyId::Longhand(LonghandId::GridColumnGap)),
    ("grid-column-start", PropertyId::Longhand(LonghandId::GridColumnStart)),
    ("grid-gap", PropertyId::Shorthand(ShorthandId::GridGap)),
    ("grid-row", PropertyId::Shorthand(ShorthandId::GridRow)),
    ("grid-row-end", PropertyId::Longhand(LonghandId::GridRowEnd)),
    ("grid-row-gap", PropertyId::Longhand(LonghandId::GridRowGap)),
    ("grid-row-start", PropertyId::Longhand(LonghandId::GridRowStart)),
    ("grid-template", PropertyId::Shorthand(ShorthandId::GridTemplate)),
    ("grid-template-areas", PropertyId::Longhand(LonghandId::GridTemplateAreas)),
    ("grid-template-columns", PropertyId::Longhand(LonghandId::GridTemplateColumns)),
    ("grid-template-rows", PropertyId::Longhand(LonghandId::GridTemplateRows)),
    ("hanging-punctuation", PropertyId::Longhand(LonghandId::HangingPunctuation)),
    ("height", PropertyId::Longhand(LonghandId::Height)),
    ("hyphenate-character", PropertyId::Longhand(LonghandId::HyphenateCharacter)),
    ("hyphenate-limit-chars", PropertyId::Longhand(LonghandId::HyphenateLimitChars)),
    ("hyphens", PropertyId::Longhand(LonghandId::Hyphens)),
    ("image-orientation", PropertyId::Longhand(LonghandId::ImageOrientation)),
    ("image-rendering", PropertyId::Longhand(LonghandId::ImageRendering)),
    ("image-resolution", PropertyId::Longhand(LonghandId::ImageResolution)),
    ("ime-mode", PropertyId::Longhand(LonghandId::ImeMode)),
    ("initial-letter", PropertyId::Longhand(LonghandId::InitialLetter)),
    ("initial-letter-align", PropertyId::Longhand(LonghandId::InitialLetterAlign)),
    ("inline-size", PropertyId::Longhand(LonghandId::InlineSize)),
    ("inset", PropertyId::Shorthand(ShorthandId::Inset)),
    ("inset-block", PropertyId::Shorthand(ShorthandId::InsetBlock)),
    ("inset-block-end", PropertyId::Longhand(LonghandId::InsetBlockEnd)),
    ("inset-block-start", PropertyId::Longhand(LonghandId::InsetBlockStart)),
    ("inset-inline", PropertyId::Shorthand(ShorthandId::InsetInline)),
    ("inset-inline-end", PropertyId::Longhand(LonghandId::InsetInlineEnd)),
    ("inset-inline-start", PropertyId::Longhand(LonghandId::InsetInlineStart)),
    ("interactivity", PropertyId::Longhand(LonghandId::Interactivity)),
    ("interest-delay", PropertyId::Shorthand(ShorthandId::InterestDelay)),
    ("interest-delay-end", PropertyId::Longhand(LonghandId::InterestDelayEnd)),
    ("interest-delay-start", PropertyId::Longhand(LonghandId::InterestDelayStart)),
    ("interpolate-size", PropertyId::Longhand(LonghandId::InterpolateSize)),
    ("isolation", PropertyId::Longhand(LonghandId::Isolation)),
    ("justify-content", PropertyId::Longhand(LonghandId::JustifyContent)),
    ("justify-items", PropertyId::Longhand(LonghandId::JustifyItems)),
    ("justify-self", PropertyId::Longhand(LonghandId::JustifySelf)),
    ("justify-tracks", PropertyId::Longhand(LonghandId::JustifyTracks)),
    ("left", PropertyId::Longhand(LonghandId::Left)),
    ("letter-spacing", PropertyId::Longhand(LonghandId::LetterSpacing)),
    ("lighting-color", PropertyId::Longhand(LonghandId::LightingColor)),
    ("line-break", PropertyId::Longhand(LonghandId::LineBreak)),
    ("line-clamp", PropertyId::Longhand(LonghandId::LineClamp)),
    ("line-height", PropertyId::Longhand(LonghandId::LineHeight)),
    ("line-height-step", PropertyId::Longhand(LonghandId::LineHeightStep)),
    ("list-style", PropertyId::Shorthand(ShorthandId::ListStyle)),
    ("list-style-image", PropertyId::Longhand(LonghandId::ListStyleImage)),
    ("list-style-position", PropertyId::Longhand(LonghandId::ListStylePosition)),
    ("list-style-type", PropertyId::Longhand(LonghandId::ListStyleType)),
    ("margin", PropertyId::Shorthand(ShorthandId::Margin)),
    ("margin-block", PropertyId::Shorthand(ShorthandId::MarginBlock)),
    ("margin-block-end", PropertyId::Longhand(LonghandId::MarginBlockEnd)),
    ("margin-block-start", PropertyId::Longhand(LonghandId::MarginBlockStart)),
    ("margin-bottom", PropertyId::Longhand(LonghandId::MarginBottom)),
    ("margin-inline", PropertyId::Shorthand(ShorthandId::MarginInline)),
    ("margin-inline-end", PropertyId::Longhand(LonghandId::MarginInlineEnd)),
    ("margin-inline-start", PropertyId::Longhand(LonghandId::MarginInlineStart)),
    ("margin-left", PropertyId::Longhand(LonghandId::MarginLeft)),
    ("margin-right", PropertyId::Longhand(LonghandId::MarginRight)),
    ("margin-top", PropertyId::Longhand(LonghandId::MarginTop)),
    ("margin-trim", PropertyId::Longhand(LonghandId::MarginTrim)),
    ("marker", PropertyId::Longhand(LonghandId::Marker)),
    ("marker-end", PropertyId::Longhand(LonghandId::MarkerEnd)),
    ("marker-mid", PropertyId::Longhand(LonghandId::MarkerMid)),
    ("marker-start", PropertyId::Longhand(LonghandId::MarkerStart)),
    ("mask", PropertyId::Shorthand(ShorthandId::Mask)),
    ("mask-border", PropertyId::Shorthand(ShorthandId::MaskBorder)),
    ("mask-border-mode", PropertyId::Longhand(LonghandId::MaskBorderMode)),
    ("mask-border-outset", PropertyId::Longhand(LonghandId::MaskBorderOutset)),
    ("mask-border-repeat", PropertyId::Longhand(LonghandId::MaskBorderRepeat)),
    ("mask-border-slice", PropertyId::Longhand(LonghandId::MaskBorderSlice)),
    ("mask-border-source", PropertyId::Longhand(LonghandId::MaskBorderSource)),
    ("mask-border-width", PropertyId::Longhand(LonghandId::MaskBorderWidth)),
    ("mask-clip", PropertyId::Longhand(LonghandId::MaskClip)),
    ("mask-composite", PropertyId::Longhand(LonghandId::MaskComposite)),
    ("mask-image", PropertyId::Longhand(LonghandId::MaskImage)),
    ("mask-mode", PropertyId::Longhand(LonghandId::MaskMode)),
    ("mask-origin", PropertyId::Longhand(LonghandId::MaskOrigin)),
    ("mask-position", PropertyId::Longhand(LonghandId::MaskPosition)),
    ("mask-repeat", PropertyId::Longhand(LonghandId::MaskRepeat)),
    ("mask-size", PropertyId::Longhand(LonghandId::MaskSize)),
    ("mask-type", PropertyId::Longhand(LonghandId::MaskType)),
    ("masonry-auto-flow", PropertyId::Longhand(LonghandId::MasonryAutoFlow)),
    ("math-depth", PropertyId::Longhand(LonghandId::MathDepth)),
    ("math-shift", PropertyId::Longhand(LonghandId::MathShift)),
    ("math-style", PropertyId::Longhand(LonghandId::MathStyle)),
    ("max-block-size", PropertyId::Longhand(LonghandId::MaxBlockSize)),
    ("max-height", PropertyId::Longhand(LonghandId::MaxHeight)),
    ("max-inline-size", PropertyId::Longhand(LonghandId::MaxInlineSize)),
    ("max-lines", PropertyId::Longhand(LonghandId::MaxLines)),
    ("max-width", PropertyId::Longhand(LonghandId::MaxWidth)),
    ("min-block-size", PropertyId::Longhand(LonghandId::MinBlockSize)),
    ("min-height", PropertyId::Longhand(LonghandId::MinHeight)),
    ("min-inline-size", PropertyId::Longhand(LonghandId::MinInlineSize)),
    ("min-width", PropertyId::Longhand(LonghandId::MinWidth)),
    ("mix-blend-mode", PropertyId::Longhand(LonghandId::MixBlendMode)),
    ("object-fit", PropertyId::Longhand(LonghandId::ObjectFit)),
    ("object-position", PropertyId::Longhand(LonghandId::ObjectPosition)),
    ("object-view-box", PropertyId::Longhand(LonghandId::ObjectViewBox)),
    ("offset", PropertyId::Shorthand(ShorthandId::Offset)),
    ("offset-anchor", PropertyId::Longhand(LonghandId::OffsetAnchor)),
    ("offset-distance", PropertyId::Longhand(LonghandId::OffsetDistance)),
    ("offset-path", PropertyId::Longhand(LonghandId::OffsetPath)),
    ("offset-position", PropertyId::Longhand(LonghandId::OffsetPosition)),
    ("offset-rotate", PropertyId::Longhand(LonghandId::OffsetRotate)),
    ("opacity", PropertyId::Longhand(LonghandId::Opacity)),
    ("order", PropertyId::Longhand(LonghandId::Order)),
    ("orphans", PropertyId::Longhand(LonghandId::Orphans)),
    ("outline", PropertyId::Shorthand(ShorthandId::Outline)),
    ("outline-color", PropertyId::Longhand(LonghandId::OutlineColor)),
    ("outline-offset", PropertyId::Longhand(LonghandId::OutlineOffset)),
    ("outline-style", PropertyId::Longhand(LonghandId::OutlineStyle)),
    ("outline-width", PropertyId::Longhand(LonghandId::OutlineWidth)),
    ("overflow", PropertyId::Shorthand(ShorthandId::Overflow)),
    ("overflow-anchor", PropertyId::Longhand(LonghandId::OverflowAnchor)),
    ("overflow-block", PropertyId::Longhand(LonghandId::OverflowBlock)),
    ("overflow-clip-box", PropertyId::Longhand(LonghandId::OverflowClipBox)),
    ("overflow-clip-margin", PropertyId::Longhand(LonghandId::OverflowClipMargin)),
    ("overflow-inline", PropertyId::Longhand(LonghandId::OverflowInline)),
    ("overflow-wrap", PropertyId::Longhand(LonghandId::OverflowWrap)),
    ("overflow-x", PropertyId::Longhand(LonghandId::OverflowX)),
    ("overflow-y", PropertyId::Longhand(LonghandId::OverflowY)),
    ("overlay", PropertyId::Longhand(LonghandId::Overlay)),
    ("overscroll-behavior", PropertyId::Shorthand(ShorthandId::OverscrollBehavior)),
    ("overscroll-behavior-block", PropertyId::Longhand(LonghandId::OverscrollBehaviorBlock)),
    ("overscroll-behavior-inline", PropertyId::Longhand(LonghandId::OverscrollBehaviorInline)),
    ("overscroll-behavior-x", PropertyId::Longhand(LonghandId::OverscrollBehaviorX)),
    ("overscroll-behavior-y", PropertyId::Longhand(LonghandId::OverscrollBehaviorY)),
    ("padding", PropertyId::Shorthand(ShorthandId::Padding)),
    ("padding-block", PropertyId::Shorthand(ShorthandId::PaddingBlock)),
    ("padding-block-end", PropertyId::Longhand(LonghandId::PaddingBlockEnd)),
    ("padding-block-start", PropertyId::Longhand(LonghandId::PaddingBlockStart)),
    ("padding-bottom", PropertyId::Longhand(LonghandId::PaddingBottom)),
    ("padding-inline", PropertyId::Shorthand(ShorthandId::PaddingInline)),
    ("padding-inline-end", PropertyId::Longhand(LonghandId::PaddingInlineEnd)),
    ("padding-inline-start", PropertyId::Longhand(LonghandId::PaddingInlineStart)),
    ("padding-left", PropertyId::Longhand(LonghandId::PaddingLeft)),
    ("padding-right", PropertyId::Longhand(LonghandId::PaddingRight)),
    ("padding-top", PropertyId::Longhand(LonghandId::PaddingTop)),
    ("page", PropertyId::Longhand(LonghandId::Page)),
    ("page-break-after", PropertyId::Longhand(LonghandId::PageBreakAfter)),
    ("page-break-before", PropertyId::Longhand(LonghandId::PageBreakBefore)),
    ("page-break-inside", PropertyId::Longhand(LonghandId::PageBreakInside)),
    ("paint-order", PropertyId::Longhand(LonghandId::PaintOrder)),
    ("perspective", PropertyId::Longhand(LonghandId::Perspective)),
    ("perspective-origin", PropertyId::Longhand(LonghandId::PerspectiveOrigin)),
    ("place-content", PropertyId::Shorthand(ShorthandId::PlaceContent)),
    ("place-items", PropertyId::Shorthand(ShorthandId::PlaceItems)),
    ("place-self", PropertyId::Shorthand(ShorthandId::PlaceSelf)),
    ("pointer-events", PropertyId::Longhand(LonghandId::PointerEvents)),
    ("position", PropertyId::Longhand(LonghandId::Position)),
    ("position-anchor", PropertyId::Longhand(LonghandId::PositionAnchor)),
    ("position-area", PropertyId::Longhand(LonghandId::PositionArea)),
    ("position-try", PropertyId::Shorthand(ShorthandId::PositionTry)),
    ("position-try-fallbacks", PropertyId::Longhand(LonghandId::PositionTryFallbacks)),
    ("position-try-order", PropertyId::Longhand(LonghandId::PositionTryOrder)),
    ("position-visibility", PropertyId::Longhand(LonghandId::PositionVisibility)),
    ("print-color-adjust", PropertyId::Longhand(LonghandId::PrintColorAdjust)),
    ("quotes", PropertyId::Longhand(LonghandId::Quotes)),
    ("r", PropertyId::Longhand(LonghandId::R)),
    ("reading-flow", PropertyId::Longhand(LonghandId::ReadingFlow)),
    ("reading-order", PropertyId::Longhand(LonghandId::ReadingOrder)),
    ("resize", PropertyId::Longhand(LonghandId::Resize)),
    ("right", PropertyId::Longhand(LonghandId::Right)),
    ("rotate", PropertyId::Longhand(LonghandId::Rotate)),
    ("row-gap", PropertyId::Longhand(LonghandId::RowGap)),
    ("ruby-align", PropertyId::Longhand(LonghandId::RubyAlign)),
    ("ruby-merge", PropertyId::Longhand(LonghandId::RubyMerge)),
    ("ruby-overhang", PropertyId::Longhand(LonghandId::RubyOverhang)),
    ("ruby-position", PropertyId::Longhand(LonghandId::RubyPosition)),
    ("rx", PropertyId::Longhand(LonghandId::Rx)),
    ("ry", PropertyId::Longhand(LonghandId::Ry)),
    ("scale", PropertyId::Longhand(LonghandId::Scale)),
    ("scroll-behavior", PropertyId::Longhand(LonghandId::ScrollBehavior)),
    ("scroll-initial-target", PropertyId::Longhand(LonghandId::ScrollInitialTarget)),
    ("scroll-margin", PropertyId::Shorthand(ShorthandId::ScrollMargin)),
    ("scroll-margin-block", PropertyId::Shorthand(ShorthandId::ScrollMarginBlock)),
    ("scroll-margin-block-end", PropertyId::Longhand(LonghandId::ScrollMarginBlockEnd)),
    ("scroll-margin-block-start", PropertyId::Longhand(LonghandId::ScrollMarginBlockStart)),
    ("scroll-margin-bottom", PropertyId::Longhand(LonghandId::ScrollMarginBottom)),
    ("scroll-margin-inline", PropertyId::Shorthand(ShorthandId::ScrollMarginInline)),
    ("scroll-margin-inline-end", PropertyId::Longhand(LonghandId::ScrollMarginInlineEnd)),
    ("scroll-margin-inline-start", PropertyId::Longhand(LonghandId::ScrollMarginInlineStart)),
    ("scroll-margin-left", PropertyId::Longhand(LonghandId::ScrollMarginLeft)),
    ("scroll-margin-right", PropertyId::Longhand(LonghandId::ScrollMarginRight)),
    ("scroll-margin-top", PropertyId::Longhand(LonghandId::ScrollMarginTop)),
    ("scroll-marker-group", PropertyId::Longhand(LonghandId::ScrollMarkerGroup)),
    ("scroll-padding", PropertyId::Shorthand(ShorthandId::ScrollPadding)),
    ("scroll-padding-block", PropertyId::Shorthand(ShorthandId::ScrollPaddingBlock)),
    ("scroll-padding-block-end", PropertyId::Longhand(LonghandId::ScrollPaddingBlockEnd)),
    ("scroll-padding-block-start", PropertyId::Longhand(LonghandId::ScrollPaddingBlockStart)),
    ("scroll-padding-bottom", PropertyId::Longhand(LonghandId::ScrollPaddingBottom)),
    ("scroll-padding-inline", PropertyId::Shorthand(ShorthandId::ScrollPaddingInline)),
    ("scroll-padding-inline-end", PropertyId::Longhand(LonghandId::ScrollPaddingInlineEnd)),
    ("scroll-padding-inline-start", PropertyId::Longhand(LonghandId::ScrollPaddingInlineStart)),
    ("scroll-padding-left", PropertyId::Longhand(LonghandId::ScrollPaddingLeft)),
    ("scroll-padding-right", PropertyId::Longhand(LonghandId::ScrollPaddingRight)),
    ("scroll-padding-top", PropertyId::Longhand(LonghandId::ScrollPaddingTop)),
    ("scroll-snap-align", PropertyId::Longhand(LonghandId::ScrollSnapAlign)),
    ("scroll-snap-coordinate", PropertyId::Longhand(LonghandId::ScrollSnapCoordinate)),
    ("scroll-snap-destination", PropertyId::Longhand(LonghandId::ScrollSnapDestination)),
    ("scroll-snap-points-x", PropertyId::Longhand(LonghandId::ScrollSnapPointsX)),
    ("scroll-snap-points-y", PropertyId::Longhand(LonghandId::ScrollSnapPointsY)),
    ("scroll-snap-stop", PropertyId::Longhand(LonghandId::ScrollSnapStop)),
    ("scroll-snap-type", PropertyId::Longhand(LonghandId::ScrollSnapType)),
    ("scroll-snap-type-x", PropertyId::Longhand(LonghandId::ScrollSnapTypeX)),
    ("scroll-snap-type-y", PropertyId::Longhand(LonghandId::ScrollSnapTypeY)),
    ("scroll-target-group", PropertyId::Longhand(LonghandId::ScrollTargetGroup)),
    ("scroll-timeline", PropertyId::Shorthand(ShorthandId::ScrollTimeline)),
    ("scroll-timeline-axis", PropertyId::Longhand(LonghandId::ScrollTimelineAxis)),
    ("scroll-timeline-name", PropertyId::Longhand(LonghandId::ScrollTimelineName)),
    ("scrollbar-color", PropertyId::Longhand(LonghandId::ScrollbarColor)),
    ("scrollbar-gutter", PropertyId::Longhand(LonghandId::ScrollbarGutter)),
    ("scrollbar-width", PropertyId::Longhand(LonghandId::ScrollbarWidth)),
    ("shape-image-threshold", PropertyId::Longhand(LonghandId::ShapeImageThreshold)),
    ("shape-margin", PropertyId::Longhand(LonghandId::ShapeMargin)),
    ("shape-outside", PropertyId::Longhand(LonghandId::ShapeOutside)),
    ("shape-rendering", PropertyId::Longhand(LonghandId::ShapeRendering)),
    ("speak-as", PropertyId::Longhand(LonghandId::SpeakAs)),
    ("stop-color", PropertyId::Longhand(LonghandId::StopColor)),
    ("stop-opacity", PropertyId::Longhand(LonghandId::StopOpacity)),
    ("stroke", PropertyId::Longhand(LonghandId::Stroke)),
    ("stroke-color", PropertyId::Longhand(LonghandId::StrokeColor)),
    ("stroke-dasharray", PropertyId::Longhand(LonghandId::StrokeDasharray)),
    ("stroke-dashoffset", PropertyId::Longhand(LonghandId::StrokeDashoffset)),
    ("stroke-linecap", PropertyId::Longhand(LonghandId::StrokeLinecap)),
    ("stroke-linejoin", PropertyId::Longhand(LonghandId::StrokeLinejoin)),
    ("stroke-miterlimit", PropertyId::Longhand(LonghandId::StrokeMiterlimit)),
    ("stroke-opacity", PropertyId::Longhand(LonghandId::StrokeOpacity)),
    ("stroke-width", PropertyId::Longhand(LonghandId::StrokeWidth)),
    ("tab-size", PropertyId::Longhand(LonghandId::TabSize)),
    ("table-layout", PropertyId::Longhand(LonghandId::TableLayout)),
    ("text-align", PropertyId::Longhand(LonghandId::TextAlign)),
    ("text-align-last", PropertyId::Longhand(LonghandId::TextAlignLast)),
    ("text-anchor", PropertyId::Longhand(LonghandId::TextAnchor)),
    ("text-autospace", PropertyId::Longhand(LonghandId::TextAutospace)),
    ("text-box", PropertyId::Longhand(LonghandId::TextBox)),
    ("text-box-edge", PropertyId::Longhand(LonghandId::TextBoxEdge)),
    ("text-box-trim", PropertyId::Longhand(LonghandId::TextBoxTrim)),
    ("text-combine-upright", PropertyId::Longhand(LonghandId::TextCombineUpright)),
    ("text-decoration", PropertyId::Shorthand(ShorthandId::TextDecoration)),
    ("text-decoration-color", PropertyId::Longhand(LonghandId::TextDecorationColor)),
    ("text-decoration-inset", PropertyId::Longhand(LonghandId::TextDecorationInset)),
    ("text-decoration-line", PropertyId::Longhand(LonghandId::TextDecorationLine)),
    ("text-decoration-skip", PropertyId::Longhand(LonghandId::TextDecorationSkip)),
    ("text-decoration-skip-ink", PropertyId::Longhand(LonghandId::TextDecorationSkipInk)),
    ("text-decoration-style", PropertyId::Longhand(LonghandId::TextDecorationStyle)),
    ("text-decoration-thickness", PropertyId::Longhand(LonghandId::TextDecorationThickness)),
    ("text-emphasis", PropertyId::Shorthand(ShorthandId::TextEmphasis)),
    ("text-emphasis-color", PropertyId::Longhand(LonghandId::TextEmphasisColor)),
    ("text-emphasis-position", PropertyId::Longhand(LonghandId::TextEmphasisPosition)),
    ("text-emphasis-style", PropertyId::Longhand(LonghandId::TextEmphasisStyle)),
    ("text-indent", PropertyId::Longhand(LonghandId::TextIndent)),
    ("text-justify", PropertyId::Longhand(LonghandId::TextJustify)),
    ("text-orientation", PropertyId::Longhand(LonghandId::TextOrientation)),
    ("text-overflow", PropertyId::Longhand(LonghandId::TextOverflow)),
    ("text-rendering", PropertyId::Longhand(LonghandId::TextRendering)),
    ("text-shadow", PropertyId::Longhand(LonghandId::TextShadow)),
    ("text-size-adjust", PropertyId::Longhand(LonghandId::TextSizeAdjust)),
    ("text-spacing-trim", PropertyId::Longhand(LonghandId::TextSpacingTrim)),
    ("text-transform", PropertyId::Longhand(LonghandId::TextTransform)),
    ("text-underline-offset", PropertyId::Longhand(LonghandId::TextUnderlineOffset)),
    ("text-underline-position", PropertyId::Longhand(LonghandId::TextUnderlinePosition)),
    ("text-wrap", PropertyId::Shorthand(ShorthandId::TextWrap)),
    ("text-wrap-mode", PropertyId::Longhand(LonghandId::TextWrapMode)),
    ("text-wrap-style", PropertyId::Longhand(LonghandId::TextWrapStyle)),
    ("timeline-scope", PropertyId::Longhand(LonghandId::TimelineScope)),
    ("timeline-trigger", PropertyId::Shorthand(ShorthandId::TimelineTrigger)),
    ("timeline-trigger-activation-range", PropertyId::Shorthand(ShorthandId::TimelineTriggerActivationRange)),
    ("timeline-trigger-activation-range-end", PropertyId::Longhand(LonghandId::TimelineTriggerActivationRangeEnd)),
    ("timeline-trigger-activation-range-start", PropertyId::Longhand(LonghandId::TimelineTriggerActivationRangeStart)),
    ("timeline-trigger-active-range", PropertyId::Shorthand(ShorthandId::TimelineTriggerActiveRange)),
    ("timeline-trigger-active-range-end", PropertyId::Longhand(LonghandId::TimelineTriggerActiveRangeEnd)),
    ("timeline-trigger-active-range-start", PropertyId::Longhand(LonghandId::TimelineTriggerActiveRangeStart)),
    ("timeline-trigger-name", PropertyId::Longhand(LonghandId::TimelineTriggerName)),
    ("timeline-trigger-source", PropertyId::Longhand(LonghandId::TimelineTriggerSource)),
    ("top", PropertyId::Longhand(LonghandId::Top)),
    ("touch-action", PropertyId::Longhand(LonghandId::TouchAction)),
    ("transform", PropertyId::Longhand(LonghandId::Transform)),
    ("transform-box", PropertyId::Longhand(LonghandId::TransformBox)),
    ("transform-origin", PropertyId::Longhand(LonghandId::TransformOrigin)),
    ("transform-style", PropertyId::Longhand(LonghandId::TransformStyle)),
    ("transition", PropertyId::Shorthand(ShorthandId::Transition)),
    ("transition-behavior", PropertyId::Longhand(LonghandId::TransitionBehavior)),
    ("transition-delay", PropertyId::Longhand(LonghandId::TransitionDelay)),
    ("transition-duration", PropertyId::Longhand(LonghandId::TransitionDuration)),
    ("transition-property", PropertyId::Longhand(LonghandId::TransitionProperty)),
    ("transition-timing-function", PropertyId::Longhand(LonghandId::TransitionTimingFunction)),
    ("translate", PropertyId::Longhand(LonghandId::Translate)),
    ("trigger-scope", PropertyId::Longhand(LonghandId::TriggerScope)),
    ("unicode-bidi", PropertyId::Longhand(LonghandId::UnicodeBidi)),
    ("user-select", PropertyId::Longhand(LonghandId::UserSelect)),
    ("vector-effect", PropertyId::Longhand(LonghandId::VectorEffect)),
    ("vertical-align", PropertyId::Longhand(LonghandId::VerticalAlign)),
    ("view-timeline", PropertyId::Shorthand(ShorthandId::ViewTimeline)),
    ("view-timeline-axis", PropertyId::Longhand(LonghandId::ViewTimelineAxis)),
    ("view-timeline-inset", PropertyId::Longhand(LonghandId::ViewTimelineInset)),
    ("view-timeline-name", PropertyId::Longhand(LonghandId::ViewTimelineName)),
    ("view-transition-class", PropertyId::Longhand(LonghandId::ViewTransitionClass)),
    ("view-transition-name", PropertyId::Longhand(LonghandId::ViewTransitionName)),
    ("view-transition-scope", PropertyId::Longhand(LonghandId::ViewTransitionScope)),
    ("visibility", PropertyId::Longhand(LonghandId::Visibility)),
    ("white-space", PropertyId::Longhand(LonghandId::WhiteSpace)),
    ("white-space-collapse", PropertyId::Longhand(LonghandId::WhiteSpaceCollapse)),
    ("widows", PropertyId::Longhand(LonghandId::Widows)),
    ("width", PropertyId::Longhand(LonghandId::Width)),
    ("will-change", PropertyId::Longhand(LonghandId::WillChange)),
    ("word-break", PropertyId::Longhand(LonghandId::WordBreak)),
    ("word-spacing", PropertyId::Longhand(LonghandId::WordSpacing)),
    ("word-wrap", PropertyId::Longhand(LonghandId::WordWrap)),
    ("writing-mode", PropertyId::Longhand(LonghandId::WritingMode)),
    ("x", PropertyId::Longhand(LonghandId::X)),
    ("y", PropertyId::Longhand(LonghandId::Y)),
    ("z-index", PropertyId::Longhand(LonghandId::ZIndex)),
    ("zoom", PropertyId::Longhand(LonghandId::Zoom)),
];

impl LonghandId {
    /// The CSS name of this property.
    #[must_use]
    pub fn name(self) -> &'static str {
        LONGHANDS[self as usize].name
    }

    /// Whether the property inherits by default.
    #[must_use]
    pub fn inherited(self) -> bool {
        LONGHANDS[self as usize].inherited
    }

    /// The initial value as the definition data writes it, before it is parsed. `None` where the
    /// data gives a list rather than a value.
    #[must_use]
    pub fn initial_source(self) -> Option<&'static str> {
        LONGHANDS[self as usize].initial
    }

    /// Whether a percentage specified for this property computes to a plain number.
    #[must_use]
    pub fn percentage_is_number(self) -> bool {
        LONGHANDS[self as usize].percentage_is_number
    }

    /// This property's slot, which is also its position in [`ALL_LONGHAND_IDS`].
    #[must_use]
    pub fn index(self) -> usize {
        self as usize
    }

    /// The longhand at `index`, or `None` when there is none.
    #[must_use]
    pub fn from_index(index: usize) -> Option<Self> {
        ALL_LONGHAND_IDS.get(index).copied()
    }
}

impl ShorthandId {
    /// The CSS name of this property.
    #[must_use]
    pub fn name(self) -> &'static str {
        SHORTHANDS[self as usize].name
    }

    /// Whether the property inherits by default.
    #[must_use]
    pub fn inherited(self) -> bool {
        SHORTHANDS[self as usize].inherited
    }

    /// The initial value as the definition data writes it, before it is parsed. A shorthand's is
    /// usually `None`: the data gives its longhand list there instead of a value.
    #[must_use]
    pub fn initial_source(self) -> Option<&'static str> {
        SHORTHANDS[self as usize].initial
    }

    /// Whether a percentage specified for this property computes to a plain number.
    #[must_use]
    pub fn percentage_is_number(self) -> bool {
        SHORTHANDS[self as usize].percentage_is_number
    }

    /// The properties this shorthand sets. A few of them are shorthands themselves.
    #[must_use]
    pub fn longhands(self) -> &'static [PropertyId] {
        SHORTHANDS[self as usize].longhands
    }

    /// This property's position in [`ALL_SHORTHAND_IDS`]. Note that a [`PropertyId::index`] puts
    /// the shorthands after the longhands, so the two are not the same number.
    #[must_use]
    pub fn index(self) -> usize {
        self as usize
    }

    /// The shorthand at `index`, or `None` when there is none.
    #[must_use]
    pub fn from_index(index: usize) -> Option<Self> {
        ALL_SHORTHAND_IDS.get(index).copied()
    }
}

impl PropertyId {
    /// A slot number below [`PROPERTY_COUNT`], unique across longhands and shorthands both. The
    /// longhands come first, so an array of [`LONGHAND_COUNT`] entries can be indexed by a
    /// [`LonghandId`] and one of [`PROPERTY_COUNT`] entries by any property.
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            PropertyId::Longhand(id) => id as usize,
            PropertyId::Shorthand(id) => LONGHAND_COUNT + id as usize,
        }
    }

    /// The property whose [`PropertyId::index`] is `index`, or `None` when there is none.
    #[must_use]
    pub fn from_index(index: usize) -> Option<Self> {
        if index < LONGHAND_COUNT {
            return LonghandId::from_index(index).map(PropertyId::Longhand);
        }
        ShorthandId::from_index(index - LONGHAND_COUNT).map(PropertyId::Shorthand)
    }

    /// The CSS name of this property.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            PropertyId::Longhand(id) => id.name(),
            PropertyId::Shorthand(id) => id.name(),
        }
    }

    /// Whether the property inherits by default.
    #[must_use]
    pub fn inherited(self) -> bool {
        match self {
            PropertyId::Longhand(id) => id.inherited(),
            PropertyId::Shorthand(id) => id.inherited(),
        }
    }

    /// The initial value as the definition data writes it, before it is parsed.
    #[must_use]
    pub fn initial_source(self) -> Option<&'static str> {
        match self {
            PropertyId::Longhand(id) => id.initial_source(),
            PropertyId::Shorthand(id) => id.initial_source(),
        }
    }

    /// Whether a percentage specified for this property computes to a plain number.
    #[must_use]
    pub fn percentage_is_number(self) -> bool {
        match self {
            PropertyId::Longhand(id) => id.percentage_is_number(),
            PropertyId::Shorthand(id) => id.percentage_is_number(),
        }
    }

    /// Whether this property distributes its value over others.
    #[must_use]
    pub fn is_shorthand(self) -> bool {
        matches!(self, PropertyId::Shorthand(_))
    }

    /// The properties a shorthand sets, empty for a longhand.
    #[must_use]
    pub fn longhands(self) -> &'static [PropertyId] {
        match self {
            PropertyId::Longhand(_) => &[],
            PropertyId::Shorthand(id) => id.longhands(),
        }
    }

    /// The property `name` denotes, or `None` when this engine has no definition for it.
    ///
    /// Property names are ASCII case-insensitive (css-syntax-3 §3.3), so `COLOR` is `color`. The
    /// table is all lowercase and the common case is a name that is too, so the exact search runs
    /// first and only a name carrying an uppercase byte costs a second one.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        if let Ok(found) = BY_NAME.binary_search_by(|(known, _)| (*known).cmp(name)) {
            return Some(BY_NAME[found].1);
        }
        if !name.bytes().any(|byte| byte.is_ascii_uppercase()) {
            return None;
        }
        let found = BY_NAME
            .binary_search_by(|(known, _)| compare_ascii_lowercase(known, name))
            .ok()?;
        Some(BY_NAME[found].1)
    }
}

/// Compare `known`, which is already lowercase, against `name` lowercased as it is read. Keeps
/// the case-insensitive search allocation-free and consistent with the table's ordering.
fn compare_ascii_lowercase(known: &str, name: &str) -> core::cmp::Ordering {
    let mut lowered = name.bytes().map(|byte| byte.to_ascii_lowercase());
    for byte in known.bytes() {
        match lowered.next() {
            None => return core::cmp::Ordering::Greater,
            Some(other) => match byte.cmp(&other) {
                core::cmp::Ordering::Equal => {}
                other => return other,
            },
        }
    }
    match lowered.next() {
        None => core::cmp::Ordering::Equal,
        Some(_) => core::cmp::Ordering::Less,
    }
}
