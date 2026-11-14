use super::{CmdError, CmdResult, Pos, Selection, Session, StoryRef, Value, json, p, pos_json};

pub(super) fn word_count(s: &mut Session, v: &Value) -> CmdResult {
    if let Some(b) = p::bool(v, "includeTextBoxes") {
        s.prefs.count_notes = b;
    }
    let stories = s.counted_stories();
    // A selected picture, shape or text box isn't a text selection: count the document.
    let text = if s.sel.is_collapsed() || crate::cmd::objects::object_selection(s).is_some() { s.counted_text() } else { s.selected_text() };
    let words = if s.sel.is_collapsed() || crate::cmd::objects::object_selection(s).is_some() {
        stories.iter().map(|st| s.doc.story_word_count(*st)).sum()
    } else {
        wordcraft_doc::count_words(&text)
    };
    let chars = wordcraft_doc::count_chars(&text);
    let chars_no_spaces = wordcraft_doc::count_chars_no_spaces(&text);
    let paragraphs = text.split('\n').filter(|l| !l.trim().is_empty()).count();
    let lines: usize = {
        let l = s.layout();
        l.pages
            .iter()
            .flat_map(|p| p.items.iter())
            .map(|it| {
                if let wordcraft_layout::Placed::Lines { l0, l1, story, .. } = it
                    && stories.contains(story)
                {
                    l1 - l0
                } else {
                    0
                }
            })
            .sum()
    };
    let pages = s.layout().pages.len();
    Ok(json!({
        "pages": pages, "words": words, "characters": chars_no_spaces, "charactersWithSpaces": chars, "paragraphs": paragraphs, "lines": lines,
        "includeTextBoxes": s.prefs.count_notes,
    }))
}

/// Issues (spelling + grammar) in one paragraph as positions, each in its run's language and
/// skipping text left unchecked — the same issues the layout underlines.
fn para_issues(s: &Session, story: StoryRef, path: &wordcraft_doc::Path) -> Vec<(Pos, Pos, wordcraft_proof::Issue)> {
    let Some(p) = s.doc.para(story, path) else { return Vec::new() };
    let runs = wordcraft_layout::para::proof_runs(&s.doc, p, None);
    wordcraft_layout::para::paragraph_issues(p, &runs, &s.prefs.spelling(), true, wordcraft_layout::para::is_list_item(&s.doc, p))
        .into_iter()
        .map(|i| (Pos { story, path: path.clone(), off: i.start }, Pos { story, path: path.clone(), off: i.end }, i))
        .collect()
}

/// The issue under a position.
pub(super) fn issue_at(s: &Session, at: &Pos) -> Option<(Pos, Pos, wordcraft_proof::Issue)> {
    let issues = para_issues(s, at.story, &at.path);
    let selected = s.sel.ordered();
    // F7 can select a spelling issue inside a larger grammar issue. Keep that
    // exact target for either selection direction before falling back to a point.
    if at == &s.sel.focus
        && let Some(issue) = issues.iter().find(|(a, b, _)| a == &selected.0 && b == &selected.1)
    {
        return Some(issue.clone());
    }
    issues.into_iter().find(|(a, b, _)| a.off <= at.off && at.off <= b.off)
}

fn issue_json(s: &Session, a: &Pos, b: &Pos, i: &wordcraft_proof::Issue) -> Value {
    let word = s.doc.para_at(a).and_then(|p| p.text.get(a.off..b.off)).unwrap_or("").to_string();
    let sugg = if i.kind == wordcraft_proof::IssueKind::Spelling { wordcraft_proof::suggest_in(&word, 6, i.lang) } else { i.suggestions.clone() };
    json!({
        "start": pos_json(a), "end": pos_json(b), "text": word,
        "kind": if i.kind == wordcraft_proof::IssueKind::Spelling { "spelling" } else { "grammar" },
        "message": i.message, "suggestions": sugg, "lang": i.lang.tag(),
    })
}

/// F7: select the next issue after the caret and return it with suggestions.
pub(super) fn next_issue(s: &mut Session, v: &Value) -> CmdResult {
    let caret = s.sel.ordered().1;
    let story = caret.story;
    let paths = s.doc.para_paths(story);
    let start = paths.iter().position(|p| *p == caret.path).unwrap_or(0);
    let wrap = p::bool(v, "wrap").unwrap_or(true);
    // Scan forward first, including the rest of the caret's paragraph. A second pass
    // optionally visits the earlier part; never select the first typo repeatedly in
    // a single-paragraph story. Dialogs disable wrapping so Ignore Once can finish.
    for (path, before) in paths
        .iter()
        .skip(start)
        .map(|path| (path, false))
        .chain(paths.iter().take(if wrap { start.saturating_add(1) } else { 0 }).map(|path| (path, true)))
    {
        for (a, b, i) in para_issues(s, story, path) {
            if *path == caret.path && (a.off < caret.off) != before {
                continue;
            }
            let r = issue_json(s, &a, &b, &i);
            s.sel = Selection { anchor: a, focus: b };
            return Ok(r);
        }
    }
    s.status = "The spelling and grammar check is complete.".into();
    Ok(json!({"done": true}))
}

pub(super) fn all_issues(s: &mut Session, _: &Value) -> CmdResult {
    let mut out = Vec::new();
    for path in s.doc.para_paths(StoryRef::Body) {
        for (a, b, i) in para_issues(s, StoryRef::Body, &path) {
            out.push(issue_json(s, &a, &b, &i));
            if out.len() >= 500 {
                return Ok(Value::Array(out));
            }
        }
    }
    Ok(Value::Array(out))
}

/// The spelling issue under the caret and the word it marks. Grammar issues are phrases, so
/// "every occurrence" only means something for spelling — which is also where Word offers
/// Change All (`docs/spelling-review.md`).
fn spelling_at_caret(s: &Session) -> Option<(Pos, Pos, String)> {
    let (a, b, i) = issue_at(s, &s.sel.focus)?;
    if i.kind != wordcraft_proof::IssueKind::Spelling {
        return None;
    }
    let word = s.doc.para_at(&a)?.text.get(a.off..b.off)?.to_string();
    (!word.trim().is_empty()).then_some((a, b, word))
}

/// Change All needs a misspelling to change, as Word only offers it on a flagged word.
pub(super) fn needs_a_misspelling(s: &Session) -> Option<&'static str> {
    spelling_at_caret(s).is_none().then_some("the caret is not on a misspelled word")
}

/// Word's Change All (#6510): replace every flagged occurrence of the misspelled word under the
/// caret, through the same path as Change so a tracked replacement is still a reviewable revision.
///
/// The occurrences are the proofing issues the layout underlines, so only whole words match: a
/// longer typo holding the word (`brwnish`) keeps its own spelling and its own suggestions. The
/// match includes case, so a differently cased form (`Brwn` beside `brwn`) is its own misspelling
/// with its own correction — lowercasing a sentence's first word would be wrong. The scope is the
/// caret's story, as for Replace All (`edit.replaceAll`) and the pane's own pass.
pub(super) fn change_all(s: &mut Session, v: &Value) -> CmdResult {
    let text = p::req_str(v, "text")?.to_string();
    let (_, _, word) = spelling_at_caret(s).ok_or_else(|| CmdError::Failed("the caret is not on a misspelled word".into()))?;
    let story = s.sel.focus.story;
    // Every occurrence is collected before the first edit, and the edits run last one first, so
    // the positions ahead of each one stay valid even where the replacement splits a paragraph.
    // The list is bounded by the story's own words, not by anything a caller passes.
    let mut targets: Vec<(Pos, Pos)> = Vec::new();
    for path in s.doc.para_paths(story) {
        for (a, b, i) in para_issues(s, story, &path) {
            if i.kind == wordcraft_proof::IssueKind::Spelling && s.doc.para_at(&a).and_then(|p| p.text.get(a.off..b.off)) == Some(word.as_str()) {
                targets.push((a, b));
            }
        }
    }
    let changed = targets.len();
    for (a, b) in targets.into_iter().rev() {
        s.sel = Selection { anchor: a, focus: b };
        crate::cmd::type_text(s, &text)?;
    }
    let mut out = crate::cmd::sel_result(s)?;
    if let Some(o) = out.as_object_mut() {
        o.insert("word".into(), Value::String(word));
        o.insert("changed".into(), json!(changed));
    }
    Ok(out)
}

pub(super) fn suggestions(s: &mut Session, v: &Value) -> CmdResult {
    // Explicit positions are point lookups, even when they equal the selection focus.
    // Only an implicit or unparseable position keeps the exact selected issue.
    let issue = match v.get("pos").and_then(crate::cmd::parse_pos) {
        Some(at) => para_issues(s, at.story, &at.path).into_iter().find(|(a, b, _)| a.off <= at.off && at.off <= b.off),
        None => issue_at(s, &s.sel.focus),
    };
    match issue {
        Some((a, b, i)) => Ok(issue_json(s, &a, &b, &i)),
        None => Ok(Value::Null),
    }
}

/// A tiny built-in thesaurus of common words (our own list).
pub(super) fn thesaurus(s: &mut Session, v: &Value) -> CmdResult {
    const T: &[(&str, &[&str])] = &[
        ("good", &["fine", "excellent", "decent", "solid", "worthy"]),
        ("bad", &["poor", "inferior", "faulty", "unpleasant"]),
        ("big", &["large", "huge", "sizable", "vast", "major"]),
        ("small", &["little", "tiny", "compact", "minor", "modest"]),
        ("fast", &["quick", "rapid", "swift", "speedy"]),
        ("slow", &["unhurried", "gradual", "leisurely", "sluggish"]),
        ("happy", &["glad", "cheerful", "content", "joyful", "pleased"]),
        ("sad", &["unhappy", "sorrowful", "downcast", "gloomy"]),
        ("important", &["significant", "key", "vital", "essential", "major"]),
        ("show", &["display", "reveal", "present", "demonstrate"]),
        ("make", &["create", "build", "produce", "craft", "form"]),
        ("use", &["employ", "apply", "utilize", "operate"]),
        ("help", &["assist", "aid", "support", "serve"]),
        ("start", &["begin", "launch", "open", "initiate"]),
        ("end", &["finish", "close", "conclude", "stop"]),
        ("say", &["state", "tell", "remark", "mention"]),
        ("think", &["believe", "consider", "reckon", "suppose"]),
        ("new", &["fresh", "novel", "recent", "modern"]),
        ("old", &["aged", "former", "ancient", "vintage"]),
        ("beautiful", &["lovely", "attractive", "gorgeous", "stunning"]),
        ("easy", &["simple", "effortless", "straightforward"]),
        ("hard", &["difficult", "tough", "demanding", "firm"]),
        ("work", &["labor", "effort", "job", "task", "operate"]),
        ("idea", &["concept", "notion", "thought", "plan"]),
        ("change", &["alter", "modify", "adjust", "shift"]),
        ("great", &["excellent", "superb", "terrific", "considerable"]),
        ("very", &["extremely", "highly", "truly", "really"]),
        ("shared", &["common", "joint", "communal", "mutual"]),
        ("community", &["group", "society", "collective", "circle"]),
    ];
    let word = match p::str(v, "word") {
        Some(w) => w.to_lowercase(),
        None => {
            let f = s.sel.focus.clone();
            let p = s.doc.para_at(&f).ok_or_else(|| CmdError::Failed("no word".into()))?;
            let (a, b) = p.word_at(f.off);
            p.text.get(a..b).unwrap_or("").trim().to_lowercase()
        }
    };
    let syn = T.iter().find(|(w, _)| *w == word).map(|(_, l)| l.to_vec()).unwrap_or_default();
    Ok(json!({"word": word, "synonyms": syn}))
}
