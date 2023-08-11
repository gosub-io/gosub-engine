use crate::html_parser::input_stream::InputStream;
use crate::html_parser::token_states::State;

// Constants that are not directly captured as visible chars
pub(crate) const CHAR_TAB: char = '\u{0009}';
pub(crate) const CHAR_LF: char = '\u{000A}';
pub(crate) const CHAR_FF: char = '\u{000C}';
pub(crate) const CHAR_REPLACEMENT: char = '\u{FFFD}';

// Different tokens types that can be emitted by the tokenizer
pub(crate) enum Token {
    EOF,                // End of file
    None,               // No token (?)
    Character(char),    // Single character (?)
    String(String),     // String of characters
}

// The tokenizer will read the input stream and emit tokens that can be used by the parser.
pub struct Tokenizer {
    pub stream: InputStream,            // HTML character input stream
    pub state: State,                   // Current state of the tokenizer
    pub consumed: Vec<char>,            // Current consumed characters for current token
}

// macro_rules! read_char {
//     () => {
//         match self.stream.read_char() {
//             Ok(c) => c,
//             Err(e) => {
//                 self.parse_error("unexpected error: {e}");
//                 return
//             },
//         }
//     };
// }

impl Tokenizer {

    pub fn new(input: InputStream) -> Self {
        return Tokenizer{
            stream: input,
            state: State::DataState,
            consumed: vec![],
        }
    }
    // Retrieves the next token from the input stream or Token::EOF when the end is reached
    pub(crate) fn next_token(&mut self) -> Token {
        match self.state {
            State::DataState => {
                let c = match self.stream.read_char() {
                    Some(c) => c,
                    None => {
                        self.parse_error("EOF");
                        return Token::None;
                    }
                };

                match c {
                    '&' => self.state = State::CharacterReferenceInDataState,
                    '<' => self.state = State::TagOpenState,
                    _ => return Token::String(String::from(c)),
                }
            }
            State::CharacterReferenceInDataState => {
                // consume character references
                self.consume_character_reference(None);
                self.state = State::DataState;
                return Token::String(self.consumed.iter().collect());
            }
            State::RcDataState => {}
            State::CharacterReferenceInRcDataState => {}
            State::RawTextState => {}
            State::ScriptDataState => {}
            State::PlaintextState => {}
            State::TagOpenState => {}
            State::EndTagOpenState => {}
            State::TagNameState => {}
            State::RcDataLessThanSignState => {}
            State::RcDataEndTagOpenState => {}
            State::RcDataEndTagNameState => {}
            State::RawTextLessThanSignState => {}
            State::RawTextEndTagOpenState => {}
            State::RawTextEndTagNameState => {}
            State::ScriptDataLessThenSignState => {}
            State::ScriptDataEndTagOpenState => {}
            State::ScriptDataEndTagNameState => {}
            State::ScriptDataEscapeStartState => {}
            State::ScriptDataEscapeStartDashState => {}
            State::ScriptDataEscapedState => {}
            State::ScriptDataEscapedDashState => {}
            State::ScriptDataEscapedLessThanSignState => {}
            State::ScriptDataEscapedEndTagOpenState => {}
            State::ScriptDataEscapedEndTagNameState => {}
            State::ScriptDataDoubleEscapeStartState => {}
            State::ScriptDataDoubleEscapedState => {}
            State::ScriptDataDoubleEscapedDashState => {}
            State::ScriptDataDoubleEscapedDashDashState => {}
            State::ScriptDataDoubleEscapedLessThanSignState => {}
            State::ScriptDataDoubleEscapeEndState => {}
            State::BeforeAttributeNameState => {}
            State::AttributeNameState => {}
            State::BeforeAttributeValueState => {}
            State::AttributeValueDoubleQuotedState => {}
            State::AttributeValueSingleQuotedState => {}
            State::AttributeValueUnquotedState => {}
            State::CharacterReferenceInAttributeValueState => {}
            State::AfterAttributeValueQuotedState => {}
            State::SelfClosingStartState => {}
            State::BogusCommentState => {}
            State::MarkupDeclarationOpenState => {}
            State::CommentStartState => {}
            State::CommentStartDashState => {}
            State::CommentState => {}
            State::CommentEndDashState => {}
            State::CommentEndState => {}
            State::CommentEndBangState => {}
            State::DocTypeState => {}
            State::BeforeDocTypeNameState => {}
            State::DocTypeNameState => {}
            State::AfterDocTypeNameState => {}
            State::AfterDocTypePublicKeywordState => {}
            State::BeforeDocTypePublicIdentifierState => {}
            State::DocTypePublicIdentifierDoubleQuotedState => {}
            State::DocTypePublicIdentifierSingleQuotedState => {}
            State::AfterDoctypePublicIdentifierState => {}
            State::BetweenDocTypePublicAndSystemIdentifiersState => {}
            State::AfterDocTypeSystemKeywordState => {}
            State::BeforeDocTypeSystemIdentifiedState => {}
            State::DocTypeSystemIdentifierDoubleQuotedState => {}
            State::DocTypeSystemIdentifierSingleQuotedState => {}
            State::AfterDocTypeSystemIdentifiedState => {}
            State::BogusDocTypeState => {}
            State::CDataSectionState => {}
        }

        return Token::None
    }

    // Consumes the given char so it can be stored in the next output token
    pub(crate) fn consume(&mut self, c: char) {
        // Add c to the current token data
        self.consumed.push(c)
    }

    // Return the length of the current consumed array. This allows easy return to a previous
    // state if tokenizing needs to return.
    pub(crate) fn get_consume_len(&self) -> usize {
        return self.consumed.len();
    }

    // Resize the consumed array to the given len. Useful when we need to backtrack to a previous consumption state
    pub(crate) fn reset_consume_len(&mut self, len: usize)
    {
        self.consumed.resize(len, 0 as char);
    }

    // Clears the current consume buffer
    pub(crate) fn clear_consume_buffer(&mut self) {
        self.consumed.clear()
    }

    // Creates a parser log error message
    pub(crate) fn parse_error(&mut self, _str: &str) {
        // Add to parse log
    }
}