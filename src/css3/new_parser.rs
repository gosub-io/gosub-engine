use super::new_tokenizer::Token;
use crate::{bytes::CharIterator, css3::new_tokenizer::Tokenizer};
use std::convert::From;

#[derive(Debug, PartialEq, Clone)]
struct Function {
    name: String,
    value: Vec<ComponentValue>,
}

#[derive(Debug, PartialEq, Clone)]
enum SimpleBlockTokenKind {
    Curly,
    Bracket,
    Paren,
}

impl From<Token> for SimpleBlockTokenKind {
    fn from(token: Token) -> SimpleBlockTokenKind {
        match token {
            _ if token.is_left_paren() => SimpleBlockTokenKind::Paren,
            _ if token.is_left_curl() => SimpleBlockTokenKind::Curly,
            _ if token.is_left_bracket() => SimpleBlockTokenKind::Bracket,
            _ => todo!(),
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
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

impl Default for QualifiedRule {
    fn default() -> Self {
        QualifiedRule {
            prelude: Vec::new(),
            block: SimpleBlock {
                kind: SimpleBlockTokenKind::Curly,
                value: Vec::new(),
            },
        }
    }
}

impl QualifiedRule {
    pub fn set_block(&mut self, block: SimpleBlock) {
        self.block = block;
    }

    pub fn add_prelude(&mut self, value: ComponentValue) {
        self.prelude.push(value)
    }
}

enum Rule {
    QualifiedRule(QualifiedRule),
    AtRule(AtRule),
}

struct StyleBlockContent {
    declarations: Vec<Declaration>,
    rules: Vec<Rule>,
}

#[derive(Debug, PartialEq)]
struct Declaration {
    name: String,
    value: Vec<ComponentValue>,
    important: bool,
}

enum DeclarationListValue {
    Declaration(Declaration),
    AtRule(AtRule),
}

#[derive(Debug, PartialEq, Clone)]
enum ComponentValue {
    /// Any token expect for `<function-token>`, `<{-token>`, `<(-token>`, and `<[-token>` (which are consumed in other higher-level objects)
    ///
    /// Note: `<}-token>`, `<)-token>`, `<]-token>`, `<bad-string-token>`, and `<bad-url-token>` are always parse errors.
    Token(Token),
    Function(Function),
    SimpleBlock(SimpleBlock),
}

impl ComponentValue {
    fn token(&self) -> Token {
        match self {
            ComponentValue::Token(t) => t.clone(),
            ComponentValue::Function(..) | ComponentValue::SimpleBlock(..) => todo!(),
        }
    }

    fn is_token(&self) -> bool {
        matches!(self, ComponentValue::Token(..))
    }

    fn is_function(&self) -> bool {
        matches!(self, ComponentValue::Function(..))
    }

    fn is_block(&self) -> bool {
        matches!(self, ComponentValue::SimpleBlock(..))
    }
}

#[derive(Debug)]
struct ValueBuffer {
    pub values: Vec<ComponentValue>,
    pub position: usize,
}

impl ValueBuffer {
    pub fn new(values: Vec<ComponentValue>) -> Self {
        Self {
            values,
            position: 0,
        }
    }

    pub fn lookahead(&self, offset: usize) -> ComponentValue {
        if self.position + offset >= self.values.len() {
            return ComponentValue::Token(Token::EOF);
        }

        self.values[self.position + offset].clone()
    }

    pub fn current(&self) -> ComponentValue {
        self.lookahead(0)
    }

    pub fn next(&self) -> ComponentValue {
        self.lookahead(1)
    }

    pub fn consume(&mut self) -> ComponentValue {
        if self.position >= self.values.len() {
            return ComponentValue::Token(Token::EOF);
        }

        let value = &self.values[self.position];
        self.position += 1;
        value.clone()
    }
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

    /// [5.3.6. Parse a declaration](https://www.w3.org/TR/css-syntax-3/#parse-declaration)
    fn parse_declaration(&self, vb: &mut ValueBuffer) -> Option<Declaration> {
        self.consume_whitespace(vb);

        // syntax error
        if !vb.current().token().is_ident() {
            return None;
        }

        // if nothing was returned from `consume_declaration`, it will be a syntax error.
        self.consume_declaration(vb)
    }

    /// [5.3.7. Parse a style block’s contents](https://www.w3.org/TR/css-syntax-3/#parse-style-blocks-contents)
    fn parse_style_block_content(&self, vb: &mut ValueBuffer) -> StyleBlockContent {
        self.consume_style_block_content(vb)
    }

    /// [5.3.8. Parse a list of declarations](https://www.w3.org/TR/css-syntax-3/#parse-list-of-declarations)
    fn parse_declaration_list(&self, vb: &mut ValueBuffer) -> Vec<DeclarationListValue> {
        self.consume_declaration_list(vb)
    }

    /// [5.3.9. Parse a component value](https://www.w3.org/TR/css-syntax-3/#parse-component-value)
    fn parse_component_value(&self, vb: &mut ValueBuffer) -> Option<ComponentValue> {
        self.consume_whitespace(vb);

        // eof: syntax error
        if vb.current().token().is_eof() {
            return None;
        }

        let value = self.consume_component_value(vb);
        self.consume_whitespace(vb);

        if vb.current().token().is_eof() {
            return Some(value);
        }

        // syntax error
        None
    }

    /// [5.3.10. Parse a list of component values](https://www.w3.org/TR/css-syntax-3/#parse-list-of-component-values)
    fn parse_component_value_list(&self, vb: &mut ValueBuffer) -> Vec<ComponentValue> {
        let mut values = Vec::new();
        loop {
            let value = self.consume_component_value(vb);

            if value.is_token() && value.token().is_eof() {
                break;
            }

            values.push(value)
        }

        values
    }

    /// [5.3.11. Parse a comma-separated list of component values](https://www.w3.org/TR/css-syntax-3/#parse-comma-list)
    fn parse_comma_separated_list(&self, vb: &mut ValueBuffer) -> Vec<Vec<ComponentValue>> {
        let mut list = Vec::new();

        loop {
            let mut values = Vec::new();
            let mut stop = true;

            loop {
                let value = self.consume_component_value(vb);
                if value.is_token() && (value.token().is_eof() || value.token().is_semicolon()) {
                    break; // final <EOF-token> or <comma-token> should not be added.
                }

                if value.token().is_comma() {
                    stop = false;
                }

                values.push(value);
            }

            list.push(values);

            if stop {
                break;
            }
        }

        list
    }

    /// [5.4.1. Consume a list of rules](https://www.w3.org/TR/css-syntax-3/#consume-list-of-rules)
    fn consume_rules_list(&mut self, vb: &mut ValueBuffer, is_top_level: bool) -> Vec<Rule> {
        let mut rules = Vec::new();

        loop {
            if vb.current().token().is_whitespace() {
                vb.consume();
                continue; // do nothing
            }

            if vb.current().token().is_eof() {
                break; // return rules list
            }

            if vb.current().token().is_cdo() || vb.current().token().is_cdc() {
                // consume `cdo` or `cdc` tokens
                vb.consume();

                if is_top_level {
                    continue; // do nothing
                }

                if let Some(rule) = self.consume_qualified_rule(vb) {
                    rules.push(Rule::QualifiedRule(rule));
                    continue;
                }
            }

            if vb.current().token().is_at_keyword() {
                rules.push(Rule::AtRule(self.consume_at_rule(vb)));
                continue;
            }

            if let Some(rule) = self.consume_qualified_rule(vb) {
                rules.push(Rule::QualifiedRule(rule));
                continue;
            }
        }

        rules
    }

    /// [5.4.2. Consume an at-rule](https://www.w3.org/TR/css-syntax-3/#consume-at-rule)
    fn consume_at_rule(&self, vb: &mut ValueBuffer) -> AtRule {
        // consume `<at-keyword-token>`
        let name = vb.consume().token().to_string();
        let mut prelude = Vec::new();
        let mut block = None;

        loop {
            // eof: parser error
            if vb.current().token().is_semicolon() || vb.current().token().is_eof() {
                vb.consume();
                break; // return the block
            }

            if vb.current().token().is_left_curl() {
                let token = vb.consume().token();
                block = Some(self.consume_simple_block(vb, &token));
                break; // return the block
            }

            prelude.push(self.consume_component_value(vb));
        }

        AtRule {
            name,
            prelude,
            block,
        }
    }

    /// [5.4.3. Consume a qualified rule](https://www.w3.org/TR/css-syntax-3/#consume-qualified-rule)
    fn consume_qualified_rule(&self, vb: &mut ValueBuffer) -> Option<QualifiedRule> {
        let mut rule = QualifiedRule::default();

        loop {
            // eof: parser error
            if vb.current().token().is_eof() {
                return None;
            }

            if vb.current().token().is_left_curl() {
                let token = vb.consume().token();
                rule.set_block(self.consume_simple_block(vb, &token));
                return Some(rule);
            }

            rule.add_prelude(self.consume_component_value(vb));
        }
    }

    /// [5.4.4. Consume a style block’s contents](https://www.w3.org/TR/css-syntax-3/#consume-style-block)
    fn consume_style_block_content(&self, vb: &mut ValueBuffer) -> StyleBlockContent {
        let mut declarations = Vec::new();
        let mut rules = Vec::new();

        loop {
            let token = vb.current().token();

            if token.is_whitespace() || token.is_semicolon() {
                vb.consume();
                continue; // do nothing
            }

            if token.is_eof() {
                // specs: Extend decls with rules, then return decls.
                break;
            }

            if token.is_at_keyword() {
                rules.push(Rule::AtRule(self.consume_at_rule(vb)))
            }

            if token.is_ident() {
                let mut components = vec![vb.consume()];

                while !vb.current().token().is_whitespace() || !vb.current().token().is_eof() {
                    components.push(self.consume_component_value(vb))
                }

                let mut components_vb = ValueBuffer::new(components);
                if let Some(decl) = self.consume_declaration(&mut components_vb) {
                    declarations.push(decl)
                }
            }

            if token == Token::Delim('&') {
                if let Some(rule) = self.consume_qualified_rule(vb) {
                    rules.push(Rule::QualifiedRule(rule))
                }
            }

            // anything else is a parser error
            // clean up: consume a component value and do nothing
            while !vb.current().token().is_eof() && !vb.current().token().is_semicolon() {
                self.consume_component_value(vb);
            }
        }

        StyleBlockContent {
            declarations,
            rules,
        }
    }

    /// [5.4.5. Consume a list of declarations](https://www.w3.org/TR/css-syntax-3/#consume-list-of-declarations)
    fn consume_declaration_list(&self, vb: &mut ValueBuffer) -> Vec<DeclarationListValue> {
        let mut declarations = Vec::new();
        loop {
            let token = vb.current().token();

            if token.is_whitespace() || token.is_semicolon() {
                vb.consume();
                continue;
            }

            if token.is_eof() {
                break;
            };

            if token.is_at_keyword() {
                declarations.push(DeclarationListValue::AtRule(self.consume_at_rule(vb)))
            }

            if token.is_ident() {
                let mut values = vec![vb.consume()];

                while !vb.current().token().is_semicolon() && !vb.current().token().is_eof() {
                    values.push(self.consume_component_value(vb))
                }

                let mut vb = ValueBuffer::new(values);
                if let Some(declaration) = self.consume_declaration(&mut vb) {
                    declarations.push(DeclarationListValue::Declaration(declaration))
                }
            }
        }

        declarations
    }

    /// [5.4.6. Consume a declaration](https://www.w3.org/TR/css-syntax-3/#consume-declaration)
    fn consume_declaration(&self, vb: &mut ValueBuffer) -> Option<Declaration> {
        let name = vb.consume().token().to_string();
        let mut value = Vec::new();

        while vb.current().token().is_whitespace() {
            vb.consume();
        }

        // parser error
        if !vb.current().token().is_semicolon() {
            return None;
        }

        vb.consume();

        while vb.current().token().is_whitespace() {
            vb.consume();
        }

        while !vb.current().token().is_eof() {
            value.push(self.consume_component_value(vb))
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
    fn consume_component_value(&self, vb: &mut ValueBuffer) -> ComponentValue {
        let token = vb.consume().token();

        match token {
            t if t.is_left_curl() || t.is_left_bracket() || t.is_left_paren() => {
                ComponentValue::SimpleBlock(self.consume_simple_block(vb, &t))
            }
            t if t.is_function() => ComponentValue::Function(self.consume_function(vb)),
            t => ComponentValue::Token(t),
        }
    }

    /// [5.4.8. Consume a simple block](https://www.w3.org/TR/css-syntax-3/#consume-a-simple-block)
    fn consume_simple_block(&self, vb: &mut ValueBuffer, ending: &Token) -> SimpleBlock {
        let mut value = Vec::new();

        loop {
            // eof: parser error
            if vb.current().token().is(ending) || vb.current().token().is_eof() {
                break;
            }

            value.push(self.consume_component_value(vb))
        }

        SimpleBlock {
            kind: SimpleBlockTokenKind::from(ending.clone()),
            value,
        }
    }

    /// [5.4.9. Consume a function](https://www.w3.org/TR/css-syntax-3/#consume-function)
    fn consume_function(&self, vb: &mut ValueBuffer) -> Function {
        let name = vb.consume().token().to_string();
        let mut value = Vec::new();

        loop {
            let token = vb.current().token();

            if token.is_left_paren() || token.is_eof() {
                // consume `(` or `EOF`
                vb.consume();
                break;
            }

            value.push(self.consume_component_value(vb));
        }

        Function { name, value }
    }

    fn consume_whitespace(&self, vb: &mut ValueBuffer) {
        while vb.current().token().is_whitespace() {
            vb.consume();
        }
    }
}
