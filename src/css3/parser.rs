use crate::css3::node::{
    Block, BlockChild, Declaration, DeclarationList, Dimension, IdSelector, Identifier, Rule,
    Selector, SelectorList, StyleSheet, StyleSheetRule, Value, ValueList,
};
use crate::css3::tokenizer::Tokenizer;
use crate::css3::tokens::{Token, TokenType};

use super::node::{
    AttributeMatcher, AttributeSelector, ClassSelector, Combinator, CssString, TypeSelector,
};

macro_rules! unexpected_token {
    ($expecting:expr, $received:expr) => {
        panic!(
            "Unexpected token: {:?}, Expecting: {:?}",
            $expecting, $received
        )
    };

    ($expecting:expr) => {
        panic!("Unexpecting end of input. Expecting a {:?}", $expecting)
    };
}

/// # CSS3 Parser
/// The parser using the Recursive Descent Parser algorithm (predictive parser).
/// The grammer rules is defined using Backus–Naur form (BNF)
#[derive(Debug, PartialEq)]
pub struct CSS3Parser {
    tokenizer: Tokenizer,
    lookahead: Option<Token>,
    raw: String,
}

impl Default for CSS3Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl CSS3Parser {
    pub fn new() -> CSS3Parser {
        CSS3Parser {
            tokenizer: Tokenizer::new(),
            lookahead: None,
            raw: "".to_string(),
        }
    }

    pub fn parse(&mut self, raw: &str) -> StyleSheet {
        self.raw = raw.to_string();
        self.tokenizer.init(raw);
        self.lookahead = self.tokenizer.get_next_token();
        self.style_sheet()
    }

    /// ```txt
    /// SytleSheet
    ///     : RulesList
    ///     ;
    /// ```
    fn style_sheet(&mut self) -> StyleSheet {
        StyleSheet::new(self.rules_list())
    }

    /// ```txt
    /// RulesList
    ///     : [Rule | AtRule]+
    ///     ;
    /// ```
    fn rules_list(&mut self) -> Vec<StyleSheetRule> {
        // note: support only "Rule" for now
        let mut rules: Vec<StyleSheetRule> = Vec::new();

        while !self.is_next_token(TokenType::LCurly) & self.lookahead.is_some() {
            rules.push(StyleSheetRule::Rule(self.rule()));
        }

        rules
    }

    /// ```txt
    /// Rule
    ///     : SelectorList Block
    ///     ;
    /// ```
    fn rule(&mut self) -> Rule {
        let selectors = self.selector_list();
        let block = self.block();
        Rule::new(selectors, block)
    }

    ///```txt
    /// SelectorList
    ///     : [Selector]*
    ///     ;
    /// ```
    fn selector_list(&mut self) -> SelectorList {
        let mut selector_list = SelectorList::default();

        while !self.is_next_token(TokenType::LCurly) {
            selector_list.add_child(self.selector())
        }

        selector_list
    }

    ///```txt
    /// Selector
    ///     : IdSelector
    ///     | ClassSelector
    ///     | AttributeSelector
    ///     | TypeSelector
    ///     | NestingSelector
    ///     | Combinator
    ///     ;
    /// ```
    fn selector(&mut self) -> Selector {
        if self.is_next_token(TokenType::Hash) {
            return Selector::IdSelector(self.id_selector());
        }

        if self.is_next_token(TokenType::Dot) {
            return Selector::ClassSelector(self.class_selector());
        }

        if self.is_next_token(TokenType::LBracket) {
            return Selector::AttributeSelector(self.attribute_selector());
        }

        if self.is_next_token(TokenType::Ident) {
            return Selector::TypeSelector(self.type_selector());
        }

        if self.is_combinator_next() {
            return Selector::Combinator(self.combinator());
        }

        unexpected_token!("Selector")
    }

    /// ```bnf
    ///  TypeSelector
    ///     : IDENT
    ///     ;   
    /// ```
    fn type_selector(&mut self) -> TypeSelector {
        TypeSelector::new(self.consume(TokenType::Ident).value)
    }

    /// ```bnf
    ///  IdSelector
    ///     : HASH IDENT
    ///     ;   
    /// ```
    fn id_selector(&mut self) -> IdSelector {
        self.consume(TokenType::Hash);
        let name = self.consume(TokenType::Ident).value;
        IdSelector::new(name)
    }

    /// ```bnf
    ///  ClassSelector
    ///     : DOT IDENT
    ///     ;   
    /// ```
    fn class_selector(&mut self) -> ClassSelector {
        self.consume(TokenType::Dot);
        let name = self.consume(TokenType::Ident).value;
        ClassSelector::new(name)
    }

    /// ```bnf
    ///  AttributeSelector
    ///     : LBRACKET IDENT [AttributeMatcher String]? [IDENT]? RBRACKET
    ///     ;   
    /// ```
    fn attribute_selector(&mut self) -> AttributeSelector {
        self.consume(TokenType::LBracket);
        let name = self.identifier();

        let matcher = if !self.is_next_token(TokenType::RBracket) {
            Some(self.attribute_matcher())
        } else {
            None
        };

        println!("matcher: {:?}", matcher);

        let value = if matcher.is_some() {
            Some(self.string())
        } else {
            None
        };

        let flag = if !self.is_next_token(TokenType::RBracket) {
            Some(self.identifier())
        } else {
            None
        };

        self.consume(TokenType::RBracket);

        AttributeSelector {
            name,
            matcher,
            value,
            flag,
        }
    }

    /// ```bnf
    ///  AttributeMatcher
    ///     :  INCLUDE_MATCH
    ///     |  DASH_MATCH
    ///     |  PREFIX_MATCH
    ///     |  SUFFIX_MATCH
    ///     |  SUBSTRING_MATCH
    ///     |  EQUAL
    ///     ;   
    /// ```
    fn attribute_matcher(&mut self) -> AttributeMatcher {
        if let Some(next_token_type) = self.get_next_token_type() {
            let matcher = match next_token_type {
                TokenType::IncludeMatch => AttributeMatcher::IncludeMatch,
                TokenType::DashMatch => AttributeMatcher::DashMatch,
                TokenType::PrefixMatch => AttributeMatcher::PrefixMatch,
                TokenType::SuffixMatch => AttributeMatcher::SuffixMatch,
                TokenType::SubstringMatch => AttributeMatcher::SubstringMatch,
                TokenType::Equal => AttributeMatcher::EqualityMatch,
                _ => panic!(
                    "Unexpected token: {:?}, Expecting selector token",
                    next_token_type
                ),
            };

            self.consume(self.get_next_token_type().unwrap());
            return matcher;
        }

        panic!("Unexpecting end of input. Expecting a AttributeMatcher");
    }

    /// ```bnf
    /// Combinator:
    ///     : CHILD_COMBINATOR
    ///     | DESCENDANT_COMBINATOR
    ///     | NAMESPACE_SEPARATOR
    ///     | NEXT_SIBLING_COMBINATOR
    ///     | SELECTOR_LIST_COMBINATOR
    ///     | SUBSEQUENT_SIBLING_COMBINATOR
    ///     ;
    /// ```
    fn combinator(&mut self) -> Combinator {
        if let Some(next_token_type) = self.get_next_token_type() {
            let combinator = match next_token_type {
                TokenType::ChildCombinator => Combinator::ChildCombinator,
                TokenType::ColumnCombinator => Combinator::ColumnCombinator,
                TokenType::DescendantCombinator => Combinator::DescendantCombinator,
                TokenType::NamespaceSeparator => Combinator::NamespaceSeparator,
                TokenType::NextSiblingCombinator => Combinator::NextSiblingCombinator,
                TokenType::SelectorListCombinator => Combinator::SelectorListCombinator,
                TokenType::SubsequentSiblingCombinator => Combinator::SubsequentSiblingCombinator,
                other => {
                    unexpected_token!(other, "Combinator")
                }
            };

            self.consume(self.get_next_token_type().unwrap());
            return combinator;
        };

        unexpected_token!("Combinator");
    }

    /// ```bnf
    ///  String
    ///     : STRING
    ///     ;
    /// ```
    fn string(&mut self) -> CssString {
        let mut value = self.consume(TokenType::String).value;

        // Remove starting and ending quotes
        value.pop();
        if !value.is_empty() {
            value.remove(0);
        }

        CssString::new(value)
    }

    /// ```bnf
    ///  Block
    ///     : LCURLY [Rule | AtRule | DeclarationList]* RCURLY
    ///     ;   
    /// ```
    fn block(&mut self) -> Block {
        // note: add support for 'DeclarationList' for now
        let mut block = Block::default();

        self.consume(TokenType::LCurly);

        while !self.is_next_token(TokenType::RCurly) {
            block.add_child(BlockChild::DeclarationList(self.declaration_list()))
        }

        self.consume(TokenType::RCurly);

        block
    }

    /// ```bnf
    ///  DeclarationList
    ///     : [Declaration]*
    ///     ;   
    /// ```
    fn declaration_list(&mut self) -> DeclarationList {
        let mut declaration_list = DeclarationList::default();

        while !self.is_next_token(TokenType::RCurly) {
            declaration_list.add_child(self.declaration())
        }

        declaration_list
    }

    /// ```bnf
    ///  Declaration
    ///     : IDENT COLON ValueList IMPORTANT SEMICOLON
    ///     ;   
    /// ```
    fn declaration(&mut self) -> Declaration {
        let mut declaration = Declaration::default();

        declaration.set_property(self.consume(TokenType::Ident).value);
        self.consume(TokenType::Colon);
        declaration.set_value(self.value_list());

        if self.is_next_token(TokenType::Important) {
            self.consume(TokenType::Important);
            declaration.set_important_as(true);
        }

        self.consume(TokenType::Semicolon);

        declaration
    }

    /// ```bnf
    ///  ValueList
    ///     : [Value]*
    ///     ;   
    /// ```
    fn value_list(&mut self) -> ValueList {
        let mut value_list = ValueList::default();

        while !self.is_next_tokens(vec![TokenType::Semicolon, TokenType::Important]) {
            value_list.add_child(self.value());
        }

        value_list
    }

    /// ```bnf
    ///  Value
    ///     : [Dimension | Identifier | Function]
    ///     ;   
    /// ```
    fn value(&mut self) -> Value {
        // note: support only "Identifier" and "Dimension" for now

        if self.is_next_token(TokenType::Ident) {
            return Value::Identifier(self.identifier());
        }

        Value::Dimension(self.dimension())
    }

    /// ```bnf
    ///  Identifier
    ///     : IDENT
    ///     ;   
    /// ```
    fn identifier(&mut self) -> Identifier {
        Identifier::new(self.consume(TokenType::Ident).value)
    }

    /// ```bnf
    ///  Dimension
    ///     : NUMBER IDENT
    ///     ;   
    /// ```
    fn dimension(&mut self) -> Dimension {
        let value = self.consume(TokenType::Number).value;

        let unit = if self.is_next_token(TokenType::Ident) {
            Some(self.consume(TokenType::Ident).value)
        } else {
            None
        };

        Dimension::new(value, unit)
    }

    fn consume(&mut self, token_type: TokenType) -> Token {
        if let Some(token) = self.lookahead.clone() {
            if token.token_type != token_type {
                panic!(
                    "Unexpected token: '{:?}', expected: '{:?}'. Got '{}' at '{}'",
                    token.token_type, token_type, token.value, self.tokenizer.cursor
                )
            }

            // Advance to the next token
            self.lookahead = self.tokenizer.get_next_token();

            println!("next token: {:#?}", self.lookahead);
            return token.clone();
        }

        panic!("Unexpected end of input, expected: {:?}", token_type)
    }

    fn is_next_token(&self, token_type: TokenType) -> bool {
        if let Some(token) = self.lookahead.clone() {
            return token.token_type == token_type;
        }

        false
    }

    fn is_next_tokens(&self, token_types: Vec<TokenType>) -> bool {
        for token_type in token_types {
            if self.is_next_token(token_type) {
                return true;
            }
        }
        false
    }

    fn get_next_token_type(&self) -> Option<TokenType> {
        if let Some(token) = self.lookahead.clone() {
            return Some(token.token_type);
        }

        None
    }

    fn is_combinator_next(&self) -> bool {
        self.is_next_tokens(vec![
            TokenType::ChildCombinator,
            TokenType::ColumnCombinator,
            TokenType::DescendantCombinator,
            TokenType::NamespaceSeparator,
            TokenType::NextSiblingCombinator,
            TokenType::SelectorListCombinator,
            TokenType::SubsequentSiblingCombinator,
        ])
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parse_css() {
        let mut parser = CSS3Parser::new();
        let style_sheet = parser.parse(
            r#"
            
                #header {
                    display: flex;
                    width: 100px;
                    font-size: 1rem !important;
                }
            "#,
        );

        assert_eq!(
            style_sheet,
            StyleSheet::new(vec![StyleSheetRule::Rule(Rule::new(
                SelectorList::new(vec![Selector::IdSelector(IdSelector::new("header"))]),
                Block::new(vec![BlockChild::DeclarationList(DeclarationList::new(
                    vec![
                        Declaration::new(
                            "display",
                            ValueList::new(vec![Value::Identifier(Identifier::new("flex"))])
                        ),
                        Declaration::new(
                            "width",
                            ValueList::new(vec![Value::Dimension(Dimension::new(
                                "100",
                                Some("px")
                            ))])
                        ),
                        Declaration {
                            important: true,
                            property: "font-size".to_string(),
                            value: ValueList::new(vec![Value::Dimension(Dimension::new(
                                "1",
                                Some("rem")
                            ))])
                        }
                    ]
                ))])
            ))])
        )
    }

    #[test]
    fn parse_attribute_selectors() {
        let mut parser = CSS3Parser::new();

        assert_eq!(
            parser.parse(
                r##"
            a {
                color: blue;
            }
        "##
            ),
            StyleSheet::new(vec![StyleSheetRule::Rule(Rule::new(
                SelectorList::new(vec![Selector::TypeSelector(TypeSelector::new("a"))]),
                Block::new(vec![BlockChild::DeclarationList(DeclarationList::new(
                    vec![Declaration::new(
                        "color",
                        ValueList::new(vec![Value::Identifier(Identifier::new("blue"))])
                    ),]
                ))])
            )),])
        );

        assert_eq!(
            parser.parse(
                r##"
            /* Internal links, beginning with "#" */
            a[href^="#"] {
                background-color: gold;
            }
        "##,
            ),
            StyleSheet::new(vec![StyleSheetRule::Rule(Rule::new(
                SelectorList::new(vec![
                    Selector::TypeSelector(TypeSelector::new("a")),
                    Selector::AttributeSelector(AttributeSelector {
                        name: Identifier::new("href"),
                        matcher: Some(AttributeMatcher::PrefixMatch),
                        value: Some(CssString::new("#")),
                        flag: None
                    })
                ]),
                Block::new(vec![BlockChild::DeclarationList(DeclarationList::new(
                    vec![Declaration::new(
                        "background-color",
                        ValueList::new(vec![Value::Identifier(Identifier::new("gold"))])
                    ),]
                ))])
            )),])
        );

        assert_eq!(
            parser.parse(
                r#"
            /* Links with "example" anywhere in the URL */
            a[href*="example"] {
                background-color: silver;
            }
        "#
            ),
            StyleSheet::new(vec![StyleSheetRule::Rule(Rule::new(
                SelectorList::new(vec![
                    Selector::TypeSelector(TypeSelector::new("a")),
                    Selector::AttributeSelector(AttributeSelector {
                        name: Identifier::new("href"),
                        matcher: Some(AttributeMatcher::SubstringMatch),
                        value: Some(CssString::new("example")),
                        flag: None
                    })
                ]),
                Block::new(vec![BlockChild::DeclarationList(DeclarationList::new(
                    vec![Declaration::new(
                        "background-color",
                        ValueList::new(vec![Value::Identifier(Identifier::new("silver"))])
                    ),]
                ))])
            )),])
        );

        assert_eq!(
            parser.parse(
                r#"
            /* Links with "insensitive" anywhere in the URL,
            regardless of capitalization */
            a[href*="insensitive" i] {
                color: cyan;
            }
        "#
            ),
            StyleSheet::new(vec![StyleSheetRule::Rule(Rule::new(
                SelectorList::new(vec![
                    Selector::TypeSelector(TypeSelector::new("a")),
                    Selector::AttributeSelector(AttributeSelector {
                        name: Identifier::new("href"),
                        matcher: Some(AttributeMatcher::SubstringMatch),
                        value: Some(CssString::new("insensitive")),
                        flag: Some(Identifier::new("i"))
                    })
                ]),
                Block::new(vec![BlockChild::DeclarationList(DeclarationList::new(
                    vec![Declaration::new(
                        "color",
                        ValueList::new(vec![Value::Identifier(Identifier::new("cyan"))])
                    ),]
                ))])
            )),])
        );

        assert_eq!(
            parser.parse(
                r#"
                    /* Links with "cAsE" anywhere in the URL,
                    with matching capitalization */
                    a[href*="cAsE" s] {
                        color: pink;
                    }
                "#,
            ),
            StyleSheet::new(vec![StyleSheetRule::Rule(Rule::new(
                SelectorList::new(vec![
                    Selector::TypeSelector(TypeSelector::new("a")),
                    Selector::AttributeSelector(AttributeSelector {
                        name: Identifier::new("href"),
                        matcher: Some(AttributeMatcher::SubstringMatch),
                        value: Some(CssString::new("cAsE")),
                        flag: Some(Identifier::new("s"))
                    })
                ]),
                Block::new(vec![BlockChild::DeclarationList(DeclarationList::new(
                    vec![Declaration::new(
                        "color",
                        ValueList::new(vec![Value::Identifier(Identifier::new("pink"))])
                    ),]
                ))])
            )),])
        );

        assert_eq!(
            parser.parse(
                r#"
            
            /* Links that end in ".org" */
            a[href$=".org"] {
                color: red;
            }

            "#
            ),
            StyleSheet::new(vec![StyleSheetRule::Rule(Rule::new(
                SelectorList::new(vec![
                    Selector::TypeSelector(TypeSelector::new("a")),
                    Selector::AttributeSelector(AttributeSelector {
                        name: Identifier::new("href"),
                        matcher: Some(AttributeMatcher::SuffixMatch),
                        value: Some(CssString::new(".org")),
                        flag: None
                    })
                ]),
                Block::new(vec![BlockChild::DeclarationList(DeclarationList::new(
                    vec![Declaration::new(
                        "color",
                        ValueList::new(vec![Value::Identifier(Identifier::new("red"))])
                    ),]
                ))])
            )),])
        );

        assert_eq!(
            parser.parse(
                r#"
            /* Links that start with "https://" and end in ".org" */
            a[href^="https://"][href$=".org"] {
                color: green;
            } 
            "#
            ),
            StyleSheet::new(vec![StyleSheetRule::Rule(Rule::new(
                SelectorList::new(vec![
                    Selector::TypeSelector(TypeSelector::new("a")),
                    Selector::AttributeSelector(AttributeSelector {
                        name: Identifier::new("href"),
                        matcher: Some(AttributeMatcher::PrefixMatch),
                        value: Some(CssString::new("https://")),
                        flag: None
                    }),
                    Selector::AttributeSelector(AttributeSelector {
                        name: Identifier::new("href"),
                        matcher: Some(AttributeMatcher::SuffixMatch),
                        value: Some(CssString::new(".org")),
                        flag: None
                    })
                ]),
                Block::new(vec![BlockChild::DeclarationList(DeclarationList::new(
                    vec![Declaration::new(
                        "color",
                        ValueList::new(vec![Value::Identifier(Identifier::new("green"))])
                    ),]
                ))])
            )),])
        );
    }

    #[test]
    fn parse_selectors_combinators() {
        let mut parser = CSS3Parser::new();

        let combinators: Vec<Combinator> = vec![
            Combinator::ChildCombinator,
            Combinator::ColumnCombinator,
            Combinator::SelectorListCombinator,
            Combinator::NamespaceSeparator,
            Combinator::NextSiblingCombinator,
            Combinator::SubsequentSiblingCombinator,
        ];

        let mut rules = vec![];

        for combinator in combinators {
            rules.push(StyleSheetRule::Rule(Rule::new(
                SelectorList::new(vec![
                    Selector::TypeSelector(TypeSelector::new("ul".to_string())),
                    Selector::Combinator(combinator),
                    Selector::TypeSelector(TypeSelector::new("li".to_string())),
                ]),
                Block::default(),
            )))
        }

        assert_eq!(
            parser.parse(
                r##"
            ul > li {}  

            ul || li {}

            ul, li {}

            ul|li {}
            
            ul + li {}

            ul ~ li {}

        "##
            ),
            StyleSheet::new(rules)
        );
    }
}
