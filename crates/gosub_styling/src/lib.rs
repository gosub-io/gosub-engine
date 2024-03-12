//! Styling functionality
//!
//! This crate connects CSS3 and HTML5 into a styling pipeline
//!

use gosub_css3::convert::ast_converter::convert_ast_to_stylesheet;
use gosub_css3::parser_config::ParserConfig;
use gosub_css3::stylesheet::{CssOrigin, CssStylesheet};
use gosub_css3::Css3;
use std::fs;

pub mod css_colors;
pub mod css_node_tree;
mod property_list;

/// Loads the default user agent stylesheet
pub fn load_default_useragent_stylesheet() -> anyhow::Result<CssStylesheet> {
    // @todo: we should be able to browse to gosub://useragent.css and see the actual useragent css file
    let location = "gosub://useragent.css";
    let config = ParserConfig {
        source: Some(String::from(location)),
        ignore_errors: true,
        ..Default::default()
    };

    let css =
        fs::read_to_string("resources/useragent.css").expect("Could not load useragent stylesheet");
    let css_ast = Css3::parse(css.as_str(), config).expect("Could not parse useragent stylesheet");

    convert_ast_to_stylesheet(&css_ast, CssOrigin::UserAgent, location)
}
