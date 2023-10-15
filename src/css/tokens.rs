#[derive(PartialEq, Debug, Clone, Copy)]
pub enum TokenType {
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
    Important,
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
static IMPORTANT: &str = "^!important";

pub static TOKEN_REFS: [(&str, Option<TokenType>); 11] = [
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
    (IMPORTANT, Some(TokenType::Important)),
];
