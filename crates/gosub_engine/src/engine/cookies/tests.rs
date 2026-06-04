//! Comprehensive table-driven cookie tests.
//!
//! Each [`SetCookieCase`] describes one `Set-Cookie` header received at a given
//! URL, and the `Cookie` header value expected on the next request to a given
//! URL.  This drives both the parsing *and* the sending sides of the jar.
//!
//! Many cases are adapted from the W3C Web Platform Tests
//! (`cookies/http-state/`) and the RFC 6265 examples.

#[cfg(test)]
mod tests {
    use crate::engine::cookies::cookie_jar::{CookieJar, DefaultCookieJar, SameSiteContext};
    use http::HeaderMap;
    use url::Url;

    fn jar() -> DefaultCookieJar {
        DefaultCookieJar::new()
    }

    fn u(s: &str) -> Url {
        s.parse().expect(s)
    }

    fn set_headers(values: &[&str]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for &v in values {
            map.append("set-cookie", v.parse().unwrap());
        }
        map
    }

    fn get_cookies(jar: &DefaultCookieJar, request_url: &str) -> Option<String> {
        jar.get_request_cookies(&u(request_url), None, SameSiteContext::SameSite)
    }

    /// A single round-trip test case.
    struct Case {
        /// Human-readable label shown on failure.
        desc: &'static str,
        /// URL from which the `Set-Cookie` header was received.
        set_from: &'static str,
        /// The `Set-Cookie` header value(s) to store.
        headers: &'static [&'static str],
        /// URL of the subsequent request.
        send_to: &'static str,
        /// Expected `Cookie` header value (`None` → cookie must not be sent).
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
                desc: "value with semicolons in rest — only first segment is value",
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
                desc: "Domain=example.com sent to exact host",
                set_from: "https://foo.example.com/",
                headers: &["a=1; Path=/; Domain=example.com"],
                send_to: "https://foo.example.com/",
                expected: Some("a=1"),
            },
            Case {
                desc: "no Domain attribute sent to exact origin only",
                set_from: "https://example.com/",
                headers: &["a=1; Path=/"],
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
                desc: "Secure cookie NOT sent over HTTP",
                set_from: "https://example.com/",
                headers: &["s=1; Path=/; Secure"],
                send_to: "http://example.com/",
                expected: None,
            },
            Case {
                desc: "Non-secure cookie sent over HTTP",
                set_from: "http://example.com/",
                headers: &["n=1; Path=/"],
                send_to: "http://example.com/",
                expected: Some("n=1"),
            },
            Case {
                desc: "Secure cookie from HTTP dropped on storage",
                set_from: "http://example.com/",
                headers: &["s=1; Path=/; Secure"],
                send_to: "https://example.com/",
                expected: None,
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
            "second Set-Cookie for same name must overwrite first"
        );
    }

    // ── SameSite attribute storage ────────────────────────────────────────────

    #[test]
    fn samesite_values_are_normalised() {
        // The SameSite value should be stored and then govern sending, not silently dropped.
        let cases = [
            ("SameSite=Lax", SameSiteContext::SameSite, true),
            ("SameSite=lax", SameSiteContext::SameSite, true),
            ("SameSite=LAX", SameSiteContext::SameSite, true),
            ("SameSite=Strict", SameSiteContext::SameSite, true),
            ("SameSite=Strict", SameSiteContext::CrossSiteNavigation, false),
            ("SameSite=Lax", SameSiteContext::CrossSiteNavigation, true),
            ("SameSite=Lax", SameSiteContext::CrossSite, false),
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
                "attr={attr:?} ctx={ctx:?}: expected sent={expect_sent}"
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
    fn duplicate_attributes_last_value_wins() {
        // Two Path attributes: the second should overwrite the first.
        let mut jar = jar();
        let origin = u("https://example.com/");
        // Two Path attributes — last one should win.
        jar.store_response_cookies(
            &origin,
            &set_headers(&["a=1; Path=/first; Path=/second"]),
            None,
        );
        // Should be sent at /second but not at /first.
        assert!(
            get_cookies(&jar, "https://example.com/second/x").is_some(),
            "second Path should apply"
        );
        assert!(
            get_cookies(&jar, "https://example.com/first/x").is_none(),
            "first Path should have been overwritten"
        );
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
                desc: "HttpOnly (mixed case) is recognised",
                set_from: "https://example.com/",
                headers: &["h=1; Path=/; HttpOnly"],
                send_to: "https://example.com/",
                expected: Some("h=1"), // HttpOnly only affects JS access, not HTTP sending
            },
        ]);
    }

    // ── Domain attribute edge cases ───────────────────────────────────────────

    #[test]
    fn domain_leading_dot_stripped() {
        // "Domain=.example.com" is equivalent to "Domain=example.com" after
        // the leading dot is stripped per RFC 6265 §5.2 step 5.
        let mut jar = jar();
        let origin = u("https://example.com/");
        jar.store_response_cookies(
            &origin,
            &set_headers(&["a=1; Path=/; Domain=.example.com"]),
            None,
        );
        assert_eq!(
            get_cookies(&jar, "https://example.com/").as_deref(),
            Some("a=1"),
            "leading dot in Domain must be stripped and cookie stored"
        );
    }

    #[test]
    fn domain_public_suffix_dropped() {
        run(&[
            Case {
                desc: "Domain=com (bare TLD) must drop the cookie",
                set_from: "https://foo.com/",
                headers: &["a=1; Path=/; Domain=com"],
                send_to: "https://foo.com/",
                expected: None,
            },
            Case {
                desc: "Domain=co.uk (eTLD) must drop the cookie",
                set_from: "https://foo.co.uk/",
                headers: &["a=1; Path=/; Domain=co.uk"],
                send_to: "https://foo.co.uk/",
                expected: None,
            },
        ]);
    }

    // ── Expiry / Max-Age ──────────────────────────────────────────────────────

    #[test]
    fn max_age_zero_deletes_cookie() {
        let mut jar = jar();
        let origin = u("https://example.com/");
        jar.store_response_cookies(&origin, &set_headers(&["a=1; Path=/"]), None);
        jar.store_response_cookies(&origin, &set_headers(&["a=1; Path=/; Max-Age=0"]), None);
        assert!(
            get_cookies(&jar, "https://example.com/").is_none(),
            "Max-Age=0 must delete the existing cookie"
        );
    }

    #[test]
    fn max_age_overrides_past_expires() {
        // Expires is in the past but Max-Age is positive — cookie should survive.
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
        // An unparseable Expires value → session cookie (never expires).
        run(&[Case {
            desc: "invalid Expires → session cookie",
            set_from: "https://example.com/",
            headers: &["a=1; Path=/; Expires=not-a-date"],
            send_to: "https://example.com/",
            expected: Some("a=1"),
        }]);
    }

    #[test]
    fn invalid_max_age_treated_as_session() {
        run(&[Case {
            desc: "non-numeric Max-Age → session cookie",
            set_from: "https://example.com/",
            headers: &["a=1; Path=/; Max-Age=invalid"],
            send_to: "https://example.com/",
            expected: Some("a=1"),
        }]);
    }

    // ── Cookie name prefixes ──────────────────────────────────────────────────

    #[test]
    fn prefix_enforcement_table() {
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
                desc: "__Host- without Secure dropped",
                set_from: "https://example.com/",
                headers: &["__Host-id=1; Path=/"],
                send_to: "https://example.com/",
                expected: None,
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
            Case {
                desc: "regular cookie with __ prefix but not __Secure- or __Host- is unaffected",
                set_from: "https://example.com/",
                headers: &["__other=1; Path=/"],
                send_to: "https://example.com/",
                expected: Some("__other=1"),
            },
        ]);
    }

    // ── HttpOnly flag ─────────────────────────────────────────────────────────

    #[test]
    fn http_only_cookie_is_sent_in_http_requests() {
        // HttpOnly restricts JS access, not HTTP sending.
        run(&[Case {
            desc: "HttpOnly cookie is still sent in HTTP requests",
            set_from: "https://example.com/",
            headers: &["h=1; Path=/; HttpOnly"],
            send_to: "https://example.com/",
            expected: Some("h=1"),
        }]);
    }

    // ── Purge ─────────────────────────────────────────────────────────────────

    #[test]
    fn purge_only_removes_expired() {
        let mut jar = jar();
        let origin = u("https://example.com/");
        jar.store_response_cookies(&origin, &set_headers(&["live=1; Path=/; Max-Age=3600"]), None);
        // Manually inject an already-expired cookie.
        jar.entries
            .entry(origin.origin().ascii_serialization())
            .or_default()
            .push(crate::engine::cookies::Cookie {
                name: "dead".into(),
                value: "0".into(),
                path: Some("/".into()),
                domain: None,
                secure: false,
                expires: Some(1), // epoch
                same_site: None,
                http_only: false,
            });

        jar.purge_expired();

        let result = get_cookies(&jar, "https://example.com/").unwrap_or_default();
        assert!(result.contains("live=1"), "live cookie must survive purge");
        assert!(!result.contains("dead=0"), "expired cookie must be removed");
    }

    // ── WPT http-state derived cases ──────────────────────────────────────────
    // Adapted from https://github.com/web-platform-tests/wpt/tree/master/cookies/http-state

    #[test]
    fn wpt_name_value() {
        run(&[
            Case {
                desc: "wpt: simple",
                set_from: "http://home.example.org:8888/cookie-parser",
                headers: &["foo=bar"],
                send_to: "http://home.example.org:8888/cookie-parser",
                expected: Some("foo=bar"),
            },
            Case {
                desc: "wpt: empty string value",
                set_from: "http://home.example.org:8888/cookie-parser",
                headers: &["foo="],
                send_to: "http://home.example.org:8888/cookie-parser",
                expected: Some("foo="),
            },
            Case {
                desc: "wpt: value with spaces",
                set_from: "http://home.example.org:8888/cookie-parser",
                headers: &["foo=bar baz"],
                send_to: "http://home.example.org:8888/cookie-parser",
                expected: Some("foo=bar baz"),
            },
            Case {
                desc: "wpt: value with tab",
                set_from: "http://home.example.org:8888/cookie-parser",
                headers: &["foo=bar\tbaz"],
                send_to: "http://home.example.org:8888/cookie-parser",
                expected: Some("foo=bar\tbaz"),
            },
            Case {
                desc: "wpt: leading semicolon in value",
                set_from: "http://home.example.org:8888/cookie-parser",
                headers: &[";foo=bar"],
                send_to: "http://home.example.org:8888/cookie-parser",
                expected: None, // ';' before name=value is invalid
            },
        ]);
    }

    #[test]
    fn wpt_path() {
        run(&[
            Case {
                desc: "wpt: path=/foo/bar matches /foo/bar",
                set_from: "http://home.example.org:8888/cookie-parser",
                headers: &["foo=bar; path=/foo/bar"],
                send_to: "http://home.example.org:8888/foo/bar",
                expected: Some("foo=bar"),
            },
            Case {
                desc: "wpt: path=/foo/bar does not match /foo",
                set_from: "http://home.example.org:8888/cookie-parser",
                headers: &["foo=bar; path=/foo/bar"],
                send_to: "http://home.example.org:8888/foo",
                expected: None,
            },
            Case {
                desc: "wpt: path=/foo/bar does not match /foobar",
                set_from: "http://home.example.org:8888/cookie-parser",
                headers: &["foo=bar; path=/foo/bar"],
                send_to: "http://home.example.org:8888/foobar",
                expected: None,
            },
        ]);
    }
}
