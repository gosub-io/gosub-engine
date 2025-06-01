use std::collections::HashMap;

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
    // MarginBlockStart,
    // MarginBlockEnd,
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
}

#[allow(unused)]
#[derive(Clone, Debug, PartialEq)]
pub enum Unit {
    Px,
    Em,
    Rem,
    Percent,
}

#[allow(unused)]
#[derive(Clone, Debug, PartialEq)]
pub enum Color {
    Rgb(u8, u8, u8),
    Rgba(u8, u8, u8, f32),
    Named(String),
}

#[allow(unused)]
#[derive(Clone, Debug, PartialEq)]
pub enum Display {
    Block,
    Inline,
    InlineBlock,
    None,
    Flex,
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

#[allow(unused)]
#[derive(Debug, Clone, PartialEq)]
pub enum StyleValue {
    Keyword(String),
    Unit(f32, Unit),
    Number(f32),
    Percentage(f32),
    Color(Color),
    None,
    Display(Display),
    FontWeight(FontWeight),
    TextWrap(TextWrap),
    TextAlign(TextAlign),
}

#[derive(Debug, Clone)]
pub struct StylePropertyList {
    pub properties: HashMap<StyleProperty, StyleValue>,
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
    Unset
}

impl StylePropertyList {
    pub fn new() -> Self {
        Self {
            properties: HashMap::new(),
        }
    }

    pub fn set_property(&mut self, prop: StyleProperty, value: StyleValue) {
        self.properties.insert(prop, value.clone());
    }

    pub fn get_property(&self, prop: StyleProperty) -> Option<&StyleValue> {
        self.properties.get(&prop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_get_property() {
        let mut style = StylePropertyList::new();

        let val = StyleValue::Color(Color::Named("red".to_string()));
        style.set_property(StyleProperty::Color, val.clone());

        assert_eq!(style.get_property(StyleProperty::Color), Some(&val.clone()));
    }
}