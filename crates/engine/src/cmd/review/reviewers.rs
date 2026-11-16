//! Review › Tracking › Show Markup › Specific People: whose markup shows on screen.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use wordcraft_doc::{Document, StoryRef};

use crate::{CmdError, CmdResult, Session};

/// The most reviewers listed and hidden: a hostile file can name any number of authors.
pub const MAX_REVIEWERS: usize = 1000;

/// The longest reviewer name `review.showReviewer` takes, in characters.
const MAX_NAME_CHARS: usize = 1024;

/// The document's reviewers: the authors of its tracked changes (in every story) and of its
/// comments, each once, sorted ignoring case; at most [`MAX_REVIEWERS`].
pub fn reviewers(doc: &Document) -> Vec<String> {
    let mut revs = BTreeSet::new();
    let stories = std::iter::once(StoryRef::Body).chain(doc.parts.keys().map(|k| StoryRef::Part(*k)));
    for st in stories {
        for path in doc.para_paths(st) {
            let Some(p) = doc.para(st, &path) else { continue };
            revs.extend(
                [p.mark.ins, p.mark.del, p.mark.fmt_change.as_ref().map(|c| c.rev), p.props.fmt_change.as_ref().map(|c| c.rev)].into_iter().flatten(),
            );
            for r in &p.runs {
                revs.extend([r.props.ins, r.props.del, r.props.fmt_change.as_ref().map(|c| c.rev)].into_iter().flatten());
            }
            // A table's, row's and cell's own changes, once per cell (at its first paragraph), for
            // every table around the paragraph: a cell that starts with a nested table has its first
            // paragraph in that table. Each step up is three indexes shorter, so the walk ends.
            let mut at = path.clone();
            while let Some((tp, row, cell)) = at.cell() {
                if at.last() == 0
                    && let Some(t) = doc.table(st, &tp)
                {
                    let row = t.rows.get(row);
                    let changes = [
                        t.props.fmt_change.as_ref().map(|c| c.rev),
                        row.and_then(|x| x.props.fmt_change.as_ref().map(|c| c.rev)),
                        row.and_then(|x| x.cells.get(cell)).and_then(|x| x.props.fmt_change.as_ref().map(|c| c.rev)),
                    ];
                    revs.extend(changes.into_iter().flatten());
                }
                at = tp;
            }
        }
    }
    let mut names = BTreeSet::new();
    let authors = revs.into_iter().filter_map(|r| doc.revisions.get(r as usize).map(|v| v.author.as_str()));
    for a in authors.chain(doc.comments.values().map(|c| c.author.as_str())) {
        if names.len() >= MAX_REVIEWERS {
            break;
        }
        names.insert(a);
    }
    let mut out: Vec<String> = names.into_iter().map(str::to_string).collect();
    out.sort_by_cached_key(|n| (n.to_lowercase(), n.clone()));
    out
}

/// `review.showReviewer`: show or hide one reviewer's markup (`author`), or everyone's (`all`).
/// A view change only; the document and its revisions stay as they are.
pub(super) fn show_reviewer(s: &mut Session, v: &Value) -> CmdResult {
    let flag = |k: &str| match v.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(CmdError::Params(format!("`{k}` must be true or false"))),
    };
    let (all, value) = (flag("all")?, flag("value")?);
    let author = match v.get("author") {
        None | Some(Value::Null) => None,
        Some(Value::String(a)) if a.chars().count() <= MAX_NAME_CHARS => Some(a.as_str()),
        Some(Value::String(_)) => return Err(CmdError::Params(format!("`author` is longer than {MAX_NAME_CHARS} characters"))),
        Some(_) => return Err(CmdError::Params("`author` must be a string".into())),
    };
    let names = reviewers(&s.doc);
    match (author, all) {
        (Some(_), Some(_)) => return Err(CmdError::Params("give `author` or `all`, not both".into())),
        (Some(name), None) => {
            if !names.iter().any(|n| n == name) {
                return Err(CmdError::Params(format!("no reviewer named {name:?} in this document")));
            }
            let hidden = &mut s.view.hidden_reviewers;
            let shown = !hidden.iter().any(|h| h == name);
            if value.unwrap_or(!shown) {
                hidden.retain(|h| h != name);
            } else if shown {
                hidden.push(name.to_string());
            }
        }
        // All Reviewers: checked shows everyone; unchecked hides everyone listed.
        (None, Some(true)) => s.view.hidden_reviewers.clear(),
        (None, Some(false)) => s.view.hidden_reviewers = names.clone(),
        // Neither: the list, unchanged.
        (None, None) => {}
    }
    Ok(reviewers_json(s, &names))
}

fn reviewers_json(s: &Session, names: &[String]) -> Value {
    let list: Vec<Value> = names.iter().map(|n| json!({"name": n, "shown": s.view.shows_reviewer(n)})).collect();
    json!({"reviewers": list, "allShown": names.iter().all(|n| s.view.shows_reviewer(n))})
}
