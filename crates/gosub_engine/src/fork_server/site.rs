//! The unit renderer processes are keyed by.

use psl::Psl as _;
use url::Url;

/// The "site" of a URL, Chromium's definition: scheme plus registrable domain
/// (eTLD+1), so `https://news.example.co.uk` and `https://example.co.uk` are
/// one site while `http://` and `https://` are two. Hosts with no registrable
/// portion (IP addresses, `localhost`, unknown suffixes) are their own site.
/// URLs without a host (`about:`, `data:`) key on the scheme alone.
pub fn site_of(url: &Url) -> String {
    let scheme = url.scheme();
    let Some(host) = url.host_str() else {
        return format!("{scheme}:");
    };
    // Only names go through the suffix list: it happily reads `127.0.0.1`
    // as the domain `0.1`.
    let registrable = match url.host() {
        Some(url::Host::Domain(name)) => psl::List
            .domain(name.as_bytes())
            .and_then(|d| std::str::from_utf8(d.as_bytes()).ok())
            .unwrap_or(host),
        _ => host,
    };
    format!("{scheme}://{registrable}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(s: &str) -> String {
        site_of(&Url::parse(s).expect("test url"))
    }

    #[test]
    fn subdomains_share_a_site_but_schemes_do_not() {
        assert_eq!(site("https://news.example.co.uk/x"), "https://example.co.uk");
        assert_eq!(site("https://example.co.uk/"), "https://example.co.uk");
        assert_ne!(site("http://example.com/"), site("https://example.com/"));
    }

    #[test]
    fn hosts_without_a_registrable_domain_stand_alone() {
        assert_eq!(site("http://localhost:8080/"), "http://localhost");
        assert_eq!(site("http://127.0.0.1/"), "http://127.0.0.1");
        assert_eq!(site("about:blank"), "about:");
    }
}
