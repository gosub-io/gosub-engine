// See https://github.com/html5lib/html5lib-tests/tree/master/tree-construction
use gosub_shared::types::{Error, Result};
use nom::{
    branch::alt,
    bytes::complete::{tag, take_until, take_until1},
    character::complete::multispace0,
    combinator::{all_consuming, map, opt},
    multi::{many0, many1, separated_list1},
    sequence::{delimited, preceded, tuple},
    Finish, IResult,
};
use nom_locate::{position, LocatedSpan};

pub const QUOTED_DOUBLE_NEWLINE: &str = ":quoted-double-newline:";

type Span<'a> = LocatedSpan<&'a str>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Position {
    pub line: usize,
    pub col: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorSpec {
    Message(String),

    Line {
        line: usize,
        message: String,
    },

    Location {
        pos: Position,
        message: String,
    },

    Span {
        start: Position,
        end: Position,
        message: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ScriptMode {
    ScriptOn,
    ScriptOff,
    #[default]
    Both,
}

#[derive(Debug, PartialEq, Default, Clone)]
pub struct TestSpec {
    /// #data section
    pub data: String,
    /// #errors section
    pub errors: Vec<ErrorSpec>,
    /// #new-errors section
    pub new_errors: Vec<ErrorSpec>,
    /// #document-fragment section
    pub document_fragment: Option<String>,
    /// #script-on, #script-off
    pub script_mode: ScriptMode,
    /// #document section
    pub document: String,
    /// Position in the input stream
    pub position: Position,
}

pub enum TreeConstructionResult {
    Success,
    Error,
}

impl TestSpec {
    pub fn parse(&self) -> Result<TreeConstructionResult> {
        Ok(TreeConstructionResult::Success)
    }
}

fn data(i: Span) -> IResult<Span, Span> {
    preceded(tag("#data\n"), preceded(multispace0, take_until("#errors")))(i)
}

fn error_1(i: Span) -> IResult<Span, ErrorSpec> {
    let location = map(
        tuple((
            tag("("),
            nom::character::complete::u64,
            tag(","),
            nom::character::complete::u64,
            tag(")"),
        )),
        |(_, line, _, col, _): (Span, u64, Span, u64, Span)| (line as usize, col as usize),
    );

    map(
        tuple((location, tag(": "), take_until1("\n"))),
        |((line, col), _, message)| ErrorSpec::Location {
            pos: Position { line, col },
            message: message.trim().into(),
        },
    )(i)
}

fn error_2(i: Span) -> IResult<Span, ErrorSpec> {
    let location = map(
        tuple((
            tag("("),
            nom::character::complete::u64,
            tag(":"),
            nom::character::complete::u64,
            tag(")"),
        )),
        |(_, line, _, col, _): (Span, u64, Span, u64, Span)| (line as usize, col as usize),
    );

    map(
        tuple((location, tag(" "), take_until1("\n"))),
        |((line, col), _, message)| ErrorSpec::Location {
            pos: Position { line, col },
            message: message.trim().into(),
        },
    )(i)
}

fn error_3(i: Span) -> IResult<Span, ErrorSpec> {
    let location = map(
        tuple((
            nom::character::complete::u64,
            tag(":"),
            nom::character::complete::u64,
        )),
        |(line, _, col): (u64, Span, u64)| (line as usize, col as usize),
    );

    map(
        tuple((location, tag(": "), take_until1("\n"))),
        |((line, col), _, message)| ErrorSpec::Location {
            pos: Position { line, col },
            message: message.trim().into(),
        },
    )(i)
}

fn error_4(i: Span) -> IResult<Span, ErrorSpec> {
    let location = map(
        tuple((
            alt((tag(" * ("), tag("* ("))),
            nom::character::complete::u64,
            tag(","),
            nom::character::complete::u64,
            tag(")"),
        )),
        |(_, line, _, col, _): (Span, u64, Span, u64, Span)| (line as usize, col as usize),
    );

    map(
        tuple((location, tag(" "), take_until1("\n"))),
        |((line, col), _, message)| ErrorSpec::Location {
            pos: Position { line, col },
            message: message.trim().into(),
        },
    )(i)
}

fn error_5(i: Span) -> IResult<Span, ErrorSpec> {
    map(
        tuple((nom::character::complete::u64, tag(": "), take_until1("\n"))),
        |(line, _, message): (u64, Span, Span)| ErrorSpec::Line {
            line: line as _,
            message: message.trim().into(),
        },
    )(i)
}

// (1:44-1:49) non-void-html-element-start-tag-with-trailing-solidus
fn error_6(i: Span) -> IResult<Span, ErrorSpec> {
    let span = map(
        tuple((
            tag("("),
            nom::character::complete::u64,
            tag(":"),
            nom::character::complete::u64,
            tag("-"),
            nom::character::complete::u64,
            tag(":"),
            nom::character::complete::u64,
            tag(")"),
        )),
        |(_, line1, _, col1, _, line2, _, col2, _): (
            Span,
            u64,
            Span,
            u64,
            Span,
            u64,
            Span,
            u64,
            Span,
        )| {
            (
                Position {
                    line: line1 as _,
                    col: col1 as _,
                },
                Position {
                    line: line2 as _,
                    col: col2 as _,
                },
            )
        },
    );

    map(
        tuple((span, tag(" "), take_until1("\n"))),
        |((start, end), _, message): ((Position, Position), Span, Span)| ErrorSpec::Span {
            start,
            end,
            message: message.to_string(),
        },
    )(i)
}

fn error_messages(i: Span) -> IResult<Span, Vec<ErrorSpec>> {
    map(take_until1("#"), |string: Span| {
        string
            .lines()
            .map(|s| ErrorSpec::Message(s.into()))
            .collect::<Vec<_>>()
    })(i)
}

fn old_errors(i: Span) -> IResult<Span, Vec<ErrorSpec>> {
    delimited(
        tuple((multispace0, tag("#errors\n"))),
        map(
            opt(alt((
                many1(delimited(
                    multispace0,
                    alt((error_1, error_2, error_3, error_4, error_5)),
                    tag("\n"),
                )),
                error_messages,
            ))),
            std::option::Option::unwrap_or_default,
        ),
        multispace0,
    )(i)
}

fn new_errors(i: Span) -> IResult<Span, Vec<ErrorSpec>> {
    delimited(
        tuple((multispace0, tag("#new-errors\n"))),
        many0(delimited(multispace0, alt((error_2, error_6)), tag("\n"))),
        multispace0,
    )(i)
}

fn document(i: Span) -> IResult<Span, Span> {
    preceded(tuple((multispace0, tag("#document\n"))), take_until("\n\n"))(i)
}

fn document_fragment(i: Span) -> IResult<Span, Span> {
    preceded(tag("#document-fragment\n"), take_until1("\n"))(i)
}

fn test(i: Span) -> IResult<Span, TestSpec> {
    let (start, _) = position(i)?;

    let position = Position {
        line: start.location_line() as usize,
        col: start.get_column(),
    };

    map(
        tuple((
            data,
            old_errors,
            opt(new_errors),
            opt(tag("#script-on\n")),
            opt(tag("#script-off\n")),
            opt(document_fragment),
            document,
        )),
        move |(data, errors, new_errors, script_on, script_off, document_fragment, document)| {
            let script_on = script_on.map(|s| *s.fragment());
            let script_off = script_off.map(|s| *s.fragment());

            let script_mode = match (script_on, script_off) {
                (Some("#script-on\n"), None) => ScriptMode::ScriptOn,
                (None, Some("#script-off\n")) => ScriptMode::ScriptOff,
                (Some(_), Some(_)) => unreachable!(),
                _ => ScriptMode::Both,
            };

            TestSpec {
                position,
                data: trim_last_newline(data.to_string()),
                errors,
                new_errors: new_errors.unwrap_or_default(),
                script_mode,
                document_fragment: document_fragment.map(|s| s.to_string()),
                document: document.to_string(),
            }
        },
    )(i)
}

/// Trims only a single newline from the string, even if there are multiple newlines present.
fn trim_last_newline(s: String) -> String {
    if let Some(s) = s.strip_suffix('\n') {
        s.to_owned()
    } else {
        s
    }
}

pub fn parse_fixture(i: &str) -> Result<Vec<TestSpec>> {
    // Deal with a corner case that makes it hard to parse tricky01.dat.
    let input = i.replace("\"\n\n\"", QUOTED_DOUBLE_NEWLINE) + "\n";

    let files = map(
        tuple((separated_list1(tag("\n\n"), test), multispace0)),
        |(tests, _)| tests,
    );

    let (_, tests) = all_consuming(files)(Span::new(&input))
        .finish()
        .map_err(|err| Error::Test(format!("{err}")))?;

    Ok(tests)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_test(i: &str) -> (Span, TestSpec) {
        test(Span::new(i.trim_start())).unwrap()
    }

    #[test]
    fn parse_data() {
        let (_, s) = data("#data\n         Test \n#errors\n".into()).unwrap();
        assert_eq!(*s.fragment(), "Test \n");

        let (_, s) = data("#data\n         Test \n#errors".into()).unwrap();
        assert_eq!(*s.fragment(), "Test \n");

        let (_, s) = data(
            "#data\n<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Frameset//EN\"
            \"http://www.w3.org/TR/xhtml1/DTD/xhtml1-frameset.dtd\"><p><table>\n#errors"
                .into(),
        )
        .unwrap();

        assert_eq!(
            *s,
            "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Frameset//EN\"\n            \"http://www.w3.org/TR/xhtml1/DTD/xhtml1-frameset.dtd\"><p><table>\n"
        );
    }

    #[test]
    fn parse_document() {
        let (_, doc) = document(
            r#"
#document
| <html>
|   <head>
|   <body>
|     "Test"

"#
            .trim_start()
            .into(),
        )
        .unwrap();
        assert_eq!(
            doc.to_string(),
            "| <html>\n|   <head>\n|   <body>\n|     \"Test\""
        );
    }

    #[test]
    fn tests1_dat_1() {
        let (_, test) = parse_test(
            r#"
#data
Test
#errors
(1,0): expected-doctype-but-got-chars
#document
| <html>
|   <head>
|   <body>
|     "Test"

"#,
        );

        assert_eq!(test.data, "Test");
        assert_eq!(
            test.errors,
            &[ErrorSpec::Location {
                pos: Position { line: 1, col: 0 },
                message: "expected-doctype-but-got-chars".into(),
            }]
        );
        assert_eq!(
            test.document,
            "| <html>\n|   <head>\n|   <body>\n|     \"Test\""
        );
    }

    #[test]
    fn quirks01_dat_1() {
        let (_, test) = parse_test(
            r#"
#data
<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.0 Frameset//EN"
"http://www.w3.org/TR/xhtml1/DTD/xhtml1-frameset.dtd"><p><table>
#errors
(2,54): unknown-doctype
(2,64): eof-in-table
#document
| <!DOCTYPE html "-//W3C//DTD XHTML 1.0 Frameset//EN" "http://www.w3.org/TR/xhtml1/DTD/xhtml1-frameset.dtd">
| <html>
|   <head>
|   <body>
|     <p>
|     <table>

"#,
        );

        assert_eq!(test.errors.len(), 2);
    }

    #[test]
    fn quirks01_dat_15() {
        let (_, test) = parse_test(
            r#"
#data
<!DOCTYPE html SYSTEM "http://www.ibm.com/data/dtd/v11/ibmxhtml1-transitional.dtd"><p><table>
#errors
(1,83): unknown-doctype
(1,93): eof-in-table
#document
| <!DOCTYPE html "" "http://www.ibm.com/data/dtd/v11/ibmxhtml1-transitional.dtd">
| <html>
|   <head>
|   <body>
|     <p>
|       <table>

"#,
        );

        assert_eq!(test.errors.len(), 2);
    }

    #[test]
    fn comments01_dat_13() {
        let (_, test) = parse_test(
            r#"
#data
FOO<!-- BAR --!>BAZ
#errors
(1,3): expected-doctype-but-got-chars
(1,15): unexpected-bang-after-double-dash-in-comment
#new-errors
(1:16) incorrectly-closed-comment
#document
| <html>
|   <head>
|   <body>
|     "FOO"
|     <!--  BAR  -->
|     "BAZ"

"#,
        );

        assert_eq!(test.errors.len(), 2);
        assert_eq!(test.new_errors.len(), 1);
    }

    #[test]
    fn comments01_dat_42() {
        let (_, test) = parse_test(
            r#"
#data
FOO<!-- BAR --!
>BAZ
#errors
(1,3): expected-doctype-but-got-chars
(2:5) eof-in-comment
#new-errors
(2:5) eof-in-comment
#document
| <html>
|   <head>
|   <body>
|     "FOO"
|     <!--  BAR --!
>BAZ -->

"#,
        );

        assert!(test.document.ends_with(">BAZ -->"));
    }

    #[test]
    fn tables01_dat_288() {
        let (_, test) = parse_test(
            r"
#data
<div><table><svg><foreignObject><select><table><s>
#errors
1:1: Expected a doctype token
1:13: 'svg' tag isn't allowed here. Currently open tags: html, body, div, table.
1:33: 'select' tag isn't allowed here. Currently open tags: html, body, div, table, svg, foreignobject.
1:41: 'table' tag isn't allowed here. Currently open tags: html, body, div, table, svg, foreignobject, select.
1:41: 'table' tag isn't allowed here. Currently open tags: html, body, div, table, svg, foreignobject.
1:48: 's' tag isn't allowed here. Currently open tags: html, body, div, table.
1:51: Premature end of file. Currently open tags: html, body, div, table, s.
#document
| <html>
|   <head>
|   <body>
|     <div>
|       <svg svg>
|         <svg foreignObject>
|           <select>
|       <table>
|       <s>
|       <table>

",
        );

        assert_eq!(test.errors.len(), 7);
    }

    #[test]
    fn template_dat_61() {
        let (_, test) = parse_test(
            r"
#data
<div><template><div><span></template><b>
#errors
 * (1,6) missing DOCTYPE
 * (1,38) mismatched template end tag
 * (1,41) unexpected end of file
#document
| <html>
|   <head>
|   <body>
|     <div>
|       <template>
|         content
|           <div>
|             <span>
|       <b>

",
        );

        assert_eq!(test.errors.len(), 3);
    }

    #[test]
    fn template_dat_1659() {
        let (_, test) = test(
            r#"            
#data
<!DOCTYPE HTML><template><tr><td>cell</td></tr>a</template>
#errors
(1,59): foster-parenting-character
#document
| <!DOCTYPE html>
| <html>
|   <head>
|     <template>
|       content
|         <tr>
|           <td>
|             "cell"
|         "a"
|   <body>

"#
            .trim_start()
            .into(),
        )
        .unwrap();

        assert_eq!(test.errors.len(), 1);
    }

    #[test]
    fn template_data_148() {
        let (_, test) = parse_test(
            r"
#data
<table><template></template><div></div>
#errors
no doctype
bad div in table
bad /div in table
eof in table
#document
| <html>
|   <head>
|   <body>
|     <div>
|     <table>
|       <template>
|         content

",
        );

        assert_eq!(test.errors.len(), 4);
    }

    #[test]
    fn template_dat_1613() {
        let (_, test) = parse_test(
            r#"
#data
<template><form><input name="q"></form><div>second</div></template>
#errors
#document-fragment
template
#document
| <template>
|   content
|     <form>
|       <input>
|         name="q"
|     <div>
|       "second"

"#,
        );

        assert_eq!(test.document_fragment, Some("template".into()));
    }

    #[test]
    fn webkit02_dat_13() {
        let (_, test) = parse_test(
            r#"
#data
<p id="status"><noscript><strong>A</strong></noscript><span>B</span></p>
#errors
(1,15): expected-doctype-but-got-start-tag
#script-on
#document
| <html>
|   <head>
|   <body>
|     <p>
|       id="status"
|       <noscript>
|         "<strong>A</strong>"
|       <span>
|         "B"

"#,
        );

        assert!(matches!(test.script_mode, ScriptMode::ScriptOn));
    }

    #[test]
    fn webkit02_dat_29() {
        let (_, test) = parse_test(
            r#"
#data
<p id="status"><noscript><strong>A</strong></noscript><span>B</span></p>
#errors
(1,15): expected-doctype-but-got-start-tag
#script-off
#document
| <html>
|   <head>
|   <body>
|     <p>
|       id="status"
|       <noscript>
|         <strong>
|           "A"
|       <span>
|         "B"

"#,
        );

        assert!(matches!(test.script_mode, ScriptMode::ScriptOff));
    }

    #[test]
    fn tests08_dat_1() {
        let (_, test) = parse_test(
            r#"
#data
<div>
<div></div>
</span>x
#errors
(1,5): expected-doctype-but-got-start-tag
(3,7): unexpected-end-tag
(3,8): expected-closing-tag-but-got-eof
#document
| <html>
|   <head>
|   <body>
|     <div>
|       "
"
|       <div>
|       "
x"

"#,
        );

        assert_eq!(test.errors.len(), 3);
        assert!(test.document.ends_with("\"\nx\""));
    }

    #[test]
    fn parse_error_5() {
        let (_, error) =
            error_5("52: End of file seen and there were open elements.\n".into()).unwrap();

        assert_eq!(
            error,
            ErrorSpec::Line {
                line: 52,
                message: "End of file seen and there were open elements.".into(),
            },
        );
    }

    #[test]
    fn parse_error_6() {
        let (_, error) =
            error_6("(1:44-1:49) non-void-html-element-start-tag-with-trailing-solidus\n".into())
                .unwrap();

        assert_eq!(
            error,
            ErrorSpec::Span {
                start: Position { line: 1, col: 44 },
                end: Position { line: 1, col: 49 },
                message: "non-void-html-element-start-tag-with-trailing-solidus".into(),
            }
        );
    }

    #[test]
    fn foreign_fragment_dat_169() {
        let (_, test) = parse_test(
            r#"
#data
<b></b><mglyph/><i></i><malignmark/><u></u><ms/>X
#errors
51: Self-closing syntax (“/>”) used on a non-void HTML element. Ignoring the slash and treating as a start tag.
52: End of file seen and there were open elements.
#new-errors
(1:44-1:49) non-void-html-element-start-tag-with-trailing-solidus
#document-fragment
math ms
#document
| <b>
| <math mglyph>
| <i>
| <math malignmark>
| <u>
| <ms>
|   "X"

"#,
        );

        assert_eq!(test.errors.len(), 2);
        let error = test.errors.first().unwrap();
        assert!(matches!(error, ErrorSpec::Line { .. }));

        assert_eq!(test.new_errors.len(), 1);
        let error = test.new_errors.first().unwrap();
        assert!(matches!(error, ErrorSpec::Span { .. }));
    }

    #[test]
    fn tests10_data_638() {
        let (_, test) = parse_test(
            r"
#data
<svg><script></script><path>
#errors
(1,5) expected-doctype-but-got-start-tag
(1,28) expected-closing-tag-but-got-eof
#document
| <html>
|   <head>
|   <body>
|     <svg svg>
|       <svg script>
|       <svg path>

",
        );

        assert!(test.document.ends_with("path>"));
    }

    #[test]
    fn tests_inner_html_1_dat_837() {
        let (_, test) = parse_test(
            r"
#data
#errors
#document-fragment
html
#document
| <head>
| <body>

",
        );

        assert_eq!(test.data, "");
    }
}
