//! Control-only handshake and size bounds for an opt-in private native port.

/// Native documents use the host's version limit, independently of small control messages.
pub const MAX_NATIVE_BYTES: usize = 32 * 1024 * 1024;

pub fn native_len_ok(length: usize) -> bool {
    length > 0 && length <= MAX_NATIVE_BYTES
}

/// The immutable bundle comes from the fetched path, never a query supplied by the parent.
pub fn bundle_from_path(path: &str) -> Option<&str> {
    let path = path.strip_prefix("/_editor").or_else(|| path.strip_prefix("/workbook-editor")).unwrap_or(path);
    let bundle = path.strip_prefix("/b/")?.strip_suffix("/index.html")?;
    (bundle.len() == 64 && bundle.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))).then_some(bundle)
}

/// `protocol` and `origin` are obtained from the browser's URL parser. Require the supplied
/// string to already be the canonical HTTP(S) origin, without path, credentials or query.
pub fn canonical_origin(input: &str, protocol: &str, origin: &str) -> bool {
    matches!(protocol, "http:" | "https:") && input == origin
}

pub fn connect_matches(value: &serde_json::Value, expected_bundle: &str, ports: usize) -> bool {
    let Some(fields) = value.as_object() else { return false };
    ports == 1
        && fields.len() == 3
        && value.get("type").and_then(serde_json::Value::as_str) == Some("wordcraft:connect")
        && value.get("protocol").and_then(serde_json::Value::as_u64) == Some(1)
        && value.get("bundle").and_then(serde_json::Value::as_str) == Some(expected_bundle)
}

/// Read JS byteLength as a Number before any u32 conversion: large buffers must not wrap.
pub fn native_js_len_ok(length: f64) -> bool {
    length.is_finite() && length.fract() == 0. && length > 0. && length <= MAX_NATIVE_BYTES as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn bundle_is_taken_only_from_a_qualified_path() {
        assert_eq!(bundle_from_path(&format!("/b/{DIGEST}/index.html")), Some(DIGEST));
        assert_eq!(bundle_from_path(&format!("/workbook-editor/b/{DIGEST}/index.html")), Some(DIGEST));
        for path in [
            format!("/other/b/{DIGEST}/index.html"),
            format!("/b/{DIGEST}/other.html"),
            format!("/b/{DIGEST}/index.html/"),
            format!("/b/{}/index.html", DIGEST.to_uppercase()),
            format!("/b/{DIGEST}/../index.html"),
        ] {
            assert_eq!(bundle_from_path(&path), None, "{path}");
        }
    }
    #[test]
    fn connect_has_exactly_three_keys_matching_one_bundle_and_one_port() {
        let good = json!({"type":"wordcraft:connect","protocol":1,"bundle":DIGEST});
        assert!(connect_matches(&good, DIGEST, 1));
        assert!(!connect_matches(&good, DIGEST, 0));
        assert!(!connect_matches(&good, DIGEST, 2));
        assert!(!connect_matches(&good, &"a".repeat(64), 1));
        for bad in [
            json!({"type":"wordcraft:connect","protocol":1,"bundle":DIGEST,"extra":true}),
            json!({"type":"wordcraft:connect","protocol":2,"bundle":DIGEST}),
            json!({"type":"wordcraft:open","protocol":1,"bundle":DIGEST}),
            json!({"type":"wordcraft:connect","protocol":1}),
            json!([]),
        ] {
            assert!(!connect_matches(&bad, DIGEST, 1));
        }
    }
    #[test]
    fn parent_origin_must_be_the_exact_canonical_http_origin() {
        assert!(canonical_origin("https://host.test", "https:", "https://host.test"));
        assert!(canonical_origin("http://127.0.0.1:8090", "http:", "http://127.0.0.1:8090"));
        for (raw, protocol, parsed) in [
            ("null", "file:", "null"),
            ("https://host.test/", "https:", "https://host.test"),
            ("https://host.test/path", "https:", "https://host.test"),
            ("https://user@host.test", "https:", "https://host.test"),
            ("HTTPS://HOST.TEST", "https:", "https://host.test"),
            ("ftp://host.test", "ftp:", "ftp://host.test"),
        ] {
            assert!(!canonical_origin(raw, protocol, parsed));
        }
    }
    #[test]
    fn native_binary_limit_accepts_more_than_four_mib_but_never_more_than_32() {
        assert!(!native_len_ok(0));
        assert!(native_len_ok(4 * 1024 * 1024 + 1));
        assert!(native_len_ok(32 * 1024 * 1024));
        assert!(!native_len_ok(32 * 1024 * 1024 + 1));
    }
    #[test]
    fn qualified_same_host_editor_prefix_is_accepted() {
        assert_eq!(bundle_from_path(&format!("/_editor/b/{DIGEST}/index.html")), Some(DIGEST));
    }

    #[test]
    fn native_js_buffer_lengths_do_not_wrap_at_four_gib() {
        assert!(native_js_len_ok(4_194_305.));
        assert!(native_js_len_ok(33_554_432.));
        for length in [0., -1., 0.5, f64::NAN, f64::INFINITY, 33_554_433., 4_294_967_297.] {
            assert!(!native_js_len_ok(length), "{length}");
        }
    }
}
