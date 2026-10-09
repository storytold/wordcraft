//! What a chat member (an invited agent) may do, and running its commands as itself.

use serde_json::{Value, json};

use crate::{MemberSel, WordApp};

const GROUPS: [&str; 14] = [
    "text.",
    "caret.",
    "select.",
    "edit.",
    "format.",
    "para.",
    "styles.",
    "table.",
    "review.",
    "insert.",
    "layout.",
    "references.",
    "design.",
    "hf.",
];
const DENY: [&str; 50] = [
    "insert.picture",
    "insert.textFromFile",
    "insert.object",
    "picture.change",
    "review.compare",
    "review.combine",
    "review.trackChanges",
    "edit.paste",
    "review.readAloud",
    "edit.repeat",
    "edit.undo",
    "edit.redo",
    "review.addToDictionary",
    "review.ignoreAll",
    "insert.quickParts",
    "insert.autoText",
    "references.citationStyle",
    "layout.linkToPrevious",
    "insert.removeHeader",
    "insert.removeFooter",
    "table.deleteTable",
    "styles.delete",
    // Never tracked by the engine (members use select.text + text.insert, which is tracked).
    "edit.replace",
    "edit.replaceAll",
    "format.hidden",
    "para.sort",
    // Found untracked by the final review's sweep; the run-time guard would refuse them anyway.
    "format.changeCase",
    "insert.coverPage",
    "insert.footer",
    "insert.header",
    "references.caption",
    "table.deleteCells",
    "table.deleteColumn",
    "table.deleteRow",
    "table.formula",
    "table.insertColumnLeft",
    "table.insertColumnRight",
    "table.insertRowAbove",
    "table.insertRowBelow",
    "table.merge",
    "table.sort",
    "table.split",
    "table.toText",
    "table.fromText",
    "references.removeToc",
    "insert.wordArt",
    // They write list label text or number formats (visible text that cannot be tracked).
    "para.defineNumber",
    "para.defineBullet",
    "para.setNumberingValue",
    "para.restartNumbering",
];
const DIRECT: [&str; 7] = ["document.inspect", "document.paragraph", "engine.commands", "review.changes", "review.comments", "review.wordCount", "view.page"];

/// Commands the gate itself provides to members (not in the engine registry): (id, label, params).
pub const GATE_COMMANDS: [(&str, &str, &str); 1] = [(
    "select.owner",
    "Select what the owner has selected (a copy: the owner's selection stays)",
    r#"{} -> {"text", "anchor", "focus", "paragraphs": [first, last], "caretOnly"?, "note"?}"#,
)];

/// `method` is the control method; `command` the engine command for `engine.execute`/`command`
/// (or the method itself when it is a bare command id).
pub fn member_allowed(method: &str, command: Option<&str>) -> bool {
    if DIRECT.contains(&method) {
        return true;
    }
    let id = match method {
        "engine.execute" | "command" => match command {
            Some(c) => c,
            None => return false,
        },
        m => m,
    };
    if DIRECT.contains(&id) {
        return true;
    }
    GROUPS.iter().any(|g| id.starts_with(g))
        && !DENY.contains(&id)
        && !id.starts_with("review.restrict")
        && !id.starts_with("edit.paste")
        && !id.starts_with("edit.copy")
        && id != "edit.cut"
}

/// Named list kinds a member may apply (`para.bullets/numbering/multilevel {"kind"}`); any other
/// kind is a custom label (a bullet character), which is list label text.
const LIST_KINDS: [&str; 10] = ["bullet", "numbered", "number", "numberedParen", "outline", "upperLetter", "lowerLetter", "lowerRoman", "legal", "multilevel"];

/// Reason for a member's reject that would leave the paragraphs it inserted split (the engine
/// does not join paragraphs on reject). The member gets it in the chat's language.
pub const REJECT_PARAGRAPHS: &str = "rejecting new paragraphs: ask the OWNER";

fn chat_lang(app: &WordApp) -> wordcraft_chat::Lang {
    app.chat.as_ref().map(|h| h.lang()).unwrap_or_default()
}

/// Run `id` as `handle`: author = handle, track changes ON for mutating commands (except
/// accept/reject), on the member's own selection. The owner's author, track-changes state,
/// selection, pending format, repeat/find/painter state and macro recording are always put
/// back, also when the command fails. Dialog requests and status text are discarded.
///
/// EVERY command is checked by [`crate::chat_guard`] afterwards (a snapshot is taken before):
/// a command that is not supposed to edit must leave the document exactly as it was (its dirty
/// flag is put back); a mutating one may leave only changes tracked under `handle` and allowed
/// formatting. Anything else is rolled back (document, selection, undo and redo) and refused with
/// "untracked change refused"; allowed formatting is announced in the chat ("@x formatted:
/// <command>"); accept/reject may only resolve tracked changes and are announced whenever they
/// changed the document, with the characters accepted/rejected and their authors; resolving a
/// comment is announced. The text of comments by others never changes.
pub fn run_as_member(app: &mut WordApp, handle: &str, id: &str, params: Value) -> Result<Value, String> {
    let params = body_by_default(id, params);
    if id == "review.deleteComment" && !owns_comment(app, handle, &params) {
        return Err(format!("not allowed for {handle}: only your own comments"));
    }
    if sets_marks_or_hidden(id, &params) {
        return Err(format!("not allowed for {handle}: formatting cannot set ins/del/hidden (revision marks, hidden text)"));
    }
    if custom_list_label(id, &params) {
        return Err(format!("not allowed for {handle}: list label text (custom bullets or numbers) is the owner's"));
    }
    if app.member_sel_gen != app.session.doc_replaced {
        app.member_sel.clear();
        app.member_sel_gen = app.session.doc_replaced;
    }
    let judging = id.starts_with("review.accept") || id.starts_with("review.reject");
    let mutates = app.session.registry.get(id).is_some_and(|c| c.mutates);
    // acceptAll/rejectAll do not use the selection.
    let uses_selection = mutates && id != "review.acceptAll" && id != "review.rejectAll";
    if uses_selection
        && let Some(ms) = app.member_sel.get(handle)
        && !ms.text.is_empty()
        && text_under(&app.session.doc, &ms.sel) != ms.text
    {
        return Err(format!("{handle}: your selection changed; select again"));
    }
    let owner_author = std::mem::replace(&mut app.session.author, handle.to_string());
    let owner_track = app.session.doc.settings.track_changes;
    let owner_sel = app.session.sel.clone();
    let owner_view = app.session.view.clone();
    let owner_pending = app.session.pending.take();
    let owner_last = app.session.last_command.clone();
    let owner_find = std::mem::take(&mut app.session.find);
    let owner_painter = app.session.painter.take();
    let owner_goal = app.session.goal_x;
    let owner_page = app.session.page_hint;
    let owner_rec = app.session.recording.take();
    let owner_ui = std::mem::take(&mut app.session.ui_requests);
    let owner_status = std::mem::take(&mut app.session.status);
    app.session.close_typing();
    if let Some(ms) = app.member_sel.get(handle) {
        app.session.sel = ms.sel.clone();
        app.session.clamp_selection();
    }
    if mutates && !judging {
        app.session.doc.settings.track_changes = true;
    }
    let snap = app.session.edit_snapshot();
    let mut r = if id == "select.owner" { Ok(select_owner(app, &owner_sel)) } else { app.session.run(id, &params).map_err(|e| e.to_string()) };
    use crate::chat_guard::Verdict;
    let rejected_paragraphs = judging && !id.starts_with("review.accept") && r.is_ok() && {
        crate::chat_guard::inserted_paragraphs(&app.session.doc) < crate::chat_guard::inserted_paragraphs(snap.doc())
    };
    let verdict = match &r {
        Err(_) => Verdict::Clean,
        Ok(_) if rejected_paragraphs => Verdict::Refuse(REJECT_PARAGRAPHS),
        Ok(_) if judging => crate::chat_guard::check_judging(snap.doc(), &app.session.doc, id.starts_with("review.accept")),
        Ok(_) if mutates => crate::chat_guard::check_member(snap.doc(), &app.session.doc, handle),
        Ok(_) if app.session.undo_depth() != snap.undo_depth() => Verdict::Refuse(crate::chat_guard::PURE),
        Ok(_) => crate::chat_guard::check_unchanged(snap.doc(), &app.session.doc),
    };
    let mut changed_from = None;
    let mut judged = None;
    let mut resolved = Vec::new();
    if r.is_err() || matches!(verdict, Verdict::Refuse(_)) {
        // Also after a plain failure: the engine's own rollback does not give the owner's redo back.
        app.session.restore(snap);
    } else if mutates {
        if judging {
            // Announced whenever the document changed (also a partial accept/reject).
            judged = (snap.doc() != &app.session.doc).then(|| crate::chat_guard::judged(snap.doc(), &app.session.doc));
        }
        if id == "review.resolveComment" {
            resolved = crate::chat_guard::resolved_comments(snap.doc(), &app.session.doc);
        }
        changed_from = Some(snap.doc().clone());
    } else {
        // Nothing changed (checked above); a command that only marked the document dirty
        // (entering an existing header) leaves the owner's "unsaved" state as it was.
        app.session.dirty = snap.dirty();
    }
    if let Verdict::Refuse(why) = verdict {
        r = Err(if why == REJECT_PARAGRAPHS { chat_lang(app).text(wordcraft_chat::Text::RejectParagraphs) } else { format!("{}: {why}", crate::chat_guard::REFUSED) });
    }
    app.session.ui_requests = owner_ui;
    app.session.status = owner_status;
    app.session.close_typing();
    let mine = MemberSel { sel: app.session.sel.clone(), text: text_under(&app.session.doc, &app.session.sel) };
    app.member_sel.insert(handle.to_string(), mine);
    app.member_sel_gen = app.session.doc_replaced;
    app.session.author = owner_author;
    app.session.doc.settings.track_changes = owner_track;
    app.session.sel = owner_sel;
    if let Some(before) = &changed_from {
        // The owner's and the other members' selections follow the edit.
        app.session.sel = crate::chat_shift::shift_selection(before, &app.session.doc, &app.session.sel);
        for (h, ms) in app.member_sel.iter_mut() {
            if h != handle {
                ms.sel = crate::chat_shift::shift_selection(before, &app.session.doc, &ms.sel);
            }
        }
    }
    app.session.clamp_selection();
    app.session.view = owner_view;
    app.session.pending = owner_pending;
    app.session.last_command = owner_last;
    app.session.find = owner_find;
    app.session.painter = owner_painter;
    app.session.goal_x = owner_goal;
    app.session.page_hint = owner_page;
    app.session.recording = owner_rec;
    if let Some(h) = &app.chat
        && r.is_ok()
    {
        if verdict == Verdict::Format {
            h.post_system(&h.lang().text(wordcraft_chat::Text::Formatted { who: handle, command: id }));
        }
        if let Some((n, authors)) = judged {
            let who = if authors.is_empty() { "?".to_string() } else { authors.join(", ") };
            let accepted = id.starts_with("review.accept");
            h.post_system(&h.lang().text(wordcraft_chat::Text::Judged { who: handle, accepted, chars: n, authors: &who }));
        }
        for (author, now_resolved) in resolved {
            h.post_system(&h.lang().text(wordcraft_chat::Text::Resolved { who: handle, reopened: !now_resolved, author: &author }));
        }
    }
    r
}

/// `select.owner`: the member's selection becomes a copy of the owner's (`owner`); with only a
/// caret, the paragraph at the caret. Read-only: the document does not change.
fn select_owner(app: &mut WordApp, owner: &wordcraft_engine::Selection) -> Value {
    let caret_only = owner.anchor == owner.focus;
    app.session.sel = owner.clone();
    app.session.clamp_selection();
    if caret_only {
        let f = app.session.sel.focus.clone();
        let len = app.session.doc.para_at(&f).map(|p| p.len()).unwrap_or(0);
        app.session.sel = wordcraft_engine::Selection {
            anchor: wordcraft_doc::Pos { off: 0, ..f.clone() },
            focus: wordcraft_doc::Pos { off: len, ..f },
        };
    }
    let (a, b) = app.session.sel.ordered();
    let body = a.story == wordcraft_doc::StoryRef::Body && b.story == wordcraft_doc::StoryRef::Body;
    let top = |p: &wordcraft_doc::Pos| p.path.0.first().copied();
    let mut v = json!({
        "text": app.session.selected_text(),
        "anchor": wordcraft_engine::cmd::pos_json(&app.session.sel.anchor),
        "focus": wordcraft_engine::cmd::pos_json(&app.session.sel.focus),
        "paragraphs": if body { json!([top(&a), top(&b)]) } else { Value::Null },
    });
    if caret_only {
        v["caretOnly"] = json!(true);
        v["note"] = json!(chat_lang(app).text(wordcraft_chat::Text::OwnerNoSelection));
    }
    v
}

/// Commands that search or read "the current story" (the engine takes the selection's story when
/// `story` is not given). A member's own selection may sit in a comment or a header, so for a
/// member they mean the body unless `story` is given: `select.text` finds the Nth occurrence from
/// the start of the document, `document.paragraph {path}` reads a body paragraph.
const BODY_BY_DEFAULT: [&str; 3] = ["select.text", "document.paragraph", "document.text"];

fn body_by_default(id: &str, mut params: Value) -> Value {
    if BODY_BY_DEFAULT.contains(&id)
        && let Some(o) = params.as_object_mut()
        && !o.contains_key("story")
    {
        o.insert("story".into(), json!("body"));
    }
    params
}

/// `para.bullets/numbering/multilevel` with a `kind` that is not a named list kind.
fn custom_list_label(id: &str, params: &Value) -> bool {
    matches!(id, "para.bullets" | "para.numbering" | "para.multilevel")
        && params.get("kind").is_some_and(|k| k.as_str().is_none_or(|k| !LIST_KINDS.contains(&k)))
}

/// The text a selection covers (empty for a caret), on a clamped copy of it.
fn text_under(doc: &wordcraft_doc::Document, sel: &wordcraft_engine::Selection) -> String {
    let (a, b) = sel.ordered();
    let (a, b) = (doc.clamp(&a), doc.clamp(&b));
    if a == b || a.story != b.story {
        return String::new();
    }
    doc.copy_range(&a, &b).plain_text()
}

/// Character formatting keys a member may never set: they forge revision marks or hide text.
const FORBIDDEN_CHAR_KEYS: [&str; 5] = ["ins", "del", "hidden", "vanish", "specvanish"];

/// `format.set {props}` and `styles.* {chr|props}` that would set a revision mark or hide text.
fn sets_marks_or_hidden(id: &str, params: &Value) -> bool {
    if id != "format.set" && !id.starts_with("styles.") {
        return false;
    }
    ["props", "chr"].iter().filter_map(|k| params.get(*k).and_then(Value::as_object)).any(|o| {
        o.keys().any(|k| {
            let k = k.to_ascii_lowercase();
            FORBIDDEN_CHAR_KEYS.contains(&k.as_str())
        })
    })
}

/// `review.deleteComment` is only for a member's own comment (and replies that are all its own).
fn owns_comment(app: &WordApp, handle: &str, params: &Value) -> bool {
    if params.get("all").and_then(Value::as_bool).unwrap_or(false) {
        return false;
    }
    let Some(id) = params.get("id").and_then(Value::as_u64).and_then(|i| u32::try_from(i).ok()) else {
        return false;
    };
    let comments = &app.session.doc.comments;
    comments.get(&id).is_some_and(|c| c.author == handle) && comments.values().filter(|c| c.parent == Some(id)).all(|c| c.author == handle)
}

/// Page `page` (1-based) as a base64 PNG, rendered in memory (no file).
pub fn view_page(app: &mut WordApp, page: u64, scale: f32) -> Result<Value, String> {
    let idx = usize::try_from(page.max(1) - 1).map_err(|e| e.to_string())?;
    let scale = scale.clamp(0.25, 2.0);
    let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let l = app.session.export_layout();
        let pg = l.pages.get(idx)?;
        let img = wordcraft_render::render_page(&app.session.doc, pg, scale, &Default::default());
        Some((img.to_png(), img.width, img.height))
    }))
    .map_err(|_| "render failed".to_string())?;
    let (png, w, h) = rendered.ok_or_else(|| format!("no page {page}"))?;
    Ok(json!({"png_base64": wordcraft_engine::cmd::insert::base64_encode(&png), "width": w, "height": h}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use wordcraft_engine::Session;

    fn app() -> WordApp {
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Default::default());
        let _ = a.run("document.setText", json!({"text": "Valor: 1.000 euros. Prazo: 12 meses."}));
        a.session.author = "Owner".into();
        let hub = wordcraft_chat::Hub::new("k".into());
        let (_, code) = hub.invite("@claude", wordcraft_chat::hub::now_ms()).unwrap_or_default();
        let _ = hub.join(&code, wordcraft_chat::hub::now_ms());
        a.chat = Some(hub);
        a
    }

    #[test]
    fn allow_list() {
        assert!(member_allowed("engine.execute", Some("text.insert")));
        assert!(member_allowed("engine.execute", Some("review.acceptAll")));
        assert!(member_allowed("document.inspect", None));
        assert!(member_allowed("view.page", None));
        assert!(member_allowed("engine.commands", None));
        for bad in [
            "file.save",
            "file.open",
            "tools.macros",
            "mailings.finish",
            "insert.picture",
            "review.compare",
            "review.trackChanges",
            "edit.paste",
            "file.setAuthor",
            "ui.click",
            "view.zoom",
        ] {
            assert!(!member_allowed("engine.execute", Some(bad)), "{bad}");
        }
        assert!(!member_allowed("ui.screenshot", None));
        assert!(!member_allowed("app.quit", None));
        assert!(!member_allowed("engine.execute", Some("future.unknownCommand")));
    }

    #[test]
    fn untracked_commands_are_denied() {
        for id in [
            "edit.replaceAll",
            "edit.replace",
            "format.hidden",
            "para.sort",
            "format.changeCase",
            "insert.coverPage",
            "insert.footer",
            "insert.header",
            "references.caption",
            "table.deleteCells",
            "table.deleteColumn",
            "table.deleteRow",
            "table.formula",
            "table.insertColumnLeft",
            "table.insertColumnRight",
            "table.insertRowAbove",
            "table.insertRowBelow",
            "table.merge",
            "table.sort",
            "table.split",
            "table.toText",
            "table.fromText",
            "references.removeToc",
            "insert.wordArt",
        ] {
            assert!(!member_allowed("engine.execute", Some(id)), "{id}");
            assert!(!member_allowed(id, None), "{id}");
        }
    }

    #[test]
    fn member_cannot_set_revision_marks_or_hidden_text() {
        let mut a = app();
        let _ = a.run("review.trackChanges", json!({"value": true}));
        let _ = a.run("select.text", json!({"text": "Valor"}));
        let _ = a.run("text.insert", json!({"text": "Montante"}));
        let _ = a.run("review.trackChanges", json!({"value": false}));
        let before = a.session.doc.clone();
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "Prazo"}));
        for (id, p) in [
            ("format.set", json!({"props": {"ins": 0}})),
            ("format.set", json!({"props": {"del": 0}})),
            ("format.set", json!({"props": {"hidden": true}})),
            ("format.set", json!({"props": {"vanish": true}})),
            ("format.set", json!({"props": {"Hidden": true, "bold": true}})),
            ("styles.modify", json!({"style": "Normal", "chr": {"hidden": true}})),
            ("styles.modify", json!({"style": "Normal", "chr": {"ins": 0}})),
            ("styles.modify", json!({"style": "Normal", "chr": {"del": 0}})),
            ("styles.create", json!({"name": "X", "chr": {"vanish": true}})),
        ] {
            let r = run_as_member(&mut a, "@claude", id, p.clone());
            assert!(r.as_ref().is_err_and(|e| e.contains("ins/del/hidden")), "{id} {p} -> {r:?}");
        }
        assert_eq!(a.session.doc, before);
    }

    fn system_lines(a: &WordApp) -> Vec<String> {
        a.chat.as_ref().map(|h| h.messages()).unwrap_or_default().into_iter().filter(|m| m.role == wordcraft_chat::Role::System).map(|m| m.text).collect()
    }

    #[test]
    fn untracked_structure_change_is_rolled_back_silently() {
        let mut a = app();
        let doc = a.session.doc.clone();
        let undo = a.session.undo_labels();
        let lines = system_lines(&a);
        let _ = run_as_member(&mut a, "@claude", "caret.docEnd", json!({}));
        let r = run_as_member(&mut a, "@claude", "insert.table", json!({"rows": 2, "cols": 2}));
        assert!(r.as_ref().is_err_and(|e| e.starts_with("untracked change refused")), "{r:?}");
        assert_eq!(a.session.doc, doc);
        assert_eq!(a.session.undo_labels(), undo, "no undo step is left behind");
        assert_eq!(system_lines(&a), lines, "nothing is posted for a refused command");
        assert_eq!(a.session.author, "Owner");
        assert!(!a.session.doc.settings.track_changes);
    }

    #[test]
    fn document_settings_change_is_refused() {
        let mut a = app();
        let doc = a.session.doc.clone();
        let r = run_as_member(&mut a, "@claude", "layout.margins", json!({"preset": "narrow"}));
        assert!(r.as_ref().is_err_and(|e| e.starts_with("untracked change refused")), "{r:?}");
        assert_eq!(a.session.doc, doc);
    }

    #[test]
    fn allowed_formatting_is_announced_once() {
        let mut a = app();
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "12 meses"}));
        let r = run_as_member(&mut a, "@claude", "format.bold", json!({}));
        assert!(r.is_ok(), "{r:?}");
        let _ = a.run("select.text", json!({"text": "12 meses"}));
        let st = a.session.run("format.state", &json!({})).unwrap_or_default();
        assert_eq!(st["bold"], true, "{st}");
        let lines = system_lines(&a);
        assert_eq!(lines.iter().filter(|l| l.as_str() == "@claude formatted: format.bold").count(), 1, "{lines:?}");
    }

    #[test]
    fn formatting_outside_the_allowed_set_is_refused() {
        let mut a = app();
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "12 meses"}));
        let doc = a.session.doc.clone();
        for (id, p) in [("format.allCaps", json!({})), ("format.set", json!({"props": {"link": "https://example.com"}})), ("para.keepNext", json!({}))] {
            let r = run_as_member(&mut a, "@claude", id, p);
            assert!(r.as_ref().is_err_and(|e| e.starts_with("untracked change refused")), "{id}: {r:?}");
            assert_eq!(a.session.doc, doc, "{id}");
        }
        assert!(!system_lines(&a).iter().any(|l| l.contains("formatted")));
    }

    #[test]
    fn tracked_edit_is_clean_and_not_announced() {
        let mut a = app();
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "12 meses"}));
        assert!(run_as_member(&mut a, "@claude", "text.insert", json!({"text": "24 meses"})).is_ok());
        assert!(run_as_member(&mut a, "@claude", "text.newParagraph", json!({})).is_ok());
        assert!(run_as_member(&mut a, "@claude", "text.insert", json!({"text": "Nova cláusula."})).is_ok());
        assert!(!system_lines(&a).iter().any(|l| l.contains("formatted")));
    }

    #[test]
    fn hidden_style_cannot_be_applied_by_a_member() {
        let mut a = app();
        let _ = a.run("select.text", json!({"text": "Prazo"}));
        let _ = a.run("styles.create", json!({"name": "Oculto"}));
        let _ = a.run("styles.modify", json!({"style": "Oculto", "chr": {"hidden": true}}));
        let _ = a.run("para.normal", json!({}));
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "Valor"}));
        let doc = a.session.doc.clone();
        let r = run_as_member(&mut a, "@claude", "para.style", json!({"style": "Oculto"}));
        assert!(r.as_ref().is_err_and(|e| e.starts_with("untracked change refused")), "{r:?}");
        assert_eq!(a.session.doc, doc);
    }

    #[test]
    fn member_rewrite_over_an_owner_deletion_is_allowed_and_tracked() {
        let mut a = app();
        let _ = a.run("review.trackChanges", json!({"value": true}));
        let _ = a.run("select.text", json!({"text": "1.000 "}));
        let _ = a.run("text.delete", json!({}));
        let _ = a.run("review.trackChanges", json!({"value": false}));
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "Valor: 1.000 euros."}));
        let r = run_as_member(&mut a, "@claude", "text.insert", json!({"text": "Valor: 2.000 euros."}));
        assert!(r.is_ok(), "{r:?}");
        let ch = a.session.run("review.changes", &json!({})).unwrap_or_default();
        let list = ch.as_array().cloned().unwrap_or_default();
        assert!(list.iter().any(|c| c["author"] == "Owner" && c["kind"] == "delete" && c["text"] == "1.000 "), "{ch}");
        assert!(list.iter().any(|c| c["author"] == "@claude" && c["kind"] == "insert"), "{ch}");
    }

    #[test]
    fn owner_redo_survives_a_refused_member_command() {
        let mut a = app();
        let _ = a.run("text.insert", json!({"text": "x"}));
        let _ = a.run("edit.undo", json!({}));
        assert!(a.session.can_redo());
        let _ = run_as_member(&mut a, "@claude", "insert.table", json!({"rows": 1, "cols": 1}));
        assert!(a.session.can_redo());
    }

    #[test]
    fn select_owner_gives_the_member_a_copy_of_the_owner_selection() {
        let mut a = app();
        let _ = a.run("select.text", json!({"text": "12 meses"}));
        let owner_sel = a.session.sel.clone();
        let doc = a.session.doc.clone();
        let r = run_as_member(&mut a, "@claude", "select.owner", json!({})).unwrap_or_default();
        assert_eq!(r["text"], "12 meses", "{r}");
        assert_eq!(r["paragraphs"], json!([0, 0]), "{r}");
        assert!(r.get("anchor").is_some() && r.get("focus").is_some(), "{r}");
        assert_eq!(a.session.doc, doc, "read-only");
        assert_eq!(a.session.sel, owner_sel, "the owner's selection is untouched");
        let r = run_as_member(&mut a, "@claude", "text.insert", json!({"text": "24 meses"}));
        assert!(r.is_ok(), "{r:?}");
        assert_eq!(a.session.selected_text(), "12 meses", "the owner still has his selection");
        let ch = a.session.run("review.changes", &json!({})).unwrap_or_default();
        let list = ch.as_array().cloned().unwrap_or_default();
        assert!(list.iter().any(|c| c["author"] == "@claude" && c["kind"] == "delete" && c["text"] == "12 meses"), "{ch}");
        assert!(list.iter().any(|c| c["author"] == "@claude" && c["kind"] == "insert" && c["text"] == "24 meses"), "{ch}");
        assert_eq!(list.len(), 2, "exactly the owner's selected text was replaced: {ch}");
        assert!(member_allowed("engine.execute", Some("select.owner")));
    }

    #[test]
    fn select_owner_with_only_a_caret_gives_the_paragraph_and_says_so() {
        let mut a = three();
        let _ = a.run("select.text", json!({"text": "epsilon"}));
        let _ = a.run("caret.left", json!({}));
        let owner_sel = a.session.sel.clone();
        let r = run_as_member(&mut a, "@claude", "select.owner", json!({})).unwrap_or_default();
        assert_eq!(r["caretOnly"], true, "{r}");
        assert_eq!(r["text"], "Delta epsilon.", "{r}");
        assert_eq!(r["paragraphs"], json!([1, 1]), "{r}");
        assert!(r["note"].as_str().is_some_and(|n| !n.is_empty()), "{r}");
        assert_eq!(a.session.sel, owner_sel);
    }

    #[test]
    fn members_search_and_read_the_body_wherever_their_selection_is() {
        // Live E2E 2: a member whose selection was in a comment (or header) could not select body
        // text, and its `read` lost the change marks (document.paragraph looked in that story).
        let mut a = three();
        let part = owner_comment(&mut a);
        for place in ["comment", "header", "doc end"] {
            match place {
                "comment" => assert!(run_as_member(&mut a, "@claude", "select.range", in_part(part, 3, 3)).is_ok()),
                "header" => {
                    let _ = a.run("insert.editHeader", json!({}));
                    let _ = a.run("insert.closeHeader", json!({}));
                    assert!(run_as_member(&mut a, "@claude", "insert.editHeader", json!({})).is_ok());
                }
                _ => assert!(run_as_member(&mut a, "@claude", "caret.docEnd", json!({})).is_ok()),
            }
            let r = run_as_member(&mut a, "@claude", "select.text", json!({"text": "Alfa"}));
            let r = r.unwrap_or_else(|e| panic!("{place}: {e}"));
            assert_eq!(r["anchor"]["story"], "body", "{place}: {r}");
            assert_eq!(r["anchor"]["path"], json!([0]), "{place}: {r}");
            // Back into that place for the paragraph read.
            match place {
                "comment" => assert!(run_as_member(&mut a, "@claude", "select.range", in_part(part, 3, 3)).is_ok()),
                "header" => assert!(run_as_member(&mut a, "@claude", "insert.editHeader", json!({})).is_ok()),
                _ => {}
            }
            let v = run_as_member(&mut a, "@claude", "document.paragraph", json!({"path": [1]})).unwrap_or_else(|e| panic!("{place}: {e}"));
            assert_eq!(v["paragraph"]["text"], "Delta epsilon.", "{place}: {v}");
            // An explicit story still works.
            let v = run_as_member(&mut a, "@claude", "select.text", json!({"text": "aceito", "story": {"part": part}}));
            assert!(v.is_ok(), "{place}: {v:?}");
        }
    }

    #[test]
    fn members_may_read_one_paragraph_in_detail() {
        let mut a = app();
        let v = done(crate::control::handle_member(&mut a, "@claude", &member_req("document.paragraph", json!({"path": [0]}))));
        assert_eq!(v["ok"], true, "{v}");
        assert!(v["result"]["paragraph"]["text"].as_str().is_some_and(|t| t.contains("Valor")), "{v}");
    }

    /// Probe E1/E10 sample: three owner paragraphs.
    fn three() -> WordApp {
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Default::default());
        a.session.author = "Owner".into();
        let _ = a.run("document.setText", json!({"text": "Alfa beta gama.\nDelta epsilon.\nZeta eta."}));
        a.chat = Some(wordcraft_chat::Hub::new("k".into()));
        a
    }

    /// A member's tracked Enter inside the owner's "Delta epsilon.": at its start or after "Delta ".
    fn split(at_start: bool) -> WordApp {
        let mut a = three();
        if at_start {
            let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "Delta"}));
            let _ = run_as_member(&mut a, "@claude", "caret.home", json!({}));
        } else {
            let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "Delta "}));
            let _ = run_as_member(&mut a, "@claude", "select.collapse", json!({"end": true}));
        }
        let r = run_as_member(&mut a, "@claude", "text.newParagraph", json!({}));
        assert!(r.is_ok(), "{r:?}");
        a
    }

    #[test]
    fn a_member_enter_does_not_hide_changes_to_the_owner_paragraph() {
        let refused: Vec<(&str, Value)> = vec![
            ("para.shading", json!({"color": "000000"})),
            ("para.borders", json!({"kind": "all"})),
            ("para.keepNext", json!({})),
            ("para.keepLines", json!({})),
            ("para.pageBreakBefore", json!({})),
            ("para.set", json!({"props": {"outlineLevel": 0}})),
            ("para.set", json!({"props": {"bidi": true}})),
            ("para.tabs", json!({"tabs": [{"pos": 72, "align": "left", "leader": "dot"}]})),
            ("para.rtl", json!({})),
            ("para.numbering", json!({"kind": "numbered"})),
            ("para.bullets", json!({})),
        ];
        // Both halves of the split, and the control case without the Enter.
        for (case, select) in [("start", "Delta epsilon."), ("middle, 2nd half", "epsilon."), ("middle, 1st half", "Delta")] {
            for (id, p) in &refused {
                let mut a = if case == "start" { split(true) } else { split(false) };
                let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": select}));
                let before = a.session.doc.clone();
                let lines = system_lines(&a);
                let r = run_as_member(&mut a, "@claude", id, p.clone());
                assert!(r.as_ref().is_err_and(|e| e.starts_with("untracked change refused")), "{case} {id} {p}: {r:?}");
                assert_eq!(a.session.doc, before, "{case} {id} {p}: rolled back");
                assert_eq!(system_lines(&a), lines, "{case} {id}: nothing announced");
                let mut c = three();
                let _ = run_as_member(&mut c, "@claude", "select.text", json!({"text": "Delta epsilon."}));
                let r = run_as_member(&mut c, "@claude", id, p.clone());
                assert!(r.as_ref().is_err_and(|e| e.starts_with("untracked change refused")), "control {id} {p}: {r:?}");
            }
            // Allowed paragraph formatting on the owner's text after the Enter is announced.
            for (id, p) in [("para.style", json!({"style": "Heading1"})), ("para.align", json!({"value": "center"}))] {
                let mut a = if case == "start" { split(true) } else { split(false) };
                let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": select}));
                let r = run_as_member(&mut a, "@claude", id, p.clone());
                assert!(r.is_ok(), "{case} {id}: {r:?}");
                let lines = system_lines(&a);
                assert_eq!(lines.iter().filter(|l| **l == format!("@claude formatted: {id}")).count(), 1, "{case} {id}: {lines:?}");
            }
        }
    }

    #[test]
    fn a_member_enter_does_not_join_an_owner_paragraph_to_the_owner_list() {
        // Probe E10.
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Default::default());
        a.session.author = "Owner".into();
        let _ = a.run("document.setText", json!({"text": "Clausula um.\nTexto solto do dono.\nClausula dois.\nClausula tres."}));
        a.chat = Some(wordcraft_chat::Hub::new("k".into()));
        let _ = a.run("select.text", json!({"text": "Clausula um."}));
        let _ = a.run("para.numbering", json!({"kind": "numbered"}));
        let num = a.session.doc.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).and_then(|p| p.props.numbering).map(|n| n.num).unwrap_or(0);
        assert_ne!(num, 0);
        for t in ["Clausula dois.", "Clausula tres."] {
            let _ = a.run("select.text", json!({"text": t}));
            let _ = a.run("para.set", json!({"props": {"numbering": {"num": num, "level": 0}}}));
        }
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "Texto solto"}));
        let _ = run_as_member(&mut a, "@claude", "caret.home", json!({}));
        assert!(run_as_member(&mut a, "@claude", "text.newParagraph", json!({})).is_ok());
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "Texto solto do dono."}));
        let before = a.session.doc.clone();
        let lines = system_lines(&a);
        let r = run_as_member(&mut a, "@claude", "para.set", json!({"props": {"numbering": {"num": num, "level": 0}}}));
        assert!(r.as_ref().is_err_and(|e| e.starts_with("untracked change refused")), "{r:?}");
        assert_eq!(a.session.doc, before);
        assert_eq!(system_lines(&a), lines);
        // Changing the level (indent of a list paragraph) of an owner clause is refused too.
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "Clausula dois."}));
        let r = run_as_member(&mut a, "@claude", "para.indent", json!({}));
        assert!(r.as_ref().is_err_and(|e| e.starts_with("untracked change refused")), "{r:?}");
    }

    #[test]
    fn member_new_paragraph_after_a_heading_is_clean() {
        let mut a = three();
        let _ = a.run("select.text", json!({"text": "Alfa"}));
        let _ = a.run("para.style", json!({"style": "Heading1"}));
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "gama."}));
        let _ = run_as_member(&mut a, "@claude", "select.collapse", json!({"end": true}));
        assert!(run_as_member(&mut a, "@claude", "text.newParagraph", json!({})).is_ok());
        assert!(run_as_member(&mut a, "@claude", "text.insert", json!({"text": "Texto novo."})).is_ok());
        // Its own new paragraph may be formatted (only its tracked text is in it).
        assert!(run_as_member(&mut a, "@claude", "para.numbering", json!({"kind": "numbered"})).is_ok());
        assert!(!system_lines(&a).iter().any(|l| l.contains("text.")), "{:?}", system_lines(&a));
    }

    #[test]
    fn list_label_text_cannot_be_written_by_a_member() {
        // Probe E4.
        for id in ["para.defineNumber", "para.defineBullet", "para.setNumberingValue", "para.restartNumbering"] {
            assert!(!member_allowed("engine.execute", Some(id)), "{id}");
        }
        let mut a = three();
        let _ = a.run("select.all", json!({}));
        let _ = a.run("para.numbering", json!({"kind": "numbered"}));
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "Delta epsilon."}));
        let before = a.session.doc.clone();
        for (id, p) in [
            ("para.defineNumber", json!({"format": "decimal", "text": "NÃO SE APLICA: "})),
            ("para.defineBullet", json!({"char": "X"})),
            ("para.setNumberingValue", json!({"value": 9})),
            ("para.restartNumbering", json!({})),
            ("para.bullets", json!({"kind": "X"})),
            ("para.numbering", json!({"kind": "§"})),
        ] {
            let r = run_as_member(&mut a, "@claude", id, p.clone());
            assert!(r.is_err(), "{id} {p}: {r:?}");
            assert_eq!(a.session.doc, before, "{id}");
        }
        assert!(!system_lines(&a).iter().any(|l| l.contains("formatted")));
    }

    #[test]
    fn commands_that_should_not_edit_are_checked_too() {
        // Probe E2: every allowed command marked as not changing the document.
        let reg = a_registry();
        let mut hits = Vec::new();
        for c in reg.all().iter().filter(|c| !c.mutates && member_allowed("engine.execute", Some(c.id))) {
            for p in [json!({}), json!({"value": true}), json!({"text": "X", "style": "Heading1", "name": "Heading1"})] {
                for place in ["Delta epsilon.", "Alfa"] {
                    let mut a = three();
                    let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": place}));
                    a.session.dirty = false;
                    let before = a.session.doc.clone();
                    let undo = a.session.undo_labels();
                    let _ = run_as_member(&mut a, "@claude", c.id, p.clone());
                    if a.session.doc != before || a.session.undo_labels() != undo || a.session.dirty {
                        hits.push(format!("{} {p} at {place}", c.id));
                    }
                }
            }
        }
        assert!(hits.is_empty(), "{hits:?}");
        // insert.editHeader with no header: the header part it would create is refused.
        let mut a = three();
        let r = run_as_member(&mut a, "@claude", "insert.editHeader", json!({}));
        assert!(r.as_ref().is_err_and(|e| e.starts_with("untracked change refused")), "{r:?}");
        // With the owner's header it only moves the member into it.
        let _ = a.run("insert.editHeader", json!({}));
        let _ = a.run("insert.closeHeader", json!({}));
        let r = run_as_member(&mut a, "@claude", "insert.editHeader", json!({}));
        assert!(r.is_ok(), "{r:?}");
    }

    fn a_registry() -> std::sync::Arc<wordcraft_engine::Registry> {
        Session::new(wordcraft_doc::Document::new()).registry.clone()
    }

    #[test]
    fn refused_command_at_a_full_undo_stack_leaves_undo_identical() {
        // Probe E3: 500 owner steps, then a refused member command.
        let mut a = app();
        let _ = a.run("select.text", json!({"text": "Valor"}));
        for i in 0..510 {
            let _ = a.run(if i % 2 == 0 { "para.alignCenter" } else { "para.alignLeft" }, json!({}));
        }
        let undo = a.session.undo_labels();
        assert_eq!(undo.len(), 500);
        let r = run_as_member(&mut a, "@claude", "insert.table", json!({"rows": 1, "cols": 1}));
        assert!(r.as_ref().is_err_and(|e| e.starts_with("untracked change refused")), "{r:?}");
        assert_eq!(a.session.undo_labels(), undo);
    }

    #[test]
    fn member_selection_resets_after_a_version_restore() {
        let mut a = app();
        let _ = a.run("file.versions", json!({"save": "v1"}));
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "12 meses"}));
        assert!(a.member_sel.contains_key("@claude"));
        let _ = a.run("file.versions", json!({"restore": 0}));
        // The next member command finds the saved selection stale and drops it.
        let _ = run_as_member(&mut a, "@pi", "caret.right", json!({}));
        assert!(!a.member_sel.contains_key("@claude"), "a version restore invalidates saved selections");
    }

    fn owner_selected(a: &WordApp) -> String {
        a.session.selected_text()
    }

    #[test]
    fn owner_selection_follows_a_member_edit_before_it() {
        let mut a = app();
        let _ = a.run("select.text", json!({"text": "Prazo"}));
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "Valor"}));
        assert!(run_as_member(&mut a, "@claude", "text.insert", json!({"text": "Montante total"})).is_ok());
        assert_eq!(owner_selected(&a), "Prazo");
        // A new paragraph before the owner's: the paragraph index moves too.
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "Montante total"}));
        let _ = run_as_member(&mut a, "@claude", "caret.left", json!({}));
        assert!(run_as_member(&mut a, "@claude", "text.newParagraph", json!({})).is_ok());
        assert!(run_as_member(&mut a, "@claude", "text.insert", json!({"text": "Preâmbulo."})).is_ok());
        let _ = run_as_member(&mut a, "@claude", "text.newParagraph", json!({}));
        assert_eq!(owner_selected(&a), "Prazo");
        assert!(a.session.sel.focus.path.last() > 0, "{:?}", a.session.sel);
    }

    #[test]
    fn other_member_selection_follows_and_its_edit_lands_right() {
        let mut a = app();
        let _ = run_as_member(&mut a, "@pi", "select.text", json!({"text": "12 meses"}));
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "Valor"}));
        assert!(run_as_member(&mut a, "@claude", "text.insert", json!({"text": "Montante total do contrato"})).is_ok());
        let r = run_as_member(&mut a, "@pi", "text.insert", json!({"text": "24 meses"}));
        assert!(r.is_ok(), "{r:?}");
        let ch = a.session.run("review.changes", &json!({})).unwrap_or_default();
        let list = ch.as_array().cloned().unwrap_or_default();
        assert!(list.iter().any(|c| c["author"] == "@pi" && c["kind"] == "delete" && c["text"] == "12 meses"), "{ch}");
    }

    #[test]
    fn stale_member_selection_is_refused() {
        let mut a = app();
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "12 meses"}));
        // The owner rewrites the text under the member's selection.
        let _ = a.run("select.text", json!({"text": "Valor: 1.000"}));
        let _ = a.run("text.insert", json!({"text": "V"}));
        let doc = a.session.doc.clone();
        let r = run_as_member(&mut a, "@claude", "text.insert", json!({"text": "24 meses"}));
        assert!(r.as_ref().is_err_and(|e| e.contains("your selection changed; select again")), "{r:?}");
        assert_eq!(a.session.doc, doc);
        // Selecting again works.
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "12 meses"}));
        assert!(run_as_member(&mut a, "@claude", "text.insert", json!({"text": "24 meses"})).is_ok());
    }

    #[test]
    fn accept_or_reject_of_nothing_posts_nothing() {
        let mut a = app();
        let before = system_lines(&a);
        assert!(run_as_member(&mut a, "@claude", "review.acceptAll", json!({})).is_ok());
        assert!(run_as_member(&mut a, "@claude", "review.rejectAll", json!({})).is_ok());
        assert_eq!(system_lines(&a), before);
    }

    #[test]
    fn owner_page_hint_comes_back() {
        let mut a = app();
        a.session.page_hint = 7;
        let _ = run_as_member(&mut a, "@claude", "caret.set", json!({"page": 0, "x": 10.0, "y": 10.0}));
        let _ = run_as_member(&mut a, "@claude", "caret.down", json!({}));
        assert_eq!(a.session.page_hint, 7);
    }

    #[test]
    fn handle_member_rechecks_membership_on_the_ui_thread() {
        let mut a = app();
        let doc = a.session.doc.clone();
        // "@ghost" never joined this hub (or was removed after its request was queued).
        let v = done(crate::control::handle_member(&mut a, "@ghost", &member_req("engine.execute", json!({"command": "text.insert", "params": {"text": "x"}}))));
        assert_eq!(v["ok"], false, "{v}");
        assert!(v["error"].as_str().is_some_and(|e| e.contains("not a member")), "{v}");
        assert_eq!(a.session.doc, doc);
        let hub = a.chat.clone().unwrap_or_else(|| wordcraft_chat::Hub::new("k".into()));
        let (_, code) = hub.invite("@ghost", wordcraft_chat::hub::now_ms()).unwrap_or_default();
        assert!(hub.join(&code, wordcraft_chat::hub::now_ms()).is_ok());
        let v = done(crate::control::handle_member(&mut a, "@ghost", &member_req("engine.execute", json!({"command": "text.insert", "params": {"text": "x"}}))));
        assert_eq!(v["ok"], true, "{v}");
        hub.remove("@ghost");
        let v = done(crate::control::handle_member(&mut a, "@ghost", &member_req("text.insert", json!({"text": "y"}))));
        assert_eq!(v["ok"], false, "{v}");
    }

    #[test]
    fn member_edit_is_tracked_with_handle_and_owner_state_comes_back() {
        let mut a = app();
        let owner_sel = a.session.sel.clone();
        assert!(!a.session.doc.settings.track_changes);
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "12 meses"}));
        let r = run_as_member(&mut a, "@claude", "text.insert", json!({"text": "24 meses"}));
        assert!(r.is_ok());
        let changes = a.session.run("review.changes", &json!({})).unwrap_or_default();
        let list = changes.as_array().cloned().unwrap_or_default();
        assert!(!list.is_empty());
        assert!(list.iter().all(|x| x["author"] == "@claude"));
        assert!(list.iter().any(|x| x.to_string().contains("12 meses")), "{changes}");
        assert_eq!(a.session.author, "Owner");
        assert!(!a.session.doc.settings.track_changes);
        assert_eq!(a.session.sel, owner_sel);
    }

    #[test]
    fn member_has_its_own_selection() {
        let mut a = app();
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "12 meses"}));
        let _ = a.run("select.text", json!({"text": "Valor"}));
        let owner_sel = a.session.sel.clone();
        let r = run_as_member(&mut a, "@claude", "text.insert", json!({"text": "24 meses"}));
        assert!(r.is_ok());
        assert_eq!(a.session.sel, owner_sel);
        let changes = a.session.run("review.changes", &json!({})).unwrap_or_default();
        let list = changes.as_array().cloned().unwrap_or_default();
        assert!(list.iter().any(|c| c.to_string().contains("24 meses")), "{changes}");
        assert!(list.iter().all(|c| !c.to_string().contains("Valor")), "{changes}");
        let text = a.session.run("document.inspect", &json!({})).map(|v| v.to_string()).unwrap_or_default();
        assert!(text.contains("Valor"), "{text}");
    }

    #[test]
    fn accept_is_logged_in_chat() {
        let mut a = app();
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "12 meses"}));
        let _ = run_as_member(&mut a, "@claude", "text.insert", json!({"text": "24 meses"}));
        let r = run_as_member(&mut a, "@pi", "review.acceptAll", json!({}));
        assert!(r.is_ok());
        let msgs = a.chat.as_ref().map(|h| h.messages()).unwrap_or_default();
        assert!(msgs.iter().any(|m| m.text.contains("@pi") && m.text.contains("accepted")));
    }

    #[test]
    fn view_page_returns_png() {
        let mut a = app();
        let v = view_page(&mut a, 1, 0.5).unwrap_or_default();
        assert!(v["png_base64"].as_str().is_some_and(|s| s.len() > 100));
    }

    fn member_req(method: &str, params: serde_json::Value) -> crate::control::ControlRequest {
        let (r, _rx) = crate::control::ControlRequest::new(method, params);
        r.with_principal(wordcraft_chat::Principal::Member("@claude".into()))
    }

    fn done(o: crate::control::Outcome) -> serde_json::Value {
        match o {
            crate::control::Outcome::Done(v) => v,
            _ => serde_json::Value::Null,
        }
    }

    fn text_of(a: &mut WordApp) -> String {
        a.session.run("document.inspect", &json!({})).map(|v| v.to_string()).unwrap_or_default()
    }

    #[test]
    fn expired_request_is_not_executed() {
        let mut a = app();
        let ctx = egui::Context::default();
        let before = a.session.doc.clone();
        let past = std::time::Instant::now().checked_sub(std::time::Duration::from_secs(1)).unwrap_or_else(std::time::Instant::now);
        let (req, rx) = crate::control::ControlRequest::new("engine.execute", json!({"command": "text.insert", "params": {"text": "tarde"}}));
        a.answer_control(&ctx, req.with_deadline(past));
        assert_eq!(rx.try_recv().ok(), Some(json!({"ok": false, "error": "expired"})));
        assert_eq!(a.session.doc, before);
        let (req, rx) = crate::control::ControlRequest::new("engine.execute", json!({"command": "text.insert", "params": {"text": "a tempo"}}));
        a.answer_control(&ctx, req.with_deadline(std::time::Instant::now() + std::time::Duration::from_secs(60)));
        assert_eq!(rx.try_recv().ok().map(|v| v["ok"].clone()), Some(json!(true)));
        assert_ne!(a.session.doc, before);
    }

    #[test]
    fn refused_ids_are_not_executed_on_every_route() {
        let mut a = app();
        let before = text_of(&mut a);
        for (m, p) in [
            ("engine.execute", json!({"command": "edit.undo"})),
            ("engine.execute", json!({"id": "edit.repeat"})),
            ("command", json!({"command": "edit.redo"})),
            ("edit.undo", json!({})),
            ("review.restrict", json!({})),
            ("file.save", json!({})),
        ] {
            let v = done(crate::control::handle_member(&mut a, "@claude", &member_req(m, p)));
            assert_eq!(v["ok"], false, "{m}");
        }
        assert_eq!(text_of(&mut a), before);
    }

    #[test]
    fn allowed_routes_work_and_engine_commands_is_filtered() {
        let mut a = app();
        for (m, p) in [
            ("engine.execute", json!({"command": "select.text", "params": {"text": "12 meses"}})),
            ("engine.execute", json!({"id": "text.insert", "params": {"text": "24 meses"}})),
            ("command", json!({"command": "text.insert", "params": {"text": "!"}})),
        ] {
            let v = done(crate::control::handle_member(&mut a, "@claude", &member_req(m, p)));
            assert_eq!(v["ok"], true, "{m} {v}");
        }
        let v = done(crate::control::handle_member(&mut a, "@claude", &member_req("select.text", json!({"text": "Valor"}))));
        assert_eq!(v["ok"], true, "{v}");
        let v = done(crate::control::handle_member(&mut a, "@claude", &member_req("engine.commands", json!({}))));
        let list = v["result"].as_array().cloned().unwrap_or_default();
        assert!(!list.is_empty());
        assert!(list.iter().any(|c| c["id"] == "text.insert"));
        for c in &list {
            let id = c["id"].as_str().unwrap_or("");
            assert!(member_allowed("engine.execute", Some(id)), "{id}");
        }
        for bad in ["edit.undo", "edit.repeat", "file.save", "edit.paste"] {
            assert!(list.iter().all(|c| c["id"] != bad), "{bad}");
        }
    }

    #[test]
    fn owner_state_comes_back_when_member_command_fails() {
        let mut a = app();
        let _ = a.run("select.text", json!({"text": "Valor"}));
        let sel = a.session.sel.clone();
        let r = run_as_member(&mut a, "@claude", "text.insert", json!({}));
        assert!(r.is_err());
        assert_eq!(a.session.author, "Owner");
        assert!(!a.session.doc.settings.track_changes);
        assert_eq!(a.session.sel, sel);
    }

    #[test]
    fn owner_repeat_pending_and_recording_are_untouched() {
        let mut a = app();
        let _ = a.run("select.text", json!({"text": "Valor"}));
        a.session.last_command = Some(("format.bold".into(), json!({})));
        a.session.recording = Some(("m".into(), Vec::new()));
        let last = a.session.last_command.clone();
        let pending = a.session.pending.clone();
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "12 meses"}));
        let _ = run_as_member(&mut a, "@claude", "text.insert", json!({"text": "24 meses"}));
        assert_eq!(a.session.last_command, last);
        assert_eq!(a.session.pending, pending);
        assert!(a.session.recording.as_ref().is_some_and(|(_, steps)| steps.is_empty()));
        assert!(a.session.ui_requests.is_empty());
    }

    #[test]
    fn accept_log_has_the_count() {
        let mut a = app();
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "12 meses"}));
        let _ = run_as_member(&mut a, "@claude", "text.insert", json!({"text": "24 meses"}));
        let _ = run_as_member(&mut a, "@pi", "review.acceptAll", json!({}));
        let lines = system_lines(&a);
        // "24 meses" inserted and "12 meses" deleted: 16 characters.
        assert!(lines.contains(&"@pi accepted 16 characters from @claude".to_string()), "{lines:?}");
    }

    /// The owner's tracked insertion " prazo de 24 meses" after "Delta" (probe N10).
    fn owner_insertion() -> WordApp {
        let mut a = three();
        let _ = a.run("review.trackChanges", json!({"value": true}));
        let _ = a.run("select.text", json!({"text": "Delta"}));
        let _ = a.run("caret.end", json!({}));
        let _ = a.run("text.insert", json!({"text": " prazo de 24 meses"}));
        let _ = a.run("review.trackChanges", json!({"value": false}));
        let _ = a.run("caret.docStart", json!({}));
        a
    }

    #[test]
    fn partial_accept_or_reject_is_announced_with_characters() {
        for (id, word, line) in [
            ("review.reject", "24", "@claude rejected 2 characters from Owner"),
            ("review.reject", "prazo", "@claude rejected 5 characters from Owner"),
            ("review.accept", "24", "@claude accepted 2 characters from Owner"),
        ] {
            let mut a = owner_insertion();
            let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": word}));
            let before = a.session.doc.clone();
            let r = run_as_member(&mut a, "@claude", id, json!({}));
            assert!(r.is_ok(), "{id} {word}: {r:?}");
            assert_ne!(a.session.doc, before, "{id} {word}");
            let lines = system_lines(&a);
            assert!(lines.contains(&line.to_string()), "{id} {word}: {lines:?}");
        }
        // Nothing to accept at the member's caret and none after it: no change, no line.
        let mut a = three();
        let n = system_lines(&a).len();
        assert!(run_as_member(&mut a, "@claude", "review.accept", json!({})).is_ok());
        assert_eq!(system_lines(&a).len(), n);
    }

    /// An owner's comment "aceito esta clausula" on "Zeta" (probe N1); returns its part.
    fn owner_comment(a: &mut WordApp) -> u32 {
        let _ = a.run("select.text", json!({"text": "Zeta"}));
        let _ = a.run("review.newComment", json!({"text": "aceito esta clausula"}));
        let _ = a.run("caret.docStart", json!({}));
        a.session.doc.comments.values().next().map(|c| c.part).unwrap_or(u32::MAX)
    }

    fn in_part(part: u32, from: usize, to: usize) -> Value {
        let at = |off| wordcraft_engine::cmd::pos_json(&wordcraft_doc::Pos { story: wordcraft_doc::StoryRef::Part(part), path: wordcraft_doc::Path::top(0), off });
        json!({"anchor": at(from), "focus": at(to)})
    }

    #[test]
    fn members_cannot_edit_comments_of_others() {
        for (from, to, id, p) in [(0, 0, "text.insert", json!({"text": "NAO "})), (0, 7, "text.delete", json!({})), (0, 7, "text.insert", json!({"text": "recuso"}))] {
            let mut a = three();
            let part = owner_comment(&mut a);
            assert!(run_as_member(&mut a, "@claude", "select.range", in_part(part, from, to)).is_ok());
            let before = a.session.doc.clone();
            let r = run_as_member(&mut a, "@claude", id, p.clone());
            assert!(r.as_ref().is_err_and(|e| e.starts_with("untracked change refused") && e.contains("comment")), "{id} {p}: {r:?}");
            assert_eq!(a.session.doc, before, "{id}");
        }
        // Its own comment and replies stay editable.
        let mut a = three();
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "Alfa"}));
        assert!(run_as_member(&mut a, "@claude", "review.newComment", json!({"text": "meu comentário"})).is_ok());
        let (id, part) = a.session.doc.comments.iter().find(|(_, c)| c.author == "@claude").map(|(k, c)| (*k, c.part)).unwrap_or((u32::MAX, u32::MAX));
        assert!(run_as_member(&mut a, "@claude", "select.range", in_part(part, 0, 0)).is_ok());
        assert!(run_as_member(&mut a, "@claude", "text.insert", json!({"text": "Nota: "})).is_ok());
        assert!(run_as_member(&mut a, "@claude", "review.reply", json!({"id": id, "text": "e mais"})).is_ok());
        let other = owner_comment(&mut a);
        let owner_id = a.session.doc.comments.iter().find(|(_, c)| c.part == other).map(|(k, _)| *k).unwrap_or(u32::MAX);
        assert!(run_as_member(&mut a, "@claude", "review.reply", json!({"id": owner_id, "text": "resposta ao dono"})).is_ok());
    }

    #[test]
    fn list_label_formatting_after_a_member_enter_is_checked() {
        // Probe N9: the owner's lettered list; clause 2's text in caps, its mark (which formats
        // the list label) not.
        let body = |i: usize, off: usize| wordcraft_engine::cmd::pos_json(&wordcraft_doc::Pos::body(i, off));
        for enter in [false, true] {
            let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Default::default());
            a.session.author = "Owner".into();
            let _ = a.run("document.setText", json!({"text": "Clausula um.\nclausula dois.\nClausula tres."}));
            let _ = a.run("select.all", json!({}));
            let _ = a.run("para.numbering", json!({"kind": "lowerLetter"}));
            for (x, y) in [(0, 13), (13, 14)] {
                let (p, q) = (body(1, x), body(1, y));
                let _ = a.run("select.range", json!({"anchor": p, "focus": q}));
                let _ = a.run("format.set", json!({"props": {"caps": true}}));
            }
            a.chat = Some(wordcraft_chat::Hub::new("k".into()));
            let blk = if enter {
                let (p, q) = (body(1, 0), body(1, 0));
                let _ = run_as_member(&mut a, "@claude", "select.range", json!({"anchor": p, "focus": q}));
                assert!(run_as_member(&mut a, "@claude", "text.newParagraph", json!({})).is_ok());
                2
            } else {
                1
            };
            let mark_caps = |a: &WordApp| a.session.doc.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(blk)).and_then(|p| p.mark.caps);
            assert_eq!(mark_caps(&a), None, "enter={enter}");
            let (p, q) = (body(blk, 0), body(blk, 14));
            let _ = run_as_member(&mut a, "@claude", "select.range", json!({"anchor": p, "focus": q}));
            let before = a.session.doc.clone();
            let lines = system_lines(&a);
            let r = run_as_member(&mut a, "@claude", "format.set", json!({"props": {"caps": true}}));
            assert!(r.as_ref().is_err_and(|e| e.starts_with("untracked change refused")), "enter={enter}: {r:?}");
            assert_eq!(a.session.doc, before, "enter={enter}");
            assert_eq!(system_lines(&a), lines, "enter={enter}");
            // Allowed formatting there is announced.
            let r = run_as_member(&mut a, "@claude", "format.set", json!({"props": {"bold": true}}));
            assert!(r.is_ok(), "enter={enter}: {r:?}");
            assert!(system_lines(&a).contains(&"@claude formatted: format.set".to_string()), "enter={enter}: {:?}", system_lines(&a));
        }
    }

    #[test]
    fn a_member_empty_paragraph_does_not_set_the_expected_mark() {
        // Probe N14: the owner's list item "delta epsilon fim." with "epsilon fim." in caps (or
        // small caps) but not its mark. The member presses Enter twice after "delta ", then one
        // format.set from its empty paragraph through the end of the owner's second half.
        let body = |i: usize, off: usize| wordcraft_engine::cmd::pos_json(&wordcraft_doc::Pos::body(i, off));
        for prop in [json!({"caps": true}), json!({"smallCaps": true})] {
            let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Default::default());
            a.session.author = "Owner".into();
            let _ = a.run("document.setText", json!({"text": "Clausula um.\ndelta epsilon fim.\nClausula tres."}));
            let _ = a.run("select.all", json!({}));
            let _ = a.run("para.numbering", json!({"kind": "lowerLetter"}));
            for (x, y) in [(6, 17), (17, 18)] {
                let (p, q) = (body(1, x), body(1, y));
                let _ = a.run("select.range", json!({"anchor": p, "focus": q}));
                let _ = a.run("format.set", json!({"props": prop.clone()}));
            }
            a.chat = Some(wordcraft_chat::Hub::new("k".into()));
            let (p, q) = (body(1, 6), body(1, 6));
            let _ = run_as_member(&mut a, "@claude", "select.range", json!({"anchor": p, "focus": q}));
            assert!(run_as_member(&mut a, "@claude", "text.newParagraph", json!({})).is_ok());
            assert!(run_as_member(&mut a, "@claude", "text.newParagraph", json!({})).is_ok());
            let len = a.session.doc.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(3)).map(|p| p.len()).unwrap_or(0);
            let (p, q) = (body(2, 0), body(3, len));
            assert!(run_as_member(&mut a, "@claude", "select.range", json!({"anchor": p, "focus": q})).is_ok());
            let before = a.session.doc.clone();
            let lines = system_lines(&a);
            let r = run_as_member(&mut a, "@claude", "format.set", json!({"props": prop.clone()}));
            assert!(r.as_ref().is_err_and(|e| e.starts_with("untracked change refused")), "{prop}: {r:?}");
            assert_eq!(a.session.doc, before, "{prop}");
            assert_eq!(system_lines(&a), lines, "{prop}");
        }
        // No false refusal: a double Enter inside formatted text, then typing.
        let mut a = three();
        let _ = a.run("select.text", json!({"text": "Delta epsilon."}));
        let _ = a.run("format.set", json!({"props": {"caps": true, "lang": "pt-PT"}}));
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "Delta ep"}));
        let _ = run_as_member(&mut a, "@claude", "select.collapse", json!({"end": true}));
        assert!(run_as_member(&mut a, "@claude", "text.newParagraph", json!({})).is_ok());
        assert!(run_as_member(&mut a, "@claude", "text.newParagraph", json!({})).is_ok());
        assert!(run_as_member(&mut a, "@claude", "text.insert", json!({"text": "meio"})).is_ok());
        assert!(!system_lines(&a).iter().any(|l| l.contains("formatted")), "{:?}", system_lines(&a));
    }

    #[test]
    fn member_text_before_the_split_does_not_set_the_expected_mark() {
        // Probe N15: Enter after "delta ", the member types "x" there, then one format.set from
        // its "x" through the end of the owner's half (runs already caps, mark not).
        let body = |i: usize, off: usize| wordcraft_engine::cmd::pos_json(&wordcraft_doc::Pos::body(i, off));
        for prop in [json!({"caps": true}), json!({"smallCaps": true})] {
            let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Default::default());
            a.session.author = "Owner".into();
            let _ = a.run("document.setText", json!({"text": "Clausula um.\ndelta epsilon fim.\nClausula tres."}));
            let _ = a.run("select.all", json!({}));
            let _ = a.run("para.numbering", json!({"kind": "lowerLetter"}));
            for (x, y) in [(6, 17), (17, 18)] {
                let (p, q) = (body(1, x), body(1, y));
                let _ = a.run("select.range", json!({"anchor": p, "focus": q}));
                let _ = a.run("format.set", json!({"props": prop.clone()}));
            }
            a.chat = Some(wordcraft_chat::Hub::new("k".into()));
            let (p, q) = (body(1, 6), body(1, 6));
            let _ = run_as_member(&mut a, "@claude", "select.range", json!({"anchor": p, "focus": q}));
            assert!(run_as_member(&mut a, "@claude", "text.newParagraph", json!({})).is_ok());
            let (p, q) = (body(1, 6), body(1, 6));
            let _ = run_as_member(&mut a, "@claude", "select.range", json!({"anchor": p, "focus": q}));
            assert!(run_as_member(&mut a, "@claude", "text.insert", json!({"text": "x"})).is_ok());
            let alen = a.session.doc.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(1)).map(|p| p.len()).unwrap_or(0);
            let blen = a.session.doc.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(2)).map(|p| p.len()).unwrap_or(0);
            let (p, q) = (body(1, alen - 1), body(2, blen));
            assert!(run_as_member(&mut a, "@claude", "select.range", json!({"anchor": p, "focus": q})).is_ok());
            let before = a.session.doc.clone();
            let lines = system_lines(&a);
            let r = run_as_member(&mut a, "@claude", "format.set", json!({"props": prop.clone()}));
            assert!(r.as_ref().is_err_and(|e| e.starts_with("untracked change refused")), "{prop}: {r:?}");
            assert_eq!(a.session.doc, before, "{prop}");
            assert_eq!(system_lines(&a), lines, "{prop}");
        }
        // No false refusal: the member types inside caps text, presses Enter twice, types again.
        let mut a = three();
        let _ = a.run("select.text", json!({"text": "Delta epsilon."}));
        let _ = a.run("format.set", json!({"props": {"caps": true, "lang": "pt-PT"}}));
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "Delta ep"}));
        let _ = run_as_member(&mut a, "@claude", "select.collapse", json!({"end": true}));
        assert!(run_as_member(&mut a, "@claude", "text.insert", json!({"text": "zz"})).is_ok());
        assert!(run_as_member(&mut a, "@claude", "text.newParagraph", json!({})).is_ok());
        assert!(run_as_member(&mut a, "@claude", "text.newParagraph", json!({})).is_ok());
        assert!(run_as_member(&mut a, "@claude", "text.insert", json!({"text": "meio"})).is_ok());
        assert!(!system_lines(&a).iter().any(|l| l.contains("formatted")), "{:?}", system_lines(&a));
    }

    #[test]
    fn enter_in_the_middle_of_formatted_owner_text_stays_clean() {
        // The engine gives the new paragraph's mark the formatting at the split point.
        let mut a = three();
        let _ = a.run("select.text", json!({"text": "Delta epsilon."}));
        let _ = a.run("format.set", json!({"props": {"caps": true, "lang": "pt-PT", "bold": true}}));
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "Delta "}));
        let _ = run_as_member(&mut a, "@claude", "select.collapse", json!({"end": true}));
        let r = run_as_member(&mut a, "@claude", "text.newParagraph", json!({}));
        assert!(r.is_ok(), "{r:?}");
        assert!(run_as_member(&mut a, "@claude", "text.insert", json!({"text": "novo "})).is_ok());
        assert!(!system_lines(&a).iter().any(|l| l.contains("formatted")), "{:?}", system_lines(&a));
    }

    #[test]
    fn accept_and_reject_count_every_story_and_name_the_authors() {
        let mut a = app();
        // The owner's tracked edits: a word in the body, a new paragraph, text in a header.
        let _ = a.run("review.trackChanges", json!({"value": true}));
        let _ = a.run("select.text", json!({"text": "Valor"}));
        let _ = a.run("text.insert", json!({"text": "Montante"}));
        let _ = a.run("caret.docEnd", json!({}));
        let _ = a.run("text.newParagraph", json!({}));
        let _ = a.run("insert.editHeader", json!({}));
        let _ = a.run("text.insert", json!({"text": "Cabeçalho"}));
        let _ = a.run("insert.closeHeader", json!({}));
        let _ = a.run("review.trackChanges", json!({"value": false}));
        let r = run_as_member(&mut a, "@claude", "review.acceptAll", json!({}));
        assert!(r.is_ok(), "{r:?}");
        // insert "Montante", delete "Valor", the paragraph mark, the header text (and the
        // header's own paragraph mark when the engine tracks it).
        let lines = system_lines(&a);
        let line = lines.iter().find(|l| l.starts_with("@claude accepted")).cloned().unwrap_or_default();
        assert!(line.ends_with("characters from Owner"), "{lines:?}");
        // "Montante" 8 + "Valor" 5 + the paragraph mark 1 + "Cabeçalho" 9 (+ the header's mark).
        let n: usize = line.split_whitespace().nth(2).and_then(|x| x.parse().ok()).unwrap_or(0);
        assert!(n >= 23, "{line}");
    }

    #[test]
    fn announcements_follow_the_chat_language() {
        let run = |lang: wordcraft_chat::Lang| {
            let mut a = owner_insertion();
            a.chat = Some(wordcraft_chat::Hub::with_lang("k".into(), lang));
            let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "24"}));
            let _ = run_as_member(&mut a, "@claude", "review.reject", json!({}));
            let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "Alfa"}));
            let _ = run_as_member(&mut a, "@claude", "format.bold", json!({}));
            let _ = a.run("select.text", json!({"text": "Zeta"}));
            let _ = a.run("review.newComment", json!({"text": "nota"}));
            let id = a.session.doc.comments.keys().next().copied().unwrap_or(0);
            let _ = run_as_member(&mut a, "@claude", "review.resolveComment", json!({"id": id, "value": true}));
            let _ = run_as_member(&mut a, "@claude", "caret.docEnd", json!({}));
            let _ = run_as_member(&mut a, "@claude", "text.newParagraph", json!({}));
            let err = run_as_member(&mut a, "@claude", "review.rejectAll", json!({})).err().unwrap_or_default();
            let _ = a.run("caret.docStart", json!({}));
            let note = run_as_member(&mut a, "@claude", "select.owner", json!({})).unwrap_or_default()["note"].clone();
            (system_lines(&a), err, note)
        };
        let (lines, err, note) = run(wordcraft_chat::Lang::En);
        assert_eq!(lines, ["@claude rejected 2 characters from Owner", "@claude formatted: format.bold", "@claude resolved the comment by Owner"]);
        assert_eq!(err, "rejecting new paragraphs: ask the OWNER");
        assert!(note.as_str().is_some_and(|n| n.starts_with("the OWNER has no text selected")), "{note}");
        let (lines, err, note) = run(wordcraft_chat::Lang::Pt);
        assert_eq!(lines, ["@claude rejeitou 2 caracteres de Owner", "@claude formatou: format.bold", "@claude resolveu o comentário de Owner"]);
        assert_eq!(err, "rejeitar parágrafos novos: pede ao DONO");
        assert!(note.as_str().is_some_and(|n| n.starts_with("o DONO não tem texto selecionado")), "{note}");
    }

    #[test]
    fn member_resolving_a_comment_is_announced() {
        let mut a = app();
        let _ = a.run("select.text", json!({"text": "Valor"}));
        let _ = a.run("review.newComment", json!({"text": "do dono"}));
        let id = a.session.doc.comments.keys().next().copied().unwrap_or(0);
        assert!(run_as_member(&mut a, "@claude", "review.resolveComment", json!({"id": id, "value": true})).is_ok());
        assert!(system_lines(&a).contains(&"@claude resolved the comment by Owner".to_string()), "{:?}", system_lines(&a));
        // Resolving it again changes nothing: no line.
        let n = system_lines(&a).len();
        assert!(run_as_member(&mut a, "@claude", "review.resolveComment", json!({"id": id, "value": true})).is_ok());
        assert_eq!(system_lines(&a).len(), n);
    }

    #[test]
    fn stale_selection_does_not_block_accept_all_or_reject_all() {
        let mut a = app();
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "12 meses"}));
        let _ = a.run("review.trackChanges", json!({"value": true}));
        let _ = a.run("select.text", json!({"text": "12 meses"}));
        let _ = a.run("text.insert", json!({"text": "6 meses"}));
        let _ = a.run("review.trackChanges", json!({"value": false}));
        let r = run_as_member(&mut a, "@claude", "review.acceptAll", json!({}));
        assert!(r.is_ok(), "{r:?}");
    }

    #[test]
    fn member_cannot_reject_new_paragraphs_and_gets_a_clear_error() {
        // Probe E6: the engine does not join paragraphs on reject.
        let mut a = app();
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "12 meses."}));
        let _ = run_as_member(&mut a, "@claude", "select.collapse", json!({"end": true}));
        assert!(run_as_member(&mut a, "@claude", "text.newParagraph", json!({})).is_ok());
        assert!(run_as_member(&mut a, "@claude", "text.insert", json!({"text": "Nova cláusula do agente."})).is_ok());
        let doc = a.session.doc.clone();
        for id in ["review.rejectAll", "review.reject"] {
            let _ = run_as_member(&mut a, "@claude", "select.all", json!({}));
            let r = run_as_member(&mut a, "@claude", id, json!({}));
            assert_eq!(r, Err("rejecting new paragraphs: ask the OWNER".to_string()), "{id}");
            assert_eq!(a.session.doc, doc, "{id}");
        }
        // Rejecting a plain tracked word is fine (in a paragraph whose mark is not new).
        let mut a = app();
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "Valor"}));
        assert!(run_as_member(&mut a, "@claude", "text.insert", json!({"text": "Preço"})).is_ok());
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "Preço"}));
        assert!(run_as_member(&mut a, "@claude", "review.reject", json!({})).is_ok());
    }

    #[test]
    fn view_page_errors_out_of_range_and_is_png() {
        let mut a = app();
        assert!(view_page(&mut a, 99, 1.0).is_err());
        let v = view_page(&mut a, 1, 0.5).unwrap_or_default();
        assert!(v["png_base64"].as_str().is_some_and(|s| s.starts_with("iVBORw0KGgo")));
    }

    #[test]
    fn member_edit_never_merges_with_owner_typing() {
        let mut a = app();
        let _ = a.run("select.text", json!({"text": "Valor"}));
        let _ = a.run("text.insert", json!({"text": "AAA"}));
        let _ = run_as_member(&mut a, "@claude", "text.insert", json!({"text": "MMM"}));
        let _ = a.run("text.insert", json!({"text": "BBB"}));
        let _ = a.run("edit.undo", json!({}));
        let t = text_of(&mut a);
        assert!(t.contains("AAA") && t.contains("MMM") && !t.contains("BBB"), "after 1st undo: {t}");
        let _ = a.run("edit.undo", json!({}));
        let t = text_of(&mut a);
        assert!(t.contains("AAA") && !t.contains("MMM"), "after 2nd undo: {t}");
    }

    #[test]
    fn selections_reset_when_document_is_replaced() {
        let mut a = app();
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "12 meses"}));
        let long = "x".repeat(200);
        let mut d = wordcraft_doc::Document::new();
        d.ensure_nonempty();
        a.session.set_document(d);
        let _ = a.run("text.insert", json!({"text": long}));
        let _ = a.run("caret.docStart", json!({}));
        let _ = run_as_member(&mut a, "@claude", "text.insert", json!({"text": "ZZ"}));
        let t = text_of(&mut a);
        assert!(t.find("ZZ") < t.find("xxxx"), "{t}");
    }

    #[test]
    fn owner_view_is_restored() {
        let mut a = app();
        a.session.view.show_markup = true;
        a.session.view.comments_pane = false;
        a.session.view.nav_pane = false;
        let _ = run_as_member(&mut a, "@claude", "review.markup", json!({"value": "noMarkup"}));
        let _ = run_as_member(&mut a, "@claude", "review.showMarkup", json!({"value": false}));
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "Valor"}));
        let _ = run_as_member(&mut a, "@claude", "review.newComment", json!({"text": "oi"}));
        let _ = run_as_member(&mut a, "@claude", "edit.find", json!({"text": "Valor"}));
        assert!(a.session.view.show_markup);
        assert!(!a.session.view.comments_pane);
        assert!(!a.session.view.nav_pane);
    }

    #[test]
    fn member_delete_comment_only_own() {
        let mut a = app();
        let _ = a.run("select.text", json!({"text": "Valor"}));
        let _ = a.run("review.newComment", json!({"text": "do dono"}));
        let owner_id = a.session.doc.comments.iter().find(|(_, c)| c.author == "Owner").map(|(k, _)| *k).unwrap_or(u32::MAX);
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "Prazo"}));
        let _ = run_as_member(&mut a, "@claude", "review.newComment", json!({"text": "meu"}));
        let own_id = a.session.doc.comments.iter().find(|(_, c)| c.author == "@claude").map(|(k, _)| *k).unwrap_or(u32::MAX);
        assert_ne!(own_id, u32::MAX);
        assert!(run_as_member(&mut a, "@claude", "review.deleteComment", json!({"all": true})).is_err());
        assert!(run_as_member(&mut a, "@claude", "review.deleteComment", json!({})).is_err());
        assert!(run_as_member(&mut a, "@claude", "review.deleteComment", json!({"id": owner_id})).is_err());
        assert!(a.session.doc.comments.contains_key(&owner_id));
        assert!(run_as_member(&mut a, "@claude", "review.deleteComment", json!({"id": own_id})).is_ok());
        assert!(!a.session.doc.comments.contains_key(&own_id));
        assert!(a.session.doc.comments.contains_key(&owner_id));
    }

    #[test]
    fn owner_pending_format_is_not_used_by_member_and_comes_back() {
        let mut a = app();
        let _ = a.run("select.text", json!({"text": "euros."}));
        let _ = a.run("caret.right", json!({}));
        let _ = a.run("format.bold", json!({"value": true}));
        assert!(a.session.pending.is_some());
        let pending = a.session.pending.clone();
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "12 meses"}));
        let _ = run_as_member(&mut a, "@claude", "text.insert", json!({"text": "24 meses"}));
        let _ = run_as_member(&mut a, "@claude", "select.text", json!({"text": "24 meses"}));
        let st = run_as_member(&mut a, "@claude", "format.state", json!({})).unwrap_or_default();
        assert_ne!(st["bold"], true, "{st}");
        assert_eq!(a.session.pending, pending);
    }

    /// Reviewed list of every command a member may run. A new upstream command in an allowed
    /// group fails this test until someone reviews it (no file, clipboard, process or window command).
    const ALLOWED: &[&str] = &[
        "caret.docEnd",
        "caret.docStart",
        "caret.down",
        "caret.end",
        "caret.home",
        "caret.left",
        "caret.pageDown",
        "caret.pageUp",
        "caret.paraDown",
        "caret.paraUp",
        "caret.right",
        "caret.set",
        "caret.up",
        "caret.wordLeft",
        "caret.wordRight",
        "design.effects",
        "design.pageBorders",
        "design.pageColor",
        "design.paragraphSpacing",
        "design.setDefault",
        "design.styleSet",
        "design.theme",
        "design.themeColors",
        "design.themeFonts",
        "design.themes",
        "design.watermark",
        "document.inspect",
        "document.paragraph",
        "edit.find",
        "edit.findNext",
        "edit.findPrevious",
        "edit.formatPainter",
        "edit.goto",
        "format.allCaps",
        "format.bold",
        "format.charStyle",
        "format.clear",
        "format.color",
        "format.doubleStrikethrough",
        "format.doubleUnderline",
        "format.emboss",
        "format.engrave",
        "format.font",
        "format.fontDialog",
        "format.growFont",
        "format.growFont1",
        "format.highlight",
        "format.italic",
        "format.outline",
        "format.position",
        "format.scale",
        "format.set",
        "format.shading",
        "format.shadow",
        "format.shrinkFont",
        "format.shrinkFont1",
        "format.size",
        "format.smallCaps",
        "format.spacing",
        "format.state",
        "format.strikethrough",
        "format.subscript",
        "format.superscript",
        "format.underline",
        "format.wordUnderline",
        "hf.next",
        "hf.position",
        "hf.previous",
        "insert.blankPage",
        "insert.bookmark",
        "insert.closeHeader",
        "insert.crossReference",
        "insert.dateTime",
        "insert.docProperty",
        "insert.dropCap",
        "insert.editFooter",
        "insert.editHeader",
        "insert.equation",
        "insert.field",
        "insert.horizontalLine",
        "insert.link",
        "insert.pageBreak",
        "insert.pageNumber",
        "insert.removeLink",
        "insert.shape",
        "insert.signatureLine",
        "insert.spreadsheet",
        "insert.symbol",
        "insert.table",
        "insert.textBox",
        "layout.break",
        "layout.columns",
        "layout.differentFirstPage",
        "layout.differentOddEven",
        "layout.hyphenation",
        "layout.lineNumbers",
        "layout.margins",
        "layout.orientation",
        "layout.pageNumberFormat",
        "layout.pageSetup",
        "layout.section",
        "layout.size",
        "layout.verticalAlign",
        "para.addSpaceBefore",
        "para.align",
        "para.alignCenter",
        "para.alignLeft",
        "para.alignRight",
        "para.borders",
        "para.bullets",
        "para.dialog",
        "para.distribute",
        "para.double",
        "para.hangingIndent",
        "para.heading1",
        "para.heading2",
        "para.heading3",
        "para.indent",
        "para.indents",
        "para.justify",
        "para.keepLines",
        "para.keepNext",
        "para.lineSpacing",
        "para.listLevel",
        "para.multilevel",
        "para.normal",
        "para.numbering",
        "para.oneAndHalf",
        "para.outdent",
        "para.outlineLevel",
        "para.pageBreakBefore",
        "para.removeHanging",
        "para.removeSpaceAfter",
        "para.rtl",
        "para.set",
        "para.shading",
        "para.single",
        "para.spacing",
        "para.style",
        "para.tabs",
        "para.widowControl",
        "references.addText",
        "references.bibliography",
        "references.citation",
        "references.endnote",
        "references.footnote",
        "references.index",
        "references.markCitation",
        "references.markEntry",
        "references.nextFootnote",
        "references.noteOptions",
        "references.notes",
        "references.researcher",
        "references.sources",
        "references.tableOfAuthorities",
        "references.tableOfFigures",
        "references.toc",
        "references.updateFields",
        "references.updateFigures",
        "references.updateIndex",
        "references.updateToc",
        "review.accept",
        "review.acceptAll",
        "review.applySuggestion",
        "review.changes",
        "review.comments",
        "review.deleteComment",
        "review.editor",
        "review.issues",
        "review.language",
        "review.markup",
        "review.newComment",
        "review.nextChange",
        "review.nextComment",
        "review.previousChange",
        "review.previousComment",
        "review.proofing",
        "review.reject",
        "review.rejectAll",
        "review.reply",
        "review.resolveComment",
        "review.showMarkup",
        "review.spelling",
        "review.suggestions",
        "review.thesaurus",
        "review.wordCount",
        "select.all",
        "select.collapse",
        "select.extend",
        "select.line",
        "select.objects",
        "select.owner",
        "select.paragraph",
        "select.range",
        "select.sentence",
        "select.similar",
        "select.text",
        "select.word",
        "styles.addToGallery",
        "styles.create",
        "styles.list",
        "styles.modify",
        "styles.pane",
        "styles.updateToMatch",
        "table.autofit",
        "table.borderPainter",
        "table.borders",
        "table.cellAlign",
        "table.cellMargins",
        "table.columnWidth",
        "table.distributeColumns",
        "table.distributeRows",
        "table.look",
        "table.properties",
        "table.quick",
        "table.repeatHeader",
        "table.rowHeight",
        "table.selectCell",
        "table.selectRow",
        "table.selectTable",
        "table.shading",
        "table.splitTable",
        "table.style",
        "table.textDirection",
        "text.backTab",
        "text.backspace",
        "text.columnBreak",
        "text.delete",
        "text.deleteWordBack",
        "text.deleteWordForward",
        "text.insert",
        "text.lineBreak",
        "text.nbHyphen",
        "text.nbsp",
        "text.newParagraph",
        "text.optionalHyphen",
        "text.pageBreak",
        "text.tab",
    ];

    #[test]
    fn allowed_command_snapshot() {
        let a = app();
        let mut ids: Vec<&str> = a.session.registry.all().iter().map(|c| c.id).filter(|id| member_allowed("engine.execute", Some(id))).collect();
        ids.extend(GATE_COMMANDS.iter().map(|c| c.0).filter(|id| member_allowed("engine.execute", Some(id))));
        ids.sort_unstable();
        if ids != ALLOWED {
            panic!("SNAPSHOT:\n{}", ids.iter().map(|i| format!("    \"{i}\",")).collect::<Vec<_>>().join("\n"));
        }
    }
}
