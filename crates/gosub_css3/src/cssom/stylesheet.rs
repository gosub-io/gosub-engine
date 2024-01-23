type MediaList = Vec<String>;

impl MediaList {
    pub fn append_medium(&mut self, medium: String) {
        self.push(medium);
    }

    pub fn delete_medium(&mut self, idx: usize) {
        self.remove(idx);
    }

    pub fn item(&self, idx: usize) -> Option<&String> {
        self.get(idx)
    }
}

struct StyleSheet {
    disabled: bool,
    href: String,
    media: MediaList,
    owner_node: Option<Node>,
    parent_style_sheet: Option<StyleSheet>,
    title: String,
    type_: String,
}

struct CSSStyleSheet {
    rules: CssRuleList,
    owner_rule: Option<CssRule>,
    stylesheet: StyleSheet,
}

impl CSSStyleSheet {
    pub fn new(stylesheet: StyleSheet) -> Self {
        Self {
            rules: vec![]
            owner_rule: None,
            stylesheet,
        }
    }

    pub fn delete_rule(&mut self, idx: usize) {
        self.rules.remove(idx);
    }

    pub fn insert_rule(&mut self, idx: usize, rule: CssRule) {
        self.rules.insert(idx, rule);
    }

    pub fn replace_async(&mut self, idx: usize, rule: CssRule) {
        self.rules[idx] = rule;
    }

    pub fn replace(&mut self, idx: usize, rule: CssRule) {
        self.rules[idx] = rule;
    }

    // CSSOM:
    //   property: rules obsolete
    //   method: addRule() obsolete
    //   method: removeRule() obsolete
}