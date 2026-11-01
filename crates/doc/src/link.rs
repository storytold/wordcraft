//! Hyperlink targets: where a `HYPERLINK` field code points, and which targets may leave the
//! document: followed by the OS, or written into an exported HTML/Markdown file where a browser
//! would run them.

/// Whether Ctrl/⌘+click may hand a document's hyperlink to the system: web and email links only,
/// so a document can't launch `file:` paths, programs or custom-scheme handlers. Exact: a leading
/// space or a control character makes it not a link.
pub fn is_web_or_mail_link(url: &str) -> bool {
    let Some((scheme, _)) = url.split_once(':') else { return false };
    ["http", "https", "mailto"].iter().any(|s| scheme.eq_ignore_ascii_case(s))
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

/// Where a `HYPERLINK` field code points, as a `w:hyperlink` reads: `url`, `url#bookmark` (`\l`)
/// or `#bookmark`. None for any other field, or one with no target.
pub fn hyperlink_target(instr: &str) -> Option<String> {
    // Only a `HYPERLINK` code is split up (a field's code may run to megabytes).
    if !instr.trim_start().get(..9).is_some_and(|k| k.eq_ignore_ascii_case("HYPERLINK")) {
        return None;
    }
    let args = field_args(instr);
    let mut args = args.iter().peekable();
    if !args.next().is_some_and(|(k, quoted)| !quoted && k.eq_ignore_ascii_case("HYPERLINK")) {
        return None;
    }
    let is_switch = |(a, quoted): &&(String, bool)| !quoted && a.starts_with('\\');
    let (mut url, mut anchor) = (None, None);
    while let Some(arg @ (a, _)) = args.next() {
        if !is_switch(&arg) {
            url = url.or(Some(a.as_str()));
            continue;
        }
        // `\l` names a bookmark, `\o` a tooltip, `\t` a frame, `\*` `\#` `\@` a format; `\m` and `\n` take none.
        let takes = matches!(a.to_ascii_lowercase().as_str(), "\\l" | "\\o" | "\\t" | "\\*" | "\\#" | "\\@");
        let value = takes.then(|| args.next_if(|v| !is_switch(v))).flatten();
        if a.eq_ignore_ascii_case("\\l")
            && let Some((v, _)) = value
        {
            anchor = Some(v.as_str());
        }
    }
    let url = url.map(str::trim).filter(|u| !u.is_empty());
    let anchor = anchor.map(|a| a.trim().trim_start_matches('#')).filter(|a| !a.is_empty());
    match (url, anchor) {
        // `\l` replaces a fragment the address already has, as in Word.
        (Some(u), Some(a)) => Some(format!("{}#{a}", u.split_once('#').map_or(u, |(base, _)| base))),
        (Some(u), None) => Some(u.to_string()),
        (None, Some(a)) => Some(format!("#{a}")),
        (None, None) => None,
    }
}

/// A field code's words: the field's name, its switches and its arguments, each with whether
/// it was quoted. In quotes a backslash escapes the next character, as in Word.
fn field_args(instr: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut chars = instr.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        let mut word = String::new();
        if c == '"' {
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' => word.extend(chars.next()),
                    _ => word.push(c),
                }
            }
            out.push((word, true));
        } else {
            word.push(c);
            while let Some(c) = chars.next_if(|c| !c.is_whitespace() && *c != '"') {
                word.push(c);
            }
            out.push((word, false));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every reader's `HYPERLINK` cases: switches with and without values, quotes, escapes, spelling.
    #[test]
    fn hyperlink_field_codes_name_their_targets() {
        for (code, want) in [
            (r#"HYPERLINK "http://example.com" \l "heading""#, Some("http://example.com#heading")),
            (r#"HYPERLINK \l "heading""#, Some("#heading")),
            (r#"HYPERLINK  \l  "_Toc1" "#, Some("#_Toc1")),
            (r#"Hyperlink "http://example.com/a path""#, Some("http://example.com/a path")),
            ("HYPERLINK https://example.org/b", Some("https://example.org/b")),
            ("HYPERLINK\t\"http://example.com\"", Some("http://example.com")),
            (r#"HYPERLINK"http://example.com""#, Some("http://example.com")),
            (r#"HYPERLINK "http://example.com/path\latest""#, Some("http://example.com/pathlatest")),
            (r#"HYPERLINK "C:\\Docs\\My file.docx""#, Some(r"C:\Docs\My file.docx")),
            (r#"HYPERLINK "say \"hi\"""#, Some(r#"say "hi""#)),
            (r#"HYPERLINK " http://example.com ""#, Some("http://example.com")),
            // Switches that take a value, before or after the address, and those that take none.
            (r#"HYPERLINK "http://example.com" \o "Help text""#, Some("http://example.com")),
            (r#"hyperlink \o "A tip" \t "_blank" "https://example.org/""#, Some("https://example.org/")),
            (r#"HYPERLINK \l "bm" \o "tip""#, Some("#bm")),
            (r#"HYPERLINK \l "Sec" \m"#, Some("#Sec")),
            (r#"HYPERLINK \n "http://example.com""#, Some("http://example.com")),
            (r#"HYPERLINK \l "x" \* MERGEFORMAT"#, Some("#x")),
            (r#"HYPERLINK "http://example.com" \l \o "tip""#, Some("http://example.com")),
            (r#"HYPERLINK \l "bm" \l"#, Some("#bm")),
            // A bookmark's own `#`, an address that has a fragment, and an empty address.
            (r##"HYPERLINK "http://example.com" \l "#top""##, Some("http://example.com#top")),
            (r#"HYPERLINK "http://example.com/#a" \l "b""#, Some("http://example.com/#b")),
            (r#"HYPERLINK "" \l "x""#, Some("#x")),
            // No target, or not a `HYPERLINK` field.
            ("HYPERLINK", None),
            (r#"HYPERLINK "" \o "tip""#, None),
            (r#"HYPERLINKS "https://example.org/""#, None),
            (r#"HYPERLINKX "http://example.com""#, None),
            (r#"HYPERLINK\l "x""#, None),
            ("PAGEREF _Toc1 \\h", None),
            ("ab日本語", None),
            ("", None),
        ] {
            assert_eq!(hyperlink_target(code).as_deref(), want, "{code:?}");
        }
    }

    #[test]
    fn only_web_and_mail_links_are_followable() {
        for ok in ["https://example.com/a?b#c", "http://example.com", "HTTPS://EXAMPLE.COM", "mailto:someone@example.com"] {
            assert!(is_web_or_mail_link(ok), "{ok}");
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
            assert!(!is_web_or_mail_link(bad), "{bad:?}");
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
