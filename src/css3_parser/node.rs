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
    String(CSSString),
    Identifier(IdSelector),
    None,
}

pub struct AttributeSelector {
    name: Identifier,
    matcher: Option<String>,
    value: AttributeSelectorValue,
    flags: Option<String>,
}

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

pub struct ClassSelector {
    name: String,
}

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

pub struct DeclarationList {
    // children: List,
}

pub struct Dimension {
    value: String,
    unit: String,
}

pub enum MediaFeatureValue {
    Identifier(Identifier),
    Number(CSSNumber),
    Dimension(Dimension),
    Ratio(Ratio),
    Function(Function),
}
pub struct MediaFeature {
    name: String,
    value: Option<MediaFeatureValue>,
}

pub struct FeatureFunction {
    kind: String,
    feature: String,
    // value: <Declaration> | <Selector>
}

pub struct FeatureRange {
    kind: String,
    // left: <Identifier> | <Number> | <Dimension> | <Ratio> | <Function>,
    // leftComparison: String,
    // middle: <Identifier> | <Number> | <Dimension> | <Ratio> | <Function>,
    // rightComparison: String | null,
    //  right: <Identifier> | <Number> | <Dimension> | <Ratio> | <Function> | null
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

pub struct GeneralEnclosed {
    kind: String,
    function: Option<String>,
    // children: List,
}

pub struct Hash {
    value: String,
}

pub struct IdSelector {
    name: String,
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

pub struct NestingSelector;

pub struct Nth {
    // nth: <AnPlusB> | <Identifier>,
    // selector: Option<SelectorList>
}

pub struct CSSNumber {
    value: String,
}
pub struct CSSString {
    value: String,
}

// todo: should be "enum"
pub struct Operator {
    value: String,
}

pub struct Percentage {
    value: String,
}

pub struct PseudoClassSelector {
    name: String,
    // children: List | null
}
pub struct PseudoElementSelector {
    name: String,
    // children: List | null
}

pub struct Ratio {
    // left: <Number> | <Function>,
    // right: <Number> | <Function> | null
}

pub struct Raw {
    value: String,
}

pub struct Rule {
    //  prelude: <SelectorList> | <Raw>,
    // block: <Block>
    block: Block,
}

pub struct Scope {
    // root: <SelectorList> | <Raw> | null,
    //  limit: <SelectorList> | <Raw> | null
}

pub enum Selector {
    IdSelector(IdSelector),
    ClassSelector(ClassSelector),
    Combinator(Combinator),
    AttributeSelector(AttributeSelector),
}

pub struct SelectorList {
    // children: List
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
