use cow_utils::CowUtils;
use crate::cookies::cookies::Cookie;
use http::HeaderMap;
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::HashMap;
use url::Url;

pub trait CookieJar: Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn store_response_cookies(&mut self, url: &Url, headers: &HeaderMap);
    fn get_request_cookies(&self, url: &Url) -> Option<String>;
    fn clear(&mut self);
    fn get_all_cookies(&self) -> Vec<(Url, String)>;
    fn remove_cookie(&mut self, url: &Url, cookie_name: &str);
    fn remove_cookies_for_url(&mut self, url: &Url);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultCookieJar {
    pub entries: HashMap<String, Vec<Cookie>>,
}

impl Default for DefaultCookieJar {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultCookieJar {
    pub fn new() -> Self {
        DefaultCookieJar {
            entries: HashMap::new(),
        }
    }
}

impl CookieJar for DefaultCookieJar {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn store_response_cookies(&mut self, url: &Url, headers: &HeaderMap) {
        let origin = url.origin().ascii_serialization();
        let _host = url.host_str().unwrap_or_default();
        let default_path = url
            .path()
            .rsplit_once('/')
            .map_or("/", |(a, _)| if a.is_empty() { "/" } else { a });

        let bucket = self.entries.entry(origin).or_default();

        for header in headers.get_all("set-cookie") {
            if let Ok(header_str) = header.to_str() {
                if let Some((name, rest)) = header_str.split_once('=') {
                    let mut cookie = Cookie {
                        name: name.trim().to_string(),
                        value: String::new(),
                        path: None,
                        domain: None,
                        secure: false,
                        expires: None,
                        same_site: None,
                        http_only: false,
                    };

                    for part in rest.split(';') {
                        let part = part.trim();
                        if cookie.value.is_empty() {
                            cookie.value = part.to_string();
                            continue;
                        }

                        if let Some((k, v)) = part.split_once('=') {
                            match k.cow_to_ascii_lowercase().as_ref() {
                                "path" => cookie.path = Some(v.to_string()),
                                "domain" => cookie.domain = Some(v.trim_start_matches('.').to_string()),
                                "expires" => cookie.expires = Some(v.to_string()),
                                "samesite" => {
                                    let val = v.trim();
                                    if val.eq_ignore_ascii_case("lax") {
                                        cookie.same_site = Some("Lax".to_string());
                                    } else if val.eq_ignore_ascii_case("strict") {
                                        cookie.same_site = Some("Strict".to_string());
                                    } else if val.eq_ignore_ascii_case("none") {
                                        cookie.same_site = Some("None".to_string());
                                    } else {
                                        cookie.same_site = Some(val.to_string());
                                    }
                                }
                                _ => {}
                            }
                        } else if part.eq_ignore_ascii_case("secure") {
                            cookie.secure = true;
                        } else if part.eq_ignore_ascii_case("httponly") {
                            cookie.http_only = true;
                        }
                    }

                    if cookie.path.is_none() {
                        cookie.path = Some(default_path.to_string());
                    }

                    if let Some(existing) = bucket.iter_mut().find(|c| c.name == cookie.name) {
                        *existing = cookie;
                    } else {
                        bucket.push(cookie);
                    }
                }
            }
        }
    }

    fn get_request_cookies(&self, url: &Url) -> Option<String> {
        let origin = url.origin().ascii_serialization();
        let host = url.host_str().unwrap_or_default();
        let path = url.path();
        let is_https = url.scheme() == "https";

        let cookies = self.entries.get(&origin)?;

        let header = cookies
            .iter()
            .filter(|cookie| match &cookie.domain {
                Some(domain) => host == domain || host.ends_with(&format!(".{}", domain)),
                None => true,
            })
            .filter(|cookie| match &cookie.path {
                Some(cookie_path) => path.starts_with(cookie_path),
                None => true,
            })
            .filter(|cookie| !cookie.secure || is_https)
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; ");

        if header.is_empty() {
            None
        } else {
            Some(header)
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
    }

    fn get_all_cookies(&self) -> Vec<(Url, String)> {
        self.entries
            .iter()
            .filter_map(|(origin, cookies)| {
                Url::parse(origin).ok().map(|url| {
                    let str_ = cookies
                        .iter()
                        .map(|c| format!("{}={}", c.name, c.value))
                        .collect::<Vec<_>>()
                        .join("; ");
                    (url, str_)
                })
            })
            .collect()
    }

    fn remove_cookie(&mut self, url: &Url, cookie_name: &str) {
        let origin = url.origin().ascii_serialization();
        if let Some(cookies) = self.entries.get_mut(&origin) {
            cookies.retain(|c| c.name != cookie_name);
        }
    }

    fn remove_cookies_for_url(&mut self, url: &Url) {
        let origin = url.origin().ascii_serialization();
        self.entries.remove(&origin);
    }
}
