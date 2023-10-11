//! This module contains all the APIs that are exposed through javascript
//!
//! A website can control many of the aspects of the useragent with javascript through different APIs.
//! For instance, there is the console api, that allows sites to log information to the console. The
//! window api, that allows sites to control the window, and the document api, that allows sites to
//! make changes in the DOM.
#[allow(dead_code)]
pub mod console;
