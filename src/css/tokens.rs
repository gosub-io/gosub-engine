#[derive(PartialEq, Debug, Clone, Copy)]
pub enum TokenType {
    // Dot,
    // GreaterThan,
    // LessThan,
    // Identifier,
    // OpeningBracket,
    // ClosingBracket,
    // OpeningParenthesis,
    // ClosingParenthesis,
    // Colon,
    // SimiColon,
    // Comma,
    // And,
    // Hash,
    // Media,
    // MediaPrinterType,
    // MediaScreenType,
    // MediaAllType,
    At,
    Hash,
    Dot,
    Number,
    String,
    CDO,
    CDC,
    Semicolon,
    Colon,
    LCurly,
    RCurly,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comment,
    Includes,
    Dashmatch,
    Ident,
    // todo: add unicode & ascii
}

#[derive(PartialEq, Debug, Clone)]
pub struct Token {
    pub token_type: TokenType,
    pub value: String,
}

impl Token {
    pub fn new(token_type: TokenType, value: String) -> Token {
        Token { token_type, value }
    }
}

// todo: support float numbers: ^([0-9]*\.[0-9]+)|[0-9]+
static NUMBER: &str = r"^[0-9]+";
static NUM_CHAR: &str = r"[_a-zA-Z0-9-]";
static NM_START: &str = r"[_a-zA-Z]";
static IDENT: &str = r"^[-]?[_a-zA-Z][_a-zA-Z0-9-]*";
static NAME: &str = r"^[_a-zA-Z0-9-]*";
static HASH: &str = r"^#";
static AT: &str = r"^@";
static DOT: &str = r"^\.";
static SEMICOLON: &str = r"^;";
static COLON: &str = r"^:";
static LCURLY: &str = r"^\{";
static RCURLY: &str = r"^\}";
static SPACE: &str = r"^[ \t\r\n\f]+";

pub static TOKEN_REFS: [(&str, Option<TokenType>); 10] = [
    (SPACE, None),
    (NUMBER, Some(TokenType::Number)),
    (IDENT, Some(TokenType::Ident)),
    (HASH, Some(TokenType::Hash)),
    (AT, Some(TokenType::At)),
    (DOT, Some(TokenType::Dot)),
    (SEMICOLON, Some(TokenType::Semicolon)),
    (LCURLY, Some(TokenType::LCurly)),
    (RCURLY, Some(TokenType::RCurly)),
    (COLON, Some(TokenType::Colon)),
    // (r"^>", Some(TokenType::GreaterThan)),
    // (r"^<", Some(TokenType::LessThan)),
    // (r"^\{", Some(TokenType::OpeningBracket)),
    // (r"^\}", Some(TokenType::ClosingBracket)),
    // (r"^\(", Some(TokenType::OpeningParenthesis)),
    // (r"^\)", Some(TokenType::ClosingParenthesis)),
    // (r"^,", Some(TokenType::Comma)),
    // (r"^:", Some(TokenType::Colon)),
    // (r"^;", Some(TokenType::SimiColon)),
    // (r"^#", Some(TokenType::Hash)),
    // // Media queries tokens
    // (r"^@media", Some(TokenType::Media)),
    // (r"^printer", Some(TokenType::MediaPrinterType)),
    // (r"^screen", Some(TokenType::MediaScreenType)),
    // (r"^all", Some(TokenType::MediaAllType)),
    // (r"^and", Some(TokenType::And)),
    // // General tokens
    // (r"^(\w|-)+", Some(TokenType::Identifier)),
    // (r"^\s+", None),
    // (r"^\/\*[^\/]*\*\/", None),
];
