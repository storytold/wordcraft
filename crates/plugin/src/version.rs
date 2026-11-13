//! Versions and version ranges for add-in requirements (manifest 1.3).
//!
//! Versions are `MAJOR.MINOR.PATCH` with missing parts 0 (`1.2` is `1.2.0`), an optional
//! pre-release (`-beta.1`, sorting before the release) and build metadata (`+abc`, ignored). An
//! Office add-in's four-part `1.0.0.0` compares as its first three parts.
//!
//! Ranges are Cargo's: comma-separated comparators that must all hold. `^1.2.3` (also a bare
//! `1.2.3`), `~1.2.3`, `>=`, `>`, `<=`, `<`, `=` with a full or partial version, wildcards `1.*`,
//! `1.2.*` (also `x`), and `*` or empty for any version. As in Cargo, a pre-release version only
//! matches a range whose comparator names a pre-release of the same `MAJOR.MINOR.PATCH`; `*` (or
//! an empty range) matches every version, pre-releases included.
//!
//! Everything here is hostile input: lengths are bounded and nothing panics.

use std::cmp::Ordering;
use std::fmt;

/// Longest range text (characters).
pub const MAX_RANGE: usize = 128;
/// Longest version text.
const MAX_VERSION: usize = 128;
/// Most comparators in one range.
const MAX_COMPARATORS: usize = 16;

/// One dot-separated part of a pre-release (`beta`, `1`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Ident {
    Num(u64),
    Alpha(String),
}

impl Ord for Ident {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Ident::Num(a), Ident::Num(b)) => a.cmp(b),
            // Numeric identifiers sort before alphanumeric ones (SemVer 2.0 §11).
            (Ident::Num(_), Ident::Alpha(_)) => Ordering::Less,
            (Ident::Alpha(_), Ident::Num(_)) => Ordering::Greater,
            (Ident::Alpha(a), Ident::Alpha(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for Ident {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A version: `MAJOR.MINOR.PATCH[-pre]`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// Empty for a release.
    pub pre: Vec<Ident>,
}

impl Version {
    pub const fn new(major: u64, minor: u64, patch: u64) -> Version {
        Version { major, minor, patch, pre: Vec::new() }
    }

    /// Reads `1`, `1.2`, `1.2.3`, `1.2.3.4` (the fourth part ignored), with `-pre` and `+build`.
    pub fn parse(text: &str) -> Result<Version, String> {
        let bad = || format!("`{}` isn't a version", shorten(text));
        let s = text.trim();
        if s.is_empty() || s.len() > MAX_VERSION {
            return Err(bad());
        }
        let s = s.split_once('+').map_or(s, |(v, build)| if valid_build(build) { v } else { "" });
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (s, None),
        };
        let parts = core.split('.').map(number).collect::<Option<Vec<u64>>>().ok_or_else(bad)?;
        if parts.is_empty() || parts.len() > 4 {
            return Err(bad());
        }
        let pre = match pre {
            Some(p) => parse_pre(p).ok_or_else(bad)?,
            None => Vec::new(),
        };
        let part = |i: usize| parts.get(i).copied().unwrap_or(0);
        Ok(Version { major: part(0), minor: part(1), patch: part(2), pre })
    }

    /// A release tag's version: `v0.1.5`, `V0.1.5` or `0.1.5`; `None` for other tags.
    pub fn from_tag(tag: &str) -> Option<Version> {
        let t = tag.trim();
        let t = t.strip_prefix(['v', 'V']).unwrap_or(t);
        if !t.starts_with(|c: char| c.is_ascii_digit()) {
            return None;
        }
        Version::parse(t).ok()
    }

    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }

    fn triple(&self) -> (u64, u64, u64) {
        (self.major, self.minor, self.patch)
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.triple().cmp(&other.triple()).then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            // A pre-release sorts before its release.
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => self.pre.cmp(&other.pre),
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        for (i, p) in self.pre.iter().enumerate() {
            f.write_str(if i == 0 { "-" } else { "." })?;
            match p {
                Ident::Num(n) => write!(f, "{n}")?,
                Ident::Alpha(s) => f.write_str(s)?,
            }
        }
        Ok(())
    }
}

fn shorten(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() > 40 { format!("{}…", s.chars().take(40).collect::<String>()) } else { s.to_string() }
}

/// A numeric part: digits only, no leading zero (except `0`), fits a u64.
fn number(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) || (s.len() > 1 && s.starts_with('0')) {
        return None;
    }
    s.parse().ok()
}

fn parse_pre(s: &str) -> Option<Vec<Ident>> {
    s.split('.')
        .map(|p| {
            if p.is_empty() || !p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                None
            } else if p.bytes().all(|b| b.is_ascii_digit()) {
                number(p).map(Ident::Num)
            } else {
                Some(Ident::Alpha(p.to_string()))
            }
        })
        .collect()
}

fn valid_build(s: &str) -> bool {
    !s.is_empty() && s.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

// ---------------------------------------------------------------- ranges

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Ge,
    Gt,
    Le,
    Lt,
    Eq,
}

/// One primitive bound.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Bound {
    op: Op,
    v: Version,
}

impl Bound {
    fn holds(&self, v: &Version) -> bool {
        let o = v.cmp(&self.v);
        match self.op {
            Op::Ge => o != Ordering::Less,
            Op::Gt => o == Ordering::Greater,
            Op::Le => o != Ordering::Greater,
            Op::Lt => o == Ordering::Less,
            Op::Eq => o == Ordering::Equal,
        }
    }
}

/// A version range (`^0.1.4`, `>=0.1, <0.3`, `1.*`, `*`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Range {
    /// As written (trimmed).
    text: String,
    /// All must hold; none: any version.
    bounds: Vec<Bound>,
    /// `MAJOR.MINOR.PATCH` of comparators naming a pre-release: the pre-releases that may match.
    pre_ok: Vec<(u64, u64, u64)>,
    any: bool,
}

/// A partial version as written in a comparator: missing parts are `None`, wildcards too.
struct Partial {
    major: u64,
    minor: Option<u64>,
    patch: Option<u64>,
    pre: Vec<Ident>,
    /// Written with a wildcard (`1.*`), not just short (`1`).
    wild: bool,
}

impl Partial {
    fn parse(s: &str) -> Option<Partial> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let s = match s.split_once('+') {
            Some((v, build)) if valid_build(build) => v,
            Some(_) => return None,
            None => s,
        };
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, parse_pre(p)?),
            None => (s, Vec::new()),
        };
        let parts: Vec<&str> = core.split('.').collect();
        if parts.len() > 3 {
            return None;
        }
        let wild = |p: &str| matches!(p, "*" | "x" | "X");
        let mut nums: Vec<Option<u64>> = Vec::new();
        let mut seen_wild = false;
        for p in &parts {
            if wild(p) {
                seen_wild = true;
                nums.push(None);
            } else if seen_wild {
                return None; // `1.*.3`
            } else {
                nums.push(Some(number(p)?));
            }
        }
        let major = nums.first().copied().flatten()?;
        let minor = nums.get(1).copied().flatten();
        let patch = nums.get(2).copied().flatten();
        if (minor.is_none() || patch.is_none()) && !pre.is_empty() {
            return None; // `1.2-beta`
        }
        Some(Partial { major, minor, patch, pre, wild: seen_wild })
    }

    fn low(&self) -> Version {
        Version { major: self.major, minor: self.minor.unwrap_or(0), patch: self.patch.unwrap_or(0), pre: self.pre.clone() }
    }
}

impl Range {
    /// Any version (`*`).
    pub fn any() -> Range {
        Range { text: "*".into(), bounds: Vec::new(), pre_ok: Vec::new(), any: true }
    }

    /// Parses a range; an error names what's wrong.
    pub fn parse(text: &str) -> Result<Range, String> {
        let t = text.trim();
        let bad = || format!("`{}` isn't a version range", shorten(t));
        if t.chars().count() > MAX_RANGE {
            return Err(format!("`{}` isn't a version range (longer than {MAX_RANGE} characters)", shorten(t)));
        }
        if t.is_empty() || t == "*" || t.eq_ignore_ascii_case("x") {
            return Ok(Range { text: if t.is_empty() { "*".into() } else { t.to_string() }, ..Range::any() });
        }
        let parts: Vec<&str> = t.split(',').collect();
        if parts.len() > MAX_COMPARATORS {
            return Err(bad());
        }
        let mut r = Range { text: t.to_string(), bounds: Vec::new(), pre_ok: Vec::new(), any: false };
        for part in parts {
            let part = part.trim();
            let (op, rest) = split_op(part);
            let rest = rest.trim();
            if rest.contains(char::is_whitespace) {
                return Err(bad());
            }
            if matches!(rest, "*" | "x" | "X") {
                // `>=*`, `^*` and the like aren't ranges; a bare `*` inside a list is.
                if op.is_some() {
                    return Err(bad());
                }
                continue;
            }
            let p = Partial::parse(rest).ok_or_else(bad)?;
            if !p.pre.is_empty() {
                r.pre_ok.push((p.major, p.minor.unwrap_or(0), p.patch.unwrap_or(0)));
            }
            r.bounds.extend(desugar(op, &p).ok_or_else(bad)?);
        }
        Ok(r)
    }

    /// Whether `v` is in the range.
    pub fn matches(&self, v: &Version) -> bool {
        if self.any {
            return true;
        }
        if v.is_prerelease() && !self.pre_ok.contains(&v.triple()) {
            return false;
        }
        self.bounds.iter().all(|b| b.holds(v))
    }

    /// Whether the range is `*` (any version).
    pub fn is_any(&self) -> bool {
        self.any || self.bounds.is_empty()
    }

    /// Whether a comparator names a pre-release (so pre-release versions can match).
    pub fn allows_prereleases(&self) -> bool {
        self.any || !self.pre_ok.is_empty()
    }

    /// The range as written (`*` for an empty one).
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// For people: `0.8 or later` for `>=0.8`, otherwise the range as written.
    pub fn describe(&self) -> String {
        if let Some(rest) = self.text.strip_prefix(">=")
            && !rest.contains(',')
        {
            return format!("{} or later", rest.trim());
        }
        self.text.clone()
    }
}

impl fmt::Display for Range {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

fn split_op(s: &str) -> (Option<&str>, &str) {
    for op in [">=", "<=", ">", "<", "=", "^", "~"] {
        if let Some(rest) = s.strip_prefix(op) {
            return (Some(op), rest);
        }
    }
    (None, s)
}

fn bump_major(p: &Partial) -> Option<Version> {
    Some(Version::new(p.major.checked_add(1)?, 0, 0))
}

fn bump_minor(p: &Partial, minor: u64) -> Option<Version> {
    Some(Version::new(p.major, minor.checked_add(1)?, 0))
}

/// Cargo's meaning of one comparator, as primitive bounds.
fn desugar(op: Option<&str>, p: &Partial) -> Option<Vec<Bound>> {
    let b = |op, v| Bound { op, v };
    let low = p.low();
    Some(match (op, p.minor, p.patch) {
        // `=1.2.3`; `=1.2` and `=1` are `1.2.*` and `1.*`.
        (Some("="), Some(_), Some(_)) => vec![b(Op::Eq, low)],
        (Some("="), Some(mi), None) => vec![b(Op::Ge, low), b(Op::Lt, bump_minor(p, mi)?)],
        (Some("="), None, _) => vec![b(Op::Ge, low), b(Op::Lt, bump_major(p)?)],
        // `>1.2` is `>=1.3.0`; `>1` is `>=2.0.0`.
        (Some(">"), Some(_), Some(_)) => vec![b(Op::Gt, low)],
        (Some(">"), Some(mi), None) => vec![b(Op::Ge, bump_minor(p, mi)?)],
        (Some(">"), None, _) => vec![b(Op::Ge, bump_major(p)?)],
        (Some(">="), ..) => vec![b(Op::Ge, low)],
        (Some("<"), ..) => vec![b(Op::Lt, low)],
        // `<=1.2` is `<1.3.0`; `<=1` is `<2.0.0`.
        (Some("<="), Some(_), Some(_)) => vec![b(Op::Le, low)],
        (Some("<="), Some(mi), None) => vec![b(Op::Lt, bump_minor(p, mi)?)],
        (Some("<="), None, _) => vec![b(Op::Lt, bump_major(p)?)],
        // `~1.2.3` and `~1.2`: `<1.3.0`; `~1`: `<2.0.0`.
        (Some("~"), Some(mi), _) => vec![b(Op::Ge, low), b(Op::Lt, bump_minor(p, mi)?)],
        (Some("~"), None, _) => vec![b(Op::Ge, low), b(Op::Lt, bump_major(p)?)],
        // Wildcards without an operator: `1.*`, `1.2.*`.
        (None, None, _) if p.wild => vec![b(Op::Ge, low), b(Op::Lt, bump_major(p)?)],
        (None, Some(mi), None) if p.wild => vec![b(Op::Ge, low), b(Op::Lt, bump_minor(p, mi)?)],
        // Caret (and a bare version): the left-most non-zero part may not change.
        (Some("^") | None, minor, patch) => {
            let upper = match (p.major, minor, patch) {
                (0, Some(0), Some(pa)) => Version::new(0, 0, pa.checked_add(1)?),
                (0, Some(mi), _) => Version::new(0, mi.checked_add(1)?, 0),
                (_, _, _) => bump_major(p)?,
            };
            vec![b(Op::Ge, low), b(Op::Lt, upper)]
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
