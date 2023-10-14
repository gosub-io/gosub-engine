// Note: every node should have a "loc" property

/// Used for the [An+B microsyntax](https://drafts.csswg.org/css-syntax/#anb-microsyntax).
pub struct AnPlusB {
    a: Option<String>,
    b: Option<String>,
}

pub enum AtRulePreludeValue {
    AtRulePrelude(AtRulePrelude),
    Raw(Raw),
    None,
}

/// CSS [At Rule](https://drafts.csswg.org/css-conditional-3/)
/// E.g. @import @media @keyframes @supports
pub struct AtRule {
    name: String,
    prelude: AtRulePreludeValue,
    block: Option<Block>,
}

pub enum AtRulePreludeChild {
    MediaQueryList(MediaQueryList),
}

pub struct AtRulePrelude {
    children: Vec<AtRulePrelude>,
}

pub enum AttributeSelectorValue {
    String(CssString),
    Identifier(IdSelector),
    None,
}

/// [Attribute Selector](https://drafts.csswg.org/selectors/#attribute-selectors)
pub struct AttributeSelector {
    name: Identifier,
    matcher: Option<String>,
    value: AttributeSelectorValue,
    flags: Option<String>,
}

/// [Id Selector](https://drafts.csswg.org/selectors/#id-selectors)
pub struct IdSelector {
    name: String,
}

/// [Class Selector](https://drafts.csswg.org/selectors/#class-html)
pub struct ClassSelector {
    name: String,
}

/// [TypeSelector](https://drafts.csswg.org/selectors/#type-selectors)
pub struct TypeSelector {
    name: String,
}

/// [Nesting Selector](https://drafts.csswg.org/css-nesting/#nest-selector)
pub struct NestingSelector;

pub enum BlockChild {
    Rule(Rule),
    AtRule(AtRule),
    Declaration(Declaration),
}

pub struct Block {
    children: Vec<BlockChild>,
}

pub struct Brackets {
    // children: List
}

pub struct Identifier {
    name: String,
}

pub struct CDC;
pub struct CDO;

pub struct Combinator {
    name: String,
}

pub enum DeclarationValue {
    Value(Value),
    Raw(Raw),
}

pub struct Declaration {
    important: bool,
    property: String,
    value: DeclarationValue,
}

pub struct Dimension {
    value: String,
    unit: String,
}

pub enum MediaFeatureValue {
    Identifier(Identifier),
    Number(CssNumber),
    Dimension(Dimension),
    Ratio(Ratio),
    Function(Function),
}

pub struct MediaFeature {
    name: String,
    value: Option<MediaFeatureValue>,
}

pub enum FunctionChild {
    Identifier(Identifier),
    Operator(Operator),
    Percentage(Percentage),
}

pub struct Function {
    name: String,
    children: Vec<FunctionChild>,
}

pub struct Hash {
    value: String,
}

pub struct Layer {
    name: String,
}

pub struct LayerList {
    children: Vec<Layer>,
}

pub enum MediaQueryChild {
    Identifier(Identifier),
    MediaFeature(MediaFeature),
}

pub struct MediaQuery {
    children: Vec<MediaQueryChild>,
}

pub struct MediaQueryList {
    children: Vec<MediaQuery>,
}

pub enum NthValue {
    AnPlusB(AnPlusB),
    Identifier(Identifier),
}
pub struct Nth {
    nth: NthValue,
    selector: Option<SelectorList>,
}

pub struct CssNumber {
    value: String,
}
pub struct CssString {
    value: String,
}

// todo: should be "enum"
pub struct Operator {
    value: String,
}

pub struct Percentage {
    value: String,
}

/// [Pseudo-classes](https://drafts.csswg.org/selectors/#pseudo-classes)
pub struct PseudoClassSelector {
    name: String,
    children: Option<SelectorList>,
}

/// [Pseudo-elements](https://drafts.csswg.org/selectors/#pseudo-elements)
pub struct PseudoElementSelector {
    name: String,
    children: Option<SelectorList>,
}

pub struct Ratio {
    left: CssNumber,
    right: CssNumber,
}

pub struct Raw {
    value: String,
}

pub enum RulePrelude {
    SelectorList(SelectorList),
    Raw(Raw),
}

pub struct Rule {
    prelude: RulePrelude,
    block: Block,
}

pub enum Selector {
    IdSelector(IdSelector),
    ClassSelector(ClassSelector),
    AttributeSelector(AttributeSelector),
    TypeSelector(TypeSelector),
    NestingSelector(NestingSelector),
}

pub struct SelectorList {
    children: Vec<Selector>,
}

/// Used for the [Unicode-Range microsyntax](https://drafts.csswg.org/css-syntax/#urange).
pub struct UnicodeRange {
    value: String,
}

pub struct Url {
    value: String,
}

pub enum ValueChild {
    Dimension(Dimension),
    Identifier(Identifier),
    Function(Function),
}

pub struct Value {
    children: Vec<ValueChild>,
}

pub enum StyleSheetChild {
    AtRule(AtRule),
    Rule(Rule),
}

pub struct StyleSheet {
    children: Vec<StyleSheetChild>,
}
