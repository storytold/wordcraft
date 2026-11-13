//! A WebDAV lock and the header values that describe it (RFC 4918 §10.5, §10.7).
use std::time::Duration;

/// The longest lock lifetime taken from a server's `Timeout` header. RFC 4918 lets a value run to
/// 2³²−1 seconds; anything past a day is treated as a day, so a refresh still happens.
const MAX_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
/// The shortest refresh interval, so a server answering `Second-0` can't make the client spin.
/// Timeouts come in whole seconds, so half a second still refreshes a 1-second lock in time.
const MIN_REFRESH: Duration = Duration::from_millis(500);
/// The longest refresh interval, also used for a lock with no timeout (`Infinite`).
const MAX_REFRESH: Duration = Duration::from_secs(5 * 60);

/// An exclusive write lock the server granted on one document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lock {
    token: String,
    timeout: Option<Duration>,
}

impl Lock {
    /// A lock with this token; `timeout` is `None` for a lock that never expires (`Infinite`).
    pub fn new(token: impl Into<String>, timeout: Option<Duration>) -> Self {
        Self { token: token.into(), timeout }
    }

    /// The lock token, without angle brackets (e.g. `opaquelocktoken:…`).
    pub fn token(&self) -> &str {
        &self.token
    }

    /// How long the server keeps the lock without a refresh; `None` for no limit.
    pub fn timeout(&self) -> Option<Duration> {
        self.timeout
    }

    /// How often to refresh: half the timeout, so a slow network still makes it, between half a
    /// second and 5 minutes. A lock that expires never gets a refresh interval at or past its expiry.
    pub fn refresh_interval(&self) -> Duration {
        match self.timeout {
            Some(timeout) => (timeout / 2).clamp(MIN_REFRESH, MAX_REFRESH),
            None => MAX_REFRESH,
        }
    }

    /// The value of an `If` header that makes a request conditional on this lock: `(<token>)`.
    pub(crate) fn if_header(&self) -> String {
        format!("(<{}>)", self.token)
    }

    /// The value of a `Lock-Token` header naming this lock: `<token>`.
    pub(crate) fn lock_token_header(&self) -> String {
        format!("<{}>", self.token)
    }
}

/// The token in a `Lock-Token` response header (`<opaquelocktoken:…>`), without the brackets.
/// `None` for an empty or malformed value.
pub(crate) fn parse_lock_token(value: &str) -> Option<String> {
    let value = value.trim();
    let inner = value.strip_prefix('<').and_then(|v| v.strip_suffix('>')).unwrap_or(value).trim();
    // A token is a URI: printable ASCII without spaces or angle brackets. Anything else could not
    // be sent back in the `If` and `Lock-Token` headers anyway.
    let valid = !inner.is_empty() && inner.bytes().all(|b| b.is_ascii_graphic() && b != b'<' && b != b'>');
    valid.then(|| inner.to_owned())
}

/// The lock lifetime in a `Timeout` value: `Second-600` → 600 s, `Infinite` → `Some(None)`.
/// When the server lists several, the first one it understands wins. `None` if none parses.
pub(crate) fn parse_timeout(value: &str) -> Option<Option<Duration>> {
    value.split(',').map(str::trim).find_map(|part| {
        if part.eq_ignore_ascii_case("infinite") {
            return Some(None);
        }
        let digits = part.get(..7).filter(|p| p.eq_ignore_ascii_case("second-")).and_then(|_| part.get(7..))?;
        let seconds: u64 = digits.parse().ok()?;
        Some(Some(Duration::from_secs(seconds).min(MAX_TIMEOUT)))
    })
}

/// The lock lifetime in a LOCK answer's `DAV:lockdiscovery` (`<D:timeout>Second-600</D:timeout>`),
/// whatever prefix the server gives the `DAV:` namespace. `None` if no `timeout` element parses.
pub(crate) fn timeout_in_lockdiscovery(xml: &str) -> Option<Option<Duration>> {
    let mut rest = xml;
    while let Some(open) = rest.find('<') {
        rest = rest.get(open + 1..)?;
        let close = rest.find('>')?;
        let tag = rest.get(..close)?;
        rest = rest.get(close + 1..)?;
        let name = tag.split_whitespace().next().unwrap_or_default();
        let local = name.rsplit(':').next().unwrap_or(name);
        // Only an opening `timeout` tag with text: not `</D:timeout>`, `<D:timeout/>` or `<?…?>`.
        if local == "timeout" && !name.starts_with('/') && !tag.ends_with('/') {
            return parse_timeout(rest.get(..rest.find('<')?)?);
        }
    }
    None
}

/// The `Timeout` a LOCK request asks for.
pub(crate) fn timeout_header(timeout: Duration) -> String {
    format!("Second-{}", timeout.as_secs().max(1))
}

/// `text` safe to place inside an XML element.
pub(crate) fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Control characters other than tab, newline and return are not allowed in XML 1.0.
            c if c.is_control() && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// The body of a LOCK request for an exclusive write lock owned by `owner` (RFC 4918 §9.10.7).
pub(crate) fn lockinfo(owner: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:lockinfo xmlns:D=\"DAV:\"><D:lockscope><D:exclusive/></D:lockscope><D:locktype><D:write/></D:locktype><D:owner>{}</D:owner></D:lockinfo>",
        xml_escape(owner)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_tokens_lose_their_brackets_and_bad_ones_are_refused() {
        assert_eq!(parse_lock_token("<opaquelocktoken:e71d4fae-5dec>").as_deref(), Some("opaquelocktoken:e71d4fae-5dec"));
        assert_eq!(parse_lock_token(" urn:uuid:1234 ").as_deref(), Some("urn:uuid:1234"));
        assert_eq!(parse_lock_token("<>"), None);
        assert_eq!(parse_lock_token(""), None);
        assert_eq!(parse_lock_token("<a b>"), None);
        assert_eq!(parse_lock_token("<a\r\nX-Injected: 1>"), None);
    }

    #[test]
    fn timeouts_parse_seconds_and_infinite_and_cap_huge_values() {
        assert_eq!(parse_timeout("Second-600"), Some(Some(Duration::from_secs(600))));
        assert_eq!(parse_timeout("second-30"), Some(Some(Duration::from_secs(30))));
        assert_eq!(parse_timeout("Infinite"), Some(None));
        assert_eq!(parse_timeout("Bogus, Second-120"), Some(Some(Duration::from_secs(120))));
        assert_eq!(parse_timeout("Second-4100000000"), Some(Some(MAX_TIMEOUT)));
        assert_eq!(parse_timeout("Second-99999999999999999999999"), None);
        assert_eq!(parse_timeout("Second-"), None);
        assert_eq!(parse_timeout("Sec"), None);
        assert_eq!(parse_timeout("Sécond-1"), None);
        assert_eq!(parse_timeout(""), None);
    }

    #[test]
    fn the_granted_timeout_is_read_from_lockdiscovery_whatever_the_prefix() {
        let answer = |timeout: &str| {
            format!(
                "<?xml version=\"1.0\"?><D:prop xmlns:D=\"DAV:\"><D:lockdiscovery><D:activelock><D:depth>0</D:depth>{timeout}<D:locktoken><D:href>urn:uuid:1</D:href></D:locktoken></D:activelock></D:lockdiscovery></D:prop>"
            )
        };
        assert_eq!(timeout_in_lockdiscovery(&answer("<D:timeout>Second-120</D:timeout>")), Some(Some(Duration::from_secs(120))));
        assert_eq!(timeout_in_lockdiscovery(&answer("<D:timeout> Infinite </D:timeout>")), Some(None));
        assert_eq!(
            timeout_in_lockdiscovery("<d:prop xmlns:d=\"DAV:\"><d:timeout>Second-30</d:timeout></d:prop>"),
            Some(Some(Duration::from_secs(30)))
        );
        assert_eq!(timeout_in_lockdiscovery("<prop xmlns=\"DAV:\"><timeout>Second-45</timeout></prop>"), Some(Some(Duration::from_secs(45))));
        assert_eq!(timeout_in_lockdiscovery(&answer("")), None);
        assert_eq!(timeout_in_lockdiscovery(&answer("<D:timeout/>")), None);
        assert_eq!(timeout_in_lockdiscovery("<D:timeout>Second-30"), None);
        assert_eq!(timeout_in_lockdiscovery("<D:timeout"), None);
        assert_eq!(timeout_in_lockdiscovery(""), None);
    }

    #[test]
    fn refresh_comes_at_half_the_timeout_within_bounds() {
        assert_eq!(Lock::new("t", Some(Duration::from_secs(600))).refresh_interval(), Duration::from_secs(300));
        assert_eq!(Lock::new("t", Some(Duration::from_secs(120))).refresh_interval(), Duration::from_secs(60));
        // Short locks still refresh before they expire.
        assert_eq!(Lock::new("t", Some(Duration::from_secs(4))).refresh_interval(), Duration::from_secs(2));
        assert_eq!(Lock::new("t", Some(Duration::from_secs(1))).refresh_interval(), Duration::from_millis(500));
        assert_eq!(Lock::new("t", Some(Duration::from_secs(0))).refresh_interval(), MIN_REFRESH);
        assert_eq!(Lock::new("t", Some(MAX_TIMEOUT)).refresh_interval(), MAX_REFRESH);
        assert_eq!(Lock::new("t", None).refresh_interval(), MAX_REFRESH);
    }

    #[test]
    fn headers_name_the_lock() {
        let lock = Lock::new("opaquelocktoken:abc", None);
        assert_eq!(lock.if_header(), "(<opaquelocktoken:abc>)");
        assert_eq!(lock.lock_token_header(), "<opaquelocktoken:abc>");
        assert_eq!(timeout_header(Duration::from_secs(600)), "Second-600");
        assert_eq!(timeout_header(Duration::ZERO), "Second-1");
    }

    #[test]
    fn the_owner_is_escaped_in_the_lock_request() {
        let body = lockinfo("Ana & <Bo>\u{1}");
        assert!(body.contains("<D:owner>Ana &amp; &lt;Bo&gt;</D:owner>"), "{body}");
        assert!(body.contains("<D:exclusive/>") && body.contains("<D:write/>"), "{body}");
    }
}
