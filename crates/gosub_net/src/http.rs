use crate::http::fetcher::RequestAgent;

pub mod fetcher;
pub mod headers;
pub mod request;
mod request_impl;
pub mod response;

pub type HttpError = <request_impl::RequestImpl as RequestAgent>::Error;
