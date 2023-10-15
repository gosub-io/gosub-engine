use super::{
    node::{
        Block, BlockChild, Declaration, DeclarationList, Dimension, IdSelector, Identifier, Rule,
        Selector, SelectorList, StyleSheet, StyleSheetRule, Value, ValueList,
    },
    tokenizer::CSSTokenizer,
    tokens::{Token, TokenType},
};

#[derive(Debug, PartialEq)]
pub struct CSSStyleSheet {
    css_rules: Vec<CSSRule>,
}

impl Default for CSSStyleSheet {
    fn default() -> Self {
        Self::new()
    }
}

impl CSSStyleSheet {
    pub fn new() -> CSSStyleSheet {
        CSSStyleSheet {
            css_rules: Vec::new(),
        }
    }
}

pub type CSSPropName = String;
pub type CSSPropValue = String;

#[derive(Debug, PartialEq)]
pub struct CSSStyleDeclaration {
    name: CSSPropName,
    value: CSSPropValue,
}

#[derive(Debug, PartialEq)]
pub struct CSSSelector {
    path: Vec<String>,
}

impl Default for CSSSelector {
    fn default() -> Self {
        Self::new()
    }
}

impl CSSSelector {
    pub fn new() -> CSSSelector {
        CSSSelector { path: Vec::new() }
    }

    pub fn from_str_slice(path: Vec<&str>) -> CSSSelector {
        CSSSelector {
            path: path.iter().map(|s| s.to_string()).collect::<Vec<String>>(),
        }
    }

    pub fn add(&mut self, value: String) -> &mut CSSSelector {
        self.path.push(value);

        self
    }
}

#[derive(Debug, PartialEq)]
pub struct CSSStyleRule {
    selector: CSSSelector,
    style_declarations: Vec<CSSStyleDeclaration>,
}

#[derive(Debug, PartialEq)]
pub struct CSSMeidaRule {
    conditions: Vec<MediaCondition>,
    css_rules: Vec<CSSRule>,
}

#[derive(Debug, PartialEq)]
pub enum MediaType {
    Screen,
    Printer,
    All,
}

#[derive(Debug, PartialEq)]
pub struct MediaFeature {
    name: String,
    value: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum MediaCondition {
    Type(MediaType),
    Feature(MediaFeature),
}

#[derive(Debug, PartialEq)]
pub enum CSSRule {
    CSSStyleRule(CSSStyleRule),
    CSSMeidaRule(CSSMeidaRule),
}

/// # CSS3 Parser
/// The parser using the Recursive Descent Parser algorithm (predictive parser).
/// The grammer rules is defined using Backus–Naur form (BNF)
#[derive(Debug, PartialEq)]
pub struct CSS3Parser {
    tokenizer: CSSTokenizer,
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
            tokenizer: CSSTokenizer::default(),
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
    ///     ;
    /// ```
    fn selector(&mut self) -> Selector {
        // note: support class & id selectors for now
        Selector::IdSelector(self.id_selector())
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
    ///     : IDENT COLON ValueList SEMICOLON
    ///     ;   
    /// ```
    fn declaration(&mut self) -> Declaration {
        let mut declaration = Declaration::default();

        declaration.set_property(self.consume(TokenType::Ident).value);
        self.consume(TokenType::Colon);
        declaration.set_value(self.value_ist());
        self.consume(TokenType::Semicolon);

        declaration
    }

    /// ```bnf
    ///  ValueList
    ///     : [Value]*
    ///     ;   
    /// ```
    fn value_ist(&mut self) -> ValueList {
        let mut value_list = ValueList::default();

        while !self.is_next_token(TokenType::Semicolon) {
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
        println!("Expecting: {:?}", token_type);

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
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn should_parse_css() {
        let mut parser = CSS3Parser::new();
        let style_sheet = parser.parse(
            r#"
            
                #header {
                    display: flex;
                    width: 100px;
                    height: 200px;
                    font-size: 18px;
                }
            "#,
        );

        println!("{:?}", style_sheet);
        // let style_sheet = parser.parse(
        //     r#"
        //     /* Shoud skip comments */

        //     .header {
        //         width: 10px;
        //         font-size: 1rem;
        //     }

        //     #nav {
        //         height: 200px;
        //     }

        //     @media screen, printer, all and (max-width: 600px) {
        //         .header {
        //             display: flex;
        //         }

        //         body {
        //             max-height: 100vh;
        //         }
        //     }

        //     @media all {}
        // "#,
        // );

        // assert_eq!(
        //     style_sheet,
        //     CSSStyleSheet {
        //         css_rules: vec![
        //             CSSRule::CSSStyleRule(CSSStyleRule {
        //                 selector: CSSSelector::from_str_slice(vec![".", "header"]),
        //                 style_declarations: vec![
        //                     CSSStyleDeclaration {
        //                         name: "width".to_string(),
        //                         value: "10px".to_string()
        //                     },
        //                     CSSStyleDeclaration {
        //                         name: "font-size".to_string(),
        //                         value: "1rem".to_string()
        //                     }
        //                 ],
        //             }),
        //             CSSRule::CSSStyleRule(CSSStyleRule {
        //                 selector: CSSSelector::from_str_slice(vec!["#", "nav"]),
        //                 style_declarations: vec![CSSStyleDeclaration {
        //                     name: "height".to_string(),
        //                     value: "200px".to_string()
        //                 }],
        //             }),
        //             CSSRule::CSSMeidaRule(CSSMeidaRule {
        //                 conditions: vec![
        //                     MediaCondition::Type(MediaType::Screen),
        //                     MediaCondition::Type(MediaType::Printer),
        //                     MediaCondition::Type(MediaType::All),
        //                     MediaCondition::Feature(MediaFeature {
        //                         name: "max-width".to_string(),
        //                         value: Some("600px".to_string())
        //                     })
        //                 ],
        //                 css_rules: vec![
        //                     CSSRule::CSSStyleRule(CSSStyleRule {
        //                         selector: CSSSelector::from_str_slice(vec![".", "header"]),
        //                         style_declarations: vec![CSSStyleDeclaration {
        //                             name: "display".to_string(),
        //                             value: "flex".to_string()
        //                         }],
        //                     }),
        //                     CSSRule::CSSStyleRule(CSSStyleRule {
        //                         selector: CSSSelector::from_str_slice(vec!["body"]),
        //                         style_declarations: vec![CSSStyleDeclaration {
        //                             name: "max-height".to_string(),
        //                             value: "100vh".to_string()
        //                         }],
        //                     })
        //                 ],
        //             }),
        //             CSSRule::CSSMeidaRule(CSSMeidaRule {
        //                 conditions: vec![MediaCondition::Type(MediaType::All)],
        //                 css_rules: Vec::new(),
        //             })
        //         ]
        //     }
        // )
    }
}
