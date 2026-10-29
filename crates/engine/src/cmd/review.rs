//! Review tab: comments, track changes, word count, spelling (via the proofing hook).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::{Comment, Paragraph, PartKind, Pos, StoryRef, para_block};

use super::{fmt_revisions, now_iso, pos_json, sel_result};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

mod changes;
mod comments;
mod proofing;

pub use changes::{
    DELETION_TEXT_CHARS, DeletionBalloon, deletion_balloons, format_changes, has_deletions, has_format_changes, move_changes, tracking_json,
};
use changes::{list_changes, nav_change, resolve_all_then, resolve_sel_then, tracking_options};
use comments::{
    comment_at_caret, comments_change, delete_comment, edit_comment, list_comments, nav_comment, new_comment, reply, thread_of, validate_comment_id,
};
pub use comments::{comment_list, comment_range_at_caret};
use proofing::{all_issues, issue_at, next_issue, suggestions, thesaurus, word_count};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("review.newComment", "New Comment", "Review › Comments", new_comment).key(super::mac_or("Mod+Alt+A / Mod+Alt+M", "Mod+Alt+M")).params(r#"{"text": string}"#),
        CommandSpec::new("review.reply", "Reply", "Review › Comments", reply)
            .params(r#"{"id": non-negative JSON integer, "text": string}"#)
            .when(comments_change),
        CommandSpec::new("review.editComment", "Edit Comment", "Review › Comments", edit_comment)
            .params(r#"{"id": non-negative JSON integer, "text": string}"#)
            .when(comments_change),
        CommandSpec::new("review.deleteComment", "Delete", "Review › Comments", delete_comment).params(r#"{"id"?: non-negative JSON integer (omit to use the caret), "all"?: bool}"#),
        CommandSpec::new("review.resolveComment", "Resolve", "Review › Comments", |s, v| {
            validate_comment_id(v)?;
            let id = p::u64(v, "id").map(|x| x as u32).or_else(|| comment_at_caret(s)).ok_or_else(|| CmdError::Params("`id` required".into()))?;
            let c = s.doc.comments.get_mut(&id).ok_or_else(|| CmdError::Params("no such comment".into()))?;
            c.resolved = p::bool(v, "value").unwrap_or(!c.resolved);
            sel_result(s)
        })
        .params(r#"{"id"?: non-negative JSON integer (omit to use the caret), "value"?: bool (omit to toggle)}"#),
        CommandSpec::new("review.nextComment", "Next", "Review › Comments", |s, _| nav_comment(s, 1)).pure(),
        CommandSpec::new("review.previousComment", "Previous", "Review › Comments", |s, _| nav_comment(s, -1)).pure(),
        CommandSpec::new("review.comments", "Show Comments", "Review › Comments", list_comments).pure(),
        CommandSpec::new("review.trackChanges", "Track Changes", "Review › Tracking", |s, v| {
            s.doc.settings.track_changes = p::bool(v, "value").unwrap_or(!s.doc.settings.track_changes);
            Ok(json!({"value": s.doc.settings.track_changes}))
        })
        .params(r#"{"value"?: bool (omit to toggle)}"#)
        .key("Mod+Shift+E"),
        CommandSpec::new("review.acceptAll", "Accept All Changes", "Review › Changes", |s, v| resolve_all_then(s, v, true))
            .params(r#"{"stopTracking"?: bool (also turn Track Changes off)}"#),
        CommandSpec::new("review.rejectAll", "Reject All Changes", "Review › Changes", |s, v| resolve_all_then(s, v, false))
            .params(r#"{"stopTracking"?: bool (also turn Track Changes off)}"#),
        CommandSpec::new("review.accept", "Accept", "Review › Changes", |s, v| resolve_sel_then(s, v, true))
            .params(r#"{"next"?: bool (then select the next change)}"#),
        CommandSpec::new("review.reject", "Reject", "Review › Changes", |s, v| resolve_sel_then(s, v, false))
            .params(r#"{"next"?: bool (then select the next change)}"#),
        CommandSpec::new("review.nextChange", "Next Change", "Review › Changes", |s, _| nav_change(s, 1)).pure(),
        CommandSpec::new("review.previousChange", "Previous Change", "Review › Changes", |s, _| nav_change(s, -1)).pure(),
        CommandSpec::new("review.markup", "Display for Review", "Review › Tracking", |s, v| {
            s.view.show_markup = p::str(v, "value").map(|m| m != "noMarkup" && m != "original").unwrap_or(!s.view.show_markup);
            // Without markup, deletions leave the layout.
            s.relayout();
            Ok(json!({"showMarkup": s.view.show_markup}))
        })
        .pure(),
        CommandSpec::new("review.trackingOptions", "Track Changes Options", "Review › Tracking", tracking_options)
            .params(
                r#"{"comments"?: bool, "ink"?: bool, "insertionsDeletions"?: bool, "formatting"?: bool, "balloons"?: "revisions|inline|commentsAndFormatting", "commentDisplay"?: "margin|selected", "insertMark"?: "underline|doubleUnderline|bold|italic|strikethrough|colorOnly|none", "insertColor"?: "byAuthor" | "RRGGBB", "deleteMark"?: "strikethrough|doubleStrikethrough|hidden|caret|hash|underline|doubleUnderline|bold|italic|colorOnly|none", "deleteColor"?: "byAuthor" | "RRGGBB", "changedLines"?: "outside|left|right|none", "changedLinesColor"?: "auto" | "RRGGBB", "trackFormatting"?: bool, "moveFromMark"?: (as deleteMark; none = plain text, no colour), "moveFromColor"?: "byAuthor" | "RRGGBB", "moveToMark"?: (as insertMark; none = plain text, no colour), "moveToColor"?: "byAuthor" | "RRGGBB", "trackMoves"?: bool (the document's setting; off = a cut and paste is a deletion and an insertion, not a move)} → the options (omit all to read them)"#,
            )
            .pure(),
        CommandSpec::new("review.wordCount", "Word Count", "Review › Proofing", word_count)
            .params(r#"{"includeTextBoxes"?: bool}"#)
            // ⌘⇧G is Find Previous on macOS.
            .key(super::non_mac("Mod+Shift+G"))
            .pure(),
        CommandSpec::new("review.changes", "Reviewing Pane", "Review › Tracking", list_changes).pure(),
        CommandSpec::new("review.spelling", "Spelling & Grammar", "Review › Proofing", next_issue)
            .key(super::mac_or("F7 / Alt+F7 / Mod+Alt+L", "F7 / Alt+F7"))
            .pure(),
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
            // The same rule as loading the dictionary file, so a word added is never dropped later.
            if wordcraft_proof::custom_word(&w).is_none() {
                let max = wordcraft_proof::MAX_CUSTOM_WORD_CHARS;
                return Err(CmdError::Params(format!("{w:?} isn't a single word of at most {max} characters")));
            }
            wordcraft_proof::add_word(w.trim());
            s.persist_user_dictionary(&[]);
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
            // For this session only, as in Word: not added to (or saved with) the dictionary.
            if !w.trim().is_empty() {
                wordcraft_proof::ignore_word(w.trim());
            }
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
            s.prefs.check_spelling = s.view.proofing;
            Ok(json!({"value": s.view.proofing}))
        })
        .pure(),
        CommandSpec::new("review.thesaurus", "Thesaurus", "Review › Proofing", thesaurus).key("Shift+F7").pure(),
    ]
}

#[cfg(test)]
mod tests;
