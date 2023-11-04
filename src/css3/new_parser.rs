use super::new_tokenizer::{Token, TokenKind};
use crate::{bytes::CharIterator, css3::new_tokenizer::Tokenizer};

#[derive(Debug, PartialEq)]
struct Function {
    name: String,
    value: Vec<ComponentValue>,
}

#[derive(Debug, PartialEq)]
enum SimpleBlockTokenKind {
    Curly,
    Bracket,
    Paren,
}

#[derive(Debug, PartialEq)]
struct SimpleBlock {
    kind: SimpleBlockTokenKind,
    value: Vec<ComponentValue>,
}

#[derive(Debug, PartialEq)]
struct AtRule {
    name: String,
    prelude: Vec<ComponentValue>,
    block: Option<SimpleBlock>,
}

#[derive(Debug, PartialEq)]
struct QualifiedRule {
    prelude: Vec<ComponentValue>,
    block: SimpleBlock,
}

#[derive(Debug, PartialEq)]
struct Declaration {
    name: String,
    value: Vec<ComponentValue>,
    important: bool,
}

#[derive(Debug, PartialEq)]
enum ComponentValue {
    /// Any token expect for `<function-token>`, `<{-token>`, `<(-token>`, and `<[-token>` (which are consumed in other higher-level objects)
    ///
    /// Note: `<}-token>`, `<)-token>`, `<]-token>`, `<bad-string-token>`, and `<bad-url-token>` are always parse errors.
    Token(Token),
    Function(Function),
    SimpleBlock(SimpleBlock),
}

// Parser output: at-rules, qualified rules, and/or declarations
pub struct CSS3Parser<'stream> {
    tokenizer: Tokenizer<'stream>,
}

impl<'stream> CSS3Parser<'stream> {
    pub fn new(tokenizer: Tokenizer) -> CSS3Parser {
        CSS3Parser { tokenizer }
    }

    pub fn from_input_stream(ci: &mut CharIterator) -> CSS3Parser {
        CSS3Parser::new(Tokenizer::new(ci))
    }

    /// [5.3.1. Parse something according to a CSS grammar](https://www.w3.org/TR/css-syntax-3/#parse-grammar)
    fn parse() {
        todo!()
    }

    /// [5.3.2. Parse A Comma-Separated List According To A CSS Grammar](https://www.w3.org/TR/css-syntax-3/#parse-comma-list)
    fn parse_comma_separated_list() {
        todo!()
    }

    /// [5.4.6. Consume a declaration](https://www.w3.org/TR/css-syntax-3/#consume-declaration)
    fn consume_declaration(&mut self) -> Option<Declaration> {
        let name = self.consume_token(TokenKind::Any).value();
        let mut value = Vec::new();

        while self.current_token().is_whitespace() {
            self.consume_token(TokenKind::Any);
        }

        // parser error
        if self.current_token().kind() != TokenKind::Semicolon {
            return None;
        }

        self.consume_token(TokenKind::Semicolon);

        while self.current_token().is_whitespace() {
            self.consume_token(TokenKind::Any);
        }

        while !self.current_token().is_eof() {
            value.push(self.consume_component_value())
        }

        let len = value.len();

        let important = len > 2
            && value.last()
                == Some(&ComponentValue::Token(Token::Ident(
                    "important".to_string(),
                )))
            && value.get(len - 2) == Some(&ComponentValue::Token(Token::Delim('!')));

        Some(Declaration {
            name,
            value,
            important,
        })
    }

    /// [5.4.7. Consume a component value](https://www.w3.org/TR/css-syntax-3/#consume-a-component-value)
    fn consume_component_value(&mut self) -> ComponentValue {
        let token = self.consume_token(TokenKind::Any);

        match token.kind() {
            TokenKind::LCurly | TokenKind::LBracket | TokenKind::LParen => {
                ComponentValue::SimpleBlock(self.consume_simple_block(token.kind()))
            }
            TokenKind::Function => ComponentValue::Function(self.consume_function()),
            _ => ComponentValue::Token(token),
        }
    }

    /// [5.4.8. Consume a simple block](https://www.w3.org/TR/css-syntax-3/#consume-a-simple-block)
    fn consume_simple_block(&mut self, ending: TokenKind) -> SimpleBlock {
        let mut value = Vec::new();

        loop {
            // eof: parser error
            if self.current_token().kind() == ending || self.current_token().is_eof() {
                break;
            }

            value.push(self.consume_component_value())
        }

        let kind = match ending {
            TokenKind::LParen => SimpleBlockTokenKind::Paren,
            TokenKind::LCurly => SimpleBlockTokenKind::Curly,
            TokenKind::LBracket => SimpleBlockTokenKind::Bracket,
            _ => todo!(),
        };

        SimpleBlock { kind, value }
    }

    /// [5.4.9. Consume a function](https://www.w3.org/TR/css-syntax-3/#consume-function)
    fn consume_function(&mut self) -> Function {
        let name = self.consume_token(TokenKind::Function).value();
        let mut value = Vec::new();

        loop {
            let token = self.current_token();

            if token.kind() == TokenKind::LParen || token.is_eof() {
                // consume `(` or `EOF`
                self.consume_token(TokenKind::Any);
                break;
            }

            value.push(self.consume_component_value());
        }

        Function { name, value }
    }

    fn current_token(&self) -> Token {
        self.tokenizer.lookahead(0)
    }

    fn next_token(&self) -> Token {
        self.tokenizer.lookahead(1)
    }

    fn consume_token(&mut self, kind: TokenKind) -> Token {
        let token = self.tokenizer.consume();

        if kind != TokenKind::Any {
            // safeguard, not to consume unexpected token
            assert_eq!(token.kind(), kind);
        }

        token
    }
}
