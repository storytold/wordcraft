//! LanguageTool's answer → proofing issues.
//!
//! The answer is `{"matches": [{"offset", "length", "message", "replacements": [{"value"}],
//! "rule": {"issueType", "category": {"id"}}}, …]}`. Offsets are UTF-16 code units in the text
//! that was sent; everything in the answer is treated as untrusted.

use serde_json::Value;
use wordcraft_proof::{Issue, IssueKind};

use crate::LtError;
use crate::offsets::utf16_range;

/// The largest answer read.
pub const MAX_RESPONSE_BYTES: usize = 4 << 20;
/// Matches kept per paragraph.
pub const MAX_MATCHES: usize = 500;
/// Suggestions kept per match.
pub const MAX_SUGGESTIONS: usize = 8;
/// Longest message / suggestion kept (characters).
const MAX_MESSAGE_CHARS: usize = 300;
const MAX_SUGGESTION_CHARS: usize = 200;

/// The issues in `text` from LanguageTool's answer `body` (sorted by start).
pub fn parse(body: &str, text: &str) -> Result<Vec<Issue>, LtError> {
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(LtError::Protocol("answer too large".into()));
    }
    let v: Value = serde_json::from_str(body).map_err(|e| LtError::Protocol(e.to_string()))?;
    let Some(matches) = v.get("matches").and_then(Value::as_array) else {
        return Err(LtError::Protocol("no `matches` in the answer".into()));
    };
    let mut out: Vec<Issue> = matches.iter().take(MAX_MATCHES).filter_map(|m| issue(m, text)).collect();
    out.sort_by_key(|i| (i.start, i.end));
    Ok(out)
}

/// One match as an issue; `None` when it has no usable range.
pub fn issue(m: &Value, text: &str) -> Option<Issue> {
    let offset = m.get("offset")?.as_u64()?;
    let length = m.get("length")?.as_u64()?;
    let (start, end) = utf16_range(text, offset, length)?;
    let message = m
        .get("message")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .or_else(|| m.get("shortMessage").and_then(Value::as_str))
        .map(|s| clip(s, MAX_MESSAGE_CHARS))
        .unwrap_or_default();
    let suggestions = m
        .get("replacements")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|r| r.get("value").and_then(Value::as_str))
                .filter(|s| !s.is_empty() && s.chars().count() <= MAX_SUGGESTION_CHARS && !s.chars().any(|c| c.is_control() && c != '\t'))
                .take(MAX_SUGGESTIONS)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Some(Issue { start, end, kind: kind_of(m), message, suggestions, lang: Default::default() })
}

/// Spelling for LanguageTool's misspellings (issue type `misspelling` or category `TYPOS`);
/// grammar for everything else (grammar, style, punctuation, typography).
pub fn kind_of(m: &Value) -> IssueKind {
    let rule = m.get("rule");
    let issue_type = rule.and_then(|r| r.get("issueType")).and_then(Value::as_str).unwrap_or("");
    let category = rule.and_then(|r| r.get("category")).and_then(|c| c.get("id")).and_then(Value::as_str).unwrap_or("");
    if issue_type.eq_ignore_ascii_case("misspelling") || category.eq_ignore_ascii_case("TYPOS") { IssueKind::Spelling } else { IssueKind::Grammar }
}

/// `s` without control characters, at most `max` characters.
fn clip(s: &str, max: usize) -> String {
    let mut out: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).take(max).collect();
    if s.chars().count() > max {
        out.push('…');
    }
    out.trim().to_string()
}
