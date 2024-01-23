type StyleSheetList = vec<StyleSheet>;

impl StyleSheetList {
    fn item(&self, idx: usize) -> Option<&StyleSheet> {
        self.get(idx)
    }

    fn length(&self) -> usize {
        self.len()
    }
}