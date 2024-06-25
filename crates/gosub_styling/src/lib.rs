//! Styling functionality
//!
//! This crate connects CSS3 and HTML5 into a styling pipeline
//!

use gosub_css3::convert::ast_converter::convert_ast_to_stylesheet;
use gosub_css3::parser_config::ParserConfig;
use gosub_css3::stylesheet::{CssOrigin, CssStylesheet};
use gosub_css3::Css3;

pub mod css_definitions;
mod errors;
pub mod render_tree;
pub mod styling;
mod syntax;
mod syntax_matcher;

/// Loads the default user agent stylesheet
pub fn load_default_useragent_stylesheet() -> anyhow::Result<CssStylesheet> {
    // @todo: we should be able to browse to gosub://useragent.css and see the actual useragent css file
    let location = "gosub://useragent.css";
    let config = ParserConfig {
        source: Some(String::from(location)),
        ignore_errors: true,
        ..Default::default()
    };

    let css = include_str!("../resources/useragent.css");
    let css_ast = Css3::parse(css, config).expect("Could not parse useragent stylesheet");

    convert_ast_to_stylesheet(&css_ast, CssOrigin::UserAgent, location)
}

// #[cfg(test)]
// mod tests {
//
//     use crate::render_tree::generate_render_tree;
//     use gosub_html5::html_compile;
//
//     #[test]
//     fn test_css_stuff() {
//         let html = r#"
//         <html>
//         <head>
//             <style>
//                 body { color: blue }
//                 h1 { border: solid black 1px; color: red }
//                 p { color: green }
//             </style>
//         </head>
//         <body>
//             <h1>Hello world</h1>
//             <p>
//                 Goodbye
//                 <h1>moon</h1>
//             </p>
//             Yes
//           </body>
//           </html>
//         "#;
//
//         let doc = html_compile(html);
//         let render_tree = generate_render_tree(doc);
//
//         // what is the border-left-color of h1?
//         dbg!(&render_tree);
//     }
// }
