//! Review tab: comments, track changes, word count, spelling (via the proofing hook).

use serde_json::{Value, json};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::{Comment, Paragraph, PartKind, Pos, StoryRef, para_block};

use super::{now_iso, pos_json, sel_result};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("review.newComment", "New Comment", "Review › Comments", new_comment).key("Mod+Alt+M").params(r#"{"text": string}"#),
        CommandSpec::new("review.reply", "Reply", "Review › Comments", reply).params(r#"{"id": n, "text": string}"#),
        CommandSpec::new("review.deleteComment", "Delete", "Review › Comments", delete_comment).params(r#"{"id"?: n, "all"?: bool}"#),
        CommandSpec::new("review.resolveComment", "Resolve", "Review › Comments", |s, v| {
            let id = p::u64(v, "id").map(|x| x as u32).or_else(|| comment_at_caret(s)).ok_or_else(|| CmdError::Params("`id` required".into()))?;
            let c = s.doc.comments.get_mut(&id).ok_or_else(|| CmdError::Params("no such comment".into()))?;
            c.resolved = p::bool(v, "value").unwrap_or(!c.resolved);
            sel_result(s)
        }),
        CommandSpec::new("review.nextComment", "Next", "Review › Comments", |s, _| nav_comment(s, 1)).pure(),
        CommandSpec::new("review.previousComment", "Previous", "Review › Comments", |s, _| nav_comment(s, -1)).pure(),
        CommandSpec::new("review.comments", "Show Comments", "Review › Comments", list_comments).pure(),
        CommandSpec::new("review.trackChanges", "Track Changes", "Review › Tracking", |s, v| {
            s.doc.settings.track_changes = p::bool(v, "value").unwrap_or(!s.doc.settings.track_changes);
            Ok(json!({"value": s.doc.settings.track_changes}))
        })
        .key("Mod+Shift+E"),
        CommandSpec::new("review.acceptAll", "Accept All Changes", "Review › Changes", |s, _| resolve_all(s, true)),
        CommandSpec::new("review.rejectAll", "Reject All Changes", "Review › Changes", |s, _| resolve_all(s, false)),
        CommandSpec::new("review.accept", "Accept", "Review › Changes", |s, _| resolve_sel(s, true)),
        CommandSpec::new("review.reject", "Reject", "Review › Changes", |s, _| resolve_sel(s, false)),
        CommandSpec::new("review.nextChange", "Next Change", "Review › Changes", |s, _| nav_change(s, 1)).pure(),
        CommandSpec::new("review.previousChange", "Previous Change", "Review › Changes", |s, _| nav_change(s, -1)).pure(),
        CommandSpec::new("review.markup", "Display for Review", "Review › Tracking", |s, v| {
            s.view.show_markup = p::str(v, "value").map(|m| m != "noMarkup" && m != "original").unwrap_or(!s.view.show_markup);
            Ok(json!({"showMarkup": s.view.show_markup}))
        })
        .pure(),
        CommandSpec::new("review.wordCount", "Word Count", "Review › Proofing", word_count).params(r#"{"includeTextBoxes"?: bool}"#).pure(),
        CommandSpec::new("review.changes", "Reviewing Pane", "Review › Tracking", list_changes).pure(),
        CommandSpec::new("review.spelling", "Spelling & Grammar", "Review › Proofing", next_issue).key("F7").pure(),
        CommandSpec::new("review.issues", "Proofing Issues", "Review › Proofing", all_issues).pure(),
        CommandSpec::new("review.suggestions", "Spelling Suggestions", "Review › Proofing", suggestions).params(r#"{"pos"?: Pos}"#).pure(),
        CommandSpec::new("review.addToDictionary", "Add to Dictionary", "Review › Proofing", |s, v| {
            let w = match p::str(v, "word") {
                Some(w) => w.to_string(),
                None => issue_at(s, &s.sel.focus.clone())
                    .map(|(a, b, _)| s.doc.para_at(&a).and_then(|p| p.text.get(a.off..b.off).map(str::to_string)).unwrap_or_default())
                    .unwrap_or_default(),
            };
            if w.is_empty() {
                return Err(CmdError::Params("no word".into()));
            }
            wordcraft_proof::add_word(&w);
            s.relayout();
            Ok(json!({"added": w}))
        })
        .params(r#"{"word"?: string}"#)
        .pure(),
        CommandSpec::new("review.ignoreAll", "Ignore All", "Review › Proofing", |s, v| {
            let w = match p::str(v, "word") {
                Some(w) => w.to_string(),
                None => issue_at(s, &s.sel.focus.clone())
                    .map(|(a, b, _)| s.doc.para_at(&a).and_then(|p| p.text.get(a.off..b.off).map(str::to_string)).unwrap_or_default())
                    .unwrap_or_default(),
            };
            wordcraft_proof::add_word(&w);
            s.relayout();
            Ok(json!({"ignored": w}))
        })
        .pure(),
        CommandSpec::new("review.applySuggestion", "Change", "Review › Proofing", |s, v| {
            let text = p::req_str(v, "text")?.to_string();
            let at = s.sel.focus.clone();
            if let Some((a, b, _)) = issue_at(s, &at) {
                s.sel = Selection { anchor: a, focus: b };
            }
            super::type_text(s, &text)?;
            sel_result(s)
        })
        .params(r#"{"text": string}"#),
        CommandSpec::new("review.proofing", "Check Spelling as You Type", "File › Options › Proofing", |s, v| {
            s.view.proofing = p::bool(v, "value").unwrap_or(!s.view.proofing);
            Ok(json!({"value": s.view.proofing}))
        })
        .pure(),
        CommandSpec::new("review.thesaurus", "Thesaurus", "Review › Proofing", thesaurus).key("Shift+F7").pure(),
    ]
}

fn new_comment(s: &mut Session, v: &Value) -> CmdResult {
    let text = p::str(v, "text").unwrap_or("").to_string();
    let (a, b) = s.sel.ordered();
    let (a, b) = if a == b {
        // Comment on the word at the caret.
        match s.doc.para_at(&a) {
            Some(para) if !para.is_empty() => {
                let (x, y) = para.word_at(a.off);
                let y = para.text.get(x..y).map(|w| x + w.trim_end().len()).unwrap_or(y);
                (Pos { off: x, ..a.clone() }, Pos { off: y, ..a })
            }
            _ => (a.clone(), a),
        }
    } else {
        (a, b)
    };
    let part = s.doc.add_part(PartKind::Comment, vec![para_block(Paragraph::with_text(&text, Default::default()))]);
    let id = s.doc.comments.keys().next_back().map(|k| k + 1).unwrap_or(0);
    let initials: String = s.author.split_whitespace().filter_map(|w| w.chars().next()).collect();
    s.doc.comments.insert(id, Comment { author: s.author.clone(), initials, date: now_iso(), parent: None, resolved: false, part });
    let props = Default::default();
    s.doc.insert_object(&b, InlineObject::CommentEnd { id }, &props)?;
    s.doc.insert_object(&a, InlineObject::CommentStart { id }, &props)?;
    s.view.comments_pane = true;
    Ok(json!({"id": id, "story": part}))
}

fn reply(s: &mut Session, v: &Value) -> CmdResult {
    let parent = p::u64(v, "id").map(|x| x as u32).ok_or_else(|| CmdError::Params("`id` required".into()))?;
    if !s.doc.comments.contains_key(&parent) {
        return Err(CmdError::Params("no such comment".into()));
    }
    let text = p::req_str(v, "text")?;
    let part = s.doc.add_part(PartKind::Comment, vec![para_block(Paragraph::with_text(text, Default::default()))]);
    let id = s.doc.comments.keys().next_back().map(|k| k + 1).unwrap_or(0);
    let initials: String = s.author.split_whitespace().filter_map(|w| w.chars().next()).collect();
    s.doc.comments.insert(id, Comment { author: s.author.clone(), initials, date: now_iso(), parent: Some(parent), resolved: false, part });
    Ok(json!({"id": id}))
}

/// Comment anchored around the caret.
fn comment_at_caret(s: &Session) -> Option<u32> {
    let f = &s.sel.focus;
    let p = s.doc.para_at(f)?;
    let mut open: Vec<u32> = Vec::new();
    for off in p.object_offsets() {
        if off > f.off {
            break;
        }
        match p.object_at(off) {
            Some(InlineObject::CommentStart { id }) => open.push(*id),
            Some(InlineObject::CommentEnd { id }) => open.retain(|x| x != id),
            _ => {}
        }
    }
    open.last().copied().or_else(|| {
        p.object_offsets().into_iter().find_map(|o| match p.object_at(o) {
            Some(InlineObject::CommentStart { id }) | Some(InlineObject::CommentEnd { id }) => Some(*id),
            _ => None,
        })
    })
}

fn remove_anchors(s: &mut Session, ids: &[u32]) -> Result<(), CmdError> {
    for story in [StoryRef::Body] {
        for path in s.doc.para_paths(story) {
            let offs: Vec<usize> = s
                .doc
                .para(story, &path)
                .map(|p| {
                    p.object_offsets()
                        .into_iter()
                        .filter(|o| matches!(p.object_at(*o), Some(InlineObject::CommentStart { id }) | Some(InlineObject::CommentEnd { id }) if ids.contains(id)))
                        .collect()
                })
                .unwrap_or_default();
            if offs.is_empty() {
                continue;
            }
            let para = s.doc.para_mut(story, &path)?;
            for o in offs.into_iter().rev() {
                para.delete(o, o + wordcraft_doc::para::OBJ.len_utf8())?;
            }
        }
    }
    Ok(())
}

fn delete_comment(s: &mut Session, v: &Value) -> CmdResult {
    let ids: Vec<u32> = if p::bool(v, "all").unwrap_or(false) {
        s.doc.comments.keys().copied().collect()
    } else {
        let id = p::u64(v, "id").map(|x| x as u32).or_else(|| comment_at_caret(s)).ok_or_else(|| CmdError::Params("no comment here".into()))?;
        // The comment and its replies.
        let mut v = vec![id];
        v.extend(s.doc.comments.iter().filter(|(_, c)| c.parent == Some(id)).map(|(k, _)| *k));
        v
    };
    remove_anchors(s, &ids)?;
    for id in &ids {
        if let Some(c) = s.doc.comments.remove(id) {
            s.doc.parts.remove(&c.part);
        }
    }
    s.clamp_selection();
    Ok(json!({"deleted": ids}))
}

/// Comments in document order with their anchors.
pub fn comment_list(s: &Session) -> Vec<(u32, Option<Pos>)> {
    let mut found: Vec<(u32, Option<Pos>)> = Vec::new();
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        for o in p.object_offsets() {
            if let Some(InlineObject::CommentStart { id }) = p.object_at(o) {
                found.push((*id, Some(Pos { story: StoryRef::Body, path: path.clone(), off: o })));
            }
        }
    }
    for id in s.doc.comments.keys() {
        if !found.iter().any(|(x, _)| x == id) {
            found.push((*id, None));
        }
    }
    found
}

fn list_comments(s: &mut Session, _: &Value) -> CmdResult {
    let list = comment_list(s);
    Ok(Value::Array(
        list.iter()
            .filter_map(|(id, pos)| {
                let c = s.doc.comments.get(id)?;
                Some(json!({
                    "id": id, "author": c.author, "date": c.date, "resolved": c.resolved, "parent": c.parent,
                    "text": s.doc.plain_text(StoryRef::Part(c.part)),
                    "anchor": pos.as_ref().map(pos_json),
                }))
            })
            .collect(),
    ))
}

fn nav_comment(s: &mut Session, dir: i32) -> CmdResult {
    let list: Vec<Pos> = comment_list(s).into_iter().filter_map(|(_, p)| p).collect();
    let caret = s.sel.focus.clone();
    let t = if dir > 0 { list.iter().find(|p| **p > caret).or(list.first()) } else { list.iter().rev().find(|p| **p < caret).or(list.last()) };
    if let Some(p) = t {
        s.sel = Selection::caret(p.clone());
    }
    sel_result(s)
}

/// Accept or reject a revision range in one paragraph.
fn resolve_para(s: &mut Session, story: StoryRef, path: &wordcraft_doc::Path, from: usize, to: usize, accept: bool) -> Result<(), CmdError> {
    let para = s.doc.para_mut(story, path)?;
    let ranges: Vec<(usize, usize, bool, bool)> = para
        .run_ranges()
        .filter(|(r, _)| r.end > from && r.start < to)
        .map(|(r, c)| (r.start.max(from), r.end.min(to), c.ins.is_some(), c.del.is_some()))
        .collect();
    for (a, b, ins, del) in ranges.into_iter().rev() {
        if (del && accept) || (ins && !accept) {
            para.delete(a, b)?;
        } else if ins || del {
            para.format(a, b, &|c| {
                c.ins = None;
                c.del = None;
            })?;
        }
    }
    if para.mark.ins.is_some() {
        para.mark.ins = None;
    }
    Ok(())
}

fn resolve_all(s: &mut Session, accept: bool) -> CmdResult {
    let stories: Vec<StoryRef> = std::iter::once(StoryRef::Body).chain(s.doc.parts.keys().map(|k| StoryRef::Part(*k))).collect();
    for st in stories {
        for path in s.doc.para_paths(st).into_iter().rev() {
            let len = s.doc.para(st, &path).map(|p| p.len()).unwrap_or(0);
            resolve_para(s, st, &path, 0, len, accept)?;
        }
    }
    s.doc.revisions.clear();
    s.clamp_selection();
    sel_result(s)
}

fn resolve_sel(s: &mut Session, accept: bool) -> CmdResult {
    let (a, b) = s.sel.ordered();
    let (a, b) = if a == b {
        // The change at the caret: the run around it.
        let found = s
            .doc
            .para_at(&a)
            .and_then(|p| p.run_ranges().find(|(r, c)| r.start <= a.off && a.off <= r.end && (c.ins.is_some() || c.del.is_some())).map(|(r, _)| r));
        match found {
            Some(r) => (Pos { off: r.start, ..a.clone() }, Pos { off: r.end, ..a }),
            None => return nav_change(s, 1),
        }
    } else {
        (a, b)
    };
    for path in s.doc.paths_between(&a, &b).into_iter().rev() {
        let len = s.doc.para(a.story, &path).map(|p| p.len()).unwrap_or(0);
        let from = if path == a.path { a.off } else { 0 };
        let to = if path == b.path { b.off } else { len };
        resolve_para(s, a.story, &path, from, to, accept)?;
    }
    s.sel = Selection::caret(a);
    s.clamp_selection();
    sel_result(s)
}

fn changes(s: &Session) -> Vec<(Pos, Pos, &'static str, Option<u32>)> {
    let mut out = Vec::new();
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        for (r, c) in p.run_ranges() {
            let kind = if c.ins.is_some() {
                "insert"
            } else if c.del.is_some() {
                "delete"
            } else {
                continue;
            };
            let mk = |off| Pos { story: StoryRef::Body, path: path.clone(), off };
            out.push((mk(r.start), mk(r.end), kind, c.ins.or(c.del)));
        }
    }
    out
}

fn nav_change(s: &mut Session, dir: i32) -> CmdResult {
    let list = changes(s);
    let caret = s.sel.ordered();
    let t = if dir > 0 { list.iter().find(|c| c.0 >= caret.1).or(list.first()) } else { list.iter().rev().find(|c| c.1 <= caret.0).or(list.last()) };
    if let Some((a, b, _, _)) = t {
        s.sel = Selection { anchor: a.clone(), focus: b.clone() };
    }
    sel_result(s)
}

fn list_changes(s: &mut Session, _: &Value) -> CmdResult {
    let list = changes(s);
    Ok(Value::Array(
        list.iter()
            .map(|(a, b, kind, rid)| {
                let rev = rid.and_then(|r| s.doc.revisions.get(r as usize));
                json!({
                    "kind": kind, "start": pos_json(a), "end": pos_json(b),
                    "text": s.doc.copy_range(a, b).plain_text(),
                    "author": rev.map(|r| r.author.clone()), "date": rev.map(|r| r.date.clone()),
                })
            })
            .collect(),
    ))
}

fn word_count(s: &mut Session, v: &Value) -> CmdResult {
    if let Some(b) = p::bool(v, "includeTextBoxes") {
        s.prefs.count_notes = b;
    }
    let stories = if s.prefs.count_notes { s.doc.counted_stories() } else { vec![StoryRef::Body] };
    // A selected picture, shape or text box isn't a text selection: count the document.
    let text = if s.sel.is_collapsed() || super::objects::object_selection(s).is_some() {
        stories.iter().map(|st| s.doc.plain_text(*st)).collect::<Vec<_>>().join("\n")
    } else {
        s.selected_text()
    };
    let words = wordcraft_doc::count_words(&text);
    let chars = text.chars().filter(|c| *c != '\n').count();
    let chars_no_spaces = text.chars().filter(|c| !c.is_whitespace()).count();
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

/// Issues (spelling + grammar) in one paragraph as positions.
fn para_issues(s: &Session, story: StoryRef, path: &wordcraft_doc::Path) -> Vec<(Pos, Pos, wordcraft_proof::Issue)> {
    let Some(p) = s.doc.para(story, path) else { return Vec::new() };
    let text = wordcraft_layout::para::proof_text(p);
    let mut v: Vec<wordcraft_proof::Issue> = wordcraft_proof::check_spelling(&text);
    v.extend(wordcraft_proof::check_grammar(&text));
    v.sort_by_key(|i| i.start);
    v.into_iter()
        .filter(|i| !p.run_ranges().any(|(r, c)| r.start < i.end && i.start < r.end && (c.no_proof == Some(true) || c.link.is_some())))
        .map(|i| (Pos { story, path: path.clone(), off: i.start }, Pos { story, path: path.clone(), off: i.end }, i))
        .collect()
}

/// The issue under a position.
fn issue_at(s: &Session, at: &Pos) -> Option<(Pos, Pos, wordcraft_proof::Issue)> {
    para_issues(s, at.story, &at.path).into_iter().find(|(a, b, _)| a.off <= at.off && at.off <= b.off)
}

fn issue_json(s: &Session, a: &Pos, b: &Pos, i: &wordcraft_proof::Issue) -> Value {
    let word = s.doc.para_at(a).and_then(|p| p.text.get(a.off..b.off)).unwrap_or("").to_string();
    let sugg = if i.kind == wordcraft_proof::IssueKind::Spelling { wordcraft_proof::suggest(&word, 6) } else { i.suggestions.clone() };
    json!({"start": pos_json(a), "end": pos_json(b), "text": word, "kind": if i.kind == wordcraft_proof::IssueKind::Spelling { "spelling" } else { "grammar" }, "message": i.message, "suggestions": sugg})
}

/// F7: select the next issue after the caret and return it with suggestions.
fn next_issue(s: &mut Session, _: &Value) -> CmdResult {
    let caret = s.sel.ordered().1;
    let story = caret.story;
    let paths = s.doc.para_paths(story);
    let start = paths.iter().position(|p| *p == caret.path).unwrap_or(0);
    for k in 0..paths.len() {
        let Some(path) = paths.get((start + k) % paths.len().max(1)) else { break };
        for (a, b, i) in para_issues(s, story, path) {
            if k == 0 && a.off < caret.off && start + k < paths.len() && paths.len() > 1 {
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

fn all_issues(s: &mut Session, _: &Value) -> CmdResult {
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

fn suggestions(s: &mut Session, v: &Value) -> CmdResult {
    let at = v.get("pos").and_then(super::parse_pos).unwrap_or_else(|| s.sel.focus.clone());
    match issue_at(s, &at) {
        Some((a, b, i)) => Ok(issue_json(s, &a, &b, &i)),
        None => Ok(Value::Null),
    }
}

/// A tiny built-in thesaurus of common words (our own list).
fn thesaurus(s: &mut Session, v: &Value) -> CmdResult {
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
