use super::{
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

    pub fn parse(&mut self, raw: &str) -> CSSStyleSheet {
        self.raw = raw.to_string();
        self.tokenizer.init(raw);
        self.lookahead = self.tokenizer.get_next_token();

        self.css_style_sheet()
    }

    /// ```txt
    /// CSSSytleSheet
    ///     : CSSRulesList
    ///     ;
    /// ```
    fn css_style_sheet(&mut self) -> CSSStyleSheet {
        CSSStyleSheet {
            css_rules: self.css_rules_list(),
        }
    }

    /// ```txt
    /// CSSRulesList
    ///     : CSSRule ....
    ///     ;
    /// ```
    fn css_rules_list(&mut self) -> Vec<CSSRule> {
        let mut rules: Vec<CSSRule> = Vec::new();

        println!("[css_rules_list]: {:#?}", self.lookahead);

        while !self.is_next_token(TokenType::ClosingBracket) & self.lookahead.is_some() {
            rules.push(self.css_rule());
        }

        rules
    }

    /// ```txt
    /// CSSRule
    ///     : CSSMediaRule
    ///     | CSSStyleRule
    ///     ;
    /// ```
    fn css_rule(&mut self) -> CSSRule {
        if let Some(token) = self.lookahead.clone() {
            match token.token_type {
                TokenType::Media => return CSSRule::CSSMeidaRule(self.css_media_rule()),
                _ => return CSSRule::CSSStyleRule(self.css_style_rule()),
            }
        }

        panic!("Unexpected end of input.");
    }

    /// ```txt
    /// CSSMediaRule
    ///     : '@media' MediaCondition '{' CSSRulesList '}'
    ///     ;
    /// ```
    fn css_media_rule(&mut self) -> CSSMeidaRule {
        self.consume(TokenType::Media);
        let conditions = self.media_conditions_list();
        self.consume(TokenType::OpeningBracket);
        let css_rules = if !self.is_next_token(TokenType::ClosingBracket) {
            self.css_rules_list()
        } else {
            Vec::new()
        };
        self.consume(TokenType::ClosingBracket);

        CSSMeidaRule {
            conditions,
            css_rules,
        }
    }

    fn media_list(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// ```txt
    /// MediaConditionsList
    ///     : MediaCondition (',' | 'and') MediaCondtion  
    ///     ;
    /// ```
    fn media_conditions_list(&mut self) -> Vec<MediaCondition> {
        let mut conditions = Vec::new();

        while !self.is_next_token(TokenType::OpeningBracket) {
            conditions.push(self.media_condition());

            if self.is_next_tokens(vec![TokenType::Comma, TokenType::And]) {
                self.consume(self.get_next_token_type().unwrap());
            }
        }

        conditions
    }

    /// ```txt
    /// MediaCondition
    ///     : MedaiType
    ///     | MediaFeature
    ///     ;
    /// ```
    fn media_condition(&mut self) -> MediaCondition {
        if self.is_next_token(TokenType::OpeningParenthesis) {
            return MediaCondition::Feature(self.media_feature());
        };

        MediaCondition::Type(self.media_type())
    }

    /// ```txt
    /// MediaType
    ///     : 'screen'
    ///     | 'print'
    ///     | 'all'
    ///     ;
    /// ```
    fn media_type(&mut self) -> MediaType {
        if self.is_next_tokens(vec![
            TokenType::MediaScreenType,
            TokenType::MediaPrinterType,
            TokenType::MediaAllType,
        ]) {
            return match self.consume(self.get_next_token_type().unwrap()).token_type {
                TokenType::MediaPrinterType => MediaType::Printer,
                TokenType::MediaScreenType => MediaType::Screen,
                TokenType::MediaAllType => MediaType::All,
                _ => {
                    panic!("[media_type] unexpected token")
                }
            };
        }

        panic!("[media_type] unexpected token")
    }

    fn media_feature(&mut self) -> MediaFeature {
        self.consume(TokenType::OpeningParenthesis);

        let name = self.consume(TokenType::Identifier).value;
        let value = if !self.is_next_token(TokenType::ClosingParenthesis) {
            self.consume(TokenType::Colon);
            Some(self.consume(TokenType::Identifier).value)
        } else {
            None
        };

        self.consume(TokenType::ClosingParenthesis);

        MediaFeature { name, value }
    }

    /// ```txt
    /// CSSStyleRule
    ///     : CSSSelector '{' CSSStyleDeclarationList '}'
    ///     ;
    /// ```
    fn css_style_rule(&mut self) -> CSSStyleRule {
        let selector = self.css_selector();
        println!("{:#?}", selector);
        self.consume(TokenType::OpeningBracket);
        let style_declarations = self.css_style_declaration_list();
        self.consume(TokenType::ClosingBracket);

        CSSStyleRule {
            selector,
            style_declarations,
        }
    }

    fn css_selector(&mut self) -> CSSSelector {
        let mut path = Vec::new();

        // todo: add better handling for the selectors
        while !self.is_next_token(TokenType::OpeningBracket) {
            match self.get_next_token_type() {
                Some(token_type) => path.push(self.consume(token_type).value),
                None => break,
            }
        }

        CSSSelector { path }
    }

    /// ```txt
    /// CSSStyleDeclarationList
    ///     :  CSSStyleDeclaration ...
    ///     ;
    /// ```
    fn css_style_declaration_list(&mut self) -> Vec<CSSStyleDeclaration> {
        let mut list = Vec::new();

        // todo: add condition
        while self.is_next_token(TokenType::Identifier) {
            list.push(self.css_style_declaration());
        }

        list
    }

    /// ```txt
    /// CSSStyleDeclaration
    ///     : PropName ':'  PropValue ';'
    ///     ;
    /// ```
    fn css_style_declaration(&mut self) -> CSSStyleDeclaration {
        let name = self.prop_name();
        self.consume(TokenType::Colon);
        let value = self.prop_value();
        self.consume(TokenType::SimiColon);

        CSSStyleDeclaration { name, value }
    }

    /// ```txt
    /// PropName
    ///     : String
    ///     ;
    /// ```
    fn prop_name(&mut self) -> String {
        let token = self.consume(TokenType::Identifier);
        token.value.to_string()
    }

    /// ```txt
    /// PropValue
    ///     : String
    ///     ;
    /// ```
    fn prop_value(&mut self) -> String {
        let token = self.consume(TokenType::Identifier);
        token.value.to_string()
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
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn should_parse_css() {
        let mut parser = CSS3Parser::new();
        let style_sheet = parser.parse(
            r#"
            /* Shoud skip comments */

            .header {
                width: 10px;
                font-size: 1rem;
            } 

            #nav {
                height: 200px;
            }

            @media screen, printer, all and (max-width: 600px) {
                .header {
                    display: flex;
                }

                body {
                    max-height: 100vh;
                }
            }


            @media all {}
        "#,
        );

        assert_eq!(
            style_sheet,
            CSSStyleSheet {
                css_rules: vec![
                    CSSRule::CSSStyleRule(CSSStyleRule {
                        selector: CSSSelector::from_str_slice(vec![".", "header"]),
                        style_declarations: vec![
                            CSSStyleDeclaration {
                                name: "width".to_string(),
                                value: "10px".to_string()
                            },
                            CSSStyleDeclaration {
                                name: "font-size".to_string(),
                                value: "1rem".to_string()
                            }
                        ],
                    }),
                    CSSRule::CSSStyleRule(CSSStyleRule {
                        selector: CSSSelector::from_str_slice(vec!["#", "nav"]),
                        style_declarations: vec![CSSStyleDeclaration {
                            name: "height".to_string(),
                            value: "200px".to_string()
                        }],
                    }),
                    CSSRule::CSSMeidaRule(CSSMeidaRule {
                        conditions: vec![
                            MediaCondition::Type(MediaType::Screen),
                            MediaCondition::Type(MediaType::Printer),
                            MediaCondition::Type(MediaType::All),
                            MediaCondition::Feature(MediaFeature {
                                name: "max-width".to_string(),
                                value: Some("600px".to_string())
                            })
                        ],
                        css_rules: vec![
                            CSSRule::CSSStyleRule(CSSStyleRule {
                                selector: CSSSelector::from_str_slice(vec![".", "header"]),
                                style_declarations: vec![CSSStyleDeclaration {
                                    name: "display".to_string(),
                                    value: "flex".to_string()
                                }],
                            }),
                            CSSRule::CSSStyleRule(CSSStyleRule {
                                selector: CSSSelector::from_str_slice(vec!["body"]),
                                style_declarations: vec![CSSStyleDeclaration {
                                    name: "max-height".to_string(),
                                    value: "100vh".to_string()
                                }],
                            })
                        ],
                    }),
                    CSSRule::CSSMeidaRule(CSSMeidaRule {
                        conditions: vec![MediaCondition::Type(MediaType::All)],
                        css_rules: Vec::new(),
                    })
                ]
            }
        )
    }
}
