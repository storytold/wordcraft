//! Tools and the long tail: Repeat, macros, AutoCorrect, Compare, accessibility checker,
//! document inspector, restrict editing, versions, templates, Quick Parts, and more.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::{CharProps, NumRef};

use wordcraft_doc::{Block, Document, Paragraph, Pos, RevisionKind, StoryRef, para_block};

use super::{delete_selection, sel_result};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

/// Smileys replaced even when typed straight after a word ("ok:)"), as Word does.
const SMILEYS: &[&str] = &[":-)", ":-(", ":)", ":(", ":|"];

/// Built-in AutoCorrect replacements (our own list of common typos and symbols).
pub const AUTOCORRECT: &[(&str, &str)] = &[
    ("teh", "the"),
    ("adn", "and"),
    ("recieve", "receive"),
    ("recieved", "received"),
    ("seperate", "separate"),
    ("definately", "definitely"),
    ("occured", "occurred"),
    ("untill", "until"),
    ("wich", "which"),
    ("becuase", "because"),
    ("thier", "their"),
    ("alot", "a lot"),
    ("accomodate", "accommodate"),
    ("acheive", "achieve"),
    ("beleive", "believe"),
    ("calender", "calendar"),
    ("goverment", "government"),
    ("tommorow", "tomorrow"),
    ("wierd", "weird"),
    ("i", "I"),
    ("dont", "don't"),
    ("cant", "can't"),
    ("wont", "won't"),
    ("didnt", "didn't"),
    ("doesnt", "doesn't"),
    ("isnt", "isn't"),
    ("(c)", "©"),
    ("(r)", "®"),
    ("(tm)", "™"),
    ("...", "…"),
    ("->", "→"),
    ("<-", "←"),
    ("=>", "⇒"),
    (":)", "☺"),
    (":-)", "☺"),
    (":(", "☹"),
    (":-(", "☹"),
    (":|", "\u{1F610}"),
    ("1/2", "½"),
    ("1/4", "¼"),
    ("3/4", "¾"),
];

/// A built-in entry that is an English word (letters and apostrophes only), not a symbol.
fn is_english_entry(from: &str) -> bool {
    from.chars().any(char::is_alphabetic) && from.chars().all(|c| c.is_alphabetic() || c == '\'')
}

/// Compare and Combine take the same settings.
const COMPARE_PARAMS: &str = r#"{"path"?: string (revised document), "text"?: string (revised text), "original"?: string (original document; default: this one), "author"?: string (label changes with), "caseChanges"?: bool, "whiteSpace"?: bool, "formatting"?: bool (default true: differences count), "level"?: "word|character", "showIn"?: "original|revised|new" (revised/new: an untitled document), "moves"|"comments"|"tables"|"headersFooters"|"footnotes"|"textBoxes"|"fields"?: bool (accepted; not honoured yet except tables, always compared)}"#;

/// AutoCorrect Options (Word keeps them per user; the front end saves them with its preferences).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AutoCorrectPrefs {
    /// All of AutoCorrect and AutoFormat As You Type.
    pub enabled: bool,
    /// Replace text as you type (the replacement list).
    pub replace_text: bool,
    /// Correct TWo INitial CApitals: "WOrd" → "Word" (not the exceptions, e.g. "IDs").
    pub two_initial_caps: bool,
    /// Capitalize the first letter of sentences.
    pub cap_sentences: bool,
    /// Capitalize the first letter of table cells.
    pub cap_cells: bool,
    /// Capitalize names of days.
    pub cap_days: bool,
    /// Correct accidental usage of cAPS LOCK: "wORD" → "Word".
    pub caps_lock: bool,
    /// AutoFormat As You Type: "straight quotes" with “smart quotes”.
    pub smart_quotes: bool,
    /// Fractions (1/2) with fraction characters (½).
    pub fractions: bool,
    /// Ordinals (1st) with superscript.
    pub ordinals: bool,
    /// Hyphens (--) with dashes.
    pub dashes: bool,
    /// Internet addresses with hyperlinks.
    pub links: bool,
    /// Automatic bulleted lists.
    pub bullets: bool,
    /// Automatic numbered lists.
    pub numbering: bool,
    /// Border lines.
    pub border_lines: bool,
    /// The user's entries: added, or replacing a built-in one.
    pub entries: Vec<(String, String)>,
    /// Built-in entries the user deleted.
    pub removed: Vec<String>,
    /// Abbreviations the user added after which a sentence doesn't end ("approx.").
    pub exceptions: Vec<String>,
    /// Built-in abbreviations the user deleted from the exceptions.
    pub exceptions_removed: Vec<String>,
    /// The user's words that keep two initial capitals (besides the built-in ones).
    pub initial_caps_exceptions: Vec<String>,
}

impl Default for AutoCorrectPrefs {
    fn default() -> Self {
        AutoCorrectPrefs {
            enabled: true,
            replace_text: true,
            two_initial_caps: true,
            cap_sentences: true,
            cap_cells: true,
            cap_days: true,
            caps_lock: true,
            smart_quotes: true,
            fractions: true,
            ordinals: true,
            dashes: true,
            links: true,
            bullets: true,
            numbering: true,
            border_lines: true,
            entries: Vec::new(),
            removed: Vec::new(),
            exceptions: Vec::new(),
            exceptions_removed: Vec::new(),
            initial_caps_exceptions: Vec::new(),
        }
    }
}

/// Longest AutoCorrect "replace" text, in characters.
const MAX_FROM: usize = 255;
/// Longest "with" text, in characters.
const MAX_TO: usize = 2000;
/// Most entries (and exceptions) kept.
const MAX_ENTRIES: usize = 10_000;

/// Built-in fractions: AutoFormat's (Fractions), not the replacement list's.
const FRACTIONS: [&str; 3] = ["1/2", "1/4", "3/4"];

impl AutoCorrectPrefs {
    /// Drop what a damaged or hostile saved value could hold: empty or overlong entries,
    /// duplicates, too many of them.
    pub fn cleaned(mut self) -> Self {
        let ok = |t: &str, max: usize| !t.trim().is_empty() && t.chars().count() <= max && !t.contains(['\n', '\r']);
        let mut seen = std::collections::HashSet::new();
        self.entries.retain(|(a, b)| ok(a, MAX_FROM) && b.chars().count() <= MAX_TO && !b.contains(['\n', '\r']) && seen.insert(a.clone()));
        self.entries.truncate(MAX_ENTRIES);
        for list in [&mut self.removed, &mut self.exceptions, &mut self.exceptions_removed, &mut self.initial_caps_exceptions] {
            let mut seen = std::collections::HashSet::new();
            list.retain(|a| ok(a, MAX_FROM) && seen.insert(a.clone()));
            list.truncate(MAX_ENTRIES);
        }
        self
    }

    /// The replacement for `word` in English text, if any: the user's entry, else a built-in
    /// one not deleted. Built-in lowercase entries also match a capitalised word ("Teh" → "The").
    pub fn replacement(&self, word: &str) -> Option<String> {
        self.replacement_in(word, true)
    }

    /// The replacement for `word` in text that is (`english`) or isn't English: the built-in
    /// English words ("teh", "i", "dont") apply only to English text, as Word keeps a list per
    /// language (#835: Polish "i" means "and"); symbols, fractions and the user's entries apply
    /// whatever the language.
    pub fn replacement_in(&self, word: &str, english: bool) -> Option<String> {
        if let Some((_, b)) = self.entries.iter().find(|(a, _)| a == word) {
            return Some(b.clone());
        }
        AUTOCORRECT
            .iter()
            .filter(|(a, _)| english || !is_english_entry(a))
            .filter(|(a, _)| !self.removed.iter().any(|r| r == a) && !self.entries.iter().any(|(e, _)| e == a))
            .filter(|(a, _)| self.fractions || !FRACTIONS.contains(a))
            .find(|(a, _)| {
                *a == word
                    || (a.chars().all(|c| c.is_lowercase()) && word.to_lowercase() == *a && word.chars().next().is_some_and(char::is_uppercase))
            })
            .map(|(a, b)| {
                if *a != word && b.chars().next().is_some_and(char::is_lowercase) {
                    let mut c = b.chars();
                    c.next().map(|x| x.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
                } else {
                    b.to_string()
                }
            })
    }

    /// The replacement list as the dialog shows it, sorted: `(from, to, built in)`.
    pub fn list(&self) -> Vec<(String, String, bool)> {
        let mut v: Vec<(String, String, bool)> = AUTOCORRECT
            .iter()
            .filter(|(a, _)| !self.removed.iter().any(|r| r == a) && !self.entries.iter().any(|(e, _)| e == a))
            .map(|(a, b)| (a.to_string(), b.to_string(), true))
            .chain(self.entries.iter().map(|(a, b)| (a.clone(), b.clone(), false)))
            .collect();
        v.sort_by(|x, y| x.0.to_lowercase().cmp(&y.0.to_lowercase()).then(x.0.cmp(&y.0)));
        v
    }

    /// Abbreviations after which a sentence doesn't end, lowercase.
    pub fn exception_list(&self) -> Vec<String> {
        let mut v: Vec<String> = NOT_SENTENCE_END
            .iter()
            .filter(|a| !self.exceptions_removed.iter().any(|r| r.eq_ignore_ascii_case(a)))
            .map(|a| a.to_string())
            .chain(self.exceptions.iter().map(|a| a.to_lowercase()))
            .collect();
        v.sort();
        v.dedup();
        v
    }
}

/// Saved AutoCorrect options: anything that doesn't read as them reads as the defaults (a
/// damaged `ui.json` never loses the other preferences).
pub fn lenient_autocorrect<'de, D: serde::Deserializer<'de>>(d: D) -> Result<AutoCorrectPrefs, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(serde_json::from_value::<AutoCorrectPrefs>(v).map(AutoCorrectPrefs::cleaned).unwrap_or_default())
}

/// `tools.autocorrect`: change AutoCorrect Options and report them.
fn autocorrect_options(s: &mut Session, v: &Value) -> CmdResult {
    let ac = &mut s.prefs.autocorrect;
    for (k, f) in [
        ("enabled", &mut ac.enabled as &mut bool),
        ("replaceText", &mut ac.replace_text),
        ("twoInitialCaps", &mut ac.two_initial_caps),
        ("capSentences", &mut ac.cap_sentences),
        ("capCells", &mut ac.cap_cells),
        ("capDays", &mut ac.cap_days),
        ("capsLock", &mut ac.caps_lock),
        ("smartQuotes", &mut ac.smart_quotes),
        ("fractions", &mut ac.fractions),
        ("ordinals", &mut ac.ordinals),
        ("dashes", &mut ac.dashes),
        ("links", &mut ac.links),
        ("bullets", &mut ac.bullets),
        ("numbering", &mut ac.numbering),
        ("borderLines", &mut ac.border_lines),
    ] {
        if let Some(b) = p::bool(v, k) {
            *f = b;
        }
    }
    let bad = |t: &str, max: usize| t.trim().is_empty() || t.chars().count() > max || t.contains(['\n', '\r']);
    if let Some(add) = v.get("add") {
        let (Some(f), Some(t)) = (add.get("from").and_then(Value::as_str), add.get("to").and_then(Value::as_str)) else {
            return Err(CmdError::Params("`add` is {\"from\": string, \"to\": string}".into()));
        };
        if bad(f, MAX_FROM) || t.chars().count() > MAX_TO || t.contains(['\n', '\r']) {
            return Err(CmdError::Params(format!("an entry replaces 1–{MAX_FROM} characters (one line) with at most {MAX_TO}")));
        }
        ac.entries.retain(|(a, _)| a != f);
        if ac.entries.len() >= MAX_ENTRIES {
            return Err(CmdError::Params("too many AutoCorrect entries".into()));
        }
        ac.entries.push((f.to_string(), t.to_string()));
        ac.removed.retain(|r| r != f);
    }
    if let Some(f) = p::str(v, "delete") {
        let n = ac.entries.len();
        ac.entries.retain(|(a, _)| a != f);
        if AUTOCORRECT.iter().any(|(a, _)| *a == f) && !ac.removed.iter().any(|r| r == f) {
            ac.removed.push(f.to_string());
        } else if n == ac.entries.len() {
            return Err(CmdError::Params(format!("no AutoCorrect entry {f:?}")));
        }
    }
    if let Some(e) = p::str(v, "addException") {
        let e = e.trim().to_lowercase();
        if bad(&e, MAX_FROM) || e.contains(char::is_whitespace) {
            return Err(CmdError::Params("an exception is one abbreviation, like \"approx.\"".into()));
        }
        ac.exceptions_removed.retain(|r| *r != e);
        if !NOT_SENTENCE_END.contains(&e.as_str()) && !ac.exceptions.contains(&e) && ac.exceptions.len() < MAX_ENTRIES {
            ac.exceptions.push(e);
        }
    }
    if let Some(e) = p::str(v, "deleteException") {
        let e = e.trim().to_lowercase();
        ac.exceptions.retain(|r| *r != e);
        if NOT_SENTENCE_END.contains(&e.as_str()) && !ac.exceptions_removed.contains(&e) {
            ac.exceptions_removed.push(e);
        }
    }
    if let Some(w) = p::str(v, "addInitialCapsException") {
        let w = w.trim();
        if bad(w, MAX_FROM) || w.contains(char::is_whitespace) {
            return Err(CmdError::Params("an INitial CAps exception is one word, like \"IDs\"".into()));
        }
        if !INITIAL_CAPS_EXCEPTIONS.contains(&w)
            && !ac.initial_caps_exceptions.iter().any(|e| e == w)
            && ac.initial_caps_exceptions.len() < MAX_ENTRIES
        {
            ac.initial_caps_exceptions.push(w.to_string());
        }
    }
    if let Some(w) = p::str(v, "deleteInitialCapsException") {
        let w = w.trim();
        ac.initial_caps_exceptions.retain(|e| e != w);
    }
    let ac = &s.prefs.autocorrect;
    Ok(json!({
        "enabled": ac.enabled,
        "replaceText": ac.replace_text,
        "twoInitialCaps": ac.two_initial_caps,
        "capSentences": ac.cap_sentences,
        "capCells": ac.cap_cells,
        "capDays": ac.cap_days,
        "capsLock": ac.caps_lock,
        "smartQuotes": ac.smart_quotes,
        "fractions": ac.fractions,
        "ordinals": ac.ordinals,
        "dashes": ac.dashes,
        "links": ac.links,
        "bullets": ac.bullets,
        "numbering": ac.numbering,
        "borderLines": ac.border_lines,
        "entries": ac.list().into_iter().map(|(a, b, builtin)| json!({"from": a, "to": b, "builtIn": builtin})).collect::<Vec<_>>(),
        "user": ac.entries,
        "exceptions": ac.exception_list(),
        "initialCapsExceptions": INITIAL_CAPS_EXCEPTIONS.iter().map(|e| e.to_string()).chain(ac.initial_caps_exceptions.iter().cloned()).collect::<Vec<_>>(),
    }))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("edit.repeat", "Repeat", "Quick Access Toolbar", |s, _| {
            let Some((id, params)) = s.last_command.clone() else { return Err(CmdError::Failed("nothing to repeat".into())) };
            s.run(&id, &params)
        })
        .key("F4")
        .pure(),
        CommandSpec::new("tools.recordMacro", "Record Macro", "View › Macros", |s, v| match s.recording.take() {
            Some((name, steps)) => {
                let n = steps.len();
                s.macros.insert(name.clone(), steps);
                Ok(json!({"recording": false, "saved": name, "steps": n}))
            }
            None => {
                let name = p::str(v, "name").unwrap_or("Macro1").to_string();
                s.recording = Some((name.clone(), Vec::new()));
                Ok(json!({"recording": true, "name": name}))
            }
        })
        .params(r#"{"name"?: string} (call again to stop)"#)
        .pure(),
        CommandSpec::new("tools.macros", "Macros", "View › Macros", macros)
            .key("Alt+F8")
            .params(r#"{"run"?: name, "define"?: {"name": string, "steps": [{"command", "params"}]}, "delete"?: name}"#)
            .pure(),
        CommandSpec::new("tools.autocorrect", "AutoCorrect Options", "File › Options › Proofing", autocorrect_options)
            .params(
                r#"{"enabled"?: bool (all of AutoCorrect), "replaceText"|"twoInitialCaps"|"capSentences"|"capCells"|"capDays"|"capsLock"|"smartQuotes"|"fractions"|"ordinals"|"dashes"|"links"|"bullets"|"numbering"|"borderLines"?: bool, "add"?: {"from": string, "to": string} (adds or replaces an entry), "delete"?: string (an entry's "from", built in or added), "addException"?: string, "deleteException"?: string, "addInitialCapsException"?: string (keeps TWo INitial CApitals, like "IDs"), "deleteInitialCapsException"?: string} → options, entries and exceptions"#,
            )
            .pure(),
        CommandSpec::new("tools.customizeKeyboard", "Customize Keyboard", "File › Options", customize_keyboard)
            .params(
                r#"{"command"?: id (its keys), "key"?: "Mod+Shift+K" (the command it runs), "assign"?: {"command": id, "key": string}, "remove"?: {"command": id, "key": string}, "reset"?: true (built-in keys only)} → {custom: {assigned: {key: id}, removed: [key]}, …}; no params opens the dialog"#,
            )
            .pure(),
        CommandSpec::new("tools.customizeRibbon", "Customize Ribbon", "File › Options", customize_ribbon)
            .params(
                r#"{"hide"?: tab, "show"?: tab, "newTab"?: {"name"?, "after"?: tab}, "newGroup"?: {"tab", "name"?}, "add"?: {"tab", "group", "command", "after"?: id}, "remove"?: {"tab", "group"?, "command"?}, "rename"?: {"tab", "group"?, "to"}, "move"?: {"tab", "group"?, "command"?, "by": int}, "qatAdd"?: id, "qatRemove"?: id, "qatMove"?: {"command", "by": int}, "reset"?: true | "ribbon" | "qat", "layout"?: object} → {tabs: [{name, custom?, hidden?, groups?: [{name, commands}]}], qat: [id]}; only custom tabs and groups can be renamed or removed (built-in tabs can be hidden and moved); no params opens the dialog"#,
            )
            .pure(),
        // Word's Search box in the title bar ("Tell me what you want to do"), Alt+Q (#819).
        CommandSpec::new("tools.search", "Search", "Title Bar", search_commands)
            .key(super::non_mac("Alt+Q"))
            .params(r#"{"query"?: string} → {matches: [{id, label, location, shortcut}]} (every word of the query in the label, location or id; at most 50); no query opens the Search box"#)
            .pure(),
        CommandSpec::new("review.compare", "Compare", "Review › Compare", compare).params(COMPARE_PARAMS),
        CommandSpec::new("review.combine", "Combine", "Review › Compare", compare).params(COMPARE_PARAMS),
        CommandSpec::new("file.accessibility", "Check Accessibility", "Review › Accessibility", accessibility).pure(),
        CommandSpec::new("file.inspect", "Inspect Document", "File › Info", inspect_doc)
            .params(r#"{"remove"?: ["comments", "revisions", "properties", "hidden", "headers"]} → {found: {comments, revisions, properties, hiddenText, hiddenCount, hidden: [{story: "body|header|footer|footnote|endnote|comment|textBox", part?, text}] (at most 1000), truncated, unsupported: [{story, object: "table|opaque", ..}] (hidden text not inspected: table styles, verbatim objects), headers}, removed, residual: [..] (hidden text a removal couldn't reach), removedNotes: [{kind, part, text}] (notes whose hidden reference was removed)}; whole hidden paragraphs go with their marks"#),
        CommandSpec::new("file.protect", "Protect Document", "File › Info", |s, v| {
            let mode = p::str(v, "mode").unwrap_or("readOnly");
            s.run("review.restrict", &json!({"mode": mode}))
        })
        .params(r#"{"mode": "none|readOnly|comments|trackedChanges"}"#),
        CommandSpec::new("file.versions", "Version History", "File › Info", |s, v| {
            if let Some(label) = p::str(v, "save") {
                s.versions.push((label.to_string(), super::now_iso(), s.doc.clone()));
                if s.versions.len() > 50 {
                    s.versions.remove(0);
                }
            }
            if let Some(i) = p::u64(v, "restore") {
                let (_, _, d) = s.versions.get(i as usize).cloned().ok_or_else(|| CmdError::Params("no such version".into()))?;
                s.checkpoint("Restore Version");
                s.doc = d;
                s.touch();
                s.clamp_selection();
            }
            Ok(Value::Array(s.versions.iter().enumerate().map(|(i, (l, d, _))| json!({"index": i, "label": l, "date": d})).collect()))
        })
        .params(r#"{"save"?: label, "restore"?: index}"#)
        .pure(),
        CommandSpec::new("file.recover", "Recover Unsaved Documents", "File › Info", |s, v| s.recover(v))
        .params(r#"{"restore"?: recoveryId, "discard"?: recoveryId, "refresh"?: true}; desktop only; returns {enabled,busy,documents,error}; restore loads asynchronously without a file path"#)
        .pure(),
        CommandSpec::new("file.newFromTemplate", "New from Template", "File › New", |s, v| {
            let path = p::req_str(v, "path")?;
            let doc = crate::io::open_path(std::path::Path::new(path)).map_err(CmdError::Failed)?;
            s.set_document(doc);
            s.path = None;
            s.dirty = true;
            sel_result(s)
        })
        .params(r#"{"path": string}"#)
        .pure(),
        CommandSpec::new("file.saveTemplate", "Save as Template", "File › Save As", |s, v| {
            let path = p::req_str(v, "path")?;
            let path = if path.ends_with(".dotx") { path.to_string() } else { format!("{path}.dotx") };
            crate::io::save_path(std::path::Path::new(&path), &s.doc).map_err(CmdError::Failed)?;
            Ok(json!({"path": path}))
        })
        .params(r#"{"path": string}"#)
        .pure(),
        CommandSpec::new("file.autosave", "AutoSave", "Quick Access Toolbar", |s, v| {
            s.autosave = p::bool(v, "value").unwrap_or(!s.autosave);
            Ok(json!({"value": s.autosave}))
        })
        .pure(),
        CommandSpec::new("file.compatibility", "Check Compatibility", "File › Info", |s, _| {
            let mut issues = Vec::new();
            if s.doc
                .para_paths(StoryRef::Body)
                .iter()
                .any(|p| s.doc.para(StoryRef::Body, p).is_some_and(|x| x.objects.iter().any(|o| matches!(o, InlineObject::Opaque { .. }))))
            {
                issues.push("Some embedded objects from the original file are shown as text only.");
            }
            Ok(json!({"issues": issues}))
        })
        .pure(),
        CommandSpec::new("file.share", "Share", "File", |s, _| {
            s.ui_requests.push(json!({"open": "saveAs"}));
            Ok(json!({"hint": "Save or export a copy (PDF, Word document) to share it."}))
        })
        .pure(),
        CommandSpec::new("insert.quickParts", "Quick Parts", "Insert › Text", quick_parts)
            .params(r#"{"save"?: name (from the selection), "insert"?: name, "delete"?: name} → list"#),
        CommandSpec::new("insert.autoText", "AutoText", "Insert › Text › Quick Parts", quick_parts)
            .key(super::non_mac("Alt+F3"))
            .params(r#"{"save"?: name, "insert"?: name}"#),
        CommandSpec::new("insert.docProperty", "Document Property", "Insert › Text › Quick Parts", |s, v| {
            let name = p::str(v, "name").unwrap_or("Title").to_string();
            let c = &s.doc.core;
            let val = match name.to_lowercase().as_str() {
                "title" => c.title.clone(),
                "author" | "creator" => c.creator.clone(),
                "subject" => c.subject.clone(),
                "keywords" => c.keywords.clone(),
                "comments" | "description" => c.description.clone(),
                "category" => c.category.clone(),
                _ => return Err(CmdError::Params("unknown property".into())),
            };
            let instr = match name.to_lowercase().as_str() {
                "author" | "creator" => "AUTHOR".to_string(),
                "title" => "TITLE".to_string(),
                other => format!("DOCPROPERTY {other}"),
            };
            let props = s.typing_props();
            let at = delete_selection(s)?;
            let end = s.doc.insert_object(&at, InlineObject::Field { instr, result: val, locked: false, dirty: false, code: false, form: String::new() }, &props)?;
            s.sel = Selection::caret(end);
            sel_result(s)
        })
        .params(r#"{"name": "Title|Author|Subject|Keywords|Comments|Category"}"#),
        CommandSpec::new("insert.signatureLine", "Signature Line", "Insert › Text", |s, v| {
            let signer = p::str(v, "signer").unwrap_or("").to_string();
            let title = p::str(v, "title").unwrap_or("").to_string();
            let at = delete_selection(s)?;
            let mut lines = vec!["X ______________________________".to_string()];
            if !signer.is_empty() {
                lines.push(signer);
            }
            if !title.is_empty() {
                lines.push(title);
            }
            let frag = wordcraft_doc::edit::Fragment {
                blocks: lines.iter().map(|l| Block::Para(Paragraph::with_text(l, CharProps::default()).styled("NoSpacing"))).collect(),
                ..Default::default()
            };
            let end = s.doc.insert_fragment(&at, &frag)?;
            s.sel = Selection::caret(end);
            sel_result(s)
        })
        .params(r#"{"signer"?: string, "title"?: string}"#),
        CommandSpec::new("insert.object", "Object", "Insert › Text", |s, v| {
            let path = p::req_str(v, "path")?;
            s.run("insert.textFromFile", &json!({"path": path}))
        })
        .params(r#"{"path": string}"#),
        CommandSpec::new("insert.spreadsheet", "Spreadsheet Table", "Insert › Tables", |s, v| {
            let csv = p::str(v, "csv").unwrap_or("A,B,C\n1,2,3\n4,5,6");
            let rows = super::mailings::parse_csv(csv);
            let cols = rows.iter().map(Vec::len).max().unwrap_or(1).clamp(1, 63);
            s.run("insert.table", &json!({"rows": rows.len().max(1), "cols": cols, "style": "GridTable1Light"}))?;
            let Some((tp, _, _)) = s.sel.focus.path.cell() else { return sel_result(s) };
            let t = s.doc.table_mut(s.sel.focus.story, &tp)?;
            for (r, row) in rows.iter().enumerate() {
                for (c, val) in row.iter().enumerate() {
                    if let Some(cell) = t.rows.get_mut(r).and_then(|x| x.cells.get_mut(c)) {
                        cell.blocks = vec![para_block(Paragraph::with_text(val, CharProps::default()))];
                    }
                }
            }
            sel_result(s)
        })
        .params(r#"{"csv"?: string}"#),
        CommandSpec::new("layout.linkToPrevious", "Link to Previous", "Header & Footer › Navigation", |s, v| {
            let header = p::bool(v, "footer").map(|f| !f).unwrap_or(true);
            let block = s.sel.focus.path.0.first().copied().unwrap_or(0) as usize;
            let sect = s.doc.section_mut(block);
            if header {
                sect.headers = Default::default();
            } else {
                sect.footers = Default::default();
            }
            Ok(json!({"linked": true}))
        })
        .key(super::non_mac("Alt+Shift+R"))
        .params(r#"{"footer"?: bool}"#),
        CommandSpec::new("hf.next", "Next Section", "Header & Footer › Navigation", |s, _| hf_nav(s, 1)).pure(),
        CommandSpec::new("hf.previous", "Previous Section", "Header & Footer › Navigation", |s, _| hf_nav(s, -1)).pure(),
        CommandSpec::new("hf.position", "Header/Footer Position", "Header & Footer › Position", |s, v| {
            let (h, f) = (p::f32(v, "header"), p::f32(v, "footer"));
            let block = s.sel.focus.path.0.first().copied().unwrap_or(0) as usize;
            let sect = s.doc.section_mut(block);
            if let Some(h) = h {
                sect.header = h.clamp(0.0, 300.0);
            }
            if let Some(f) = f {
                sect.footer = f.clamp(0.0, 300.0);
            }
            Ok(json!({"header": sect.header, "footer": sect.footer}))
        })
        .params(r#"{"header"?: pt, "footer"?: pt}"#),
        CommandSpec::new("table.cellMargins", "Cell Margins", "Table Layout › Alignment", |s, v| {
            let Some((tp, _, _)) = s.sel.focus.path.cell() else { return Err(CmdError::Disabled("not in a table".into())) };
            let m = [
                p::f32(v, "top").unwrap_or(0.0),
                p::f32(v, "left").unwrap_or(5.4),
                p::f32(v, "bottom").unwrap_or(0.0),
                p::f32(v, "right").unwrap_or(5.4),
            ]
            .map(|x| x.clamp(0.0, 144.0));
            let story = s.sel.focus.story;
            s.doc.table_mut(story, &tp)?.props.cell_margins = Some(m);
            for path in s.doc.para_paths(story).into_iter().filter(|p| p.0.starts_with(&tp.0) && p.0.len() > tp.0.len()) {
                s.doc.para_mut(story, &path)?.touch();
            }
            sel_result(s)
        })
        .params(r#"{"top"?, "left"?, "bottom"?, "right"? (pt)}"#),
        CommandSpec::new("table.borderPainter", "Border Painter", "Table Design › Borders", |s, v| {
            s.run(
                "table.borders",
                &json!({"kind": p::str(v, "kind").unwrap_or("all"), "width": p::f32(v, "width").unwrap_or(0.5), "color": p::str(v, "color")}),
            )
        }),
        CommandSpec::new("para.setNumberingValue", "Set Numbering Value", "Home › Paragraph › Numbering", |s, v| {
            let value = p::u64(v, "value").unwrap_or(1).clamp(0, 100_000) as u32;
            let f = s.sel.focus.clone();
            let n = s
                .doc
                .para_at(&f)
                .and_then(|x| x.props.numbering)
                .filter(|n| n.num != 0)
                .ok_or_else(|| CmdError::Failed("not in a numbered list".into()))?;
            let new = s.doc.numbering.restart(n.num).ok_or_else(|| CmdError::Failed("bad list".into()))?;
            if let Some(num) = s.doc.numbering.nums.iter_mut().find(|x| x.id == new) {
                num.start_overrides = vec![(n.level, value)];
            }
            let paths: Vec<_> = s.doc.para_paths(f.story).into_iter().filter(|q| *q >= f.path).collect();
            for path in paths {
                let para = s.doc.para_mut(f.story, &path)?;
                match para.props.numbering {
                    Some(x) if x.num == n.num => para.props.numbering = Some(NumRef { num: new, level: x.level }),
                    _ => break,
                }
                para.touch();
            }
            sel_result(s)
        })
        .params(r#"{"value": n}"#),
        CommandSpec::new("para.defineBullet", "Define New Bullet", "Home › Paragraph › Bullets", |s, v| {
            s.run("para.bullets", &json!({"kind": p::str(v, "char").unwrap_or("★")}))
        })
        .params(r#"{"char": string}"#),
        CommandSpec::new("para.defineNumber", "Define New Number Format", "Home › Paragraph › Numbering", |s, v| {
            let fmt = wordcraft_doc::section::NumFormat::from_ooxml(p::str(v, "format").unwrap_or("decimal"));
            let text = p::str(v, "text").unwrap_or("%1.").to_string();
            let start = p::u64(v, "start").unwrap_or(1).min(100_000) as u32;
            let id = s.doc.numbering.try_add_list(wordcraft_doc::ListKind::Numbered)?;
            if let Some(a) = s.doc.numbering.abstract_of(id).map(|a| a.id)
                && let Some(abs) = s.doc.numbering.abstracts.iter_mut().find(|x| x.id == a)
                && let Some(l) = abs.levels.first_mut()
            {
                l.format = fmt;
                l.text = text;
                l.start = start;
            }
            super::para::fmt(s, &|pp| {
                pp.numbering = Some(NumRef { num: id, level: 0 });
                pp.style = Some("ListParagraph".into());
            })
        })
        .params(r#"{"format": "decimal|upperRoman|lowerLetter…", "text"?: "%1.", "start"?: n}"#),
        CommandSpec::new("review.language", "Language", "Review › Language", language).params(
            r#"{"lang"?: "en-US|en-GB|pl-PL|fr-FR|…" (BCP 47; default en-US), "noProof"?: bool, "detect"?: bool (detect language automatically, a preference), "default"?: bool (also the document's default language)}"#,
        ),
        CommandSpec::new("review.showMarkup", "Show Markup", "Review › Tracking", |s, v| {
            s.view.show_markup = p::bool(v, "value").unwrap_or(!s.view.show_markup);
            s.relayout();
            Ok(json!({"value": s.view.show_markup}))
        })
        .pure(),
        CommandSpec::new("review.editor", "Editor", "Home › Editor", |s, v| s.run("review.spelling", v)).pure(),
        CommandSpec::new("select.similar", "Select Text with Similar Formatting", "Home › Editing › Select", select_similar).pure(),
        CommandSpec::new("select.extend", "Extend Selection", "Editing › Selection", |s, _| {
            // F8 grows the selection: word → sentence → paragraph → document.
            let (a, b) = s.sel.ordered();
            let para_len = s.doc.para_at(&a).map(|x| x.len()).unwrap_or(0);
            let id = if a == b {
                "select.word"
            } else if a.path == b.path && (a.off > 0 || b.off < para_len) {
                if s.doc.para_at(&a).map(|x| x.sentence_at(a.off)) == Some((a.off, b.off)) { "select.paragraph" } else { "select.sentence" }
            } else {
                "select.all"
            };
            let params = if id == "select.all" { json!({"whole": true}) } else { json!({}) };
            s.run(id, &params)
        })
        .key("F8")
        .pure(),
        CommandSpec::new("design.effects", "Effects", "Design › Document Formatting", |s, v| {
            s.doc.settings.theme_name =
                format!("{} ({})", s.doc.settings.theme_name.split(" (").next().unwrap_or("Craft"), p::str(v, "name").unwrap_or("Subtle"));
            sel_result(s)
        }),
        CommandSpec::new("view.immersive", "Immersive Reader", "View › Immersive", |s, _| {
            s.view.read_mode = !s.view.read_mode;
            Ok(json!({"value": s.view.read_mode}))
        })
        .pure(),
        CommandSpec::new("view.vertical", "Vertical", "View › Page Movement", |s, _| {
            s.view.multi_page = false;
            s.view.fit.clear();
            Ok(json!({"pageMovement": "vertical"}))
        })
        .pure(),
        CommandSpec::new("view.sideToSide", "Side to Side", "View › Page Movement", |s, _| {
            s.view.multi_page = true;
            s.view.single_column = false;
            s.view.fit = "multiplePages".into();
            Ok(json!({"pageMovement": "sideToSide"}))
        })
        .pure(),
    ]
}

/// Set Proofing Language: the selection's language and "do not check spelling or grammar",
/// or (`default`) the document's default language; `detect` is the per-user preference.
/// Nothing at all sets US English, as before.
fn language(s: &mut Session, v: &Value) -> CmdResult {
    let lang = match p::str(v, "lang") {
        Some(t) => Some(
            wordcraft_proof::lang::normalize_tag(t)
                .ok_or_else(|| CmdError::Params(format!("`lang` must be a language tag such as pl-PL or en-US, not {t:?}")))?,
        ),
        None => None,
    };
    let no_proof = p::bool(v, "noProof");
    let detect = p::bool(v, "detect");
    if let Some(d) = detect {
        s.prefs.detect_language = d;
    }
    if p::bool(v, "default") == Some(true) {
        let l = lang.clone().ok_or_else(|| CmdError::Params("`default` needs `lang`".into()))?;
        s.doc.styles.default_chr.lang = Some(l.clone());
        // A Normal style that names its own language follows the new default.
        if let Some(n) = s.doc.styles.styles.iter_mut().find(|x| x.id == "Normal")
            && n.chr.lang.is_some()
        {
            n.chr.lang = Some(l);
        }
    }
    let lang = match (lang, no_proof, detect) {
        (None, None, None) => Some("en-US".to_string()),
        (l, ..) => l,
    };
    if lang.is_none() && no_proof.is_none() {
        return Ok(json!({"detect": s.prefs.detect_language}));
    }
    super::format::apply(s, &|c| {
        if let Some(l) = &lang {
            c.lang = Some(l.clone());
        }
        if let Some(np) = no_proof {
            c.no_proof = Some(np);
        }
    })
}

fn macros(s: &mut Session, v: &Value) -> CmdResult {
    if let Some(d) = v.get("define") {
        let name = d.get("name").and_then(Value::as_str).unwrap_or("Macro").to_string();
        let steps: Vec<(String, Value)> = d
            .get("steps")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter().filter_map(|st| Some((st.get("command")?.as_str()?.to_string(), st.get("params").cloned().unwrap_or(json!({}))))).collect()
            })
            .unwrap_or_default();
        s.macros.insert(name, steps);
    }
    if let Some(n) = p::str(v, "delete") {
        s.macros.remove(n);
    }
    if let Some(n) = p::str(v, "run") {
        let steps = s.macros.get(n).cloned().ok_or_else(|| CmdError::Params(format!("no macro `{n}`")))?;
        for (id, params) in steps.iter().take(10_000) {
            if id == "tools.macros" {
                continue;
            }
            s.run(id, params)?;
        }
        return Ok(json!({"ran": n, "steps": steps.len()}));
    }
    Ok(Value::Object(
        s.macros.iter().map(|(k, v)| (k.clone(), json!(v.iter().map(|(c, p)| json!({"command": c, "params": p})).collect::<Vec<_>>()))).collect(),
    ))
}

/// Apply AutoCorrect to the word just before the caret (called after typing a space/punctuation):
/// replacements, then ordinal superscripts, web addresses as links and sentence capitals.
pub fn autocorrect(s: &mut Session) -> Result<(), CmdError> {
    autocorrect_word(s, false)
}

/// AutoCorrect the word ending at the caret; `on_enter` means Enter finished it (no trigger
/// character was typed).
pub fn autocorrect_word(s: &mut Session, on_enter: bool) -> Result<(), CmdError> {
    if !s.prefs.autocorrect.enabled {
        return Ok(());
    }
    let ac = s.prefs.autocorrect.clone();
    let mut f = s.sel.focus.clone();
    let Some(text) = s.doc.para_at(&f).map(|p| p.text.clone()) else { return Ok(()) };
    let Some(before) = text.get(..f.off) else { return Ok(()) };
    let Some(last) = before.chars().next_back() else { return Ok(()) };
    if !(on_enter || last == ' ' || ",.;:!?".contains(last)) {
        return Ok(());
    }
    let trigger = if on_enter { 0 } else { last.len_utf8() };
    let Some(body) = before.get(..before.len() - trigger) else { return Ok(()) };
    let start = body.rfind(char::is_whitespace).map(|i| i + 1).unwrap_or(0);
    let Some(mut word) = body.get(start..).map(str::to_string) else { return Ok(()) };
    if word.is_empty() {
        return Ok(());
    }
    // TWo INitial CApitals and cAPS LOCK come first, so "TEh" is still a replacement's typo.
    if let Some((at, reps)) = case_fix(&ac, &word) {
        let para = s.doc.para_mut(f.story, &f.path)?;
        let letters = word.get(at..).unwrap_or("").chars().take(reps.len()).collect::<String>();
        let mut end = start + at + letters.len();
        for (c, rep) in letters.chars().rev().zip(reps.iter().rev()) {
            let from = end.saturating_sub(c.len_utf8());
            if rep.chars().ne(std::iter::once(c)) {
                let props = para.props_of_char(from).clone();
                para.delete(from, end)?;
                para.insert_text(from, rep, &props)?;
            }
            end = from;
        }
        let fixed: String = reps.concat();
        f.off = (f.off + fixed.len()).saturating_sub(letters.len());
        s.sel = Selection::caret(f.clone());
        word = format!("{}{fixed}{}", word.get(..at).unwrap_or(""), word.get(at + letters.len()..).unwrap_or(""));
    }
    // A whole-word replacement, else "wait..." → "wait…".
    // The word's language (run, paragraph style, document default) picks the list (#835).
    // Text with no language at all is in the editing language (a Polish interface writes Polish),
    // else English. Proofing decides what is English the same way.
    let english = s.doc.para_at(&f).is_none_or(|p| {
        let rc = s.doc.styles.resolve_char(p.props.style.as_deref(), p.props_of_char(start));
        let lang = rc.lang.as_deref().or(s.editing_lang.as_deref());
        wordcraft_proof::ProofLang::from_tag(lang) == wordcraft_proof::ProofLang::En
    });
    let found = if ac.replace_text { ac.replacement_in(&word, english) } else { None };
    let (from, rep) = match found {
        Some(r) => (start, Some(r)),
        None if ac.replace_text && word.len() > 3 && word.ends_with("...") => (start + word.len() - 3, Some("…".to_string())),
        None if ac.replace_text => match SMILEYS.iter().find(|m| word.len() > m.len() && word.ends_with(**m)) {
            // "ok:)" → "ok☺": only the smiley at the end of the word is replaced.
            Some(m) => (start + word.len() - m.len(), AUTOCORRECT.iter().find(|(a, _)| a == m).map(|(_, b)| b.to_string())),
            None => (start, None),
        },
        None => (start, None),
    };
    let mut caret = f.off;
    if let Some(rep) = rep {
        let end = start + word.len();
        let para = s.doc.para_mut(f.story, &f.path)?;
        let props = para.props_of_char(from).clone();
        para.delete(from, end)?;
        para.insert_text(from, &rep, &props)?;
        caret = (f.off + rep.len()).saturating_sub(end - from);
        s.sel = Selection::caret(Pos { off: caret, ..f.clone() });
    }
    let end = caret.saturating_sub(trigger);
    let Some(word) = s.doc.para_at(&f).and_then(|p| p.text.get(start..end)).map(str::to_string) else { return Ok(()) };
    let at = |off: usize| Pos { off, ..f.clone() };
    // 1st, 22nd, 103rd → superscript suffix.
    if let Some(n) = ordinal_suffix(&word).filter(|_| ac.ordinals) {
        s.doc.format_range(&at(end - n), &at(end), &|c| c.vert_align = Some(wordcraft_doc::props::VertAlign::Superscript))?;
        return Ok(());
    }
    // Web addresses become links (trailing punctuation stays outside).
    let lower = word.to_ascii_lowercase();
    if ac.links && (lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("www.")) && word.len() > 8 {
        let url = word.trim_end_matches(|c: char| ".,;:!?)]\"'”’".contains(c));
        let href = if lower.starts_with("www.") { format!("http://{url}") } else { url.to_string() };
        let url_end = start + url.len();
        s.doc.format_range(&at(start), &at(url_end), &|c| {
            c.link = Some(href.clone());
            c.style = Some("Hyperlink".into());
        })?;
        // Typing after the link isn't part of it (but keeps the text's other formatting).
        let mut after = s.doc.para_at(&f).map(|p| p.props_of_char(start).clone()).unwrap_or_default();
        after.link = None;
        after.style = None;
        s.pending = Some(after);
        return Ok(());
    }
    let before_word = text.get(..start).unwrap_or("");
    // The first word in a table cell follows its own option; other sentences the sentence one.
    let sentence_on = if before_word.trim().is_empty() && f.path.cell().is_some() { ac.cap_cells } else { ac.cap_sentences };
    let exceptions = ac.exception_list();
    // A replacement of several words ("brb" → "be right back") is capitalised by its first.
    let first = word.split_whitespace().next().unwrap_or("");
    let next = if first.len() < word.len() {
        ' '
    } else if on_enter {
        '\n'
    } else {
        last
    };
    let sentence = sentence_on && starts_sentence(before_word, &exceptions) && should_capitalize(first, next);
    let day = ac.cap_days && DAYS.contains(&first);
    if sentence || day {
        let para = s.doc.para_mut(f.story, &f.path)?;
        if let Some(c) = word.chars().next() {
            let up: String = c.to_uppercase().collect();
            let props = para.props_of_char(start).clone();
            para.delete(start, start + c.len_utf8())?;
            para.insert_text(start, &up, &props)?;
            let off = (caret + up.len()).saturating_sub(c.len_utf8());
            s.sel = Selection::caret(Pos { off, ..f });
        }
    }
    Ok(())
}

/// Built-in words that keep their two initial capitals (plurals of two-letter abbreviations).
pub const INITIAL_CAPS_EXCEPTIONS: &[&str] = &["CDs", "IDs", "PCs", "TVs", "DJs", "GBs", "KBs", "MBs", "TBs", "VMs"];

/// Words with a lowercase first letter and the rest in capitals that aren't cAPS LOCK slips.
const CAPS_LOCK_EXCEPTIONS: &[&str] = &["iOS", "mRNA", "tRNA", "rRNA", "cDNA", "eSIM", "eSATA", "iSCSI", "vCPU"];

/// The capitalisation fix for a finished word: TWo INitial CApitals ("WOrd" → "Word") or cAPS
/// LOCK ("wORD" → "Word"). Returns the byte offset of the word's letters and each letter's
/// replacement. Only words of three or more letters, optionally in quotes or brackets.
fn case_fix(caps: &AutoCorrectPrefs, word: &str) -> Option<(usize, Vec<String>)> {
    if !caps.two_initial_caps && !caps.caps_lock {
        return None;
    }
    let at = word.find(char::is_alphabetic)?;
    let rest = word.get(at..)?;
    let letters = rest.get(..rest.find(|c: char| !c.is_alphabetic()).unwrap_or(rest.len()))?;
    let tail = rest.get(letters.len()..)?;
    if !word.get(..at)?.chars().all(|c| "\"'“‘([".contains(c)) || !tail.chars().all(|c| "\"'”’)]".contains(c)) {
        return None;
    }
    let cs: Vec<char> = letters.chars().collect();
    let (&a, &b, more) = match cs.as_slice() {
        [a, b, more @ ..] if !more.is_empty() => (a, b, more),
        _ => return None,
    };
    let one = |c: char| c.to_string();
    let lower = |c: char| c.to_lowercase().collect::<String>();
    if caps.two_initial_caps
        && a.is_uppercase()
        && b.is_uppercase()
        && more.iter().all(|c| c.is_lowercase())
        && !INITIAL_CAPS_EXCEPTIONS.contains(&letters)
        && !caps.initial_caps_exceptions.iter().any(|e| e == letters)
    {
        return Some((at, std::iter::once(one(a)).chain(std::iter::once(lower(b))).chain(more.iter().map(|&c| one(c))).collect()));
    }
    if caps.caps_lock && a.is_lowercase() && b.is_uppercase() && more.iter().all(|c| c.is_uppercase()) && !CAPS_LOCK_EXCEPTIONS.contains(&letters) {
        let up = a.to_uppercase().collect::<String>();
        return Some((at, std::iter::once(up).chain(std::iter::once(lower(b))).chain(more.iter().map(|&c| lower(c))).collect()));
    }
    None
}

/// Byte length of an ordinal suffix to superscript ("1st" → 2), if `word` is a correct ordinal.
fn ordinal_suffix(word: &str) -> Option<usize> {
    let digits = word.find(|c: char| !c.is_ascii_digit())?;
    let (num, suffix) = word.split_at(digits);
    if num.is_empty() || num.len() > 9 {
        return None;
    }
    let n: u64 = num.parse().ok()?;
    let want = match (n % 100, n % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    };
    (suffix == want).then_some(2)
}

/// Abbreviations whose full stop doesn't end a sentence.
const NOT_SENTENCE_END: &[&str] = &[
    "e.g.", "i.e.", "etc.", "vs.", "cf.", "mr.", "mrs.", "ms.", "dr.", "st.", "no.", "approx.", "fig.", "p.", "pp.", "jr.", "sr.", "ca.", "inc.",
    "ltd.", "co.", "vol.", "a.m.", "p.m.",
];

/// Names of days, capitalised as you type (Capitalize names of days).
const DAYS: [&str; 7] = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];

/// Does a word typed after `before` (the paragraph text up to the word) begin a sentence?
/// `exceptions` are the lowercase abbreviations whose full stop doesn't end one.
fn starts_sentence(before: &str, exceptions: &[String]) -> bool {
    let t = before.trim_end();
    if t.is_empty() {
        return true;
    }
    if t.len() == before.len() {
        return false; // no space between the previous text and the word
    }
    let t = t.trim_end_matches(|c: char| "\"'”’)]".contains(c));
    if !t.ends_with(['.', '!', '?']) || t.ends_with("..") || t.ends_with('…') {
        return false;
    }
    let prev = t.rsplit(char::is_whitespace).next().unwrap_or(t).trim_start_matches(|c: char| "\"'“‘([".contains(c));
    let lower = prev.to_lowercase();
    // "J." (an initial) or a known abbreviation.
    let initial = prev.chars().count() == 2 && prev.chars().next().is_some_and(char::is_uppercase);
    !initial && !exceptions.contains(&lower)
}

/// Only plain lowercase words are capitalised (not "iPhone", "x2", file names or addresses).
fn should_capitalize(word: &str, next: char) -> bool {
    // A single letter before a full stop is a list label or an initial ("a.", "j.").
    if (next == '.' || word.ends_with('.')) && word.trim_end_matches('.').chars().count() < 2 {
        return false;
    }
    if word == "e.g." || word == "i.e." {
        return true;
    }
    word.chars().next().is_some_and(char::is_lowercase)
        && word.chars().all(|c| c.is_alphabetic() || "'’-.".contains(c))
        && !word.chars().any(char::is_uppercase)
        && !word.trim_end_matches('.').contains('.')
}

/// What Compare / Combine compares (the More >> settings of Word's dialog).
struct CompareOptions {
    /// Differences only in letter case count as changes.
    case: bool,
    /// Differences only in white space count as changes.
    white_space: bool,
    /// Formatting differences become tracked formatting changes.
    formatting: bool,
    /// Changed paragraphs are compared character by character (else word by word).
    characters: bool,
}

impl CompareOptions {
    /// The text a comparison sees: case folded and white space collapsed as the options say.
    fn key(&self, t: &str) -> String {
        let t = if self.white_space { t.to_string() } else { t.split_whitespace().collect::<Vec<_>>().join(" ") };
        if self.case { t } else { t.to_lowercase() }
    }
}

/// Changed paragraphs longer than this (in characters) are compared word by word even at
/// character level, so the comparison table stays small.
const MAX_CHAR_DIFF: usize = 2000;

/// Compare the current document (original, or `original` from a file) with a revised one: the
/// result shows the differences as tracked insertions, deletions and formatting changes, labelled
/// with `author`. Options Word offers that the comparison can't honour yet (moves, comments,
/// headers and footers, footnotes, text boxes, fields) are accepted and listed in the result.
fn compare(s: &mut Session, v: &Value) -> CmdResult {
    let revised = if let Some(t) = p::str(v, "text") {
        Document::from_text(t)
    } else if let Some(path) = p::str(v, "path") {
        crate::io::open_path(std::path::Path::new(path)).map_err(CmdError::Failed)?
    } else {
        return Err(CmdError::Params("`path` or `text` of the revised document is required".into()));
    };
    let show_in = p::str(v, "showIn").unwrap_or("original");
    if !matches!(show_in, "original" | "revised" | "new") {
        return Err(CmdError::Params("`showIn` is \"original\", \"revised\" or \"new\"".into()));
    }
    if let Some(path) = p::str(v, "original") {
        let doc = crate::io::open_path(std::path::Path::new(path)).map_err(CmdError::Failed)?;
        s.doc = doc;
        s.doc.ensure_nonempty();
    }
    let level = p::str(v, "level").unwrap_or("word");
    let opts = CompareOptions {
        case: p::bool(v, "caseChanges").unwrap_or(true),
        white_space: p::bool(v, "whiteSpace").unwrap_or(true),
        formatting: p::bool(v, "formatting").unwrap_or(true),
        characters: matches!(level, "character" | "char"),
    };
    let not_honoured: Vec<&str> =
        ["moves", "comments", "headersFooters", "footnotes", "textBoxes", "fields"].into_iter().filter(|k| v.get(*k).is_some()).collect();
    let author = p::str(v, "author").map(str::trim).filter(|a| !a.is_empty()).map(str::to_string).unwrap_or_else(|| s.author.clone());
    let date = super::now_iso();
    let old: Vec<Arc<Block>> = s.doc.body.clone();
    let new: Vec<Arc<Block>> = revised.body.clone();
    s.doc.revisions.push(wordcraft_doc::Revision { kind: RevisionKind::Insert, author: author.clone(), date: date.clone(), move_name: None });
    let ins = (s.doc.revisions.len() - 1) as u32;
    s.doc.revisions.push(wordcraft_doc::Revision { kind: RevisionKind::Delete, author: author.clone(), date: date.clone(), move_name: None });
    let del = (s.doc.revisions.len() - 1) as u32;
    let key = |b: &Arc<Block>| match &**b {
        Block::Para(p) => opts.key(&p.plain_text()),
        // Tables match as a whole, by their text.
        Block::Table(_) => format!("\u{0}table\u{0}{}", opts.key(&block_text(b, 0))),
    };
    let ops = lcs_ops(&old.iter().map(key).collect::<Vec<_>>(), &new.iter().map(key).collect::<Vec<_>>());
    let mut out: Vec<Arc<Block>> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    let mut changes = 0usize;
    let mut k = 0;
    while k < ops.len() {
        match ops.get(k) {
            Some(Op::Same) => {
                let (a, b) = (old.get(i), new.get(j));
                // The revised block, with its formatting changes tracked.
                if let (Some(Block::Para(pa)), Some(Block::Para(pb))) = (a.map(|x| &**x), b.map(|x| &**x)) {
                    let mut p = pb.clone();
                    if opts.formatting && pa.text == pb.text {
                        let before = s.doc.revisions.len();
                        super::fmt_revisions::compare_formatting(pa, &mut p, &mut s.doc.revisions, &author, &date);
                        if s.doc.revisions.len() > before || p != *pb {
                            changes += 1;
                        }
                    }
                    out.push(para_block(p));
                } else if let Some(x) = b.or(a) {
                    out.push(x.clone());
                }
                i += 1;
                j += 1;
                k += 1;
            }
            Some(_) => {
                // A run of deletions and insertions: changed paragraphs pair up and are diffed
                // word by word (or by character); the rest are deleted or inserted whole.
                let run = ops.get(k..).unwrap_or_default().iter().take_while(|o| **o != Op::Same).count();
                let dels = ops.get(k..k + run).unwrap_or_default().iter().filter(|o| **o == Op::Del).count();
                let inss = run - dels;
                for x in 0..dels.max(inss) {
                    let (a, b) = (if x < dels { old.get(i + x) } else { None }, if x < inss { new.get(j + x) } else { None });
                    match (a.map(|x| &**x), b.map(|x| &**x)) {
                        (Some(Block::Para(pa)), Some(Block::Para(pb))) => out.push(para_block(word_diff(pa, pb, ins, del, &opts))),
                        _ => {
                            if let Some(x) = a {
                                out.push(Arc::new(mark_block(x, &|c| c.del = Some(del), 0)));
                            }
                            if let Some(x) = b {
                                out.push(Arc::new(mark_block(x, &|c| c.ins = Some(ins), 0)));
                            }
                        }
                    }
                    changes += 1;
                }
                i += dels;
                j += inss;
                k += run.max(1);
            }
            None => break,
        }
    }
    s.doc.body = out;
    s.doc.ensure_nonempty();
    if show_in != "original" || p::str(v, "original").is_some() {
        // The result is a new, untitled document: saving it never overwrites either file.
        s.path = None;
        s.file_name = None;
    }
    s.sel = Selection::caret(s.doc.start_of(StoryRef::Body));
    Ok(json!({"changes": changes, "author": author, "notHonoured": not_honoured}))
}

/// A block's text (a table's cells in order), for matching blocks.
fn block_text(b: &Block, depth: usize) -> String {
    match b {
        Block::Para(p) => p.plain_text(),
        Block::Table(t) if depth < 16 => {
            t.rows.iter().flat_map(|r| r.cells.iter()).flat_map(|c| c.blocks.iter()).map(|x| block_text(x, depth + 1)).collect::<Vec<_>>().join("\t")
        }
        Block::Table(_) => String::new(),
    }
}

/// A copy of the block with every run (in tables, every cell's) changed by `f`.
fn mark_block(b: &Block, f: &dyn Fn(&mut CharProps), depth: usize) -> Block {
    match b {
        Block::Para(p) => {
            let mut p = p.clone();
            let len = p.len();
            let _ = p.format(0, len, f);
            Block::Para(p)
        }
        Block::Table(t) => {
            let mut t = t.clone();
            if depth < 16 {
                for c in t.rows.iter_mut().flat_map(|r| r.cells.iter_mut()) {
                    c.blocks = c.blocks.iter().map(|x| Arc::new(mark_block(x, f, depth + 1))).collect();
                }
            }
            Block::Table(t)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Same,
    Del,
    Ins,
}

/// Longest sequence the comparison table is built for; the rest of a longer sequence is
/// compared as deleted and inserted, never dropped.
const MAX_LCS: usize = 4000;

/// LCS edit script between two sequences: the common start and end are matched first, then the
/// middle (bounded size).
fn lcs_ops<T: PartialEq>(a: &[T], b: &[T]) -> Vec<Op> {
    let pre = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let (ra, rb) = (a.get(pre..).unwrap_or_default(), b.get(pre..).unwrap_or_default());
    let suf = ra.iter().rev().zip(rb.iter().rev()).take_while(|(x, y)| x == y).count();
    let (a, b) = (ra.get(..ra.len() - suf).unwrap_or_default(), rb.get(..rb.len() - suf).unwrap_or_default());
    let mut ops = vec![Op::Same; pre];
    let (n, m) = (a.len().min(MAX_LCS), b.len().min(MAX_LCS));
    let mut t = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let v = if a.get(i) == b.get(j) {
                t.get(i + 1).and_then(|r| r.get(j + 1)).copied().unwrap_or(0) + 1
            } else {
                t.get(i + 1).and_then(|r| r.get(j)).copied().unwrap_or(0).max(t.get(i).and_then(|r| r.get(j + 1)).copied().unwrap_or(0))
            };
            if let Some(c) = t.get_mut(i).and_then(|r| r.get_mut(j)) {
                *c = v;
            }
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a.get(i) == b.get(j) {
            ops.push(Op::Same);
            i += 1;
            j += 1;
        } else if t.get(i + 1).and_then(|r| r.get(j)) >= t.get(i).and_then(|r| r.get(j + 1)) {
            ops.push(Op::Del);
            i += 1;
        } else {
            ops.push(Op::Ins);
            j += 1;
        }
    }
    ops.extend(std::iter::repeat_n(Op::Del, a.len() - i));
    ops.extend(std::iter::repeat_n(Op::Ins, b.len() - j));
    ops.extend(std::iter::repeat_n(Op::Same, suf));
    ops
}

/// Words with the spaces after them ("The ", "cat "), or single characters.
fn tokens(s: &str, characters: bool) -> Vec<String> {
    if characters {
        return s.chars().map(String::from).collect();
    }
    let mut v = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        cur.push(c);
        if c == ' ' {
            v.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        v.push(cur);
    }
    v
}

fn word_diff(a: &Paragraph, b: &Paragraph, ins: u32, del: u32, opts: &CompareOptions) -> Paragraph {
    let (at, bt) = (a.plain_text(), b.plain_text());
    let chars = opts.characters && at.chars().count().max(bt.chars().count()) <= MAX_CHAR_DIFF;
    let (ta, tb) = (tokens(&at, chars), tokens(&bt, chars));
    let ops = lcs_ops(&ta.iter().map(|t| opts.key(t)).collect::<Vec<_>>(), &tb.iter().map(|t| opts.key(t)).collect::<Vec<_>>());
    let mut p = Paragraph::new();
    p.props = b.props.clone();
    let base = a.runs.first().map(|r| r.props.clone()).unwrap_or_default();
    let (mut i, mut j) = (0, 0);
    for op in ops {
        let at = p.len();
        match op {
            Op::Same => {
                // Equal as compared (perhaps up to case or spacing): the revised text.
                let _ = p.insert_text(at, tb.get(j).map(String::as_str).unwrap_or(""), &base);
                i += 1;
                j += 1;
            }
            Op::Del => {
                let _ = p.insert_text(at, ta.get(i).map(String::as_str).unwrap_or(""), &CharProps { del: Some(del), ..base.clone() });
                i += 1;
            }
            Op::Ins => {
                let _ = p.insert_text(at, tb.get(j).map(String::as_str).unwrap_or(""), &CharProps { ins: Some(ins), ..base.clone() });
                j += 1;
            }
        }
    }
    p
}

fn accessibility(s: &mut Session, _: &Value) -> CmdResult {
    let issues = super::accessibility::check(&s.doc);
    Ok(json!({"ok": issues.is_empty(), "issues": issues}))
}

fn inspect_doc(s: &mut Session, v: &Value) -> CmdResult {
    let remove: Vec<String> =
        v.get("remove").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let hidden = hidden_text(&s.doc);
    let report = json!({
        "comments": s.doc.comments.len(),
        "revisions": s.doc.para_paths(StoryRef::Body).iter().filter(|p| s.doc.para(StoryRef::Body, p).is_some_and(|x| x.runs.iter().any(|r| r.props.ins.is_some() || r.props.del.is_some()))).count(),
        "properties": !s.doc.core.creator.is_empty() || !s.doc.core.last_modified_by.is_empty(),
        "hiddenText": !hidden.is_empty(),
        "hiddenCount": hidden.len(),
        "hidden": hidden.iter().take(MAX_HIDDEN_LISTED).map(|h| h.report(&s.doc)).collect::<Vec<_>>(),
        "truncated": hidden.len() > MAX_HIDDEN_LISTED,
        "unsupported": unsupported_hidden(&s.doc),
        "headers": s.doc.parts.values().filter(|p| matches!(p.kind, wordcraft_doc::PartKind::Header | wordcraft_doc::PartKind::Footer)).count(),
    });
    let mut removed_notes = Vec::new();
    for r in &remove {
        match r.as_str() {
            "comments" => {
                s.run("review.deleteComment", &json!({"all": true}))?;
            }
            "revisions" => {
                s.run("review.acceptAll", &json!({}))?;
            }
            "properties" => {
                let title = s.doc.core.title.clone();
                s.doc.core = Default::default();
                s.doc.core.title = title;
            }
            "hidden" => removed_notes = remove_hidden(&mut s.doc)?,
            "headers" => {
                s.run("insert.removeHeader", &json!({}))?;
                s.run("insert.removeFooter", &json!({}))?;
            }
            _ => {}
        }
    }
    s.clamp_selection();
    // What a removal couldn't reach: hidden text still there, or kept in objects we don't model.
    let residual: Vec<Value> = if remove.iter().any(|r| r == "hidden") {
        let mut v: Vec<Value> = hidden_text(&s.doc).iter().take(MAX_HIDDEN_LISTED).map(|h| h.report(&s.doc)).collect();
        v.extend(unsupported_hidden(&s.doc));
        v
    } else {
        Vec::new()
    };
    Ok(json!({"found": report, "removed": remove, "residual": residual, "removedNotes": removed_notes}))
}

/// Most hidden pieces (and removed notes) `file.inspect` lists (the count is always exact).
const MAX_HIDDEN_LISTED: usize = 1000;
/// Most characters of a removed note's text `file.inspect` echoes.
const MAX_NOTE_TEXT: usize = 1000;

/// A piece of hidden text: a byte range of a paragraph in a story. `whole`: the paragraph's mark
/// is hidden too and so is all its text, so the paragraph itself goes.
struct Hidden {
    story: StoryRef,
    path: wordcraft_doc::Path,
    range: std::ops::Range<usize>,
    whole: bool,
}

/// Remove All for hidden text (#1344): every hidden piece, last first so earlier offsets and
/// paths stay valid; whole hidden paragraphs with their marks; and the notes whose references
/// were hidden, as deleting a reference deletes its note. Returns the notes removed.
fn remove_hidden(doc: &mut Document) -> Result<Vec<Value>, CmdError> {
    let mut refs: Vec<u32> = Vec::new();
    for h in hidden_text(doc).into_iter().rev() {
        if let Some(p) = doc.para(h.story, &h.path) {
            for off in p.object_offsets() {
                if h.range.contains(&off)
                    && let Some(InlineObject::NoteRef { id, .. }) = p.object_at(off)
                {
                    refs.push(*id);
                }
            }
        }
        remove_piece(doc, &h)?;
    }
    if refs.is_empty() {
        return Ok(Vec::new());
    }
    // Notes still referenced from elsewhere (a note's own mark doesn't count) stay.
    let mut kept = std::collections::BTreeSet::new();
    for story in all_stories(doc) {
        for path in doc.para_paths(story) {
            let Some(p) = doc.para(story, &path) else { continue };
            for off in p.object_offsets() {
                if let Some(InlineObject::NoteRef { id, .. }) = p.object_at(off)
                    && story != StoryRef::Part(*id)
                {
                    kept.insert(*id);
                }
            }
        }
    }
    refs.sort_unstable();
    refs.dedup();
    let mut out = Vec::new();
    for id in refs {
        if kept.contains(&id)
            || !doc.parts.get(&id).is_some_and(|p| matches!(p.kind, wordcraft_doc::PartKind::Footnote | wordcraft_doc::PartKind::Endnote))
        {
            continue;
        }
        // Only the echo is capped; every such note goes.
        if out.len() < MAX_HIDDEN_LISTED {
            let text: String = doc.plain_text(StoryRef::Part(id)).replace('\u{FFFC}', "").trim().chars().take(MAX_NOTE_TEXT).collect();
            out.push(json!({"kind": story_kind(doc, StoryRef::Part(id)), "part": id, "text": text}));
        }
        doc.parts.remove(&id);
    }
    Ok(out)
}

/// Whether a whole hidden paragraph can go, mark and all. It stays when it is the last block of
/// its story or cell (they end with a paragraph), carries a section break, holds content control
/// markers (their partners may be elsewhere), or sits between two tables (they'd join on reopen).
fn removable(doc: &Document, story: StoryRef, path: &wordcraft_doc::Path, p: &Paragraph) -> bool {
    let i = path.last();
    let Some(c) = doc.container(story, path) else { return false };
    let is_table = |j: Option<usize>| j.and_then(|j| c.get(j)).is_some_and(|b| matches!(**b, Block::Table(_)));
    i + 1 < c.len()
        && p.section.is_none()
        && !p.object_offsets().into_iter().any(|o| matches!(p.object_at(o), Some(InlineObject::ControlStart { .. } | InlineObject::ControlEnd)))
        && !(is_table(i.checked_sub(1)) && is_table(i.checked_add(1)))
}

/// Whether `range` of `p` holds text or an object, not only zero-width markers.
fn has_content(p: &Paragraph, range: &std::ops::Range<usize>) -> bool {
    p.text
        .get(range.clone())
        .is_some_and(|t| t.char_indices().any(|(i, ch)| ch != '\u{FFFC}' || p.object_at(range.start + i).is_some_and(|o| !o.is_marker())))
}

/// Delete one hidden piece. Within a paragraph: straight in the paragraph, unless a content
/// control marker is inside (then through `delete_range`, which keeps markers whose partner is
/// outside). A whole hidden paragraph goes with its mark when [`removable`]; otherwise its text
/// goes and it stays, empty, its mark still hidden: invisible and holding no text, so it isn't
/// reported again.
fn remove_piece(doc: &mut Document, h: &Hidden) -> Result<(), CmdError> {
    let Some(p) = doc.para(h.story, &h.path) else { return Ok(()) };
    let has_control = p
        .object_offsets()
        .into_iter()
        .any(|o| h.range.contains(&o) && matches!(p.object_at(o), Some(InlineObject::ControlStart { .. } | InlineObject::ControlEnd)));
    if h.whole && removable(doc, h.story, &h.path, p) {
        let i = h.path.last();
        let blocks = doc.container_mut(h.story, &h.path)?;
        if i < blocks.len() {
            blocks.remove(i);
        }
        return Ok(());
    }
    if !h.range.is_empty() {
        if has_control {
            let at = |off| Pos { story: h.story, path: h.path.clone(), off };
            doc.delete_range(&at(h.range.start), &at(h.range.end))?;
        } else {
            doc.para_mut(h.story, &h.path)?.delete(h.range.start, h.range.end)?;
        }
    }
    Ok(())
}

impl Hidden {
    fn report(&self, doc: &Document) -> Value {
        let text = doc.para(self.story, &self.path).and_then(|p| p.text.get(self.range.clone())).unwrap_or("");
        let mut v = json!({"story": story_kind(doc, self.story), "text": text});
        if let (StoryRef::Part(id), Value::Object(m)) = (self.story, &mut v) {
            m.insert("part".into(), json!(id));
        }
        v
    }
}

/// Every story of the document: the body, then headers, footers, notes, comments and text boxes.
fn all_stories(doc: &Document) -> Vec<StoryRef> {
    std::iter::once(StoryRef::Body).chain(doc.parts.keys().map(|k| StoryRef::Part(*k))).collect()
}

/// What the inspector calls a story.
fn story_kind(doc: &Document, st: StoryRef) -> &'static str {
    use wordcraft_doc::PartKind;
    match st {
        StoryRef::Body => "body",
        StoryRef::Part(id) => match doc.parts.get(&id).map(|p| p.kind) {
            Some(PartKind::Header) => "header",
            Some(PartKind::Footer) => "footer",
            Some(PartKind::Footnote) => "footnote",
            Some(PartKind::Endnote) => "endnote",
            Some(PartKind::Comment) => "comment",
            Some(PartKind::TextBox) => "textBox",
            None => "unknown",
        },
    }
}

/// The hidden text of the document (Inspect Document, #1344): runs whose effective formatting
/// (document defaults, paragraph style chain, character style chain, direct) is hidden, in every
/// story, adjacent runs merged. Ranges holding only zero-width markers (bookmarks, comment
/// ranges, content control and field markers) aren't text and are left out.
fn hidden_text(doc: &Document) -> Vec<Hidden> {
    let mut out = Vec::new();
    for story in all_stories(doc) {
        for path in doc.para_paths(story) {
            let Some(p) = doc.para(story, &path) else { continue };
            let style = p.props.style.as_deref();
            let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
            let mut all_hidden = true;
            for (r, c) in p.run_ranges() {
                if r.is_empty() {
                    continue;
                }
                if !doc.styles.resolve_char(style, c).hidden {
                    all_hidden = false;
                    continue;
                }
                match ranges.last_mut() {
                    Some(last) if last.end == r.start => last.end = r.end,
                    _ => ranges.push(r),
                }
            }
            if all_hidden && doc.styles.resolve_char(style, &p.mark).hidden {
                // An empty hidden paragraph that has to stay is invisible and holds no text.
                if removable(doc, story, &path, p) || has_content(p, &(0..p.len())) {
                    out.push(Hidden { story, path: path.clone(), range: 0..p.len(), whole: true });
                }
                continue;
            }
            for range in ranges {
                if has_content(p, &range) {
                    out.push(Hidden { story, path: path.clone(), range, whole: false });
                }
            }
        }
    }
    out
}

/// Hidden text the inspector can't reach: objects kept verbatim from the source file
/// (`Opaque`) whose markup hides text (`w:vanish`), outside hidden runs (those go with the run).
fn unsupported_hidden(doc: &Document) -> Vec<Value> {
    let mut out = Vec::new();
    for story in all_stories(doc) {
        for path in doc.para_paths(story) {
            let Some(p) = doc.para(story, &path) else { continue };
            let style = p.props.style.as_deref();
            for off in p.object_offsets() {
                if let Some(InlineObject::Opaque { format, xml, .. }) = p.object_at(off)
                    && xml.contains("vanish")
                    && !doc.styles.resolve_char(style, p.props_of_char(off)).hidden
                {
                    out.push(json!({"story": story_kind(doc, story), "object": "opaque", "format": format, "reason": "hidden text inside an object kept from the source file"}));
                }
            }
        }
        if let Some(blocks) = doc.story(story) {
            tables_hiding_text(doc, story, blocks, 0, &mut out);
        }
    }
    out
}

/// Tables whose table style (or one of its conditional parts: header row, bands, first or last
/// column, total row) hides text. The inspector doesn't resolve table styles, so it lists them.
fn tables_hiding_text(doc: &Document, story: StoryRef, blocks: &wordcraft_doc::Blocks, depth: usize, out: &mut Vec<Value>) {
    if depth > 32 {
        return;
    }
    for b in blocks {
        let Block::Table(t) = &**b else { continue };
        if let Some(id) = t.props.style.as_deref()
            && let Some(ts) = doc.styles.table_style(id)
        {
            let q = &ts.parts;
            if [&ts.chr, &q.header_chr, &q.band_chr, &q.col_band_chr, &q.first_col_chr, &q.last_col_chr, &q.total_chr]
                .iter()
                .any(|c| c.hidden == Some(true))
            {
                out.push(json!({"story": story_kind(doc, story), "object": "table", "style": id, "reason": "text hidden by a table style isn't inspected or removed"}));
            }
        }
        for row in &t.rows {
            for cell in &row.cells {
                tables_hiding_text(doc, story, &cell.blocks, depth + 1, out);
            }
        }
    }
}

fn quick_parts(s: &mut Session, v: &Value) -> CmdResult {
    if let Some(name) = p::str(v, "save") {
        let (a, b) = s.sel.ordered();
        if a == b {
            return Err(CmdError::Params("select the content to save".into()));
        }
        let frag = s.doc.copy_range(&a, &b);
        s.building_blocks.insert(name.to_string(), frag);
    }
    if let Some(name) = p::str(v, "delete") {
        s.building_blocks.remove(name);
    }
    if let Some(name) = p::str(v, "insert") {
        let frag = s.building_blocks.get(name).cloned().ok_or_else(|| CmdError::Params(format!("no Quick Part `{name}`")))?;
        let at = delete_selection(s)?;
        let end = s.doc.insert_fragment(&at, &frag)?;
        s.sel = Selection::caret(end);
    }
    Ok(json!({"parts": s.building_blocks.keys().collect::<Vec<_>>()}))
}

fn hf_nav(s: &mut Session, dir: i32) -> CmdResult {
    let StoryRef::Part(cur) = s.sel.focus.story else { return Err(CmdError::Disabled("edit a header or footer first".into())) };
    let mut ids: Vec<u32> = Vec::new();
    for (_, sect) in s.doc.sections() {
        for id in [sect.headers.default, sect.headers.first, sect.headers.even, sect.footers.default, sect.footers.first, sect.footers.even]
            .into_iter()
            .flatten()
        {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    let i = ids.iter().position(|x| *x == cur).unwrap_or(0) as i32;
    let n = ids.len().max(1) as i32;
    let next = ids.get(((i + dir).rem_euclid(n)) as usize).copied().unwrap_or(cur);
    s.sel = Selection::caret(s.doc.start_of(StoryRef::Part(next)));
    sel_result(s)
}

fn select_similar(s: &mut Session, _: &Value) -> CmdResult {
    let f = s.sel.ordered().0;
    let target = s.doc.para_at(&f).map(|p| p.props_of_char(f.off).clone()).unwrap_or_default();
    let mut first: Option<Pos> = None;
    let mut last: Option<Pos> = None;
    let mut count = 0;
    for path in s.doc.para_paths(f.story) {
        let Some(p) = s.doc.para(f.story, &path) else { continue };
        for (r, c) in p.run_ranges() {
            if *c == target {
                count += 1;
                if first.is_none() {
                    first = Some(Pos { story: f.story, path: path.clone(), off: r.start });
                }
                last = Some(Pos { story: f.story, path: path.clone(), off: r.end });
            }
        }
    }
    if let (Some(a), Some(b)) = (first, last) {
        s.sel = Selection { anchor: a, focus: b };
    }
    Ok(json!({"runs": count}))
}

/// Tools › Customize Keyboard: read and change the custom keyboard shortcuts ([`crate::KeyMap`]).
/// Without params it opens the dialog; with any, it only does what they ask (no dialog).
fn customize_keyboard(s: &mut Session, v: &Value) -> CmdResult {
    let reg = s.registry.clone();
    let pair = |x: &Value| -> Result<(String, String), CmdError> { Ok((p::req_str(x, "command")?.to_string(), p::req_str(x, "key")?.to_string())) };
    let mut out = serde_json::Map::new();
    let mut asked = false;
    if p::bool(v, "reset") == Some(true) {
        asked = true;
        s.keymap.reset();
    }
    if let Some(a) = v.get("assign") {
        asked = true;
        let (id, key) = pair(a)?;
        let previous = s.keymap.assign(&reg, &id, &key).map_err(CmdError::Params)?;
        out.insert("replaced".into(), json!(previous));
    }
    if let Some(r) = v.get("remove") {
        asked = true;
        let (id, key) = pair(r)?;
        out.insert("removed".into(), json!(s.keymap.remove(&reg, &id, &key).map_err(CmdError::Params)?));
    }
    if let Some(id) = p::str(v, "command") {
        asked = true;
        if reg.get(id).is_none() {
            return Err(CmdError::Params(format!("unknown command `{id}`")));
        }
        out.insert("keys".into(), json!(s.keymap.keys_for(&reg, id)));
    }
    if let Some(key) = p::str(v, "key") {
        asked = true;
        let key = crate::keymap::normalize(key).ok_or_else(|| CmdError::Params(format!("`{key}` isn't a key (e.g. Mod+Shift+K, Alt+F7, F2)")))?;
        out.insert("assignedTo".into(), json!(s.keymap.resolve(&reg, &key).map(|c| c.id)));
        out.insert("key".into(), json!(key));
    }
    out.insert("custom".into(), serde_json::to_value(&s.keymap).unwrap_or(Value::Null));
    if !asked {
        s.ui_requests.push(json!({"open": "customizeKeyboard"}));
    }
    Ok(Value::Object(out))
}

/// `tools.search`: the commands whose label, location or id hold every word of `query`, or,
/// with no query, the Search box (`ui_requests` → the UI's command search).
fn search_commands(s: &mut Session, v: &Value) -> CmdResult {
    let Some(query) = p::str(v, "query") else {
        s.ui_requests.push(json!({"open": "commands"}));
        return Ok(json!({"matches": []}));
    };
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let matches: Vec<Value> = if words.is_empty() {
        Vec::new()
    } else {
        s.registry
            .all()
            .iter()
            .filter(|c| {
                let hay = format!("{} {} {}", c.label, c.location, c.id).to_lowercase();
                words.iter().all(|w| hay.contains(w.as_str()))
            })
            .take(50)
            .map(|c| json!({"id": c.id, "label": c.label, "location": c.location, "shortcut": c.shortcut}))
            .collect()
    };
    Ok(json!({"matches": matches}))
}

/// Tools › Customize Ribbon: read and change the ribbon's tabs, custom groups and the Quick Access
/// Toolbar ([`crate::RibbonLayout`]). Without params it opens the dialog; with any, it only does
/// what they ask (no dialog).
fn customize_ribbon(s: &mut Session, v: &Value) -> CmdResult {
    let reg = s.registry.clone();
    let l = &mut s.ribbon;
    let e = CmdError::Params;
    let str_of = |x: &Value, k: &str| x.get(k).and_then(Value::as_str).map(str::to_string);
    let req = |x: &Value, k: &str| p::req_str(x, k).map(str::to_string);
    let by = |x: &Value| x.get("by").and_then(Value::as_i64).ok_or_else(|| e("`by` (integer, e.g. -1 or 1) is required".into()));
    let mut out = serde_json::Map::new();
    let mut asked = false;
    if let Some(x) = v.get("layout") {
        asked = true;
        *l = crate::RibbonLayout::from_value(x);
        l.retain_known(&reg);
    }
    if let Some(r) = v.get("reset") {
        asked = true;
        match (r.as_bool(), r.as_str()) {
            (Some(true), _) => *l = crate::RibbonLayout::default(),
            (_, Some("ribbon")) => l.reset_ribbon(),
            (_, Some("qat")) => l.reset_qat(),
            _ => return Err(e("`reset` is true, \"ribbon\" or \"qat\"".into())),
        }
    }
    for (key, hidden) in [("hide", true), ("show", false)] {
        if let Some(tab) = p::str(v, key) {
            asked = true;
            l.set_hidden(tab, hidden).map_err(e)?;
        }
    }
    if let Some(x) = v.get("newTab") {
        asked = true;
        let name = l.new_tab(str_of(x, "name").as_deref(), str_of(x, "after").as_deref()).map_err(e)?;
        out.insert("tab".into(), json!(name));
    }
    if let Some(x) = v.get("newGroup") {
        asked = true;
        let name = l.new_group(&req(x, "tab")?, str_of(x, "name").as_deref()).map_err(e)?;
        out.insert("group".into(), json!(name));
    }
    if let Some(x) = v.get("add") {
        asked = true;
        let added = l.add(&reg, &req(x, "tab")?, &req(x, "group")?, &req(x, "command")?, str_of(x, "after").as_deref()).map_err(e)?;
        out.insert("added".into(), json!(added));
    }
    if let Some(x) = v.get("remove") {
        asked = true;
        l.remove(&req(x, "tab")?, str_of(x, "group").as_deref(), str_of(x, "command").as_deref()).map_err(e)?;
    }
    if let Some(x) = v.get("rename") {
        asked = true;
        let name = l.rename(&req(x, "tab")?, str_of(x, "group").as_deref(), &req(x, "to")?).map_err(e)?;
        out.insert("name".into(), json!(name));
    }
    if let Some(x) = v.get("move") {
        asked = true;
        l.move_by(&req(x, "tab")?, str_of(x, "group").as_deref(), str_of(x, "command").as_deref(), by(x)?).map_err(e)?;
    }
    if let Some(id) = p::str(v, "qatAdd") {
        asked = true;
        out.insert("added".into(), json!(l.qat_add(&reg, id, None).map_err(e)?));
    }
    if let Some(id) = p::str(v, "qatRemove") {
        asked = true;
        out.insert("removed".into(), json!(l.qat_remove(id)));
    }
    if let Some(x) = v.get("qatMove") {
        asked = true;
        l.qat_move(&req(x, "command")?, by(x)?).map_err(e)?;
    }
    out.insert("tabs".into(), serde_json::to_value(l.tabs().as_ref()).unwrap_or(Value::Null));
    out.insert("qat".into(), json!(l.qat()));
    if !asked {
        s.ui_requests.push(json!({"open": "customizeRibbon"}));
    }
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_marks_changes() {
        let mut s = Session::new(Document::from_text("The cat sat.\nKeep this.\nOld line."));
        let r = s.run("review.compare", &json!({"text": "The dog sat.\nKeep this.\nNew line here."})).unwrap();
        assert!(r["changes"].as_u64().unwrap() >= 2);
        s.run("review.acceptAll", &json!({})).unwrap();
        assert_eq!(s.doc.plain_text(StoryRef::Body), "The dog sat.\nKeep this.\nNew line here.");
    }

    #[test]
    fn compare_options_ignore_case_and_white_space() {
        let original = "The Cat sat.\nTwo  spaces here.";
        let revised = "The cat sat.\nTwo spaces here.";
        // By default both differences are changes.
        let mut s = Session::new(Document::from_text(original));
        let r = s.run("review.compare", &json!({"text": revised, "author": "Reviewer"})).unwrap();
        assert_eq!(r["changes"], 2);
        assert!(s.doc.revisions.iter().any(|x| x.author == "Reviewer"));
        // Ignoring case and white space: no changes, and the result reads as the revised text.
        let mut s = Session::new(Document::from_text(original));
        let r = s.run("review.compare", &json!({"text": revised, "caseChanges": false, "whiteSpace": false})).unwrap();
        assert_eq!(r["changes"], 0);
        assert_eq!(s.doc.plain_text(StoryRef::Body), revised);
        // Character level marks only the changed letters.
        let mut s = Session::new(Document::from_text("colour"));
        s.run("review.compare", &json!({"text": "color", "level": "character"})).unwrap();
        let p = s.doc.body[0].as_para().unwrap();
        let deleted: String = p.run_ranges().filter(|(_, c)| c.del.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
        assert_eq!(deleted, "u");
        // Formatting differences become formatting changes.
        let mut s = Session::new(Document::from_text("Same words."));
        let mut rev = Document::from_text("Same words.");
        rev.format_range(&rev.start_of(StoryRef::Body), &rev.end_of(StoryRef::Body), &|c| c.bold = Some(true)).unwrap();
        let path = std::env::temp_dir().join(format!("wc-compare-{}.docx", std::process::id()));
        crate::io::save_path(&path, &rev).unwrap();
        let r = s.run("review.compare", &json!({"path": path.to_string_lossy()})).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(r["changes"], 1);
        assert!(s.doc.revisions.iter().any(|x| x.kind == RevisionKind::Format));
    }

    #[test]
    fn macros_record_and_run() {
        let mut s = Session::new(Document::new());
        s.run("tools.recordMacro", &json!({"name": "hi"})).unwrap();
        s.run("text.insert", &json!({"text": "Hi "})).unwrap();
        s.run("format.bold", &json!({})).unwrap();
        s.run("tools.recordMacro", &json!({})).unwrap();
        s.run("tools.macros", &json!({"run": "hi"})).unwrap();
        assert_eq!(s.doc.plain_text(StoryRef::Body), "Hi Hi ");
        s.run("edit.repeat", &json!({})).unwrap();
    }

    #[test]
    fn autocorrect_on_space() {
        let mut s = Session::new(Document::new());
        for t in ["teh", " ", "Teh", " ", "(c)", " "] {
            s.run("text.insert", &json!({"text": t})).unwrap();
        }
        // The first word of a sentence is capitalised too.
        assert_eq!(s.doc.plain_text(StoryRef::Body), "The The © ");
    }

    #[test]
    fn protection_blocks_edits() {
        let mut s = Session::new(Document::from_text("locked"));
        s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
        assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
        s.run("review.restrict", &json!({"mode": "none"})).unwrap();
        assert!(s.run("text.insert", &json!({"text": "x"})).is_ok());
    }

    #[test]
    fn accessibility_and_quick_parts() {
        let mut s = Session::new(crate::sample::sample_document());
        let r = s.run("file.accessibility", &json!({})).unwrap();
        assert!(r["issues"].is_array());
        s.run("select.text", &json!({"text": "Membership"})).unwrap();
        s.run("insert.quickParts", &json!({"save": "m"})).unwrap();
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("insert.quickParts", &json!({"insert": "m"})).unwrap();
        assert!(s.doc.plain_text(StoryRef::Body).ends_with("Membership"));
    }

    /// #407 Language: the selection's language and "do not check", the default language and
    /// the detect preference; a bad tag is refused.
    #[test]
    fn proofing_language_options() {
        let mut s = Session::new(Document::from_text("Bonjour"));
        s.run("select.all", &json!({})).unwrap();
        s.run("review.language", &json!({"lang": "fr-FR", "noProof": true, "detect": false})).unwrap();
        let rc = s.doc.para_at(&Pos::body(0, 1)).map(|p| p.props_at(1).clone()).unwrap();
        assert_eq!((rc.lang.as_deref(), rc.no_proof), (Some("fr-FR"), Some(true)));
        assert!(!s.prefs.detect_language);
        s.run("review.language", &json!({"lang": "de-DE", "default": true})).unwrap();
        assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some("de-DE"));
        assert!(s.run("review.language", &json!({"lang": "<script>"})).is_err());
    }
}
