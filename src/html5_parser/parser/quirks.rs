use crate::html5_parser::parser::Html5Parser;

#[derive(PartialEq, Debug)]
pub enum QuirksMode {
    Quirks,
    LimitedQuirks,
    NoQuirks,
}

impl<'a> Html5Parser<'a> {

    // returns the correct quirk mode for the given doctype
    pub(crate) fn identify_quirks_mode(&self, name: &Option<String>, pub_identifer: Option<String>, sys_identifier: Option<String>, force_quirks: bool) -> QuirksMode
    {
        if force_quirks || name.as_ref().map_or("", |s| &s[..]).to_uppercase() != "HTML" {
            return QuirksMode::Quirks;
        }

        if pub_identifer.is_some() {
            let pub_id = pub_identifer.unwrap().to_lowercase();
            if QUIRKS_PUB_IDENTIFIER_EQ.contains(&pub_id.as_str()) {
                return QuirksMode::Quirks;
            }
            if QUIRKS_PUB_IDENTIFIER_PREFIX.iter().any(|&prefix| pub_id.as_str().starts_with(&prefix)) {
                return QuirksMode::Quirks;
            }

            if sys_identifier.is_none() {
                if QUIRKS_PUB_IDENTIFIER_PREFIX_MISSING_SYS.iter().any(|&prefix| pub_id.as_str().starts_with(&prefix)) {
                    return QuirksMode::Quirks;
                }
            }

            if LIMITED_QUIRKS_PUB_IDENTIFIER_PREFIX.iter().any(|&prefix| pub_id.as_str().starts_with(&prefix)) {
                return QuirksMode::LimitedQuirks;
            }

            if sys_identifier.is_some() {
                if LIMITED_QUIRKS_PUB_IDENTIFIER_PREFIX.iter().any(|&prefix| pub_id.as_str().starts_with(&prefix)) {
                    return QuirksMode::LimitedQuirks;
                }
            }
        }

        if sys_identifier.is_some() {
            let sys_id = sys_identifier.unwrap().to_lowercase();
            if QUIRKS_SYS_IDENTIFIER_EQ.iter().any(|&prefix| sys_id.as_str().starts_with(&prefix)) {
                return QuirksMode::Quirks;
            }
        }

        return QuirksMode::NoQuirks;
    }
}

static QUIRKS_PUB_IDENTIFIER_EQ: &'static [&'static str] = &[
    "-//W3O//DTD W3 HTML Strict 3.0//EN//",
    "-/W3C/DTD HTML 4.0 Transitional/EN",
    "HTML"
];

static QUIRKS_PUB_IDENTIFIER_PREFIX: &'static [&'static str] = &[
    "+//Silmaril//dtd html Pro v0r11 19970101//",
    "-//AS//DTD HTML 3.0 asWedit + extensions//",
    "-//AdvaSoft Ltd//DTD HTML 3.0 asWedit + extensions//",
    "-//IETF//DTD HTML 2.0 Level 1//",
    "-//IETF//DTD HTML 2.0 Level 2//",
    "-//IETF//DTD HTML 2.0 Strict Level 1//",
    "-//IETF//DTD HTML 2.0 Strict Level 2//",
    "-//IETF//DTD HTML 2.0 Strict//",
    "-//IETF//DTD HTML 2.0//",
    "-//IETF//DTD HTML 2.1E//",
    "-//IETF//DTD HTML 3.0//",
    "-//IETF//DTD HTML 3.2 Final//",
    "-//IETF//DTD HTML 3.2//",
    "-//IETF//DTD HTML 3//",
    "-//IETF//DTD HTML Level 0//",
    "-//IETF//DTD HTML Level 1//",
    "-//IETF//DTD HTML Level 2//",
    "-//IETF//DTD HTML Level 3//",
    "-//IETF//DTD HTML Strict Level 0//",
    "-//IETF//DTD HTML Strict Level 1//",
    "-//IETF//DTD HTML Strict Level 2//",
    "-//IETF//DTD HTML Strict Level 3//",
    "-//IETF//DTD HTML Strict//",
    "-//IETF//DTD HTML//",
    "-//Metrius//DTD Metrius Presentational//",
    "-//Microsoft//DTD Internet Explorer 2.0 HTML Strict//",
    "-//Microsoft//DTD Internet Explorer 2.0 HTML//",
    "-//Microsoft//DTD Internet Explorer 2.0 Tables//",
    "-//Microsoft//DTD Internet Explorer 3.0 HTML Strict//",
    "-//Microsoft//DTD Internet Explorer 3.0 HTML//",
    "-//Microsoft//DTD Internet Explorer 3.0 Tables//",
    "-//Netscape Comm. Corp.//DTD HTML//",
    "-//Netscape Comm. Corp.//DTD Strict HTML//",
    "-//O'Reilly and Associates//DTD HTML 2.0//",
    "-//O'Reilly and Associates//DTD HTML Extended 1.0//",
    "-//O'Reilly and Associates//DTD HTML Extended Relaxed 1.0//",
    "-//SQ//DTD HTML 2.0 HoTMetaL + extensions//",
    "-//SoftQuad Software//DTD HoTMetaL PRO 6.0::19990601::extensions to HTML 4.0//",
    "-//SoftQuad//DTD HoTMetaL PRO 4.0::19971010::extensions to HTML 4.0//",
    "-//Spyglass//DTD HTML 2.0 Extended//",
    "-//Sun Microsystems Corp.//DTD HotJava HTML//",
    "-//Sun Microsystems Corp.//DTD HotJava Strict HTML//",
    "-//W3C//DTD HTML 3 1995-03-24//",
    "-//W3C//DTD HTML 3.2 Draft//",
    "-//W3C//DTD HTML 3.2 Final//",
    "-//W3C//DTD HTML 3.2//",
    "-//W3C//DTD HTML 3.2S Draft//",
    "-//W3C//DTD HTML 4.0 Frameset//",
    "-//W3C//DTD HTML 4.0 Transitional//",
    "-//W3C//DTD HTML Experimental 19960712//",
    "-//W3C//DTD HTML Experimental 970421//",
    "-//W3C//DTD W3 HTML//",
    "-//W3O//DTD W3 HTML 3.0//",
    "-//WebTechs//DTD Mozilla HTML 2.0//",
    "-//WebTechs//DTD Mozilla HTML//",
];

static QUIRKS_PUB_IDENTIFIER_PREFIX_MISSING_SYS: &'static [&'static str] = &[
    "-//W3C//DTD HTML 4.01 Frameset//",
    "-//W3C//DTD HTML 4.01 Transitional//",
];

static QUIRKS_SYS_IDENTIFIER_EQ: &'static [&'static str] = &[
    "http://www.ibm.com/data/dtd/v11/ibmxhtml1-transitional.dtd"
];

static LIMITED_QUIRKS_PUB_IDENTIFIER_PREFIX: &'static [&'static str] = &[
    "-//W3C//DTD XHTML 1.0 Frameset//",
    "-//W3C//DTD XHTML 1.0 Transitional//"
];

static LIMITED_QUIRKS_PUB_IDENTIFIER_PREFIX_NOT_MISSING_SYS: &'static [&'static str] = &[
    "-//W3C//DTD HTML 4.01 Frameset//",
    "-//W3C//DTD HTML 4.01 Transitional//",
];

#[cfg(test)]
mod tests {
    use crate::html5_parser::input_stream::InputStream;
    use crate::html5_parser::parser::Html5Parser;
    use crate::html5_parser::parser::QuirksMode;

    #[test]
    fn test_quirks_mode() {
        let mut stream = InputStream::new();
        let parser = Html5Parser::new(&mut stream);

        assert_eq!(parser.identify_quirks_mode(&None, None, None, false), QuirksMode::Quirks);
        assert_eq!(parser.identify_quirks_mode(&Some("html".to_string()), None, None, false), QuirksMode::NoQuirks);
        assert_eq!(parser.identify_quirks_mode(&Some("html".to_string()), Some("-//W3O//DTD W3 HTML Strict 3.0//EN//".to_string()), None, false), QuirksMode::Quirks);
        assert_eq!(parser.identify_quirks_mode(&Some("html".to_string()), Some("-//W3C//DTD HTML 4.0 Transitional//EN".to_string()), None, false), QuirksMode::Quirks);
        assert_eq!(parser.identify_quirks_mode(&Some("html".to_string()), Some("-/W3C/DTD HTML 4.0 Transitional/EN".to_string()), None, false), QuirksMode::Quirks);
        assert_eq!(parser.identify_quirks_mode(&Some("html".to_string()), Some("-/W3C/DTD HTML 4.0 Transitional/EN".to_string()), None, false), QuirksMode::Quirks);
        assert_eq!(parser.identify_quirks_mode(&Some("html".to_string()), Some("-//W3C//DTD HTML 4.01 Frameset//".to_string()), None, false), QuirksMode::LimitedQuirks);
        assert_eq!(parser.identify_quirks_mode(&Some("html".to_string()), Some("-//W3C//DTD HTML 4.01 Transitional//".to_string()), None, false), QuirksMode::LimitedQuirks);
        assert_eq!(parser.identify_quirks_mode(&Some("html".to_string()), Some("-//W3C//DTD XHTML 1.0 Frameset//".to_string()), None, false), QuirksMode::LimitedQuirks);
    }

    #[test]
    fn test_quirks_mode_force() {
        let mut stream = InputStream::new();
        let parser = Html5Parser::new(&mut stream);

        assert_eq!(parser.identify_quirks_mode(&Some("html".to_string()), None, None, true), QuirksMode::Quirks);
        assert_eq!(parser.identify_quirks_mode(&Some("html".to_string()), Some("-//W3O//DTD W3 HTML Strict 3.0//EN//".to_string()), None, true), QuirksMode::Quirks);
        assert_eq!(parser.identify_quirks_mode(&Some("html".to_string()), Some("-//W3C//DTD HTML 4.0 Transitional//EN".to_string()), None, true), QuirksMode::Quirks);
        assert_eq!(parser.identify_quirks_mode(&Some("html".to_string()), Some("-/W3C/DTD HTML 4.0 Transitional/EN".to_string()), None, true), QuirksMode::Quirks);
        assert_eq!(parser.identify_quirks_mode(&Some("html".to_string()), Some("-/W3C/DTD HTML 4.0 Transitional/EN".to_string()), None, true), QuirksMode::Quirks);
        assert_eq!(parser.identify_quirks_mode(&Some("html".to_string()), Some("-//W3C//DTD HTML 4.01 Frameset//".to_string()), None, true), QuirksMode::Quirks);
        assert_eq!(parser.identify_quirks_mode(&Some("html".to_string()), Some("-//W3C//DTD HTML 4.01 Transitional//".to_string()), None, true), QuirksMode::Quirks);
        assert_eq!(parser.identify_quirks_mode(&Some("html".to_string()), Some("-//W3C//DTD XHTML 1.0 Frameset//".to_string()), None, true), QuirksMode::Quirks);
    }

    #[test]
    fn test_quirks_mode_sys() {
        let mut stream = InputStream::new();
        let parser = Html5Parser::new(&mut stream);

        assert_eq!(parser.identify_quirks_mode(&Some("html".to_string()), Some("-//W3C//DTD HTML 4.0 Transitional//EN".to_string()), Some("http://www.w3.org/TR/html4/loose.dtd".to_string()), false), QuirksMode::Quirks);
        assert_eq!(parser.identify_quirks_mode(&Some("html".to_string()), Some("-//W3C//DTD HTML 4.01 Frameset//".to_string()), Some("http://www.w3.org/TR/html4/frameset.dtd".to_string()), false), QuirksMode::LimitedQuirks);
    }

    #[test]
    fn test_quirks_mode_sys_missing() {
        let mut stream = InputStream::new();
        let parser = Html5Parser::new(&mut stream);

        assert_eq!(parser.identify_quirks_mode(&Some("html".to_string()), Some("-//W3C//DTD HTML 4.01 Frameset//".to_string()), None, false), QuirksMode::LimitedQuirks);
    }
}