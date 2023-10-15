use crate::css::node::{
    Block, BlockChild, Declaration, DeclarationList, Dimension, IdSelector, Identifier, Rule,
    Selector, SelectorList, StyleSheet, StyleSheetRule, Value, ValueList,
};
use crate::css::tokenizer::Tokenizer;
use crate::css::tokens::{Token, TokenType};

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
    ///     : IDENT COLON ValueList IMPORTANT SEMICOLON
    ///     ;   
    /// ```
    fn declaration(&mut self) -> Declaration {
        let mut declaration = Declaration::default();

        declaration.set_property(self.consume(TokenType::Ident).value);
        self.consume(TokenType::Colon);
        declaration.set_value(self.value_ist());

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
    fn value_ist(&mut self) -> ValueList {
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
                SelectorList::new(vec![Selector::IdSelector(IdSelector::new(
                    "header".to_string()
                ))]),
                Block::new(vec![BlockChild::DeclarationList(DeclarationList::new(
                    vec![
                        Declaration::new(
                            "display".to_string(),
                            ValueList::new(vec![Value::Identifier(Identifier::new(
                                "flex".to_string()
                            ))])
                        ),
                        Declaration::new(
                            "width".to_string(),
                            ValueList::new(vec![Value::Dimension(Dimension::new(
                                "100".to_string(),
                                Some("px".to_string())
                            ))])
                        ),
                        Declaration {
                            important: true,
                            property: "font-size".to_string(),
                            value: ValueList::new(vec![Value::Dimension(Dimension::new(
                                "1".to_string(),
                                Some("rem".to_string())
                            ))])
                        }
                    ]
                ))])
            ))])
        )
    }
}
