//! Which hyperlink targets may leave the document: followed by the OS, or written into an
//! exported HTML/Markdown file where a browser would run them.

/// The URL Ctrl/⌘+click may hand to the system, if any: a web link with a host or an email link,
/// so a document can't launch `file:` paths, programs or custom-scheme handlers. Exact: a leading
/// space or a control character makes it not a link. Returns what must be opened (see
/// `opened_form`).
pub fn followable_url(url: &str) -> Option<String> {
    let (scheme, _) = url.split_once(':')?;
    if !["http", "https", "mailto"].iter().any(|s| scheme.eq_ignore_ascii_case(s)) {
        return None;
    }
    opened_form(url)
}

/// Web build: the browser's `window.open` reads the link itself and never opens a local file.
#[cfg(target_arch = "wasm32")]
fn opened_form(url: &str) -> Option<String> {
    Some(url.to_string())
}

/// Desktop: the parsed, normalised URL, with a host for http/https. The opener (`webbrowser`)
/// reads any string `url` can't parse, such as `https://host:99999/`, as a local file path (#3732).
#[cfg(not(target_arch = "wasm32"))]
fn opened_form(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let ok = parsed.scheme() == "mailto" || parsed.host_str().is_some_and(|h| !h.is_empty());
    // `webbrowser` on WSL runs `cmd.exe /c start <url>`, escaping only `^` and `&`, and `url`
    // leaves `|` as it is: a raw `|` would pipe into another command there.
    ok.then(|| parsed.as_str().replace('|', "%7C"))
}

/// Whether a hyperlink may be written as an `href` (or Markdown destination, RTF `HYPERLINK`
/// field, ODT `xlink:href`, LaTeX `\href`) in an exported file: a web or email link, a bookmark
/// (`#name`) or a relative path. Scripts (`javascript:`, `vbscript:`), `data:`, `file:` and every
/// other scheme are refused, read the way a browser reads a URL: ASCII control characters and
/// spaces are dropped first, so `java\nscript:` is a script. A protocol-relative reference
/// (`//host/…`) or a UNC path (`\\host\…`, or `//host` once backslashes are read as separators)
/// names another machine — following it on Windows leaks the user's NTLM credentials — so it is
/// refused too.
pub fn is_safe_href(url: &str) -> bool {
    let cleaned: String = url.chars().filter(|c| !c.is_ascii_control() && *c != ' ').collect();
    // `//host` (protocol-relative) and `\\host` (UNC), with `\` read as `/` the way Windows does.
    if cleaned.replace('\\', "/").starts_with("//") {
        return false;
    }
    match scheme(url) {
        Some(s) => matches!(s.as_str(), "http" | "https" | "mailto"),
        None => !cleaned.is_empty(),
    }
}

/// The scheme a browser would see in `url` (RFC 3986 §3.1: a letter, then letters, digits, `+`,
/// `-` or `.`, up to the first `:`), lower-cased; `None` for a relative reference.
fn scheme(url: &str) -> Option<String> {
    let cleaned: String = url.chars().filter(|c| !c.is_ascii_control() && *c != ' ').collect();
    let (s, _) = cleaned.split_once(':')?;
    let mut chars = s.chars();
    let first = chars.next()?;
    (first.is_ascii_alphabetic() && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))).then(|| s.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_and_mail_links_are_followable() {
        for ok in ["https://example.com/a?b#c", "http://example.com", "HTTPS://EXAMPLE.COM", "mailto:someone@example.com"] {
            assert!(followable_url(ok).is_some(), "{ok}");
        }
        for bad in [
            "file:///etc/passwd",
            "C:\\Windows\\System32\\calc.exe",
            "/usr/bin/xterm",
            "javascript:alert(1)",
            "ms-msdt:/id",
            " https://example.com",
            "https\u{0}://example.com",
            "",
            "example.com",
            "#top",
        ] {
            assert!(followable_url(bad).is_none(), "{bad:?}");
        }
    }

    /// Desktop: what gets opened is the parsed URL, not the document's string (#3732).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn followed_links_are_parsed_first() {
        for (ok, opened) in [
            ("https://example.com/a?b#c", "https://example.com/a?b#c"),
            ("http://example.com", "http://example.com/"),
            ("HTTPS://EXAMPLE.COM", "https://example.com/"),
            ("mailto:someone@example.com", "mailto:someone@example.com"),
            ("MAILTO:a@b.c?subject=hi there", "mailto:a@b.c?subject=hi%20there"),
            ("https:\\\\example.com\\a\\b", "https://example.com/a/b"),
            ("https://us er:p@ss@example.com/", "https://us%20er:p%40ss@example.com/"),
            // A `|` would pipe into another command where the opener runs `cmd.exe /c start`.
            ("https://x.test/a|calc?q=|#|", "https://x.test/a%7Ccalc?q=%7C#%7C"),
            ("mailto:a|b@c.d", "mailto:a%7Cb@c.d"),
        ] {
            assert_eq!(followable_url(ok).as_deref(), Some(opened), "{ok}");
        }
        for bad in [
            // An allowed scheme `url` can't parse: the opener would read it as a file path.
            "https://example.com:99999/",
            "https://",
            "http:",
            "https://exa mple.com/",
            "http://[::1/",
        ] {
            assert!(followable_url(bad).is_none(), "{bad:?}");
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    proptest::proptest! {
        /// What gets opened is itself a followable URL: same scheme, no `|`, unchanged when
        /// checked again.
        #[test]
        fn followable_urls_are_stable(url in "(https?|HTTPS|mailto):[\\\\/]{0,2}[ -~]{0,30}") {
            if let Some(u) = followable_url(&url) {
                let parsed = url::Url::parse(&u);
                proptest::prop_assert!(parsed.is_ok(), "{u}");
                let scheme = url.split_once(':').map(|(s, _)| s.to_ascii_lowercase());
                proptest::prop_assert_eq!(parsed.ok().map(|p| p.scheme().to_string()), scheme);
                proptest::prop_assert!(!u.contains('|'), "{u}");
                proptest::prop_assert_eq!(followable_url(&u), Some(u.clone()));
            }
        }
    }

    /// An exported `href` may be a web/mail link, a bookmark or a relative path: never a script
    /// or another scheme, however a browser would normalise it.
    #[test]
    fn exported_hrefs_allow_bookmarks_and_relative_paths_but_no_scripts() {
        for ok in ["https://example.com/x", "mailto:a@b.c", "#top", "other.html", "docs/guide.html#s2", "../up.html", "?q=1"] {
            assert!(is_safe_href(ok), "{ok}");
        }
        for bad in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "java\nscript:alert(1)",
            "\tjavascript:alert(1)",
            " javascript:alert(1)",
            "java\u{0}script:alert(1)",
            "vbscript:MsgBox(1)",
            "data:text/html,<script>alert(1)</script>",
            "file:///etc/passwd",
            "ms-msdt:/id",
            // Protocol-relative and UNC references name another machine (NTLM leak on Windows).
            "//attacker.example/share/x",
            "\\\\attacker.example\\share\\x",
            "/\\attacker.example\\x",
            " //attacker.example/x",
            "",
        ] {
            assert!(!is_safe_href(bad), "{bad:?}");
        }
    }
}
