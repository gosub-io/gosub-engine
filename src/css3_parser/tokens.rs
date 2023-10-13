#[derive(PartialEq, Debug, Clone, Copy)]
pub enum TokenType {
    Dot,
    GreaterThan,
    LessThan,
    Identifier,
    OpeningBracket,
    ClosingBracket,
    OpeningParenthesis,
    ClosingParenthesis,
    Colon,
    SimiColon,
    Comma,
    And,
    Hash,
    Media,
    MediaPrinterType,
    MediaScreenType,
    MediaAllType,
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

pub static TOKENS_REGEXS: [(&'static str, Option<TokenType>); 19] = [
    (r"^\.", Some(TokenType::Dot)),
    (r"^>", Some(TokenType::GreaterThan)),
    (r"^<", Some(TokenType::LessThan)),
    (r"^\{", Some(TokenType::OpeningBracket)),
    (r"^\}", Some(TokenType::ClosingBracket)),
    (r"^\(", Some(TokenType::OpeningParenthesis)),
    (r"^\)", Some(TokenType::ClosingParenthesis)),
    (r"^,", Some(TokenType::Comma)),
    (r"^:", Some(TokenType::Colon)),
    (r"^;", Some(TokenType::SimiColon)),
    (r"^#", Some(TokenType::Hash)),
    // Media queries tokens
    (r"^@media", Some(TokenType::Media)),
    (r"^printer", Some(TokenType::MediaPrinterType)),
    (r"^screen", Some(TokenType::MediaScreenType)),
    (r"^all", Some(TokenType::MediaAllType)),
    (r"^and", Some(TokenType::And)),
    // General tokens
    (r"^(\w|-)+", Some(TokenType::Identifier)),
    (r"^\s+", None),
    (r"^\/\*[^\/]*\*\/", None),
];
