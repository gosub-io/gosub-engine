/// These are the states in which the tokenizer can be in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// 8.2.4.36 After attribute name state
    AfterAttributeName,

    /// 8.2.4.42 After attribute value (quoted) state
    AfterAttributeValueQuoted,

    /// 8.2.4.55 After DOCTYPE name state
    AfterDOCTYPEName,

    /// 8.2.4.60 After DOCTYPE public identifier state
    AfterDOCTYPEPublicIdentifier,

    /// 8.2.4.56 After DOCTYPE public keyword state
    AfterDOCTYPEPublicKeyword,

    /// 8.2.4.66 After DOCTYPE system identifier state
    AfterDOCTYPESystemIdentifier,

    /// 8.2.4.62 After DOCTYPE system keyword state
    AfterDOCTYPESystemKeyword,

    /// 8.2.4.35 Attribute name state
    AttributeName,

    /// 8.2.4.38 Attribute value (double-quoted) state
    AttributeValueDoubleQuoted,

    /// 8.2.4.39 Attribute value (single-quoted) state
    AttributeValueSingleQuoted,

    /// 8.2.4.40 Attribute value (unquoted) state
    AttributeValueUnquoted,

    /// 8.2.4.34 Before attribute name state
    BeforeAttributeName,

    /// 8.2.4.37 Before attribute value state
    BeforeAttributeValue,

    /// 8.2.4.53 Before DOCTYPE name state
    BeforeDOCTYPEName,

    /// 8.2.4.57 Before DOCTYPE public identifier state
    BeforeDOCTYPEPublicIdentifier,

    /// 8.2.4.63 Before DOCTYPE system identifier state
    BeforeDOCTYPESystemIdentifier,

    /// 8.2.4.61 Between DOCTYPE public and system identifiers state
    BetweenDOCTYPEPublicAndSystemIdentifiers,

    /// 8.2.4.44 Bogus comment state
    BogusComment,

    /// 8.2.4.67 Bogus DOCTYPE state
    BogusDOCTYPE,

    /// 8.2.4.68 CDATA section state
    CDATASection,

    CDATASectionBracket,
    CDATASectionEnd,

    /// 8.2.4.41 Character reference in attribute value state
    CharacterReferenceInAttributeValue,

    /// 8.2.4.2 Character reference in data state
    CharacterReferenceInData,

    /// 8.2.4.4 Character reference in RCDATA state
    CharacterReferenceInRcData,

    /// 8.2.4.48 Comment state
    Comment,

    /// 8.2.4.50 Comment end state
    CommentEnd,

    /// 8.2.4.51 Comment end bang state
    CommentEndBang,

    /// 8.2.4.49 Comment end dash state
    CommentEndDash,

    CommentLessThanSign,
    CommentLessThanSignBang,
    CommentLessThanSignBangDash,
    CommentLessThanSignBangDashDash,

    /// 8.2.4.46 Comment start state
    CommentStart,

    /// 8.2.4.47 Comment start dash state
    CommentStartDash,

    /// 8.2.4.1 Data state
    Data,

    /// 8.2.4.52 DOCTYPE state
    DOCTYPE,

    /// 8.2.4.54 DOCTYPE name state
    DOCTYPEName,

    /// 8.2.4.58 DOCTYPE public identifier (double-quoted) state
    DOCTYPEPublicIdentifierDoubleQuoted,

    /// 8.2.4.59 DOCTYPE public identifier (single-quoted) state
    DOCTYPEPublicIdentifierSingleQuoted,

    /// 8.2.4.64 DOCTYPE system identifier (double-quoted) state
    DOCTYPESystemIdentifierDoubleQuoted,

    /// 8.2.4.65 DOCTYPE system identifier (single-quoted) state
    DOCTYPESystemIdentifierSingleQuoted,

    EndTagOpen,

    /// 8.2.4.45 Markup declaration open state
    MarkupDeclarationOpen,

    /// 8.2.4.7 PLAINTEXT state
    PLAINTEXT,

    /// 8.2.4.5 RAWTEXT state
    RAWTEXT,

    /// 8.2.4.16 RAWTEXT end tag name state
    RAWTEXTEndTagName,

    /// 8.2.4.15 RAWTEXT end tag open state
    RAWTEXTEndTagOpen,

    /// 8.2.4.14 RAWTEXT less-than sign state
    RAWTEXTLessThanSign,

    /// 8.2.4.3 RCDATA state
    RCDATA,

    /// 8.2.4.13 RCDATA end tag name state
    RCDATAEndTagName,

    /// 8.2.4.12 RCDATA end tag open state
    RCDATAEndTagOpen,

    /// 8.2.4.11 RCDATA less-than sign state
    RCDATALessThanSign,

    /// 8.2.4.6 Script data state
    ScriptData,

    /// 8.2.4.29 Script data double escaped state
    ScriptDataDoubleEscaped,

    /// 8.2.4.30 Script data double escaped dash state
    ScriptDataDoubleEscapedDash,

    /// 8.2.4.31 Script data double escaped dash dash state
    ScriptDataDoubleEscapedDashDash,

    /// 8.2.4.32 Script data double escaped less-than sign state
    ScriptDataDoubleEscapedLessThanSign,

    /// 8.2.4.33 Script data double escape end state
    ScriptDataDoubleEscapeEnd,

    /// 8.2.4.28 Script data double escape start state
    ScriptDataDoubleEscapeStart,

    /// 8.2.4.19 Script data end tag name state
    ScriptDataEndTagName,

    /// 8.2.4.18 Script data end tag open state
    ScriptDataEndTagOpen,

    /// 8.2.4.22 Script data escaped state
    ScriptDataEscaped,

    /// 8.2.4.23 Script data escaped dash state
    ScriptDataEscapedDash,

    /// 8.2.4.24 Script data escaped dash dash state
    ScriptDataEscapedDashDash,

    /// 8.2.4.27 Script data escaped end tag name state
    ScriptDataEscapedEndTagName,

    /// 8.2.4.26 Script data escaped end tag open state
    ScriptDataEscapedEndTagOpen,

    /// 8.2.4.25 Script data escaped less-than sign state
    ScriptDataEscapedLessThanSign,

    /// 8.2.4.20 Script data escape start state
    ScriptDataEscapeStart,

    /// 8.2.4.21 Script data escape start dash state
    ScriptDataEscapeStartDash,

    /// 8.2.4.17 Script data less-than sign state
    ScriptDataLessThenSign,

    /// 8.2.4.43 Self-closing start tag state
    SelfClosingStart,

    /// 8.2.4.10 Tag name state
    TagName,

    /// 8.2.4.8 Tag open state
    TagOpen,
}
