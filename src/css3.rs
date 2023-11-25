//! CSS3 tokenizer and parser
//!
//! # Token Reference
//!
//! ```txt
//! IDENT           {ident}
//! ATKEYWORD       @{ident}
//! STRING          {string}
//! INVALID         {invalid}
//! HASH            #{name}
//! NUMBER          {num}
//! PERCENTAGE      {num}%
//! DIMENSION       {num}{ident}
//! URI                url\({w}{string}{w}\)
//!                 |  url\({w}([!#$%&*-~]|{nonascii}|{escape})*{w}\)   
//! UNICODE-RANGE   U\+[0-9A-F?]{1,6}(-[0-9A-F]{1,6})?
//! CDO             <!--
//! CDC             -->
//! SEMICOLON       ;
//! LCURLY          \{
//! RCURLY          \
//! LPAREN          \(
//! RPAREN          \)
//! LBRACKET        \[
//! RBRACKET        \]
//! S               [ \t\r\n\f]+
//! COMMENT         \/\*[^*]*\*+([^/*][^*]*\*+)*\/
//! FUNCTION        {ident}\(
//! INCLUDES        ~=
//! DASHMATCH       |=
//! DELIM           any other character not matched by the above rules, and neither a single nor a double quotes
//!
//! The macros in curly braces ({}) above are defined as follows:
//!
//! Macro Definition
//! ================
//! ident       [-]?{nmstart}{nmchar}*
//! name        {nmchar}+
//! nmstart     [_a-zA-Z] | {nonascii} | {escape}
//! nonascii    [^\0-\177]
//! unicode     \\[0-9a-f]{1,6}(\r\n | [ \n\r\t\f])?
//! escape      {unicode} | \\[^\n\r\f0-9a-f]
//! nmchar      [_a-zA-Z0-9-] | {nonascii} | {escape}
//! num         [0-9]+ | [0-9]*\.[0-9]+
//! string      {string_1} | {string_2}
//! string_1     \"([^\n\r\f\\"]|\\{nl} | {escape})*\"
//! string_2     \'([^\n\r\f\\']|\\{nl} | {escape})*\'
//! invalid     {invalid_1} | {invalid_2}
//! invalid_1    \"([^\n\r\f\\"] | \\{nl} | {escape})*
//! invalid_2    \'([^\n\r\f\\'] | \\{nl} | {escape})*
//! nl          \n |\r\n |\r |\f
//! w           [ \t\r\n\f]*
//! ```
//!
//! # Backus Naur Form (BNF)
//!
//! ```txt
//! StyleSheet
//!     : [CDO | CDC | Statement]
//!     ;
//!
//! Statement
//!     : RuleSet
//!     | AtRule
//!     ;
//!
//! AtRule
//!     : ATKEYWORD Any* [Block | SEMICOLON]
//!     ;
//!
//! Block
//!     : LCURLY [Any | Block | ATKEYWORD | SEMICOLON]* RCURLY
//!     ;
//!
//! RuleSet
//!     : Selector? LCURLY Declaration? [SEMICOLON Declaration?]* RCURLY
//!     ;
//!
//! Selector
//!     : Any+
//!     ;
//!
//! Declaration
//!     : DELIM? Property COLON Value
//!     ;
//!
//! Property
//!     : IDENTY
//!     ;
//!
//! Value
//!     : [Any | Block | ATKEYWORD]+
//!     ;
//!
//! Any
//! :   [   IDENT
//!     |   NUMBER
//!     |   PERCENTAGE
//!     |   DIMENSION
//!     |   STRING
//!     |   DELIM
//!     |   URI
//!     |   HASH
//!     |   UNICODE-RANGE
//!     |   INCLUDES
//!     |   DASHMATCH
//!     |   FUNCTION Any* RPAREN
//!     |   LPAREN Any* RPAREN
//!     |   LBRACKET Any* RBRACKET
//!     ]
//! ;
//! ```
//!
use self::{node::StyleSheet, parser::CSS3Parser};
use crate::types::Result;

pub mod new_parser;
pub mod new_tokenizer;
pub mod node;
pub mod parser;
pub mod tokenizer;
pub mod tokens;
pub mod unicode;

pub fn parse(s: &str) -> Result<StyleSheet> {
    CSS3Parser::new().parse(s)
}
