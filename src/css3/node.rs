// Note: every node should have a "loc" property

use std::fmt::{self, Debug, Formatter};

/// Used for the [An+B microsyntax](https://drafts.csswg.org/css-syntax/#anb-microsyntax).
#[derive(Debug, PartialEq)]
pub struct AnPlusB {
    a: Option<String>,
    b: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum AtRulePreludeValue {
    AtRulePrelude(AtRulePrelude),
    Raw(Raw),
    None,
}

/// CSS [At Rule](https://drafts.csswg.org/css-conditional-3/)
/// E.g. @import @media @keyframes @supports
#[derive(Debug, PartialEq)]
pub struct AtRule {
    name: String,
    prelude: AtRulePreludeValue,
    block: Option<Block>,
}

#[derive(Debug, PartialEq)]
pub enum AtRulePreludeChild {
    MediaQueryList(MediaQueryList),
}

#[derive(Debug, PartialEq, Default)]
pub struct AtRulePrelude {
    children: Vec<AtRulePrelude>,
}

#[derive(Debug, PartialEq)]
pub enum AttributeSelectorValue {
    String(CssString),
    Identifier(IdSelector),
    None,
}

#[derive(PartialEq)]
pub enum AttributeMatcher {
    IncludeMatch,
    DashMatch,
    PrefixMatch,
    SuffixMatch,
    SubstringMatch,
    EqualityMatch,
}

impl Debug for AttributeMatcher {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let matcher = match self {
            AttributeMatcher::IncludeMatch => "~=",
            AttributeMatcher::DashMatch => "|=",
            AttributeMatcher::PrefixMatch => "^=",
            AttributeMatcher::SuffixMatch => "$=",
            AttributeMatcher::SubstringMatch => "*=",
            AttributeMatcher::EqualityMatch => "=",
        };

        write!(f, "{matcher}")
    }
}

/// [Attribute Selector](https://drafts.csswg.org/selectors/#attribute-selectors)
#[derive(Debug, PartialEq)]
pub struct AttributeSelector {
    pub name: Identifier,
    pub matcher: Option<AttributeMatcher>,
    pub value: Option<CssString>,
    pub flag: Option<Identifier>,
}

/// [Id Selector](https://drafts.csswg.org/selectors/#id-selectors)
#[derive(PartialEq, Default)]
pub struct IdSelector {
    name: String,
}

impl Debug for IdSelector {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.name)
    }
}

impl IdSelector {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

/// [Class Selector](https://drafts.csswg.org/selectors/#class-html)
#[derive(Debug, PartialEq, Default)]
pub struct ClassSelector {
    name: String,
}

impl ClassSelector {
    #[must_use]
    pub fn new(name: String) -> Self {
        Self { name }
    }
}

/// [TypeSelector](https://drafts.csswg.org/selectors/#type-selectors)
#[derive(PartialEq)]
pub struct TypeSelector {
    name: String,
}

impl Debug for TypeSelector {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl TypeSelector {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

/// [Nesting Selector](https://drafts.csswg.org/css-nesting/#nest-selector)
#[derive(Debug, PartialEq)]
pub struct NestingSelector;

#[derive(Debug, PartialEq)]
pub enum BlockChild {
    Rule(Rule),
    AtRule(AtRule),
    DeclarationList(DeclarationList),
}

#[derive(Debug, PartialEq, Default)]
pub struct Block {
    children: Vec<BlockChild>,
}

impl Block {
    #[must_use]
    pub fn new(children: Vec<BlockChild>) -> Self {
        Self { children }
    }

    pub fn add_child(&mut self, child: BlockChild) {
        self.children.push(child);
    }
}

#[derive(PartialEq, Default)]
pub struct Identifier {
    name: String,
}

impl Debug for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl Identifier {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[derive(Debug, PartialEq)]
pub struct CDC;
#[derive(Debug, PartialEq)]
pub struct CDO;

#[derive(PartialEq)]
pub enum Combinator {
    ChildCombinator,
    ColumnCombinator,
    SelectorListCombinator,
    DescendantCombinator,
    NamespaceSeparator,
    NextSiblingCombinator,
    SubsequentSiblingCombinator,
}

impl Debug for Combinator {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let combinator = match self {
            Combinator::ChildCombinator => ">",
            Combinator::ColumnCombinator => "||",
            Combinator::SelectorListCombinator => ",",
            Combinator::DescendantCombinator => "' '",
            Combinator::NamespaceSeparator => "|",
            Combinator::NextSiblingCombinator => "+",
            Combinator::SubsequentSiblingCombinator => "~",
        };

        write!(f, "{combinator}")
    }
}

#[derive(Debug, PartialEq, Default)]
pub struct Declaration {
    pub important: bool,
    pub property: String,
    pub value: ValueList,
}

impl Declaration {
    #[must_use]
    pub fn new(property: impl Into<String>, value: ValueList) -> Self {
        Self {
            important: false,
            property: property.into(),
            value,
        }
    }

    pub fn set_important_as(&mut self, important: bool) {
        self.important = important;
    }

    pub fn set_property(&mut self, property: impl Into<String>) {
        self.property = property.into();
    }

    pub fn set_value(&mut self, value: ValueList) {
        self.value = value;
    }
}

#[derive(Debug, PartialEq, Default)]
pub struct DeclarationList {
    children: Vec<Declaration>,
}

impl DeclarationList {
    #[must_use]
    pub fn new(children: Vec<Declaration>) -> Self {
        Self { children }
    }

    pub fn add_child(&mut self, child: Declaration) {
        self.children.push(child);
    }
}

#[derive(PartialEq, Default)]
pub struct Dimension {
    value: String,
    unit: Option<String>,
}

impl Debug for Dimension {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.value, self.unit.clone().unwrap_or_default())
    }
}

impl Dimension {
    #[must_use]
    pub fn new(value: impl Into<String>, unit: Option<impl Into<String>>) -> Self {
        Self {
            value: value.into(),
            unit: unit.map(Into::into),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum MediaFeatureValue {
    Identifier(Identifier),
    Number(CssNumber),
    Dimension(Dimension),
    Ratio(Ratio),
    Function(Function),
}

#[derive(Debug, PartialEq)]
pub struct MediaFeature {
    name: String,
    value: Option<MediaFeatureValue>,
}

#[derive(Debug, PartialEq)]
pub enum FunctionChild {
    Identifier(Identifier),
    Operator(Operator),
    Percentage(Percentage),
}

#[derive(Debug, PartialEq)]
pub struct Function {
    name: String,
    children: Vec<FunctionChild>,
}

#[derive(Debug, PartialEq)]
pub struct Hash {
    value: String,
}

#[derive(Debug, PartialEq)]
pub struct Layer {
    name: String,
}

#[derive(Debug, PartialEq)]
pub struct LayerList {
    children: Vec<Layer>,
}

#[derive(Debug, PartialEq)]
pub enum MediaQueryChild {
    Identifier(Identifier),
    MediaFeature(MediaFeature),
}

#[derive(Debug, PartialEq)]
pub struct MediaQuery {
    children: Vec<MediaQueryChild>,
}

#[derive(Debug, PartialEq)]
pub struct MediaQueryList {
    children: Vec<MediaQuery>,
}

#[derive(Debug, PartialEq)]
pub enum NthValue {
    AnPlusB(AnPlusB),
    Identifier(Identifier),
}
#[derive(Debug, PartialEq)]
pub struct Nth {
    nth: NthValue,
    selector: Option<SelectorList>,
}

#[derive(Debug, PartialEq)]
pub struct CssNumber {
    value: String,
}

#[derive(Default, Debug, PartialEq)]
pub struct CssString {
    value: String,
}

impl CssString {
    #[must_use]
    pub fn new<S: Into<String>>(value: S) -> Self {
        Self {
            value: value.into(),
        }
    }
}

// todo: should be "enum"
#[derive(Debug, PartialEq)]
pub struct Operator {
    value: String,
}

#[derive(Debug, PartialEq)]
pub struct Percentage {
    value: String,
}

/// [Pseudo-classes](https://drafts.csswg.org/selectors/#pseudo-classes)
#[derive(Debug, PartialEq)]
pub struct PseudoClassSelector {
    name: String,
    children: Option<SelectorList>,
}

/// [Pseudo-elements](https://drafts.csswg.org/selectors/#pseudo-elements)
#[derive(Debug, PartialEq)]
pub struct PseudoElementSelector {
    name: String,
    children: Option<SelectorList>,
}

#[derive(Debug, PartialEq)]
pub struct Ratio {
    left: CssNumber,
    right: CssNumber,
}

#[derive(Debug, PartialEq)]
pub struct Raw {
    value: String,
}

#[derive(Debug, PartialEq, Default)]
pub struct Rule {
    selectors: SelectorList,
    block: Block,
}

impl Rule {
    #[must_use]
    pub fn new(selectors: SelectorList, block: Block) -> Self {
        Self { selectors, block }
    }
}

#[derive(Debug, PartialEq)]
pub enum Selector {
    IdSelector(IdSelector),
    ClassSelector(ClassSelector),
    AttributeSelector(AttributeSelector),
    TypeSelector(TypeSelector),
    NestingSelector(NestingSelector),
    Combinator(Combinator),
}

impl Selector {
    pub fn is_combinator(&self) -> bool {
        matches!(self, Selector::Combinator(..))
    }

    pub fn is_descendant_combinator(&self) -> bool {
        matches!(self, Selector::Combinator(Combinator::DescendantCombinator))
    }
}

#[derive(Debug, PartialEq, Default)]
pub struct SelectorList {
    children: Vec<Selector>,
}

impl SelectorList {
    #[must_use]
    pub fn new(children: Vec<Selector>) -> Self {
        Self { children }
    }

    pub fn push(&mut self, selector: Selector) -> &mut Self {
        self.children.push(selector);

        self
    }

    pub fn pop(&mut self) -> &mut Self {
        self.children.pop();

        self
    }

    pub fn last(&self) -> Option<&Selector> {
        self.children.last()
    }

    pub fn is_last_child_descendant_combinator(&self) -> bool {
        let selector_list = &self.children;

        if let Some(last) = selector_list.last() {
            return last.is_descendant_combinator();
        }

        false
    }
}

/// Used for the [Unicode-Range microsyntax](https://drafts.csswg.org/css-syntax/#urange).
#[derive(Debug, PartialEq)]
pub struct UnicodeRange {
    value: String,
}

#[derive(Debug, PartialEq)]
pub struct Url {
    value: String,
}

#[derive(Debug, PartialEq)]
pub enum Value {
    Dimension(Dimension),
    Identifier(Identifier),
    Function(Function),
}

#[derive(Debug, PartialEq, Default)]
pub struct ValueList {
    pub children: Vec<Value>,
}

impl ValueList {
    pub fn new(children: Vec<Value>) -> ValueList {
        ValueList { children }
    }

    pub fn add_child(&mut self, child: Value) {
        self.children.push(child);
    }
}

#[derive(Debug, PartialEq)]
pub enum StyleSheetRule {
    AtRule(AtRule),
    Rule(Rule),
}

#[derive(Debug, PartialEq, Default)]
pub struct StyleSheet {
    pub children: Vec<StyleSheetRule>,
}

impl StyleSheet {
    #[must_use]
    pub fn new(children: Vec<StyleSheetRule>) -> Self {
        Self { children }
    }
}
