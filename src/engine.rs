use crate::bytes::{CharIterator, Confidence, Encoding};
use crate::dns::ResolveType;
use crate::html5::parser::document::{Document, DocumentBuilder, DocumentHandle};
use crate::html5::parser::Html5Parser;
use crate::net::errors::Error;
use crate::net::http::headers::Headers;
use crate::net::http::request::Request;
use crate::net::http::response::Response;
use crate::timing::{Timing, TimingTable};
use crate::types::ParseError;
use cookie::CookieJar;
use core::fmt::Debug;
use core::str::FromStr;
use reqwest::header::HeaderName;
use reqwest::Method;
use url::Url;

/// Response that is returned from the fetch function
pub struct FetchResponse {
    /// Request that has been send
    pub request: Request,
    /// Response that has been received
    pub response: Response,
    /// Document tree that is made from the response
    pub document: DocumentHandle,
    /// Parse errors that occurred while parsing the document tree
    pub parse_errors: Vec<ParseError>,
    /// Rendertree that is generated from the document tree and css tree
    pub render_tree: String,
    /// Timing table that contains all the timings
    pub timings: TimingTable,
}

impl Debug for FetchResponse {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "Request:")?;
        writeln!(f, "{}", self.request)?;
        writeln!(f, "Response:")?;
        writeln!(f, "{}", self.response)?;
        writeln!(f, "Document tree:")?;
        writeln!(f, "{}", self.document)?;
        writeln!(f, "Parse errors:")?;
        for error in &self.parse_errors {
            writeln!(f, "  ({}:{}) {}", error.line, error.col, error.message)?;
        }
        writeln!(f, "Render tree:")?;
        writeln!(f, "{}", self.render_tree)?;
        writeln!(f, "Timings:")?;
        writeln!(f, "{}", self.timings)?;

        Ok(())
    }
}

fn fetch_url(
    method: &str,
    url: &str,
    headers: Headers,
    cookies: CookieJar,
) -> Result<FetchResponse, Error> {
    let mut http_req = Request::new(method, url, "HTTP/1.1");
    http_req.headers = headers.clone();
    http_req.cookies = cookies.clone();

    let parts = Url::parse(url);
    if parts.is_err() {
        return Err(Error::Generic(format!("Failed to parse URL: {}", url)));
    }

    let mut fetch_response = FetchResponse {
        request: http_req,
        response: Response::new(),
        document: DocumentBuilder::new_document(),
        parse_errors: vec![],
        render_tree: String::new(),
        timings: TimingTable::default(),
    };

    // For now, we do a DNS lookup here. We don't use this information yet, but it allows us to
    // measure the DNS lookup time.
    fetch_response.timings.start(Timing::DnsLookup);

    let mut resolver = crate::dns::Dns::new();
    let res = resolver.resolve(parts.unwrap().host_str().unwrap(), ResolveType::Ipv4);
    if res.is_err() {
        return Err(Error::Generic(format!("Failed to resolve domain: {}", url)));
    }

    fetch_response.timings.end(Timing::DnsLookup);

    // Fetch the HTML document from the site
    fetch_response.timings.start(Timing::ContentTransfer);

    let m = Method::from_str(method).unwrap();
    let u = Url::parse(url).unwrap();
    let mut req = reqwest::blocking::Request::new(m, u);
    for (key, value) in headers.sorted() {
        req.headers_mut()
            .insert(HeaderName::from_str(key).unwrap(), value.parse().unwrap());
    }

    match reqwest::blocking::Client::new().execute(req) {
        Ok(resp) => {
            fetch_response.response = Response::new();
            fetch_response.response.status = resp.status().as_u16();
            fetch_response.response.version = format!("{:?}", resp.version());
            for (key, value) in resp.headers().iter() {
                fetch_response
                    .response
                    .headers
                    .set(key.as_str(), value.to_str().unwrap());
            }
            // for cookie in resp.cookies() {
            //     fetch_response.response.cookies.insert(cookie.name().to_string(), cookie.value().to_string());
            // }
            fetch_response.response.body = resp.bytes().unwrap().to_vec();
        }
        Err(e) => {
            return Err(Error::Generic(format!("Failed to fetch URL: {}", e)));
        }
    }
    fetch_response.timings.end(Timing::ContentTransfer);

    println!("resp: {:?}", fetch_response.response);

    fetch_response.timings.start(Timing::HtmlParse);

    let mut chars = CharIterator::new();
    let _ = chars.read_from_bytes(&fetch_response.response.body, Some(Encoding::UTF8));
    chars.set_confidence(Confidence::Certain);
    fetch_response.document = DocumentBuilder::new_document();

    match Html5Parser::parse_document(&mut chars, Document::clone(&fetch_response.document), None) {
        Ok(parse_errors) => {
            fetch_response.parse_errors = parse_errors;
        }
        Err(e) => {
            return Err(Error::Generic(format!("Failed to parse HTML: {}", e)));
        }
    }
    fetch_response.timings.end(Timing::HtmlParse);

    Ok(fetch_response)

    // reqwest::ClientBuilder::new()
    //     .execute()
    //
    //     .execute(resp.request)
    //     .get(url)
    //     .headers(headers.all())
    //     .cookie(cookies)
    //     .send()?

    // resp = reqwest::blocking::Client::new()
    //     .execute(resp.request)
    //     .get(url)
    //     .headers(headers.all())
    //     .cookie(cookies)
    //     .send()
    //
    // resp = reqwest::blocking::Client::new()
    //     .get(url)
    //     .headers(headers.all())
    //     .cookie(cookies)
    //     .send()?
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER_AGENT: &str = "Mozilla/5.0 (compatible; gosub/0.1; +https://gosub.io)";

    #[test]
    fn test_fetch_url() {
        let url = "https://gosub.io/";
        let mut headers = Headers::new();
        headers.set("User-Agent", USER_AGENT);
        let cookies = CookieJar::new();

        let resp = fetch_url("GET", url, headers, cookies);
        assert!(resp.is_ok());

        let resp = resp.unwrap();
        print!("{:?}", resp);
    }
}
