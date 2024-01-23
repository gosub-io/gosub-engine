type CssRuleList = vec<CssRule>;

impl CssRuleList {
    fn item(&self, idx: usize) -> Option<&CssRule> {
        self.get(idx)
    }

    fn length(&self) -> usize {
        self.len()
    }
}