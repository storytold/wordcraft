//! Legal spell-check supplement and confusion-pair proofing for LawCraft.

use serde_json::{Value, json};
use wordcraft_doc::{Document, Pos, StoryRef};
use wordcraft_engine::{Session, lint::Finding};
use wordcraft_proof::{Issue, IssueKind, ModuleEn, ProofLang};

use crate::checks::findings_json;

const LEGAL_WORDS_TXT: &str = include_str!("../data/legal-words.txt");

/// Every legal base form from [`LEGAL_WORDS_TXT`], sorted.
pub fn legal_words() -> &'static [&'static str] {
    static WORDS: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    WORDS
        .get_or_init(|| {
            let mut v: Vec<&'static str> = LEGAL_WORDS_TXT.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
            v.sort();
            v.dedup();
            v
        })
        .as_slice()
}

/// Per-session English proofing hooks for LawCraft (legal dictionary + confusion checks).
pub fn module_hooks() -> ModuleEn {
    ModuleEn::with_spelling(Some(legal_words())).with_grammar(Some(confusion_issues))
}

/// Confusion issues in one paragraph of English text (for proofing and [`confusion_findings`]).
pub fn confusion_issues(text: &str) -> Vec<Issue> {
    let tokens: Vec<(usize, usize, String)> =
        wordcraft_proof::words(text).into_iter().filter_map(|(a, b)| text.get(a..b).map(|w| (a, b, w.to_lowercase()))).collect();
    let mut out = Vec::new();
    for (idx, (a, b, lower)) in tokens.iter().enumerate() {
        let surface = text.get(*a..*b).unwrap_or("");
        for rule in CONFUSION_RULES {
            if !lower.eq_ignore_ascii_case(rule.wrong) {
                continue;
            }
            if !phrase_follows(&tokens, idx, text, rule.following, rule.following_any)
                || !phrase_precedes(&tokens, idx, text, rule.preceding)
                || !any_precedes(&tokens, idx, text, rule.preceding_any)
                || !medical_subject_before(&tokens, idx, text, rule.requires_medical_subject)
            {
                continue;
            }
            out.push(Issue {
                start: *a,
                end: *b,
                kind: IssueKind::Grammar,
                message: rule.message.into(),
                suggestions: vec![capitalize_like(surface, rule.right)],
                lang: ProofLang::En,
            });
        }
    }
    out.sort_by_key(|i| i.start);
    out.dedup_by_key(|i| i.start);
    out
}

/// Scan the open document's body for confusion-pair issues.
pub fn confusion_findings(doc: &Document) -> Vec<Finding> {
    let mut out = Vec::new();
    for path in doc.para_paths(StoryRef::Body) {
        let Some(para) = doc.para(StoryRef::Body, &path) else { continue };
        let text = para.proof_text();
        if text.trim().is_empty() || text.len() > 100_000 {
            continue;
        }
        for issue in confusion_issues(&text) {
            let start = Pos { story: StoryRef::Body, path: path.clone(), off: issue.start };
            let end = Pos { story: StoryRef::Body, path: path.clone(), off: issue.end };
            out.push(Finding::at("law.confusion", "suggestion", issue.message.clone(), start, end));
            if out.len() >= wordcraft_engine::lint::MAX_FINDINGS {
                return out;
            }
        }
    }
    out
}

/// Run confusion checks and optionally store them for Review › Proofing.
pub fn run_confusion_checks(session: &mut Session, store: bool) -> Value {
    let findings = confusion_findings(&session.doc);
    let items = findings_json(&findings);
    if store {
        session.set_lint_findings("law.proof", findings);
    }
    json!({"findings": items})
}

struct ConfusionRule {
    wrong: &'static str,
    right: &'static str,
    message: &'static str,
    /// When non-empty, the words immediately after `wrong` in the same phrase.
    following: &'static [&'static str],
    /// When non-empty, the one word that must follow the [`Self::following`] phrase, chosen from
    /// this set. Used for a category — a party role after `council for the` — where the phrase is
    /// the same but the word that completes it varies. The gap before it must be bare, like every
    /// other phrase word.
    following_any: &'static [&'static str],
    /// When non-empty, the exact words immediately before `wrong` in the same phrase, in order.
    preceding: &'static [&'static str],
    /// When non-empty, any one of these words may sit immediately before `wrong`. Use for a
    /// category (an auxiliary or `to` that only leads a verb) where the exact word varies but the
    /// context it signals is the same.
    preceding_any: &'static [&'static str],
    /// When true, a medical professional must be the immediate subject of `wrong` (see
    /// [`medical_subject_before`]); used so a law or regulation that *proscribes* medication is
    /// not mistaken for a physician who would *prescribe* it.
    requires_medical_subject: bool,
}

/// Auxiliaries that only precede a *base verb*. A word after one of these is doing verb work, so
/// the following word is almost certainly the confused verb (`effect` → `affect`, `illicit` →
/// `elicit`). The `illicit a response` rule needs one; the bare `illicit testimony` phrase the
/// issue names does not. The infinitive/preposition `to` is deliberately absent: telling an
/// infinitival `to` from a prepositional one needs part-of-speech context this checker doesn't
/// have, so it is safer to drop `to` than to risk flagging correct prose.
const VERB_LEADS: &[&str] = &["will", "would", "can", "could", "may", "might", "must", "shall", "should", "did", "does", "do"];

/// People who order medical treatment.
const MEDICAL_ACTORS: &[&str] = &["doctor", "physician", "surgeon", "nurse", "clinician", "pharmacist", "dentist", "psychiatrist", "veterinarian"];

/// Context-sensitive legal word-pair checks (original wording; no manual copied).
///
/// Every entry fires on the phrase in [`Self::following`] / [`Self::preceding`] and nowhere else,
/// so a clean sentence that merely uses the right word stays quiet. A flag is a suggestion for the
/// writer to accept or dismiss; it never changes text. The phrases come from common legal typos
/// (statute/statue, principle/principal, counsel/council, affect/effect, precedent/precedence,
/// prescribe/proscribe, adverse/averse, elicit/illicit, complement/compliment, discrete/discreet).
const CONFUSION_RULES: &[ConfusionRule] = &[
    ConfusionRule {
        wrong: "statue",
        right: "statute",
        message: "\"Statute\" is the law; \"statue\" is a sculpture — did you mean statute of limitations?",
        following: &["of", "limitations"],
        following_any: &[],
        preceding: &[],
        preceding_any: &[],
        requires_medical_subject: false,
    },
    ConfusionRule {
        wrong: "statue",
        right: "statute",
        message: "\"Statute\" is the law; \"statue\" is a sculpture — did you mean statute of frauds?",
        following: &["of", "frauds"],
        following_any: &[],
        preceding: &[],
        preceding_any: &[],
        requires_medical_subject: false,
    },
    ConfusionRule {
        wrong: "principle",
        right: "principal",
        message: "In legal writing, the main issue is usually the \"principal\" issue, not \"principle\".",
        following: &["issue"],
        following_any: &[],
        preceding: &[],
        preceding_any: &[],
        requires_medical_subject: false,
    },
    ConfusionRule {
        wrong: "principle",
        right: "principal",
        message: "The main office is the \"principal\" place of business, not \"principle\".",
        following: &["place", "of", "business"],
        following_any: &[],
        preceding: &[],
        preceding_any: &[],
        requires_medical_subject: false,
    },
    ConfusionRule {
        wrong: "principle",
        right: "principal",
        message: "The main sum is the \"principal\" amount, not \"principle\".",
        following: &["amount"],
        following_any: &[],
        preceding: &[],
        preceding_any: &[],
        requires_medical_subject: false,
    },
    ConfusionRule {
        wrong: "principal",
        right: "principle",
        message: "A rule of law is a \"principle\", not \"principal\".",
        following: &["of", "law"],
        following_any: &[],
        preceding: &[],
        preceding_any: &[],
        requires_medical_subject: false,
    },
    ConfusionRule {
        wrong: "council",
        right: "counsel",
        message: "Court-appointed representation is \"counsel\", not \"council\" (a governing body).",
        following: &["of", "record"],
        following_any: &[],
        preceding: &[],
        preceding_any: &[],
        requires_medical_subject: false,
    },
    ConfusionRule {
        wrong: "council",
        right: "counsel",
        message: "Opposing representation is \"counsel\", not \"council\" (a governing body).",
        following: &[],
        following_any: &[],
        preceding: &["opposing"],
        preceding_any: &[],
        requires_medical_subject: false,
    },
    ConfusionRule {
        wrong: "council",
        right: "counsel",
        message: "A party's lawyer is \"counsel\", not \"council\" (a governing body).",
        following: &["for", "the"],
        following_any: &["plaintiff", "defendant", "appellant", "respondent", "petitioner", "movant"],
        preceding: &[],
        preceding_any: &[],
        requires_medical_subject: false,
    },
    ConfusionRule {
        wrong: "affect",
        right: "effect",
        message: "A result or outcome is an \"effect\"; \"affect\" is usually the verb.",
        following: &["of", "the"],
        following_any: &[],
        preceding: &[],
        preceding_any: &[],
        requires_medical_subject: false,
    },
    ConfusionRule {
        wrong: "effect",
        right: "affect",
        message: "To influence an outcome is to \"affect\" it, not \"effect\" (except \"effect change\").",
        following: &["the", "outcome"],
        following_any: &[],
        preceding: &[],
        preceding_any: VERB_LEADS,
        requires_medical_subject: false,
    },
    ConfusionRule {
        wrong: "precedence",
        right: "precedent",
        message: "A prior appellate decision is \"precedent\", not \"precedence\" (priority).",
        following: &[],
        following_any: &[],
        preceding: &["binding"],
        preceding_any: &[],
        requires_medical_subject: false,
    },
    ConfusionRule {
        wrong: "precedence",
        right: "precedent",
        message: "Controlling case law is \"precedent\", not \"precedence\".",
        following: &[],
        following_any: &[],
        preceding: &["controlling"],
        preceding_any: &[],
        requires_medical_subject: false,
    },
    ConfusionRule {
        wrong: "proscribe",
        right: "prescribe",
        message: "A doctor \"prescribes\" treatment; \"proscribe\" means to forbid.",
        following: &["medication"],
        following_any: &[],
        preceding: &[],
        preceding_any: &[],
        requires_medical_subject: true,
    },
    ConfusionRule {
        wrong: "adverse",
        right: "averse",
        message: "Being unwilling is \"averse\"; \"adverse\" describes harm or opposition.",
        following: &["to", "risk"],
        following_any: &[],
        preceding: &[],
        preceding_any: &[],
        requires_medical_subject: false,
    },
    ConfusionRule {
        wrong: "averse",
        right: "adverse",
        message: "A hostile claim to land is \"adverse possession\", not \"averse\" (unwilling).",
        following: &["possession"],
        following_any: &[],
        preceding: &[],
        preceding_any: &[],
        requires_medical_subject: false,
    },
    ConfusionRule {
        wrong: "illicit",
        right: "elicit",
        message: "To draw out testimony is to \"elicit\" it, not \"illicit\" (unlawful).",
        following: &["testimony"],
        following_any: &[],
        preceding: &[],
        preceding_any: &[],
        requires_medical_subject: false,
    },
    ConfusionRule {
        wrong: "illicit",
        right: "elicit",
        message: "To draw out a response is to \"elicit\" it, not \"illicit\".",
        following: &["a", "response"],
        following_any: &[],
        preceding: &[],
        preceding_any: VERB_LEADS,
        requires_medical_subject: false,
    },
    ConfusionRule {
        wrong: "compliment",
        right: "complement",
        message: "To complete or supplement the record is to \"complement\" it, not \"compliment\".",
        following: &["the", "record"],
        following_any: &[],
        preceding: &[],
        preceding_any: &[],
        requires_medical_subject: false,
    },
    ConfusionRule {
        wrong: "discreet",
        right: "discrete",
        message: "Separate or distinct issues are \"discrete\", not \"discreet\" (tactful).",
        following: &["issues"],
        following_any: &[],
        preceding: &[],
        preceding_any: &[],
        requires_medical_subject: false,
    },
];

fn is_sentence_end(c: char) -> bool {
    matches!(c, '.' | '!' | '?')
}

/// Whether the raw text between two adjacent tokens holds a sentence end. Used by
/// [`medical_subject_before`], whose backward scan may cross the words between subject and verb.
fn gap_has_sentence_end(text: &str, from: usize, to: usize) -> bool {
    text.get(from..to).is_some_and(|gap| gap.chars().any(is_sentence_end))
}

/// Whether the gap between two phrase words is bare: only whitespace, no punctuation. A comma,
/// dash, parenthesis or the like means the words are not one phrase — "the governing principle,
/// issue preclusion, bars relitigation" must not read as "principle issue", and "the city council,
/// for the plaintiff, met" must not read as "council for the plaintiff". A sentence end is a strict
/// subset of non-whitespace, so requiring a bare gap also keeps the old no-crossing-sentences rule.
/// Like [`gap_has_sentence_end`], it looks at only the gap, so the work stays proportional to the
/// phrase length instead of collecting a whole sentence per candidate.
fn gap_is_bare(text: &str, from: usize, to: usize) -> bool {
    text.get(from..to).is_some_and(|gap| gap.chars().all(char::is_whitespace))
}

/// Whether `phrase` immediately follows the token at `idx`, then (when `any` is non-empty) the one
/// word after it is a member of `any`. Every step must sit across a bare gap.
fn phrase_follows(tokens: &[(usize, usize, String)], idx: usize, text: &str, phrase: &[&str], any: &[&str]) -> bool {
    let Some(first) = tokens.get(idx) else { return false };
    let mut prev_end = first.1;
    let mut i = idx.saturating_add(1);
    for want in phrase {
        let Some(t) = tokens.get(i) else { return false };
        if t.2 != *want || !gap_is_bare(text, prev_end, t.0) {
            return false;
        }
        prev_end = t.1;
        i = i.saturating_add(1);
    }
    if !any.is_empty() {
        let Some(t) = tokens.get(i) else { return false };
        if !any.contains(&t.2.as_str()) || !gap_is_bare(text, prev_end, t.0) {
            return false;
        }
    }
    true
}

fn phrase_precedes(tokens: &[(usize, usize, String)], idx: usize, text: &str, phrase: &[&str]) -> bool {
    if phrase.is_empty() {
        return true;
    }
    let Some(first) = tokens.get(idx) else { return false };
    let mut next_start = first.0;
    for (n, want) in phrase.iter().rev().enumerate() {
        let Some(i) = idx.checked_sub(n.saturating_add(1)) else { return false };
        let Some(t) = tokens.get(i) else { return false };
        if t.2 != *want || !gap_is_bare(text, t.1, next_start) {
            return false;
        }
        next_start = t.0;
    }
    true
}

/// Whether any word in `set` sits immediately before the token at `idx`, across a bare gap. A set
/// (unlike [`phrase_precedes`]'s ordered phrase) matches any one member, so rules can name a
/// category such as the auxiliaries that only lead a verb.
fn any_precedes(tokens: &[(usize, usize, String)], idx: usize, text: &str, set: &[&str]) -> bool {
    if set.is_empty() {
        return true;
    }
    let Some(i) = idx.checked_sub(1) else { return false };
    let (Some(prev), Some(cur)) = (tokens.get(i), tokens.get(idx)) else { return false };
    set.contains(&prev.2.as_str()) && gap_is_bare(text, prev.1, cur.0)
}

/// Whether a medical professional is the immediate subject of the verb at `idx`: skip auxiliaries
/// and determiners backward, then require a [`MEDICAL_ACTORS`] token with no intervening sentence
/// end. Stops at the first non-skipped token so "the physician testified that the regulation will
/// proscribe" does not treat `physician` as the subject of `proscribe`.
fn medical_subject_before(tokens: &[(usize, usize, String)], idx: usize, text: &str, required: bool) -> bool {
    if !required {
        return true;
    }
    let Some(cur) = tokens.get(idx) else { return false };
    let Some(mut i) = idx.checked_sub(1) else { return false };
    let mut bound = cur.0;
    loop {
        let Some(t) = tokens.get(i) else { return false };
        if gap_has_sentence_end(text, t.1, bound) {
            return false;
        }
        let w = t.2.as_str();
        if VERB_LEADS.contains(&w) || matches!(w, "the" | "a" | "an") {
            if i == 0 {
                return false;
            }
            bound = t.0;
            i -= 1;
            continue;
        }
        return MEDICAL_ACTORS.contains(&w);
    }
}

fn capitalize_like(original: &str, replacement: &str) -> String {
    if original.chars().next().is_some_and(|c| c.is_uppercase()) {
        let mut s = replacement.to_string();
        if let Some(c) = s.get_mut(0..1) {
            c.make_ascii_uppercase();
        }
        s
    } else {
        replacement.to_string()
    }
}
