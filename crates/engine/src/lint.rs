//! Document-level lint findings (court checks, accessibility-style rules modules run on demand).
//! Modules push findings here; Review › Proofing and `review.issues` surface them beside
//! spelling and grammar. No domain logic lives in this file — only storage and JSON shape.

use serde_json::{Value, json};
use wordcraft_doc::Pos;

/// Most findings kept per run (a hostile document can't make the list huge).
pub const MAX_FINDINGS: usize = 500;

/// A document-level finding, before it is turned into JSON for commands and the UI.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    /// Stable id for the rule (`wordLimit`, `noHyperlinks`, …).
    pub check_id: String,
    /// `error` or `warning`.
    pub severity: String,
    pub message: String,
    /// When the problem is tied to a range in the document.
    pub start: Option<Pos>,
    pub end: Option<Pos>,
}

impl Finding {
    pub fn error(check_id: impl Into<String>, message: impl Into<String>) -> Self {
        Finding { check_id: check_id.into(), severity: "error".into(), message: message.into(), start: None, end: None }
    }

    pub fn warning(check_id: impl Into<String>, message: impl Into<String>) -> Self {
        Finding { check_id: check_id.into(), severity: "warning".into(), message: message.into(), start: None, end: None }
    }

    pub fn at(check_id: impl Into<String>, severity: impl Into<String>, message: impl Into<String>, start: Pos, end: Pos) -> Self {
        Finding { check_id: check_id.into(), severity: severity.into(), message: message.into(), start: Some(start), end: Some(end) }
    }
}

/// One finding as `review.issues` returns it.
pub fn finding_json(f: &Finding, pos_json: impl Fn(&Pos) -> Value) -> Value {
    let mut v = json!({
        "kind": "lint",
        "checkId": f.check_id,
        "severity": f.severity,
        "message": f.message,
    });
    if let (Some(a), Some(b)) = (&f.start, &f.end)
        && let Some(o) = v.as_object_mut()
    {
        o.insert("start".into(), pos_json(a));
        o.insert("end".into(), pos_json(b));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::{Document, Pos, StoryRef};

    #[test]
    fn findings_round_trip_through_review() {
        let mut s = crate::Session::new(Document::new());
        let start = Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 0 };
        let end = Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 1 };
        let warn = Finding::warning("warnRule", "careful");
        assert_eq!(warn.severity, "warning");
        assert!(warn.start.is_none() && warn.end.is_none());
        s.set_lint_findings("test", vec![Finding::at("testRule", "warning", "example", start, end), warn]);
        assert_eq!(s.lint_findings().len(), 2);
        let issues = s.run("review.issues", &serde_json::json!({})).expect("issues");
        assert!(issues.as_array().is_some_and(|a| a.iter().any(|i| i["kind"] == "lint")));
    }
}
