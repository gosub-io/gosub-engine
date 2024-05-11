#[derive(Debug, PartialEq, Eq)]
pub enum Condition {
    EqualsTag(String),
    EqualsId(String),
    ContainsClass(String),
    ContainsAttribute(String),
    ContainsChildTag(String),
    HasParentTag(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum SearchType {
    Uninitialized,
    FindFirst,
    FindAll,
}

pub struct Query {
    pub(crate) conditions: Vec<Condition>,
    pub(crate) search_type: SearchType,
}

impl Query {
    #[allow(dead_code)]
    pub(crate) fn new() -> Self {
        Self {
            conditions: Vec::new(),
            search_type: SearchType::Uninitialized,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn equals_tag(mut self, tag_name: &str) -> Self {
        self.conditions
            .push(Condition::EqualsTag(tag_name.to_owned()));
        self
    }

    #[allow(dead_code)]
    pub(crate) fn equals_id(mut self, id: &str) -> Self {
        self.conditions.push(Condition::EqualsId(id.to_owned()));
        self
    }

    #[allow(dead_code)]
    pub(crate) fn contains_class(mut self, class: &str) -> Self {
        self.conditions
            .push(Condition::ContainsClass(class.to_owned()));
        self
    }

    #[allow(dead_code)]
    pub(crate) fn contains_attribute(mut self, attribute: &str) -> Self {
        self.conditions
            .push(Condition::ContainsAttribute(attribute.to_owned()));
        self
    }

    #[allow(dead_code)]
    pub(crate) fn contains_child_tag(mut self, child_tag: &str) -> Self {
        self.conditions
            .push(Condition::ContainsChildTag(child_tag.to_owned()));
        self
    }

    #[allow(dead_code)]
    pub(crate) fn has_parent_tag(mut self, parent_tag: &str) -> Self {
        self.conditions
            .push(Condition::HasParentTag(parent_tag.to_owned()));
        self
    }

    #[allow(dead_code)]
    pub(crate) fn find_first(mut self) -> Self {
        self.search_type = SearchType::FindFirst;
        self
    }

    #[allow(dead_code)]
    pub(crate) fn find_all(mut self) -> Self {
        self.search_type = SearchType::FindAll;
        self
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::query::{Condition, Query, SearchType};

    #[test]
    fn uninitialized() {
        let query = Query::new().equals_tag("div").equals_id("myid");
        assert_eq!(query.search_type, SearchType::Uninitialized);
    }

    #[test]
    fn find_first() {
        let query = Query::new().find_first();
        assert_eq!(query.search_type, SearchType::FindFirst);
    }

    #[test]
    fn find_all() {
        let query = Query::new().find_all();
        assert_eq!(query.search_type, SearchType::FindAll);
    }

    #[test]
    fn build_conditions() {
        let query = Query::new()
            .equals_tag("div")
            .equals_id("myid")
            .contains_class("myclass")
            .contains_attribute("myattr")
            .contains_child_tag("h1")
            .has_parent_tag("html")
            .find_first();

        assert_eq!(query.conditions.len(), 6);
        assert_eq!(query.conditions[0], Condition::EqualsTag("div".to_owned()));
        assert_eq!(query.conditions[1], Condition::EqualsId("myid".to_owned()));
        assert_eq!(
            query.conditions[2],
            Condition::ContainsClass("myclass".to_owned())
        );
        assert_eq!(
            query.conditions[3],
            Condition::ContainsAttribute("myattr".to_owned())
        );
        assert_eq!(
            query.conditions[4],
            Condition::ContainsChildTag("h1".to_owned())
        );
        assert_eq!(
            query.conditions[5],
            Condition::HasParentTag("html".to_owned())
        );
    }
}
