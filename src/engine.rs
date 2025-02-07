use gosub_interface::config::{HasDocument, HasHtmlParser};
#[cfg(not(target_arch = "wasm32"))]
use {
    cookie::CookieJar,
    core::fmt::Debug,
    gosub_interface::document::DocumentBuilder,
    gosub_interface::html5::Html5Parser as Html5ParserT,
    gosub_net::dns::{Dns, ResolveType},
    gosub_net::errors::Error,
    gosub_net::http::headers::Headers,
    gosub_net::http::request::Request,
    gosub_net::http::response::Response,
    gosub_shared::byte_stream::{ByteStream, Encoding},
    gosub_shared::types::{ParseError, Result},
    gosub_shared::{timing_start, timing_stop},
    std::io::Read,
    url::Url,
};

#[allow(dead_code)]
const USER_AGENT: &str = "Mozilla/5.0 (compatible; gosub/0.1; +https://gosub.io)";

#[allow(dead_code)]
const MAX_BYTES: u64 = 10_000_000;

/// Response that is returned from the fetch function
#[cfg(not(target_arch = "wasm32"))]
pub struct FetchResponse<C: HasDocument> {
    /// Request that has been send
    pub request: Request,
    /// Response that has been received
    pub response: Response,
    /// Document tree that is made from the response
    pub document: C::Document,
    /// Parse errors that occurred while parsing the document tree
    pub parse_errors: Vec<ParseError>,
    /// Render tree that is generated from the document tree and css tree
    pub render_tree: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl<C: HasDocument> Debug for FetchResponse<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "Request:")?;
        writeln!(f, "{}", self.request)?;
        writeln!(f, "Response:")?;
        writeln!(f, "{}", self.response)?;
        writeln!(f, "Document tree:")?;
        writeln!(f, "{:?}", self.document)?;
        writeln!(f, "Parse errors:")?;
        for error in &self.parse_errors {
            writeln!(
                f,
                "  ({}:{}) {}",
                error.location.line, error.location.column, error.message
            )?;
        }
        writeln!(f, "Render tree:")?;
        writeln!(f, "{}", self.render_tree)?;

        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)]
fn fetch_url<C: HasHtmlParser>(
    method: &str,
    url: &str,
    headers: Headers,
    cookies: CookieJar,
) -> Result<FetchResponse<C>> {
    let mut http_req = Request::new(method, url, "HTTP/1.1");
    http_req.headers = headers.clone();
    http_req.cookies = cookies.clone();

    let parts = Url::parse(url)?;

    let mut fetch_response = FetchResponse {
        request: http_req,
        response: Response::new(),
        document: C::DocumentBuilder::new_document(Some(parts.clone())),
        parse_errors: vec![],
        render_tree: String::new(),
    };

    // For now, we do a DNS lookup here. We don't use this information yet, but it allows us to
    // measure the DNS lookup time.
    let t_id = timing_start!("dns.lookup", parts.host_str().unwrap());

    let mut resolver = Dns::new();
    let Some(hostname) = parts.host_str() else {
        return Err(Error::Generic(format!("invalid hostname: {}", url)).into());
    };
    let _ = resolver.resolve(hostname, ResolveType::Ipv4)?;

    timing_stop!(t_id);

    // Fetch the HTML document from the site
    let t_id = timing_start!("http.transfer", parts.host_str().unwrap());

    let mut req = Request::new(method, url, "HTTP/1.1");
    req.add_header("User-Agent", USER_AGENT);
    for (key, value) in headers.sorted() {
        req.add_header(key, value);
    }

    match req.call() {
        Ok(resp) => {
            fetch_response.response = Response::new();
            fetch_response.response.status = resp.status();
            fetch_response.response.version = format!("{:?}", resp.http_version());
            for key in &resp.headers_names() {
                for value in resp.all(key) {
                    fetch_response.response.headers.set_str(key.as_str(), value);
                }
            }
            // TODO: cookies
            // for cookie in resp.cookies() {
            //     fetch_response.response.cookies.insert(cookie.name().to_string(), cookie.value().to_string());
            // }

            let len = if let Some(header) = resp.header("Content-Length") {
                header.parse::<usize>().unwrap_or_default()
            } else {
                MAX_BYTES as usize
            };

            let mut bytes: Vec<u8> = Vec::with_capacity(len);
            resp.into_reader().take(MAX_BYTES).read_to_end(&mut bytes)?;
            fetch_response.response.body = bytes;
        }
        Err(e) => {
            return Err(Error::Generic(format!("Failed to fetch URL: {}", e)).into());
        }
    }
    timing_stop!(t_id);

    println!("resp: {:?}", fetch_response.response);

    let t_id = timing_start!("html.parse", parts.as_str());

    let mut stream = ByteStream::new(Encoding::UTF8, None);
    let _ = stream.read_from_bytes(&fetch_response.response.body);
    fetch_response.document = C::DocumentBuilder::new_document(Some(parts));

    match C::HtmlParser::parse(&mut stream, &mut fetch_response.document, None) {
        Ok(parse_errors) => {
            fetch_response.parse_errors = parse_errors;
        }
        Err(e) => {
            return Err(Error::Generic(format!("Failed to parse HTML: {}", e)).into());
        }
    }

    timing_stop!(t_id);

    Ok(fetch_response)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use cookie::CookieJar;
    use gosub_css3::system::Css3System;
    use gosub_html5::document::builder::DocumentBuilderImpl;
    use gosub_html5::document::document_impl::DocumentImpl;
    use gosub_html5::document::fragment::DocumentFragmentImpl;
    use gosub_html5::parser::Html5Parser;
    use gosub_interface::config::HasCssSystem;

    #[derive(Clone, Debug, PartialEq)]
    struct Config;

    impl HasCssSystem for Config {
        type CssSystem = Css3System;
    }
    impl HasDocument for Config {
        type Document = DocumentImpl<Self>;
        type DocumentFragment = DocumentFragmentImpl<Self>;
        type DocumentBuilder = DocumentBuilderImpl;
    }

    impl HasHtmlParser for Config {
        type HtmlParser = Html5Parser<'static, Self>;
    }

    #[test]
    fn test_fetch_url() {
        let url = "https://gosub.io/";
        let mut headers = Headers::new();
        headers.set_str("User-Agent", USER_AGENT);
        let cookies = CookieJar::new();

        let resp = fetch_url::<Config>("GET", url, headers, cookies);
        dbg!(&resp);
        assert!(resp.is_ok());
    }
}
