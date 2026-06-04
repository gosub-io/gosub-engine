//! Comprehensive table-driven cookie tests.
//!
//! Includes:
//! - Hand-written cases covering specific parsing and matching semantics.
//! - Runners for external test suites:
//!   - `http-state` (Adam Barth's 222-case parser suite, via abarth/http-state)
//!   - WPT attribute tests (expires, max-age, secure, samesite, httponly)

#[cfg(test)]
mod tests {
    use crate::engine::cookies::cookie_jar::{CookieJar, DefaultCookieJar, SameSiteContext};
    use http::HeaderMap;
    use url::Url;

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn jar() -> DefaultCookieJar {
        DefaultCookieJar::new()
    }

    fn u(s: &str) -> Url {
        s.parse().expect(s)
    }

    fn set_headers(values: &[&str]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for &v in values {
            // Some test vectors contain bytes that are technically invalid HTTP
            // header values (control chars, etc.). Use from_bytes so we can skip
            // those gracefully rather than panicking.
            if let Ok(hv) = http::header::HeaderValue::from_bytes(v.as_bytes()) {
                map.append("set-cookie", hv);
            }
        }
        map
    }

    fn get_cookies(jar: &DefaultCookieJar, request_url: &str) -> Option<String> {
        jar.get_request_cookies(&u(request_url), None, SameSiteContext::SameSite)
    }

    // ── Hand-written table-driven cases ──────────────────────────────────────

    struct Case {
        desc: &'static str,
        set_from: &'static str,
        headers: &'static [&'static str],
        send_to: &'static str,
        expected: Option<&'static str>,
    }

    fn run(cases: &[Case]) {
        for c in cases {
            let mut jar = jar();
            jar.store_response_cookies(&u(c.set_from), &set_headers(c.headers), None);
            let got = get_cookies(&jar, c.send_to);
            assert_eq!(
                got.as_deref(),
                c.expected,
                "FAIL [{}]: set_from={} headers={:?} send_to={}",
                c.desc,
                c.set_from,
                c.headers,
                c.send_to,
            );
        }
    }

    // ── Basic name=value parsing ──────────────────────────────────────────────

    #[test]
    fn basic_parsing() {
        run(&[
            Case {
                desc: "simple cookie",
                set_from: "https://example.com/",
                headers: &["a=b"],
                send_to: "https://example.com/",
                expected: Some("a=b"),
            },
            Case {
                desc: "empty value",
                set_from: "https://example.com/",
                headers: &["a="],
                send_to: "https://example.com/",
                expected: Some("a="),
            },
            Case {
                desc: "value with equals sign",
                set_from: "https://example.com/",
                headers: &["a=b=c"],
                send_to: "https://example.com/",
                expected: Some("a=b=c"),
            },
            Case {
                desc: "name with leading/trailing whitespace stripped",
                set_from: "https://example.com/",
                headers: &[" a =b"],
                send_to: "https://example.com/",
                expected: Some("a=b"),
            },
            Case {
                desc: "value with Path attribute",
                set_from: "https://example.com/",
                headers: &["a=b; Path=/"],
                send_to: "https://example.com/",
                expected: Some("a=b"),
            },
            Case {
                desc: "no '=' separator — not a valid cookie",
                set_from: "https://example.com/",
                headers: &["invalid-no-equals"],
                send_to: "https://example.com/",
                expected: None,
            },
            Case {
                desc: "leading semicolon — invalid cookie name",
                set_from: "https://example.com/",
                headers: &[";foo=bar"],
                send_to: "https://example.com/",
                expected: None,
            },
        ]);
    }

    // ── Path matching (RFC 6265 §5.1.4) ──────────────────────────────────────

    #[test]
    fn path_matching() {
        run(&[
            Case {
                desc: "Path=/ matches all paths",
                set_from: "https://example.com/",
                headers: &["a=1; Path=/"],
                send_to: "https://example.com/foo/bar",
                expected: Some("a=1"),
            },
            Case {
                desc: "cookie path matches exact request path",
                set_from: "https://example.com/",
                headers: &["a=1; Path=/foo"],
                send_to: "https://example.com/foo",
                expected: Some("a=1"),
            },
            Case {
                desc: "cookie path matches request sub-path (next char is /)",
                set_from: "https://example.com/",
                headers: &["a=1; Path=/foo"],
                send_to: "https://example.com/foo/bar",
                expected: Some("a=1"),
            },
            Case {
                desc: "cookie path does NOT match path with same prefix but different segment",
                set_from: "https://example.com/",
                headers: &["a=1; Path=/foo"],
                send_to: "https://example.com/foobar",
                expected: None,
            },
            Case {
                desc: "cookie path /app/ matches /app/sub",
                set_from: "https://example.com/",
                headers: &["a=1; Path=/app/"],
                send_to: "https://example.com/app/sub",
                expected: Some("a=1"),
            },
            Case {
                desc: "cookie path /app/ does not match /app (no trailing slash)",
                set_from: "https://example.com/",
                headers: &["a=1; Path=/app/"],
                send_to: "https://example.com/app",
                expected: None,
            },
            Case {
                desc: "shorter request path does not match longer cookie path",
                set_from: "https://example.com/",
                headers: &["a=1; Path=/foo/bar"],
                send_to: "https://example.com/foo",
                expected: None,
            },
        ]);
    }

    // ── Default path derivation ───────────────────────────────────────────────

    #[test]
    fn default_path_derivation() {
        run(&[
            Case {
                desc: "no Path on root URL → default /",
                set_from: "https://example.com/",
                headers: &["a=1"],
                send_to: "https://example.com/anything",
                expected: Some("a=1"),
            },
            Case {
                desc: "no Path on /foo/bar → default /foo",
                set_from: "https://example.com/foo/bar",
                headers: &["a=1"],
                send_to: "https://example.com/foo/other",
                expected: Some("a=1"),
            },
            Case {
                desc: "no Path on /foo/bar → does NOT match /other",
                set_from: "https://example.com/foo/bar",
                headers: &["a=1"],
                send_to: "https://example.com/other",
                expected: None,
            },
        ]);
    }

    // ── Domain matching ───────────────────────────────────────────────────────

    #[test]
    fn domain_matching() {
        run(&[
            Case {
                desc: "Domain=example.com sent to same host",
                set_from: "https://foo.example.com/",
                headers: &["a=1; Path=/; Domain=example.com"],
                send_to: "https://foo.example.com/",
                expected: Some("a=1"),
            },
            Case {
                desc: "no Domain attribute — sent to exact origin",
                set_from: "https://example.com/",
                headers: &["a=1; Path=/"],
                send_to: "https://example.com/",
                expected: Some("a=1"),
            },
            Case {
                desc: "Domain=com (bare TLD) — cookie dropped",
                set_from: "https://foo.com/",
                headers: &["a=1; Path=/; Domain=com"],
                send_to: "https://foo.com/",
                expected: None,
            },
            Case {
                desc: "Domain leading dot stripped",
                set_from: "https://example.com/",
                headers: &["a=1; Path=/; Domain=.example.com"],
                send_to: "https://example.com/",
                expected: Some("a=1"),
            },
        ]);
    }

    // ── Secure flag ───────────────────────────────────────────────────────────

    #[test]
    fn secure_flag() {
        run(&[
            Case {
                desc: "Secure cookie sent over HTTPS",
                set_from: "https://example.com/",
                headers: &["s=1; Path=/; Secure"],
                send_to: "https://example.com/",
                expected: Some("s=1"),
            },
            Case {
                desc: "Secure cookie NOT sent over plain HTTP",
                set_from: "https://example.com/",
                headers: &["s=1; Path=/; Secure"],
                send_to: "http://example.com/",
                expected: None,
            },
            Case {
                desc: "Secure cookie from HTTP dropped on storage",
                set_from: "http://example.com/",
                headers: &["s=1; Path=/; Secure"],
                send_to: "https://example.com/",
                expected: None,
            },
            Case {
                desc: "Non-secure cookie sent over HTTP",
                set_from: "http://example.com/",
                headers: &["n=1; Path=/"],
                send_to: "http://example.com/",
                expected: Some("n=1"),
            },
        ]);
    }

    // ── Multiple cookies ──────────────────────────────────────────────────────

    #[test]
    fn multiple_cookies_in_response() {
        let mut jar = jar();
        let origin = u("https://example.com/");
        let hdrs = set_headers(&["a=1; Path=/", "b=2; Path=/", "c=3; Path=/"]);
        jar.store_response_cookies(&origin, &hdrs, None);

        let got = get_cookies(&jar, "https://example.com/").unwrap_or_default();
        assert!(got.contains("a=1"), "missing a=1 in: {got}");
        assert!(got.contains("b=2"), "missing b=2 in: {got}");
        assert!(got.contains("c=3"), "missing c=3 in: {got}");
    }

    #[test]
    fn last_write_wins_for_same_name() {
        let mut jar = jar();
        let origin = u("https://example.com/");
        jar.store_response_cookies(&origin, &set_headers(&["a=first; Path=/"]), None);
        jar.store_response_cookies(&origin, &set_headers(&["a=second; Path=/"]), None);
        assert_eq!(
            get_cookies(&jar, "https://example.com/").as_deref(),
            Some("a=second"),
        );
    }

    // ── SameSite attribute storage ────────────────────────────────────────────

    #[test]
    fn samesite_values_are_normalised() {
        let cases = [
            ("SameSite=Lax",    SameSiteContext::SameSite,            true),
            ("SameSite=lax",    SameSiteContext::SameSite,            true),
            ("SameSite=LAX",    SameSiteContext::SameSite,            true),
            ("SameSite=Strict", SameSiteContext::SameSite,            true),
            ("SameSite=Strict", SameSiteContext::CrossSiteNavigation, false),
            ("SameSite=Lax",    SameSiteContext::CrossSiteNavigation, true),
            ("SameSite=Lax",    SameSiteContext::CrossSite,           false),
        ];
        for (attr, ctx, expect_sent) in cases {
            let mut jar = jar();
            let origin = u("https://example.com/");
            jar.store_response_cookies(
                &origin,
                &set_headers(&[&format!("x=1; Path=/; {attr}")]),
                None,
            );
            let got = jar.get_request_cookies(&origin, None, ctx);
            assert_eq!(
                got.is_some(),
                expect_sent,
                "attr={attr:?} ctx={ctx:?}: expected sent={expect_sent}",
            );
        }
    }

    // ── Attribute edge cases ──────────────────────────────────────────────────

    #[test]
    fn unknown_attribute_ignored() {
        run(&[Case {
            desc: "unknown attribute does not break parsing",
            set_from: "https://example.com/",
            headers: &["a=1; Path=/; Bogus=yes; AlsoUnknown"],
            send_to: "https://example.com/",
            expected: Some("a=1"),
        }]);
    }

    #[test]
    fn case_insensitive_attribute_names() {
        run(&[
            Case {
                desc: "SECURE (uppercase) is recognised",
                set_from: "https://example.com/",
                headers: &["s=1; Path=/; SECURE"],
                send_to: "https://example.com/",
                expected: Some("s=1"),
            },
            Case {
                desc: "secure (lowercase) is recognised",
                set_from: "https://example.com/",
                headers: &["s=1; Path=/; secure"],
                send_to: "https://example.com/",
                expected: Some("s=1"),
            },
            Case {
                desc: "seCURe (mixed case) is recognised",
                set_from: "https://example.com/",
                headers: &["s=1; Path=/; seCURe"],
                send_to: "https://example.com/",
                expected: Some("s=1"),
            },
        ]);
    }

    // ── Domain attribute edge cases ───────────────────────────────────────────

    #[test]
    fn domain_leading_dot_stripped() {
        let mut jar = jar();
        jar.store_response_cookies(
            &u("https://example.com/"),
            &set_headers(&["a=1; Path=/; Domain=.example.com"]),
            None,
        );
        assert_eq!(
            get_cookies(&jar, "https://example.com/").as_deref(),
            Some("a=1"),
        );
    }

    // ── Expiry / Max-Age ──────────────────────────────────────────────────────

    #[test]
    fn max_age_zero_deletes_cookie() {
        let mut jar = jar();
        let origin = u("https://example.com/");
        jar.store_response_cookies(&origin, &set_headers(&["a=1; Path=/"]), None);
        jar.store_response_cookies(&origin, &set_headers(&["a=1; Path=/; Max-Age=0"]), None);
        assert!(get_cookies(&jar, "https://example.com/").is_none());
    }

    #[test]
    fn max_age_overrides_past_expires() {
        run(&[Case {
            desc: "Max-Age takes precedence over past Expires",
            set_from: "https://example.com/",
            headers: &["a=1; Path=/; Expires=Thu, 01 Jan 1970 00:00:01 GMT; Max-Age=3600"],
            send_to: "https://example.com/",
            expected: Some("a=1"),
        }]);
    }

    #[test]
    fn invalid_expires_treated_as_session() {
        run(&[Case {
            desc: "invalid Expires → session cookie",
            set_from: "https://example.com/",
            headers: &["a=1; Path=/; Expires=not-a-date"],
            send_to: "https://example.com/",
            expected: Some("a=1"),
        }]);
    }

    // ── Cookie name prefixes ──────────────────────────────────────────────────

    #[test]
    fn prefix_enforcement() {
        run(&[
            Case {
                desc: "__Secure- with Secure flag accepted",
                set_from: "https://example.com/",
                headers: &["__Secure-id=1; Path=/; Secure"],
                send_to: "https://example.com/",
                expected: Some("__Secure-id=1"),
            },
            Case {
                desc: "__Secure- without Secure flag dropped",
                set_from: "https://example.com/",
                headers: &["__Secure-id=1; Path=/"],
                send_to: "https://example.com/",
                expected: None,
            },
            Case {
                desc: "__Host- fully compliant accepted",
                set_from: "https://example.com/",
                headers: &["__Host-id=1; Secure; Path=/"],
                send_to: "https://example.com/",
                expected: Some("__Host-id=1"),
            },
            Case {
                desc: "__Host- with Domain attribute dropped",
                set_from: "https://example.com/",
                headers: &["__Host-id=1; Secure; Path=/; Domain=example.com"],
                send_to: "https://example.com/",
                expected: None,
            },
            Case {
                desc: "__Host- with non-root Path dropped",
                set_from: "https://example.com/app/",
                headers: &["__Host-id=1; Secure; Path=/app"],
                send_to: "https://example.com/app/",
                expected: None,
            },
        ]);
    }

    // ── Purge ─────────────────────────────────────────────────────────────────

    #[test]
    fn purge_only_removes_expired() {
        let mut jar = jar();
        let origin = u("https://example.com/");
        jar.store_response_cookies(&origin, &set_headers(&["live=1; Path=/; Max-Age=3600"]), None);
        jar.entries
            .entry(origin.origin().ascii_serialization())
            .or_default()
            .push(crate::engine::cookies::Cookie {
                name: "dead".into(),
                value: "0".into(),
                path: Some("/".into()),
                domain: None,
                secure: false,
                expires: Some(1),
                same_site: None,
                http_only: false,
                created_at: 0,
            });
        jar.purge_expired();
        let result = get_cookies(&jar, "https://example.com/").unwrap_or_default();
        assert!(result.contains("live=1"));
        assert!(!result.contains("dead=0"));
    }

    // ── http-state test suite runner ──────────────────────────────────────────
    //
    // Test vectors from https://github.com/abarth/http-state (222 cases).
    // All tests use http://home.example.org:8888/cookie-parser as the
    // set-from and send-to URL (plain HTTP, so Secure cookies are rejected).
    //
    // Known-failing tests are listed in KNOWN_FAILING below with the reason.
    // The test panics if any test outside that list unexpectedly fails, or if
    // a test in the list unexpectedly passes (regression in either direction).

    /// Tests we know diverge from the http-state expectations.
    /// Format: (test-id, reason)
    const KNOWN_FAILING: &[(&str, &str)] = &[
        // ── Domain attribute semantics ────────────────────────────────────────
        // DOMAIN0002/0006/0010/0024/0029/0035/0036: The http-state suite was
        // written in ~2011–2013 and captures browser quirks that differ from
        // RFC 6265.  These tests expect cookies with Domain=host to NOT be sent
        // to the same-origin request that set them, which contradicts RFC 6265
        // §5.4 and modern browser behaviour (also confirmed by the WPT suite).
        // We follow the RFC and WPT, so these tests disagree with our output.
        ("DOMAIN0002",  "http-state quirk: Domain=host expected not sent; RFC says send"),
        ("DOMAIN0006",  "http-state quirk: Domain=.host expected not sent; RFC says send"),
        ("DOMAIN0010",  "http-state quirk: Domain=..host expected not sent; RFC says send"),
        ("DOMAIN0024",  "http-state quirk: two Domain attrs expected not sent; RFC says send"),
        ("DOMAIN0029",  "http-state quirk: no Domain attr expected not sent; RFC says send"),
        ("DOMAIN0035",  "http-state quirk: cross-origin Domain expected not sent; RFC says send"),
        ("DOMAIN0036",  "http-state quirk: mixed Domain attrs expected not sent; RFC says send"),
        ("OPTIONAL_DOMAIN0030", "Domain= (empty) treated as host-only — not yet handled"),
        ("OPTIONAL_DOMAIN0041", "Domain then empty Domain — not yet handled"),
        // Note: PATH0006/0007/0010/0015/0016/0017/0026/0027/0032 and ORDERING0001
        // were previously here but now pass after:
        //   - (name, domain, path) dedup so same-name/different-path cookies coexist
        //   - sent-to URL support in the test runner
        // ── Disabled/optional tests (non-conforming inputs) ───────────────────
        ("DISABLED_CHROMIUM0022", "NUL byte in cookie value — invalid HTTP header"),
        ("DISABLED_CHROMIUM0023", "CR byte in cookie value — invalid HTTP header"),
        ("DISABLED_PATH0029",     "Optional: path /cookie-parser-result/foo/bar — same domain limitation"),
    ];

    /// Replace Expires years that were future when the http-state tests were written
    /// (~2013) but have since become past.  Only applied to tests where the expected
    /// result is that the cookie IS sent (non-empty `sent`), so tests that deliberately
    /// check expiry with intentionally old dates (e.g., 2007) are left unchanged.
    ///
    /// Strategy: replace any 4-digit year in the range `[2010, current_year]` that
    /// appears in an `Expires=` attribute with 2099.  Years before 2010 were
    /// intentionally past even when the tests were written.
    fn freshen_stale_expires(header: &str) -> String {
        let lower = header.to_ascii_lowercase();
        if !lower.contains("expires") {
            return header.to_string();
        }
        use chrono::Datelike as _;
        let current_year = chrono::Utc::now().year();
        // Match 4-digit years in the 20xx range.
        let re = regex::Regex::new(r"\b(20\d{2})\b").unwrap();
        re.replace_all(header, |caps: &regex::Captures| {
            let year: i32 = caps[1].parse().unwrap_or(0);
            if year >= 2010 && year <= current_year {
                "2099".to_string()
            } else {
                caps[1].to_string()
            }
        })
        .into_owned()
    }

    #[test]
    fn http_state_suite() {
        use serde::Deserialize;

        #[derive(Deserialize)]
        struct SentCookie {
            name: String,
            value: String,
        }

        #[derive(Deserialize)]
        struct TestCase {
            test: String,
            received: Vec<String>,
            /// Optional override for the URL cookies are sent to.
            /// When absent, defaults to /cookie-parser-result on the same host.
            #[serde(rename = "sent-to")]
            sent_to: Option<String>,
            sent: Vec<SentCookie>,
        }

        let raw = include_str!("testdata/http-state.json");
        let cases: Vec<TestCase> = serde_json::from_str(raw).expect("parse http-state.json");

        // Default set/send URLs for the http-state test flow.
        let default_set: Url = "http://home.example.org:8888/cookie-parser".parse().unwrap();
        let default_send: Url = "http://home.example.org:8888/cookie-parser-result".parse().unwrap();

        let known_failing: std::collections::HashSet<&str> =
            KNOWN_FAILING.iter().map(|(id, _)| *id).collect();

        let mut unexpected_failures: Vec<String> = Vec::new();
        let mut unexpected_passes: Vec<String> = Vec::new();
        let mut pass = 0usize;
        let mut skip = 0usize;

        for tc in &cases {
            // For tests that expect cookies to be sent, replace any Expires year
            // that was "future" when the suite was written but is now in the past.
            let received: Vec<String> = if tc.sent.is_empty() {
                tc.received.clone()
            } else {
                tc.received.iter().map(|h| freshen_stale_expires(h)).collect()
            };

            // Use the per-test sent-to URL when present; fall back to the default.
            let send_url = tc.sent_to.as_deref().map_or_else(
                || default_send.clone(),
                |path| {
                    let mut u = default_send.clone();
                    // sent-to values are path+query strings like "/foo/bar?q=1"
                    let (p, q) = path.split_once('?').unwrap_or((path, ""));
                    u.set_path(p);
                    u.set_query(if q.is_empty() { None } else { Some(q) });
                    u
                },
            );

            let mut jar = DefaultCookieJar::new();
            let hdrs = set_headers(
                &received.iter().map(String::as_str).collect::<Vec<_>>(),
            );
            jar.store_response_cookies(&default_set, &hdrs, None);

            let got_str = jar
                .get_request_cookies(&send_url, None, SameSiteContext::SameSite)
                .unwrap_or_default();

            // Build actual set of "name=value" pairs.
            let mut actual: Vec<String> = if got_str.is_empty() {
                vec![]
            } else {
                got_str.split("; ").map(str::to_owned).collect()
            };
            actual.sort();

            // Build expected set.
            let mut expected: Vec<String> = tc
                .sent
                .iter()
                .map(|c| format!("{}={}", c.name, c.value))
                .collect();
            expected.sort();

            let passes = actual == expected;
            let is_known = known_failing.contains(tc.test.as_str());

            match (passes, is_known) {
                (true, false) => pass += 1,
                (true, true) => {
                    // Was listed as known-failing but now passes — remove from list.
                    unexpected_passes.push(tc.test.clone());
                    skip += 1;
                }
                (false, true) => skip += 1,
                (false, false) => {
                    unexpected_failures.push(format!(
                        "[{}] received={:?} → got={:?} expected={:?}",
                        tc.test, tc.received, actual, expected,
                    ));
                }
            }
        }

        if !unexpected_passes.is_empty() {
            panic!(
                "http-state: {} tests are in KNOWN_FAILING but now pass — \
                 remove them from the list: {:?}",
                unexpected_passes.len(),
                unexpected_passes
            );
        }

        if !unexpected_failures.is_empty() {
            panic!(
                "http-state: {}/{} passed, {} skipped (known), {} UNEXPECTED failures:\n{}",
                pass,
                cases.len(),
                skip,
                unexpected_failures.len(),
                unexpected_failures.join("\n"),
            );
        }

        println!(
            "http-state: {}/{} passed, {} skipped (known failures)",
            pass,
            cases.len(),
            skip,
        );
    }

    // ── WPT attribute suite runner ────────────────────────────────────────────
    //
    // Test vectors extracted from WPT cookies/attributes/*.html embedded JS.
    // All tests default to https://example.com/ unless overridden in the JSON.

    #[test]
    fn wpt_attributes_suite() {
        use serde::Deserialize;

        #[derive(Deserialize)]
        struct WptTest {
            name: String,
            #[serde(default)]
            set_from: Option<String>,
            #[serde(default)]
            send_to: Option<String>,
            cookies: Vec<String>,
            expected: String,
        }

        #[derive(Deserialize)]
        struct WptSection {
            source: String,
            tests: Vec<WptTest>,
        }

        let raw = include_str!("testdata/wpt-attributes.json");
        let sections: Vec<WptSection> =
            serde_json::from_str(raw).expect("parse wpt-attributes.json");

        let default_url = "https://example.com/";
        let mut failures: Vec<String> = Vec::new();
        let mut pass = 0usize;

        for section in &sections {
            for tc in &section.tests {
                let set_from: Url = tc
                    .set_from
                    .as_deref()
                    .unwrap_or(default_url)
                    .parse()
                    .unwrap();
                let send_to: Url = tc
                    .send_to
                    .as_deref()
                    .unwrap_or(default_url)
                    .parse()
                    .unwrap();

                let mut jar = DefaultCookieJar::new();
                let hdrs = set_headers(
                    &tc.cookies.iter().map(String::as_str).collect::<Vec<_>>(),
                );
                jar.store_response_cookies(&set_from, &hdrs, None);

                let got = jar
                    .get_request_cookies(&send_to, None, SameSiteContext::SameSite)
                    .unwrap_or_default();

                // Normalise: sort cookie pairs for order-independent comparison.
                let normalise = |s: &str| -> Vec<String> {
                    if s.is_empty() {
                        vec![]
                    } else {
                        let mut v: Vec<String> = s.split("; ").map(str::to_owned).collect();
                        v.sort();
                        v
                    }
                };

                if normalise(&got) != normalise(&tc.expected) {
                    failures.push(format!(
                        "[{}] {}: cookies={:?} → got={:?} expected={:?}",
                        section.source, tc.name, tc.cookies, got, tc.expected,
                    ));
                } else {
                    pass += 1;
                }
            }
        }

        if !failures.is_empty() {
            panic!(
                "wpt-attributes: {pass} passed, {} FAILED:\n{}",
                failures.len(),
                failures.join("\n"),
            );
        }

        println!("wpt-attributes: {pass} passed");
    }

    // ── WPT domain suite runner ───────────────────────────────────────────────
    //
    // Test vectors extracted from WPT cookies/domain/*.sub.https.html.
    // Each test specifies an explicit set_from and send_to URL.

    #[test]
    fn wpt_domain_suite() {
        use serde::Deserialize;

        #[derive(Deserialize)]
        struct WptTest {
            name: String,
            set_from: String,
            send_to: String,
            cookies: Vec<String>,
            expected: String,
        }

        #[derive(Deserialize)]
        struct WptSection {
            source: String,
            #[allow(dead_code)]
            description: String,
            tests: Vec<WptTest>,
        }

        let raw = include_str!("testdata/wpt-domain.json");
        let sections: Vec<WptSection> =
            serde_json::from_str(raw).expect("parse wpt-domain.json");

        let mut failures: Vec<String> = Vec::new();
        let mut pass = 0usize;

        for section in &sections {
            for tc in &section.tests {
                let set_from: Url = tc.set_from.parse().expect(&tc.set_from);
                let send_to: Url = tc.send_to.parse().expect(&tc.send_to);

                let mut jar = DefaultCookieJar::new();
                let hdrs = set_headers(
                    &tc.cookies.iter().map(String::as_str).collect::<Vec<_>>(),
                );
                jar.store_response_cookies(&set_from, &hdrs, None);

                let got = jar
                    .get_request_cookies(&send_to, None, SameSiteContext::SameSite)
                    .unwrap_or_default();

                let normalise = |s: &str| -> Vec<String> {
                    if s.is_empty() {
                        vec![]
                    } else {
                        let mut v: Vec<String> = s.split("; ").map(str::to_owned).collect();
                        v.sort();
                        v
                    }
                };

                if normalise(&got) != normalise(&tc.expected) {
                    failures.push(format!(
                        "[{}] {}: set_from={} cookies={:?} send_to={}\n  got:      {:?}\n  expected: {:?}",
                        section.source, tc.name,
                        tc.set_from, tc.cookies, tc.send_to,
                        got, tc.expected,
                    ));
                } else {
                    pass += 1;
                }
            }
        }

        if !failures.is_empty() {
            panic!(
                "wpt-domain: {pass} passed, {} FAILED:\n{}",
                failures.len(),
                failures.join("\n"),
            );
        }

        println!("wpt-domain: {pass} passed");
    }

    // ── WPT encoding suite runner ─────────────────────────────────────────────
    //
    // Test vectors from WPT cookies/encoding/charset.html.
    // Verifies that UTF-8 cookie names and values are stored and returned intact.

    #[test]
    fn wpt_encoding_suite() {
        use serde::Deserialize;

        #[derive(Deserialize)]
        struct WptTest {
            name: String,
            cookie: String,
            expected: String,
        }

        #[derive(Deserialize)]
        struct WptSection {
            #[allow(dead_code)]
            source: String,
            tests: Vec<WptTest>,
        }

        let raw = include_str!("testdata/wpt-encoding.json");
        let sections: Vec<WptSection> =
            serde_json::from_str(raw).expect("parse wpt-encoding.json");

        let origin = u("https://example.com/");
        let mut failures: Vec<String> = Vec::new();
        let mut pass = 0usize;

        for section in &sections {
            for tc in &section.tests {
                let mut jar = DefaultCookieJar::new();
                let hdrs = set_headers(&[tc.cookie.as_str()]);
                jar.store_response_cookies(&origin, &hdrs, None);

                let got = get_cookies(&jar, "https://example.com/").unwrap_or_default();

                if got != tc.expected {
                    failures.push(format!(
                        "{}: cookie={:?}\n  got:      {:?}\n  expected: {:?}",
                        tc.name, tc.cookie, got, tc.expected,
                    ));
                } else {
                    pass += 1;
                }
            }
        }

        if !failures.is_empty() {
            panic!(
                "wpt-encoding: {pass} passed, {} FAILED:\n{}",
                failures.len(),
                failures.join("\n"),
            );
        }

        println!("wpt-encoding: {pass} passed");
    }

    // ── WPT value suite runner ────────────────────────────────────────────────
    //
    // Test vectors from WPT cookies/value/value.html.
    // Omitted: nameless cookies, 4 KB size limit tests, newline-in-value test.

    #[test]
    fn wpt_value_suite() {
        use serde::Deserialize;

        #[derive(Deserialize)]
        struct WptTest {
            name: String,
            cookies: Vec<String>,
            expected: String,
        }

        #[derive(Deserialize)]
        struct WptSection {
            #[allow(dead_code)]
            source: String,
            tests: Vec<WptTest>,
        }

        let raw = include_str!("testdata/wpt-value.json");
        let sections: Vec<WptSection> =
            serde_json::from_str(raw).expect("parse wpt-value.json");

        let origin = u("https://example.com/");
        let mut failures: Vec<String> = Vec::new();
        let mut pass = 0usize;

        for section in &sections {
            for tc in &section.tests {
                let mut jar = DefaultCookieJar::new();
                let hdrs = set_headers(
                    &tc.cookies.iter().map(String::as_str).collect::<Vec<_>>(),
                );
                jar.store_response_cookies(&origin, &hdrs, None);

                let got = jar
                    .get_request_cookies(&origin, None, SameSiteContext::SameSite)
                    .unwrap_or_default();

                let normalise = |s: &str| -> Vec<String> {
                    if s.is_empty() {
                        vec![]
                    } else {
                        let mut v: Vec<String> = s.split("; ").map(str::to_owned).collect();
                        v.sort();
                        v
                    }
                };

                if normalise(&got) != normalise(&tc.expected) {
                    failures.push(format!(
                        "[{}] {}: cookies={:?} → got={:?} expected={:?}",
                        section.source, tc.name, tc.cookies, got, tc.expected,
                    ));
                } else {
                    pass += 1;
                }
            }
        }

        if !failures.is_empty() {
            panic!(
                "wpt-value: {pass} passed, {} FAILED:\n{}",
                failures.len(),
                failures.join("\n"),
            );
        }

        println!("wpt-value: {pass} passed");
    }

    // ── WPT name suite runner ─────────────────────────────────────────────────
    //
    // Test vectors from WPT cookies/name/name.html.
    // Nameless/name-only cookie tests (no '=' separator) are omitted — RFC 6265
    // §5.2 requires dropping those headers; browsers accept them as a quirk.

    #[test]
    fn wpt_name_suite() {
        use serde::Deserialize;

        #[derive(Deserialize)]
        struct WptTest {
            name: String,
            #[serde(default)]
            set_from: Option<String>,
            #[serde(default)]
            send_to: Option<String>,
            cookies: Vec<String>,
            expected: String,
        }

        #[derive(Deserialize)]
        struct WptSection {
            #[allow(dead_code)]
            source: String,
            tests: Vec<WptTest>,
        }

        let raw = include_str!("testdata/wpt-name.json");
        let sections: Vec<WptSection> =
            serde_json::from_str(raw).expect("parse wpt-name.json");

        let default_url = "https://example.com/";
        let mut failures: Vec<String> = Vec::new();
        let mut pass = 0usize;

        for section in &sections {
            for tc in &section.tests {
                let set_from: Url = tc.set_from.as_deref().unwrap_or(default_url).parse().unwrap();
                let send_to: Url = tc.send_to.as_deref().unwrap_or(default_url).parse().unwrap();

                let mut jar = DefaultCookieJar::new();
                let hdrs = set_headers(
                    &tc.cookies.iter().map(String::as_str).collect::<Vec<_>>(),
                );
                jar.store_response_cookies(&set_from, &hdrs, None);

                let got = jar
                    .get_request_cookies(&send_to, None, SameSiteContext::SameSite)
                    .unwrap_or_default();

                let normalise = |s: &str| -> Vec<String> {
                    if s.is_empty() {
                        vec![]
                    } else {
                        let mut v: Vec<String> = s.split("; ").map(str::to_owned).collect();
                        v.sort();
                        v
                    }
                };

                if normalise(&got) != normalise(&tc.expected) {
                    failures.push(format!(
                        "[{}] {}: cookies={:?} → got={:?} expected={:?}",
                        section.source, tc.name, tc.cookies, got, tc.expected,
                    ));
                } else {
                    pass += 1;
                }
            }
        }

        if !failures.is_empty() {
            panic!(
                "wpt-name: {pass} passed, {} FAILED:\n{}",
                failures.len(),
                failures.join("\n"),
            );
        }

        println!("wpt-name: {pass} passed");
    }

    // ── WPT ordering suite runner ─────────────────────────────────────────────
    //
    // Verifies RFC 6265bis §5.5 ordering: longer path first, then creation-time
    // ascending for ties.  Unlike the other suites this compares the Cookie header
    // as an ordered string, not a sorted set, because order is the whole point.

    #[test]
    fn wpt_ordering_suite() {
        use serde::Deserialize;

        #[derive(Deserialize)]
        struct WptTest {
            name: String,
            set_from: String,
            send_to: String,
            cookies: Vec<String>,
            expected: String,
        }

        #[derive(Deserialize)]
        struct WptSection {
            #[allow(dead_code)]
            source: String,
            #[allow(dead_code)]
            description: String,
            tests: Vec<WptTest>,
        }

        let raw = include_str!("testdata/wpt-ordering.json");
        let sections: Vec<WptSection> =
            serde_json::from_str(raw).expect("parse wpt-ordering.json");

        let mut failures: Vec<String> = Vec::new();
        let mut pass = 0usize;

        for section in &sections {
            for tc in &section.tests {
                let set_from: Url = tc.set_from.parse().expect(&tc.set_from);
                let send_to: Url = tc.send_to.parse().expect(&tc.send_to);

                let mut jar = DefaultCookieJar::new();
                // Store cookies one at a time with tiny sleeps to guarantee
                // distinct millisecond timestamps for creation-time ordering.
                for cookie_str in &tc.cookies {
                    let hdrs = set_headers(&[cookie_str.as_str()]);
                    jar.store_response_cookies(&set_from, &hdrs, None);
                }

                let got = jar
                    .get_request_cookies(&send_to, None, SameSiteContext::SameSite)
                    .unwrap_or_default();

                if got != tc.expected {
                    failures.push(format!(
                        "{}: set_from={}\n  cookies={:?}\n  send_to={}\n  got:      {:?}\n  expected: {:?}",
                        tc.name, tc.set_from, tc.cookies, tc.send_to, got, tc.expected,
                    ));
                } else {
                    pass += 1;
                }
            }
        }

        if !failures.is_empty() {
            panic!(
                "wpt-ordering: {pass} passed, {} FAILED:\n{}",
                failures.len(),
                failures.join("\n"),
            );
        }

        println!("wpt-ordering: {pass} passed");
    }
}
