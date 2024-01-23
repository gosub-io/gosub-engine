pub enum CssRuleType {
    UnknownRule = 0,
    StyleRule = 1,
    CharsetRule = 2,        // Obsolete
    ImportRule = 3,
    MediaRule = 4,
    FontFaceRule = 5,
    PageRule = 6,
    KeyframesRule = 7,
    KeyframeRule = 8,
    MarginRule = 9,         // Obsolete
    NamespaceRule = 10,
    CounterStyleRule = 11,
    SupportsRule = 12,
    DocumentRule = 13,      // Obsolete
    FontFeatureValuesRule = 14,
    ViewportRule = 15,      // Obsolete
    RegionStyleRule = 16,   // Obsolete
}

pub enum CssTypeRuleType {
    StyleRule(CssStyleRule),
    CharsetRule(CssCharsetRule),
    ImportRule(CssImportRule),
    MediaRule(CssMediaRule),
    FontFaceRule(CssFontFaceRule),
    PageRule(CssPageRule),
    KeyframesRule(CssKeyframesRule),
    KeyframeRule(CssKeyframeRule),
    MarginRule(CssMarginRule),
    NamespaceRule(CssNamespaceRule),
    CounterStyleRule(CssCounterStyleRule),
    SupportsRule(CssSupportsRule),
    DocumentRule(CssDocumentRule),
    FontFeatureValuesRule(CssFontFeatureValuesRule),
    ViewportRule(CssViewportRule),
    RegionStyleRule(CssRegionStyleRule),
}

struct CssStyleDeclaration {
    css_float: String,
    css_text: String,
    length: usize,
    /// All the properties that are defined
    property_list: HashMap<String, String>,
    parent_rule: Rc<CssRule>
}

impl CssStyleDeclaration {
    pub fn get_property_priority(&self, property: &str) -> Option<&str> {
        None
    }

    pub fn get_property_value(&self, property: &str) -> Option<&str> {
        self.property_list.get(property).map(|s| s.as_str())
    }

    pub fn item(&self, idx: usize) -> Option<&str> {
        None
    }

    pub fn remove_property(&mut self, property: &str) {
        self.property_list.remove(property);
    }

    pub fn set_property(&mut self, property: &str, value: &str) {
        self.property_list.insert(property.to_string(), value.to_string());
    }

    pub fn get_property_css_value(&self, property: &str) -> Option<&str> {
        None
    }
}


struct CssRule {
    text: String,
    parent_rule: Option<Rc<CssRule>>,
    parent_stylesheet: Option<Rc<CSSStyleSheet>>,
    type_: CssRuleType,
}

struct CssGroupingRule {
    css_rules: CssRuleList,
    parent: CssRule,
}

impl CssGroupingRule {
    pub fn delete_rule(&mut self, idx: usize) {
        self.css_rules.remove(idx);
    }

    pub fn insert_rule(&mut self, idx: usize, rule: CssRule) {
        self.css_rules.insert(idx, rule);
    }
}

struct CssStyleRule {
    selector_text: String,
    style: CssStyleDeclaration,
    // style_map: StylePropertyMap,     // This is basically the same as the style, but in a different format I think
    parent: CssGroupingRule,
}

struct CssImportRule {
    href: String,
    layer_name: String,
    media: String,
    style_sheet: Rc<CSSStyleSheet>,
    supports_rule: Option<String>,
    parent: CssRule,
}

struct CssMediaRule {
    media: MediaList,
    parent: CssGroupingRule,
}

struct CssFontFaceRule {
    style: CssStyleDeclaration,
    parent: CssGroupingRule,
}

struct CssPageRule {
    selector_text: String,
    style: CssStyleDeclaration,
    parent: CssGroupingRule,
}

struct CssNamespaceRule {
    namespace: String,
    prefix: String,
    parent: CssRule,
}

struct CssKeyframesRule {
    name: String,
    css_rules: CssRuleList,
    parent: CssRule,
}

struct CssKeyframeRule {
    key_text: String,
    style: CssStyleDeclaration,
    parent: CssRule,
}

struct CssCounterStyleRule {
    name: String,
    system: String,
    symbols: String,
    additive_symbols: String,
    negative: String,
    prefix: String,
    suffix: String,
    range: String,
    pad: String,
    speak_as: String,
    fallback: String,
    parent: CssRule,
}

struct CssSupportsRule {
    parent: CssConditionRule,
}

struct CssFontFeatureValuesRule {
    font_family: String,
    parent: CssRule,
}

struct CssFontPaletteValuesRule {
    name: String,
    font_family: String,
    base_palette: String,
    override_colors: String,
    parent: CssRule,
}

struct CssLayerBlockRule {
    name: String,
    parent: CssGroupingRule,
}

struct CssLayerStatementRule {
    name_list: NameList,
    parent: CssRule,
}

struct CssPropertyRule {
    inherits: String,
    initial_value: String,
    name: String,
    syntax: String,
    parent: CssRule,
}


