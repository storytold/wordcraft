//! Tools and the long tail: Repeat, macros, AutoCorrect, Compare, accessibility checker,
//! document inspector, restrict editing, versions, templates, Quick Parts, and more.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
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
    ("-->", "→"),
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
const COMPARE_PARAMS: &str = r#"{"path"?: string (revised document), "text"?: string (revised text), "original"?: string (original document; default: this one), "author"?: string (label changes with), "caseChanges"?: bool, "whiteSpace"?: bool, "formatting"?: bool (default true: differences count), "level"?: "word|character", "showIn"?: "original|revised|new" (revised/new: an untitled document), "moves"|"comments"|"tables"|"headersFooters"|"footnotes"|"textBoxes"|"fields"?: bool (accepted; not honoured yet except tables, always compared)} → {changes: n, author, notHonoured: [option], dropped: [{what: "textBoxStory|noteReference|commentAnchor|embeddedObject", count: n, why}] (what of the revised document could not be brought over)}"#;

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
    /// Undoing (or backspacing and retyping) an automatic sentence capital adds the word before
    /// it to the exceptions (First Letter's "Automatically add words to list").
    pub auto_add_first_letter: bool,
    /// The same for undoing a TWo INitial CApitals fix (INitial CAps's switch).
    pub auto_add_initial_caps: bool,
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
            auto_add_first_letter: true,
            auto_add_initial_caps: true,
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
                *a == word || (a.chars().all(char::is_lowercase) && word.to_lowercase() == *a && word.chars().next().is_some_and(char::is_uppercase))
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

    /// Add a First Letter exception (lowercase; one the built-in list has just stops being deleted).
    fn add_exception(&mut self, e: String) {
        if !plain_word(&e) {
            return;
        }
        self.exceptions_removed.retain(|r| *r != e);
        if !NOT_SENTENCE_END.contains(&e.as_str()) && !self.exceptions.contains(&e) && self.exceptions.len() < MAX_ENTRIES {
            self.exceptions.push(e);
        }
    }

    /// Add a word that keeps its two initial capitals.
    fn add_initial_caps_exception(&mut self, w: &str) {
        if !plain_word(w) {
            return;
        }
        if !INITIAL_CAPS_EXCEPTIONS.contains(&w)
            && !self.initial_caps_exceptions.iter().any(|e| e == w)
            && self.initial_caps_exceptions.len() < MAX_ENTRIES
        {
            self.initial_caps_exceptions.push(w.to_string());
        }
    }

    /// Abbreviations after which a sentence doesn't end, lowercase.
    pub fn exception_list(&self) -> Vec<String> {
        let mut v: Vec<String> = NOT_SENTENCE_END
            .iter()
            .filter(|a| !self.exceptions_removed.iter().any(|r| r.eq_ignore_ascii_case(a)))
            .map(ToString::to_string)
            .chain(self.exceptions.iter().map(|a| a.to_lowercase()))
            .collect();
        v.sort();
        v.dedup();
        v
    }
}

/// Is `w` a word an exception list may hold: one short line, no spaces or object characters?
fn plain_word(w: &str) -> bool {
    !w.trim().is_empty() && w.chars().count() <= MAX_FROM && !w.contains(|c: char| c.is_whitespace() || c == '\u{FFFC}')
}

/// The newest automatic capitalisation (a sentence capital, a TWo INitial CApitals fix), kept so
/// that undoing it, or backspacing it and typing the word again, makes an exception (#1857).
#[derive(Clone, Debug)]
pub struct AutoCap {
    /// Undo steps pushed (ever) right after the change (`u64::MAX` until the caller has pushed its step).
    pub(crate) depth: u64,
    story: wordcraft_doc::StoryRef,
    path: wordcraft_doc::Path,
    /// Byte offset of the word in its paragraph.
    start: usize,
    /// The token before a sentence capital's word, lowercase ("there."); empty when none or when
    /// the capital wasn't a sentence one.
    prev: String,
    /// The TWo INitial CApitals word as typed ("TWo").
    two_caps: Option<String>,
    /// The user backspaced into the word after the change.
    backspaced: bool,
}

impl AutoCap {
    pub(crate) fn add_exceptions(self, ac: &mut AutoCorrectPrefs) {
        if ac.auto_add_first_letter && !self.prev.is_empty() {
            ac.add_exception(self.prev);
        }
        if ac.auto_add_initial_caps
            && let Some(w) = self.two_caps
        {
            ac.add_initial_caps_exception(&w);
        }
    }

    /// A Backspace deleted `a..f`; it reached back over the capitalised word's first letter.
    pub(crate) fn note_backspace(&mut self, a: &Pos, f: &Pos) {
        if f.story == self.story && f.path == self.path && a.off <= self.start && f.off > self.start {
            self.backspaced = true;
        }
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
        ("autoAddFirstLetter", &mut ac.auto_add_first_letter),
        ("autoAddInitialCaps", &mut ac.auto_add_initial_caps),
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
        ac.add_exception(e);
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
        ac.add_initial_caps_exception(w);
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
        "autoAddFirstLetter": ac.auto_add_first_letter,
        "autoAddInitialCaps": ac.auto_add_initial_caps,
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
                r#"{"enabled"?: bool (all of AutoCorrect), "replaceText"|"twoInitialCaps"|"capSentences"|"capCells"|"capDays"|"capsLock"|"smartQuotes"|"fractions"|"ordinals"|"dashes"|"links"|"bullets"|"numbering"|"borderLines"?: bool, "autoAddFirstLetter"|"autoAddInitialCaps"?: bool (Automatically add words to the exception lists when an automatic capital is undone; default true), "add"?: {"from": string, "to": string} (adds or replaces an entry), "delete"?: string (an entry's "from", built in or added), "addException"?: string, "deleteException"?: string, "addInitialCapsException"?: string (keeps TWo INitial CApitals, like "IDs"), "deleteInitialCapsException"?: string} → options, entries and exceptions"#,
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
            .params(r#"{"remove"?: ["comments", "revisions", "properties", "hidden", "headers"]} → {found: {comments, revisions, properties, hiddenText, hiddenCount, hidden: [{story: "body|header|footer|footnote|endnote|comment|textBox", part?, text}] (at most 1000), truncated, unsupported: [{story, object: "table|opaque", ..}] (hidden text not inspected: table styles, verbatim objects), headers}, removed, residual: [..] (hidden text a removal couldn't reach), removedNotes: [{kind, part, text}] (notes whose hidden reference was removed)}; whole hidden paragraphs go with their marks; the removals are refused while the document tracks changes"#),
        CommandSpec::new("file.protect", "Protect Document", "File › Info", |s, v| {
            let mode = p::str(v, "mode").unwrap_or("readOnly");
            s.run("review.restrict", &json!({"mode": mode}))
        })
        .params(r#"{"mode": "none|readOnly|comments|trackedChanges"} (starting enforcement; "none" doesn't stop a protected document — use review.restrict {"stop": true})"#),
        CommandSpec::new("file.versions", "Version History", "File › Info", |s, v| {
            if let Some(label) = p::str(v, "save") {
                s.versions.push((label.to_string(), super::now_iso(), s.doc.clone()));
                if s.versions.len() > 50 {
                    s.versions.remove(0);
                }
            }
            if let Some(i) = p::u64(v, "restore") {
                let (_, _, d) = s.versions.get(i as usize).cloned().ok_or_else(|| CmdError::Params("no such version".into()))?;
                // The restore replaces the whole document, including the content the protection
                // kept read-only, so the protection is stopped first (with its password, when it
                // has one) — a restore must not go around it.
                super::protect::check_version_restore(s)?;
                s.checkpoint("Restore Version");
                // A snapshot's own protection is stale: it was taken at some earlier moment, and
                // restoring it must not bring a protection back that the live document doesn't
                // have (#1327 follow-up).
                let protection = s.doc.settings.protection.clone();
                let protect_hash = s.doc.settings.protect_hash.clone();
                let protect_formatting = s.doc.settings.protect_formatting;
                s.doc = d;
                s.doc.settings.protection = protection;
                s.doc.settings.protect_hash = protect_hash;
                s.doc.settings.protect_formatting = protect_formatting;
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
            let mut doc = crate::io::open_path(std::path::Path::new(path)).map_err(CmdError::Failed)?;
            // As in Word, the new document is attached to the template it came from (#2852).
            doc.settings.attached_template = Some(path.to_string());
            s.set_document(doc);
            s.path = None;
            s.dirty = true;
            sel_result(s)
        })
        .params(r#"{"path": string} → a new, untitled document from the template, attached to it"#)
        .pure(),
        CommandSpec::new("file.saveTemplate", "Save as Template", "File › Save As", |s, v| {
            let path = p::req_str(v, "path")?;
            // A .dotm keeps its name (#2852); anything else becomes a .dotx.
            let path = if super::user_templates::is_template_name(path) { path.to_string() } else { format!("{path}.dotx") };
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
            s.push_ui_request(json!({"open": "saveAs"}));
            Ok(json!({"hint": "Save or export a copy (PDF, Word document) to share it."}))
        })
        .pure(),
        CommandSpec::new("insert.quickParts", "Quick Parts", "Insert › Text", |s, v| quick_parts(s, v, wordcraft_doc::QUICK_PARTS_GALLERY)).params(
            r#"{"save"?: name (from the selection), "in"?: "document" (save into the document's own building blocks, kept in its file; else this session's), "insert"?: name, "delete"?: name} → {"parts": names (this session's, then the document's Quick Parts), "document": the document's Quick Parts}"#,
        ),
        CommandSpec::new("insert.autoText", "AutoText", "Insert › Text › Quick Parts", |s, v| quick_parts(s, v, wordcraft_doc::AUTO_TEXT_GALLERY))
            .key(super::non_mac("Alt+F3"))
            .params(r#"{"save"?: name, "in"?: "document", "insert"?: name, "delete"?: name} → {"parts": names, "document": the document's AutoText}"#),
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
            let story = s.sel.focus.story;
            let current = {
                let table = s.doc.table(story, &tp).ok_or_else(|| CmdError::Failed("no table".into()))?;
                let style = table.props.style.as_deref().and_then(|id| s.doc.styles.table_style(id));
                table.props.cell_margins.or(style.and_then(|st| st.parts.cell_margins)).unwrap_or([0.0, 5.4, 0.0, 5.4])
            };
            let m = [
                p::f32(v, "top").unwrap_or(current[0]),
                p::f32(v, "left").unwrap_or(current[1]),
                p::f32(v, "bottom").unwrap_or(current[2]),
                p::f32(v, "right").unwrap_or(current[3]),
            ]
            .map(|x| x.clamp(0.0, 144.0));
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
            r#"{"lang"?: "en-US|en-GB|pl-PL|fr-FR|…" (BCP 47; default en-US), "noProof"?: bool, "detect"?: bool (detect language automatically, a preference), "default"?: bool (also the document's default and the saved language for future blank documents)}"#,
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
/// or (`default`) the document's default and the saved language for future blank documents;
/// `detect` is the per-user detection preference.
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
    let result = super::format::apply(s, &|c| {
        if let Some(l) = &lang {
            c.lang = Some(l.clone());
        }
        if let Some(np) = no_proof {
            c.no_proof = Some(np);
        }
    })?;
    if p::bool(v, "default") == Some(true) {
        s.prefs.default_language = lang;
    }
    Ok(result)
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
    // The word's language (run, paragraph style, document default) picks the replacement list
    // (#835) and the case mapping (#1923). Text with no language at all is in the editing
    // language (a Polish interface writes Polish), else English.
    let lang: Option<String> = s.doc.para_at(&f).and_then(|p| {
        let rc = s.doc.styles.resolve_char(p.props.style.as_deref(), p.props_of_char(start));
        rc.lang.clone()
    });
    let lang = lang.or_else(|| s.editing_lang.clone());
    let turkic = is_turkic(lang.as_deref());
    // TWo INitial CApitals and cAPS LOCK come first, so "TEh" is still a replacement's typo.
    if let Some((at, reps)) = case_fix(&ac, &word, turkic) {
        let para = s.doc.para_mut(f.story, &f.path)?;
        let letters = word.get(at..).unwrap_or("").chars().take(reps.len()).collect::<String>();
        let mut end = start + at + letters.len();
        for (c, rep) in letters.chars().rev().zip(reps.iter().rev()) {
            let from = end.saturating_sub(c.len_utf8());
            if rep.chars().ne(std::iter::once(c)) {
                let props = para.props_of_char(from).clone().without_symbol();
                para.delete(from, end)?;
                para.insert_text(from, rep, &props)?;
            }
            end = from;
        }
        s.autocap = Some(AutoCap {
            depth: u64::MAX,
            story: f.story,
            path: f.path.clone(),
            start,
            prev: String::new(),
            two_caps: (ac.two_initial_caps
                && letters.chars().nth(1).is_some_and(char::is_uppercase)
                && letters.chars().next().is_some_and(char::is_uppercase))
            .then(|| letters.clone()),
            backspaced: false,
        });
        let fixed: String = reps.concat();
        f.off = (f.off + fixed.len()).saturating_sub(letters.len());
        s.sel = Selection::caret(f.clone());
        word = format!("{}{fixed}{}", word.get(..at).unwrap_or(""), word.get(at + letters.len()..).unwrap_or(""));
    }
    // A whole-word replacement, else "wait..." → "wait…". Proofing decides what is English the
    // same way (#835); every English (en-GB, en-CA…) counts (#1281).
    let english = s.doc.para_at(&f).is_none_or(|_| wordcraft_proof::ProofLang::from_tag(lang.as_deref()).is_english());
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
    let replaced = rep.is_some();
    let mut caret = f.off;
    if let Some(rep) = rep {
        let end = start + word.len();
        let para = s.doc.para_mut(f.story, &f.path)?;
        // The replacement is text: a symbol it replaces doesn't give it its symbol font (#2036).
        let props = para.props_of_char(from).clone().without_symbol();
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
    let mut sentence = sentence_on && starts_sentence(before_word, &exceptions) && should_capitalize(first, next);
    let prev = if sentence { prev_token(before_word) } else { String::new() };
    // Typing the word again after backspacing an automatic capital: the user doesn't want it
    // (Word's "Automatically add words to list"); an exception, so no capital this time either.
    if sentence
        && !prev.is_empty()
        && s.autocap.take_if(|c| c.backspaced && c.story == f.story && c.path == f.path && c.start == start && c.prev == prev).is_some()
        && ac.auto_add_first_letter
    {
        s.prefs.autocorrect.add_exception(prev.clone());
        sentence = false;
    }
    let day = ac.cap_days && DAYS.contains(&first);
    if sentence || day {
        let para = s.doc.para_mut(f.story, &f.path)?;
        if let Some(c) = word.chars().next() {
            let up = upper_char(c, turkic);
            let props = para.props_of_char(start).clone().without_symbol();
            para.delete(start, start + c.len_utf8())?;
            para.insert_text(start, &up, &props)?;
            let off = (caret + up.len()).saturating_sub(c.len_utf8());
            s.sel = Selection::caret(Pos { off, ..f.clone() });
            if sentence && !prev.is_empty() && !replaced {
                match s.autocap.as_mut().filter(|c| c.depth == u64::MAX) {
                    Some(c) => c.prev = prev,
                    None => {
                        s.autocap = Some(AutoCap { depth: u64::MAX, story: f.story, path: f.path, start, prev, two_caps: None, backspaced: false })
                    }
                }
            }
        }
    }
    Ok(())
}

/// The lowercase token before a word that starts a sentence ("there." in "there. world"), as
/// [`starts_sentence`] reads it; empty at the start of a paragraph.
fn prev_token(before: &str) -> String {
    let t = before.trim_end().trim_end_matches(|c: char| "\"'”’)]".contains(c));
    // Only full stops: Word's exception was observed with "there." (`there?` isn't added).
    if !t.ends_with('.') {
        return String::new();
    }
    t.rsplit(char::is_whitespace).next().unwrap_or("").trim_start_matches(|c: char| "\"'“‘([".contains(c)).to_lowercase()
}

/// Turkish, Azerbaijani (and Crimean Tatar) map `i`/`İ` and `ı`/`I` as pairs (#1923).
pub(crate) fn is_turkic(lang: Option<&str>) -> bool {
    if wordcraft_proof::ProofLang::from_tag(lang) == wordcraft_proof::ProofLang::Tr {
        return true; // tr, tur, any region, same normalisation as proofing
    }
    let Some(tag) = lang.map(str::trim) else { return false };
    let primary = tag.split(['-', '_']).next().unwrap_or("");
    ["az", "aze", "crh"].iter().any(|c| primary.eq_ignore_ascii_case(c))
}

/// A letter in capitals: `i` → `İ` in Turkic languages, otherwise the Unicode default.
pub(crate) fn upper_char(c: char, turkic: bool) -> String {
    if turkic && c == 'i' {
        return "\u{130}".to_string();
    }
    c.to_uppercase().collect()
}

/// A letter in lowercase: `I` → `ı` in Turkic languages. `İ` is always the single letter `i`,
/// never `i` plus a combining dot.
pub(crate) fn lower_char(c: char, turkic: bool) -> String {
    match c {
        'I' if turkic => "\u{131}".to_string(),
        '\u{130}' => "i".to_string(),
        _ => c.to_lowercase().collect(),
    }
}

/// Built-in words that keep their two initial capitals (plurals of two-letter abbreviations).
pub const INITIAL_CAPS_EXCEPTIONS: &[&str] = &["CDs", "IDs", "PCs", "TVs", "DJs", "GBs", "KBs", "MBs", "TBs", "VMs"];

/// Words with a lowercase first letter and the rest in capitals that aren't cAPS LOCK slips.
const CAPS_LOCK_EXCEPTIONS: &[&str] = &["iOS", "mRNA", "tRNA", "rRNA", "cDNA", "eSIM", "eSATA", "iSCSI", "vCPU"];

/// The capitalisation fix for a finished word: TWo INitial CApitals ("WOrd" → "Word") or cAPS
/// LOCK ("wORD" → "Word"). Returns the byte offset of the word's letters and each letter's
/// replacement. Only words of three or more letters, optionally in quotes or brackets.
fn case_fix(caps: &AutoCorrectPrefs, word: &str, turkic: bool) -> Option<(usize, Vec<String>)> {
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
    let lower = |c: char| lower_char(c, turkic);
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
        let up = upper_char(a, turkic);
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
    compare_with(s, revised, v)
}

/// [`compare`] with the revised document already open.
fn compare_with(s: &mut Session, revised: Document, v: &Value) -> CmdResult {
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
    // The revised content, pointing at our copies of the pictures, lists, styles, notes and
    // comments it refers to (#1317).
    let mut dropped: Vec<Value> = Vec::new();
    // What the original holds before anything is brought over: copies made for content that turns
    // out unchanged are removed again, and only embedded objects not from the original are checked.
    let parts_before: BTreeSet<u32> = s.doc.parts.keys().copied().collect();
    let comments_before: BTreeSet<u32> = s.doc.comments.keys().copied().collect();
    let originals = embedded_ptrs(&s.doc);
    let revised_parts = revised.passthrough.clone();
    let new = import_revised(&mut s.doc, revised, &mut dropped);
    let old: Vec<Arc<Block>> = s.doc.body.clone();
    s.doc.revisions.push(wordcraft_doc::Revision { kind: RevisionKind::Insert, author: author.clone(), date: date.clone(), move_name: None });
    let ins = (s.doc.revisions.len() - 1) as u32;
    s.doc.revisions.push(wordcraft_doc::Revision { kind: RevisionKind::Delete, author: author.clone(), date: date.clone(), move_name: None });
    let del = (s.doc.revisions.len() - 1) as u32;
    // Formatting changes are recorded here while the document is read for the comparison.
    let mut revisions = std::mem::take(&mut s.doc.revisions);
    // Comments of the revised document matched to the original's by one of their marks: the copy's
    // other marks are pointed at the original, so a comment whose range changed stays one.
    let mut merged: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
    // Paragraphs match by their text and objects (by what they show, never by id); tables as a
    // whole, by their text and objects and, when formatting counts, their formatting.
    let doc = &s.doc;
    let key = |b: &Arc<Block>| match &**b {
        Block::Para(p) => para_key(doc, p, &opts),
        Block::Table(_) => block_key(doc, b, &opts, 0),
    };
    let ops = lcs_ops(&old.iter().map(key).collect::<Vec<_>>(), &new.iter().map(key).collect::<Vec<_>>());
    let mut out: Vec<(Arc<Block>, Fate)> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    let mut changes = 0usize;
    let mut k = 0;
    while k < ops.len() {
        match ops.get(k) {
            Some(Op::Same) => {
                let (a, b) = (old.get(i), new.get(j));
                // The revised block, with its formatting changes tracked.
                if let (Some(Block::Para(pa)), Some(Block::Para(pb))) = (a.map(|x| &**x), b.map(|x| &**x)) {
                    // The same objects (they match one for one): the original's, so its notes,
                    // comments and text boxes stay themselves rather than copies.
                    let mut base = pb.clone();
                    if base.objects.len() == pa.objects.len() {
                        pair_comments(&base.objects, &pa.objects, &mut merged);
                        base.objects = pa.objects.clone();
                    }
                    keep_sections(&mut base, Some(pa));
                    let mut p = base.clone();
                    if opts.formatting && pa.text == pb.text {
                        let before = revisions.len();
                        super::fmt_revisions::compare_formatting(pa, &mut p, &mut revisions, &author, &date);
                        if revisions.len() > before || p != base {
                            changes += 1;
                        }
                    }
                    out.push((para_block(p), Fate::Kept));
                } else if let (Some(x), Some(y)) = (a, b) {
                    // A table the same as compared: the original's when its text is the same too
                    // (so its notes and text boxes stay themselves), else the revised one.
                    out.push((if block_text(x, 0) == block_text(y, 0) { x.clone() } else { y.clone() }, Fate::Kept));
                } else if let Some(x) = b.or(a) {
                    out.push((x.clone(), Fate::Kept));
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
                        (Some(Block::Para(pa)), Some(Block::Para(pb))) => {
                            let mut p = word_diff(doc, pa, pb, (ins, del), &opts, (&mut revisions, &author, &date, &mut merged));
                            keep_sections(&mut p, Some(pa));
                            out.push((para_block(p), Fate::Kept));
                        }
                        _ => {
                            if let Some(x) = a {
                                out.push((Arc::new(mark_whole(x, Fate::Deleted, del)), Fate::Deleted));
                            }
                            if let Some(x) = b {
                                let mut x = mark_whole(x, Fate::Inserted, ins);
                                if let Block::Para(p) = &mut x {
                                    keep_sections(p, None);
                                }
                                out.push((Arc::new(x), Fate::Inserted));
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
    s.doc.revisions = revisions;
    let mut body = mark_paragraph_marks(out, ins, del);
    if !merged.is_empty() {
        for b in &mut body {
            let untouched = matches!(&**b, Block::Para(p) if !p.objects.iter().any(|o| comment_mark(o).is_some_and(|id| merged.contains_key(&id))));
            if untouched {
                continue;
            }
            super::paste::each_para(Arc::make_mut(b), 0, &mut |p| {
                for o in &mut p.objects {
                    if let InlineObject::CommentStart { id } | InlineObject::CommentEnd { id } = o
                        && let Some(to) = merged.get(id)
                    {
                        *id = *to;
                    }
                }
            });
        }
    }
    s.doc.body = body;
    s.doc.ensure_nonempty();
    drop_unused_copies(&mut s.doc, &parts_before, &comments_before);
    let lost = drop_foreign_embedded(&mut s.doc, &originals, &revised_parts);
    if lost > 0 {
        dropped.push(json!({"what": "embeddedObject", "count": lost, "why": "a chart, diagram, ink or embedded object whose package parts can't be brought over (a part of the original has the same name) keeps only its picture"}));
    }
    if show_in != "original" || p::str(v, "original").is_some() {
        // The result is a new, untitled document: saving it never overwrites either file.
        s.path = None;
        s.file_name = None;
        // Version snapshots belong to the document they were taken from, including when
        // Compare creates an untitled result from the current document.
        s.clear_versions();
    }
    s.sel = Selection::caret(s.doc.start_of(StoryRef::Body));
    Ok(json!({"changes": changes, "author": author, "notHonoured": not_honoured, "dropped": dropped}))
}

/// The comment a comment mark belongs to.
fn comment_mark(o: &InlineObject) -> Option<u32> {
    match o {
        InlineObject::CommentStart { id } | InlineObject::CommentEnd { id } => Some(*id),
        _ => None,
    }
}

/// Note which revised comments (copies) the original's comment marks matched, one for one.
fn pair_comments(revised: &[InlineObject], original: &[InlineObject], merged: &mut std::collections::BTreeMap<u32, u32>) {
    for (r, o) in revised.iter().zip(original) {
        if let (Some(r), Some(o)) = (comment_mark(r), comment_mark(o))
            && r != o
        {
            merged.insert(r, o);
        }
    }
}

/// What became of a block of the compared document: kept (perhaps changed inside), or inserted
/// or deleted whole.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fate {
    Kept,
    Inserted,
    Deleted,
}

/// A block inserted or deleted whole: every run marked (in tables, every cell's) and, for a
/// table, its rows, so resolving the change removes the table rather than emptying it.
fn mark_whole(b: &Block, fate: Fate, rid: u32) -> Block {
    let ins = fate == Fate::Inserted;
    let mut b = mark_block(b, &|c| if ins { c.ins = Some(rid) } else { c.del = Some(rid) }, 0);
    if let Block::Table(t) = &mut b {
        for r in &mut t.rows {
            if ins {
                r.props.ins = Some(rid);
            } else {
                r.props.del = Some(rid);
            }
        }
    }
    b
}

/// The compared blocks, with the paragraph marks of paragraphs inserted or deleted whole marked
/// too (ECMA-376 Part 1 §17.13.5.15, §17.13.5.17), so Accept All and Reject All remove those
/// paragraphs instead of leaving them empty. A container's last paragraph mark can't go, so for
/// whole paragraphs at the end the mark before each one carries its change instead.
fn mark_paragraph_marks(out: Vec<(Arc<Block>, Fate)>, ins: u32, del: u32) -> Vec<Arc<Block>> {
    let whole = |(b, f): &(Arc<Block>, Fate)| *f != Fate::Kept && matches!(**b, Block::Para(_));
    let tail = out.len() - out.iter().rev().take_while(|x| whole(x)).count();
    let fates: Vec<Fate> = out.iter().map(|(_, f)| *f).collect();
    let mut blocks: Vec<Arc<Block>> = out.into_iter().map(|(b, _)| b).collect();
    let set = |b: Option<&mut Arc<Block>>, f: Fate| {
        if let Some(b) = b
            && let Block::Para(p) = Arc::make_mut(b)
        {
            match f {
                Fate::Inserted => p.mark.ins = Some(ins),
                Fate::Deleted => p.mark.del = Some(del),
                Fate::Kept => {}
            }
        }
    };
    for (i, f) in fates.iter().enumerate() {
        if *f == Fate::Kept || !matches!(blocks.get(i).map(|b| &**b), Some(Block::Para(_))) {
            continue;
        }
        if i < tail {
            set(blocks.get_mut(i), *f);
        } else if let Some(prev) = i.checked_sub(1) {
            set(blocks.get_mut(prev), *f);
        }
    }
    blocks
}

/// A section break from the revised document keeps the original's headers and footers (the
/// revised document's aren't brought over: comparing them is not done), or, where the original
/// had none, links to the previous section's, as pasting does.
fn keep_sections(p: &mut Paragraph, original: Option<&Paragraph>) {
    let Some(sec) = p.section.as_deref_mut() else { return };
    match original.and_then(|o| o.section.as_deref()) {
        Some(o) => {
            sec.headers = o.headers;
            sec.footers = o.footers;
            sec.title_page = o.title_page;
        }
        None => {
            sec.headers = Default::default();
            sec.footers = Default::default();
            sec.title_page = false;
        }
    }
}

/// The revised document's body as content of `doc`: its pictures, lists, styles and revisions
/// brought over with ids that can't collide with `doc`'s (the importer paste uses), its text
/// boxes, notes and comments given stories of their own in `doc`. What can't be brought over
/// (a reference to a note or comment the revised document lacks) is left out and listed in
/// `dropped`.
fn import_revised(doc: &mut Document, revised: Document, dropped: &mut Vec<Value>) -> Vec<Arc<Block>> {
    use std::collections::BTreeMap;
    use wordcraft_doc::{Part, PartKind};
    let im = super::paste::import_resources(doc, &revised, true);
    let mut blocks: Vec<Block> = revised.body.iter().map(|b| (**b).clone()).collect();
    for b in &mut blocks {
        im.remap_block(b);
    }
    // Text boxes get stories of their own (their content pointed at our copies too); a box
    // whose story the revised document lacks shows none rather than one of ours.
    let mut boxes: BTreeMap<u32, Part> = revised.parts.iter().filter(|(_, p)| p.kind == PartKind::TextBox).map(|(k, p)| (*k, p.clone())).collect();
    for part in boxes.values_mut() {
        for b in &mut part.blocks {
            im.remap_block(Arc::make_mut(b));
        }
    }
    let mut lost_boxes = 0usize;
    for b in &mut blocks {
        super::paste::each_para(b, 0, &mut |p| {
            for o in &mut p.objects {
                for story in o.text_box_slots() {
                    if story.is_some_and(|id| !boxes.contains_key(&id)) {
                        *story = None;
                        lost_boxes += 1;
                    }
                }
            }
        });
    }
    doc.adopt_text_box_stories(&mut blocks, &boxes);
    let mut refs = Refs { doc, src: &revised, im: &im, notes: BTreeMap::new(), comments: BTreeMap::new() };
    let (mut lost_notes, mut lost_comments) = (0usize, 0usize);
    for b in &mut blocks {
        super::paste::each_para(b, 0, &mut |p| {
            let mut gone = Vec::new();
            for (k, o) in p.objects.iter_mut().enumerate() {
                // Whether the reference is lost: `Some(true)` a note's, `Some(false)` a comment's.
                let lost = match o {
                    InlineObject::NoteRef { id, .. } => refs.note(*id).map(|n| *id = n).is_none().then_some(true),
                    InlineObject::CommentStart { id } | InlineObject::CommentEnd { id } => {
                        refs.comment(*id).map(|n| *id = n).is_none().then_some(false)
                    }
                    _ => None,
                };
                match lost {
                    Some(true) => lost_notes += 1,
                    Some(false) => lost_comments += 1,
                    None => continue,
                }
                gone.push(k);
            }
            if !gone.is_empty() {
                let offsets = p.object_offsets();
                for k in gone.iter().rev() {
                    if let Some(off) = offsets.get(*k) {
                        let _ = p.delete(*off, off + wordcraft_doc::para::OBJ.len_utf8());
                    }
                }
            }
        });
    }
    for (what, count, why) in [
        ("textBoxStory", lost_boxes, "a text box whose text the revised document lacks shows none"),
        ("noteReference", lost_notes, "a note reference whose note the revised document lacks"),
        ("commentAnchor", lost_comments, "a comment mark whose comment the revised document lacks"),
    ] {
        if count > 0 {
            dropped.push(json!({"what": what, "count": count, "why": why}));
        }
    }
    blocks.into_iter().map(Arc::new).collect()
}

/// Footnotes, endnotes and comments of the revised document copied into ours as they are
/// referenced (each once), by their ids there.
struct Refs<'a> {
    doc: &'a mut Document,
    src: &'a Document,
    im: &'a super::paste::Imported,
    notes: std::collections::BTreeMap<u32, Option<u32>>,
    comments: std::collections::BTreeMap<u32, Option<u32>>,
}

impl Refs<'_> {
    /// A copy of the source's story `id` when it is one of `kinds`; its content points at our
    /// copies of what it refers to.
    fn part(&mut self, id: u32, kinds: &[wordcraft_doc::PartKind]) -> Option<u32> {
        let part = self.src.parts.get(&id).filter(|p| kinds.contains(&p.kind))?;
        let mut blocks = part.blocks.clone();
        for b in &mut blocks {
            self.im.remap_block(Arc::make_mut(b));
        }
        self.doc.try_add_part(part.kind, blocks).ok()
    }

    /// Our id for the source's footnote or endnote story `id`.
    fn note(&mut self, id: u32) -> Option<u32> {
        use wordcraft_doc::PartKind;
        if let Some(n) = self.notes.get(&id) {
            return *n;
        }
        let n = self.part(id, &[PartKind::Footnote, PartKind::Endnote]);
        self.notes.insert(id, n);
        n
    }

    /// Our id for the source's comment `id` (its text copied, a reply kept a reply when its
    /// parent came first).
    fn comment(&mut self, id: u32) -> Option<u32> {
        if let Some(n) = self.comments.get(&id) {
            return *n;
        }
        let n = self.src.comments.get(&id).cloned().and_then(|c| {
            let part = self.part(c.part, &[wordcraft_doc::PartKind::Comment])?;
            let new = match self.doc.comments.keys().next_back() {
                None => Some(0),
                Some(k) => k.checked_add(1),
            };
            // At the top of the range: the first free id (there is one below the number of comments).
            let new = new.or_else(|| (0..=u32::MAX).find(|i| !self.doc.comments.contains_key(i)))?;
            let parent = c.parent.and_then(|p| self.comments.get(&p).copied().flatten());
            self.doc.comments.insert(new, wordcraft_doc::Comment { part, parent, durable_id: None, ..c });
            Some(new)
        });
        self.comments.insert(id, n);
        n
    }
}

/// A paragraph's text as compared: objects stand for what they show (a different picture or
/// field is a change even where the text around it is the same).
fn para_key(doc: &Document, p: &Paragraph, opts: &CompareOptions) -> String {
    let mut out = String::with_capacity(p.text.len());
    let mut k = 0;
    for c in p.text.chars() {
        if c == wordcraft_doc::para::OBJ {
            out.push_str(&object_key(doc, p.objects.get(k)));
            k += 1;
        } else {
            out.push(c);
        }
    }
    opts.key(&out)
}

/// How an object compares: by what it shows, never by the ids that point at its parts (a
/// footnote by its text, a comment mark by its comment's author and text, a text box by its
/// text), and without the package parts an embedded object was read with.
fn object_key(doc: &Document, o: Option<&InlineObject>) -> String {
    let story = |id: u32| doc.plain_text(StoryRef::Part(id));
    let comment = |id: &u32| doc.comments.get(id).map(|c| format!("{}\u{1}{}", c.author, story(c.part))).unwrap_or_default();
    let body = match o {
        None => String::new(),
        Some(InlineObject::NoteRef { kind, id, custom }) => format!("note\u{1}{kind:?}\u{1}{custom}\u{1}{}", story(*id)),
        Some(InlineObject::CommentStart { id }) => format!("commentStart\u{1}{}", comment(id)),
        Some(InlineObject::CommentEnd { id }) => format!("commentEnd\u{1}{}", comment(id)),
        Some(o) => {
            let mut x = o.clone();
            let boxes: Vec<String> = x.text_boxes().into_iter().map(story).collect();
            for slot in x.text_box_slots() {
                *slot = None;
            }
            strip_embedded(&mut x, 0);
            format!("{}\u{1}{}", serde_json::to_string(&x).unwrap_or_default(), boxes.join("\u{1}"))
        }
    };
    format!("\u{FFFC}{body}\u{FFFC}")
}

/// `o` without the package parts it was read with (an OLE object's, a chart's, diagram's or ink's).
fn strip_embedded(o: &mut InlineObject, depth: usize) {
    match o {
        InlineObject::Image { ole, .. } => *ole = None,
        InlineObject::Graphic { graphic, .. } if graphic.source.is_some() => Arc::make_mut(graphic).source = None,
        InlineObject::Group { children, .. } if depth < MAX_KEY_DEPTH => {
            for c in children {
                strip_embedded(&mut c.obj, depth + 1);
            }
        }
        _ => {}
    }
}

/// Deepest group or table nesting a comparison key follows.
const MAX_KEY_DEPTH: usize = 16;

/// A block as compared inside a table: its text and objects and, when formatting counts, its
/// paragraph, run, cell, row and table formatting (a table matched as unchanged is not diffed
/// further, so every difference has to show here).
fn block_key(doc: &Document, b: &Block, opts: &CompareOptions, depth: usize) -> String {
    let json = |v: Result<String, serde_json::Error>| if opts.formatting { v.unwrap_or_default() } else { String::new() };
    match b {
        Block::Para(p) => {
            let runs: Vec<(usize, CharProps)> = p.runs.iter().map(|r| (r.len, r.props.formatting())).collect();
            format!(
                "{}\u{2}{}\u{2}{}",
                para_key(doc, p, opts),
                json(serde_json::to_string(&p.props.formatting())),
                json(serde_json::to_string(&runs))
            )
        }
        Block::Table(t) if depth < MAX_KEY_DEPTH => {
            let mut out =
                format!("\u{0}table\u{0}{}", json(serde_json::to_string(&wordcraft_doc::props::TableProps { fmt_change: None, ..t.props.clone() })));
            for r in &t.rows {
                let props = wordcraft_doc::props::RowProps { fmt_change: None, ins: None, del: None, ..r.props.clone() };
                out.push_str(&format!("\u{3}{}", json(serde_json::to_string(&props))));
                for c in &r.cells {
                    out.push_str(&format!(
                        "\u{4}{}",
                        json(serde_json::to_string(&wordcraft_doc::props::CellProps { fmt_change: None, ..c.props.clone() }))
                    ));
                    for x in &c.blocks {
                        out.push_str(&format!("\u{5}{}", block_key(doc, x, opts, depth + 1)));
                    }
                }
            }
            out
        }
        Block::Table(_) => String::new(),
    }
}

/// The embedded objects (their kept package parts) a document's stories hold, by address: the
/// original's own, which never need checking.
fn embedded_ptrs(doc: &Document) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    each_object(doc, &mut |o| {
        for e in embedded_of(o, 0) {
            out.insert(Arc::as_ptr(e) as usize);
        }
    });
    out
}

/// Every object in every story of `doc` (tables included, bounded depth).
fn each_object(doc: &Document, f: &mut dyn FnMut(&InlineObject)) {
    fn walk(blocks: &[Arc<Block>], depth: usize, f: &mut dyn FnMut(&InlineObject)) {
        for b in blocks {
            match &**b {
                Block::Para(p) => p.objects.iter().for_each(&mut *f),
                Block::Table(t) if depth < MAX_KEY_DEPTH => {
                    for c in t.rows.iter().flat_map(|r| r.cells.iter()) {
                        walk(&c.blocks, depth + 1, f);
                    }
                }
                Block::Table(_) => {}
            }
        }
    }
    walk(&doc.body, 0, f);
    for part in doc.parts.values() {
        walk(&part.blocks, 0, f);
    }
}

/// The kept package parts of an object and its group members.
fn embedded_of(o: &InlineObject, depth: usize) -> Vec<&Arc<wordcraft_doc::graphic::Embedded>> {
    match o {
        InlineObject::Image { ole: Some(e), .. } => vec![e],
        InlineObject::Graphic { graphic, .. } => graphic.source.iter().collect(),
        InlineObject::Group { children, .. } if depth < MAX_KEY_DEPTH => children.iter().flat_map(|c| embedded_of(&c.obj, depth + 1)).collect(),
        _ => Vec::new(),
    }
}

/// Embedded objects brought from the revised document keep their package parts only when the
/// original holds the very same parts under those names (the parts aren't copied: their names
/// would collide); otherwise the object keeps just its picture. Returns how many lost theirs.
fn drop_foreign_embedded(doc: &mut Document, originals: &BTreeSet<usize>, revised: &std::collections::BTreeMap<String, Arc<Vec<u8>>>) -> usize {
    let have = doc.passthrough.clone();
    let ok = |e: &Arc<wordcraft_doc::graphic::Embedded>| {
        originals.contains(&(Arc::as_ptr(e) as usize))
            || e.refs.iter().filter(|r| !r.external).all(|r| matches!((have.get(&r.target), revised.get(&r.target)), (Some(x), Some(y)) if x == y))
    };
    fn fix(o: &mut InlineObject, ok: &dyn Fn(&Arc<wordcraft_doc::graphic::Embedded>) -> bool, depth: usize) -> usize {
        match o {
            InlineObject::Image { ole, .. } if ole.as_ref().is_some_and(|e| !ok(e)) => {
                *ole = None;
                1
            }
            InlineObject::Graphic { graphic, .. } if graphic.source.as_ref().is_some_and(|e| !ok(e)) => {
                Arc::make_mut(graphic).source = None;
                1
            }
            InlineObject::Group { children, .. } if depth < MAX_KEY_DEPTH => children.iter_mut().map(|c| fix(&mut c.obj, ok, depth + 1)).sum(),
            _ => 0,
        }
    }
    fn bad(b: &Block, ok: &dyn Fn(&Arc<wordcraft_doc::graphic::Embedded>) -> bool, depth: usize) -> bool {
        match b {
            Block::Para(p) => p.objects.iter().any(|o| embedded_of(o, 0).into_iter().any(|e| !ok(e))),
            Block::Table(t) if depth < MAX_KEY_DEPTH => {
                t.rows.iter().flat_map(|r| r.cells.iter()).flat_map(|c| c.blocks.iter()).any(|x| bad(x, ok, depth + 1))
            }
            Block::Table(_) => false,
        }
    }
    let mut lost = 0;
    let mut fix_blocks = |blocks: &mut Vec<Arc<Block>>| {
        for b in blocks.iter_mut() {
            if bad(b, &ok, 0) {
                super::paste::each_para(Arc::make_mut(b), 0, &mut |p| {
                    for o in &mut p.objects {
                        lost += fix(o, &ok, 0);
                    }
                });
            }
        }
    };
    fix_blocks(&mut doc.body);
    for part in doc.parts.values_mut() {
        fix_blocks(&mut part.blocks);
    }
    lost
}

/// Remove the notes, comments and text box stories copied from the revised document that
/// nothing refers to any more (the content turned out unchanged and kept the original's).
fn drop_unused_copies(doc: &mut Document, parts_before: &BTreeSet<u32>, comments_before: &BTreeSet<u32>) {
    // Each pass can free stories only another unused copy referred to; bounded by their number.
    for _ in 0..=doc.parts.len() {
        let (mut parts, mut comments) = (BTreeSet::new(), BTreeSet::new());
        each_object(doc, &mut |o| match o {
            InlineObject::NoteRef { id, .. } => {
                parts.insert(*id);
            }
            InlineObject::CommentStart { id } | InlineObject::CommentEnd { id } => {
                comments.insert(*id);
            }
            o => parts.extend(o.text_boxes()),
        });
        let unused: Vec<u32> = doc.comments.keys().copied().filter(|k| !comments_before.contains(k) && !comments.contains(k)).collect();
        for k in unused {
            doc.comments.remove(&k);
        }
        parts.extend(doc.comments.values().map(|c| c.part));
        let orphans: Vec<u32> = doc.parts.keys().copied().filter(|k| !parts_before.contains(k) && !parts.contains(k)).collect();
        if orphans.is_empty() {
            return;
        }
        for k in orphans {
            doc.parts.remove(&k);
        }
    }
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

/// How short an interior match can be, inside a run of edits, before it's folded into the
/// surrounding delete/insert instead of staying its own little island of "same" text. Two short
/// strings that happen to share a character in a different order (common with digits in dates,
/// IDs, phone numbers) otherwise come out of `lcs_ops` as a true shortest edit script, but render
/// as a "ransom note" of alternating single-character deletions and insertions where a reader
/// (and Word) would expect one clean replacement.
const NOISE_MATCH_LEN: usize = 1;

/// Folds single-token matches that sit strictly inside a run of edits into the surrounding
/// delete/insert, so a character-level diff doesn't fragment a replaced run. A matched run that
/// opens or closes the whole sequence is the genuine common prefix/suffix `lcs_ops` already
/// stripped before running its table, so it's left untouched regardless of length; only a short
/// match produced by the table's own backtracking, which can only ever be interior, is a
/// candidate.
fn collapse_noise_matches(ops: &[Op]) -> Vec<Op> {
    #[derive(Clone, Copy, PartialEq)]
    enum Kind {
        Keep,
        Fold,
    }
    let mut runs: Vec<(Op, usize, Kind)> = Vec::new();
    let mut k = 0;
    while k < ops.len() {
        let op = ops[k];
        let len = ops[k..].iter().take_while(|o| **o == op).count();
        let kind = if op == Op::Same && k != 0 && k + len != ops.len() && len <= NOISE_MATCH_LEN { Kind::Fold } else { Kind::Keep };
        runs.push((op, len, kind));
        k += len;
    }
    if !runs.iter().any(|(_, _, kind)| *kind == Kind::Fold) {
        return ops.to_vec();
    }
    let mut out = Vec::with_capacity(ops.len());
    let mut i = 0;
    while i < runs.len() {
        if runs[i].0 == Op::Same && runs[i].2 == Kind::Keep {
            out.extend(std::iter::repeat_n(Op::Same, runs[i].1));
            i += 1;
            continue;
        }
        let start = i;
        while i < runs.len() && !(runs[i].0 == Op::Same && runs[i].2 == Kind::Keep) {
            i += 1;
        }
        let region = &runs[start..i];
        if region.iter().any(|(_, _, kind)| *kind == Kind::Fold) {
            // Flatten the region: every original character first (in its own order), then every
            // revised character (in its own order) — one clean delete, one clean insert.
            let dels: usize = region.iter().map(|(op, len, kind)| if *kind == Kind::Fold || *op == Op::Del { *len } else { 0 }).sum();
            let inss: usize = region.iter().map(|(op, len, kind)| if *kind == Kind::Fold || *op == Op::Ins { *len } else { 0 }).sum();
            out.extend(std::iter::repeat_n(Op::Del, dels));
            out.extend(std::iter::repeat_n(Op::Ins, inss));
        } else {
            for (op, len, _) in region {
                out.extend(std::iter::repeat_n(*op, *len));
            }
        }
    }
    out
}

/// A paragraph's tokens as byte ranges: words with the spaces after them ("The ", "cat "), or
/// single characters; an object is always a token of its own.
fn tokens(p: &Paragraph, characters: bool) -> Vec<std::ops::Range<usize>> {
    let mut v = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in p.text.char_indices() {
        let end = i + c.len_utf8();
        if c == wordcraft_doc::para::OBJ || characters {
            if let Some(s0) = start.take() {
                v.push(s0..i);
            }
            v.push(i..end);
            continue;
        }
        let s0 = *start.get_or_insert(i);
        if c == ' ' {
            v.push(s0..end);
            start = None;
        }
    }
    if let Some(s0) = start {
        v.push(s0..p.text.len());
    }
    v
}

/// How a token compares: its text as the options see it, or the object it stands for.
fn token_key(doc: &Document, p: &Paragraph, r: &std::ops::Range<usize>, opts: &CompareOptions) -> String {
    let t = p.text.get(r.clone()).unwrap_or("");
    if t.starts_with(wordcraft_doc::para::OBJ) { object_key(doc, p.object_at(r.start)) } else { opts.key(t) }
}

/// The byte range covered by `n` tokens from the `i`th.
fn token_span(t: &[std::ops::Range<usize>], i: usize, n: usize) -> std::ops::Range<usize> {
    let a = t.get(i).map_or(0, |r| r.start);
    let b = n.checked_sub(1).and_then(|l| t.get(i + l)).map_or(a, |r| r.end);
    a..b.max(a)
}

/// The content of `p` in the byte range `r` (text, runs and objects) as a paragraph of its own,
/// without `p`'s paragraph properties.
fn slice(p: &Paragraph, r: std::ops::Range<usize>) -> Paragraph {
    let mut q = Paragraph::new();
    let Some(text) = p.text.get(r.clone()) else { return q };
    let obj = wordcraft_doc::para::OBJ;
    let before = p.text.get(..r.start).map_or(0, |t| t.matches(obj).count());
    q.objects = p.objects.iter().skip(before).take(text.matches(obj).count()).cloned().collect();
    q.runs = p
        .run_ranges()
        .filter_map(|(x, c)| {
            let (a, b) = (x.start.max(r.start), x.end.min(r.end));
            (b > a).then(|| wordcraft_doc::Run { len: b - a, props: c.clone() })
        })
        .collect();
    q.text = text.to_string();
    q.normalize();
    q
}

/// A changed paragraph: the revised paragraph's content where it matches the original (its
/// formatting changes tracked), the original's where it was deleted, the revised one's where
/// inserted, each piece keeping its runs, objects (pictures, fields…) and links. `revs` are the
/// insertion and deletion revisions; `log` where formatting changes are recorded, and the revised
/// comments whose marks matched the original's.
fn word_diff(
    doc: &Document,
    a: &Paragraph,
    b: &Paragraph,
    revs: (u32, u32),
    opts: &CompareOptions,
    log: (&mut Vec<wordcraft_doc::Revision>, &str, &str, &mut std::collections::BTreeMap<u32, u32>),
) -> Paragraph {
    let (ins, del) = revs;
    let (revisions, author, date, merged) = log;
    let chars = opts.characters && a.text.chars().count().max(b.text.chars().count()) <= MAX_CHAR_DIFF;
    let (ta, tb) = (tokens(a, chars), tokens(b, chars));
    let key = |p: &Paragraph, t: &[std::ops::Range<usize>]| t.iter().map(|r| token_key(doc, p, r, opts)).collect::<Vec<_>>();
    let ops = lcs_ops(&key(a, &ta), &key(b, &tb));
    // Only at character granularity: a matched whole word almost never happens by coincidence,
    // but a matched single character very often does (digits, punctuation), so only there does
    // a short interior match need folding back into its surrounding edit.
    let ops = if chars { collapse_noise_matches(&ops) } else { ops };
    let mut p = Paragraph::new();
    p.props = b.props.clone();
    p.mark = b.mark.clone();
    p.section = b.section.clone();
    if opts.formatting {
        // The paragraph's own formatting (and its mark's).
        let mut old = Paragraph::new();
        old.props = a.props.clone();
        old.mark = a.mark.clone();
        super::fmt_revisions::compare_formatting(&old, &mut p, revisions, author, date);
    }
    let (mut i, mut j, mut k) = (0, 0, 0);
    while let Some(op) = ops.get(k).copied() {
        let n = ops.get(k..).unwrap_or_default().iter().take_while(|o| **o == op).count().max(1);
        let piece = match op {
            Op::Same => {
                // Equal as compared (perhaps up to case or spacing): the revised content.
                let mut piece = slice(b, token_span(&tb, j, n));
                let old = slice(a, token_span(&ta, i, n));
                // The same objects: the original's (its notes and comments stay themselves).
                if piece.objects.len() == old.objects.len() {
                    pair_comments(&piece.objects, &old.objects, merged);
                    piece.objects.clone_from(&old.objects);
                }
                if opts.formatting && old.text == piece.text {
                    super::fmt_revisions::compare_formatting(&old, &mut piece, revisions, author, date);
                }
                i += n;
                j += n;
                piece
            }
            Op::Del => {
                let mut piece = slice(a, token_span(&ta, i, n));
                let len = piece.len();
                let _ = piece.format(0, len, &|c| c.del = Some(del));
                i += n;
                piece
            }
            Op::Ins => {
                let mut piece = slice(b, token_span(&tb, j, n));
                let len = piece.len();
                let _ = piece.format(0, len, &|c| c.ins = Some(ins));
                j += n;
                piece
            }
        };
        p.append(piece);
        k += n;
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
    // The removals aren't tracked: they delete straight from the document, so on a document that
    // tracks changes the text would go with no revision to show for it, and the revision trail
    // would be broken behind the user's back. Refuse the whole removal instead of quietly
    // untracking it. Reading the document (no `remove`) is always allowed.
    if !remove.is_empty() && s.doc.settings.track_changes {
        return Err(CmdError::Disabled("the document tracks changes: stop tracking before removing content".into()));
    }
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

/// Insert › Quick Parts and AutoText: this session's building blocks, and the document's own
/// (`Document::glossary`, kept in its file) in `gallery`. A name is looked up in the session's
/// first, then in the document's (`gallery` first).
fn quick_parts(s: &mut Session, v: &Value, gallery: &str) -> CmdResult {
    let in_document = p::str(v, "in") == Some("document");
    if let Some(name) = p::str(v, "save") {
        let (a, b) = s.sel.ordered();
        if a == b {
            return Err(CmdError::Params("select the content to save".into()));
        }
        let frag = s.doc.copy_range(&a, &b);
        if in_document {
            if name.trim().is_empty() {
                return Err(CmdError::Params("a building block needs a name".into()));
            }
            let blocks: wordcraft_doc::Blocks = frag.blocks.into_iter().map(Arc::new).collect();
            match s.doc.glossary.iter_mut().find(|e| e.name == name && e.gallery == gallery) {
                Some(e) => e.blocks = blocks,
                None => s.doc.glossary.push(wordcraft_doc::BuildingBlock {
                    name: name.to_string(),
                    gallery: gallery.to_string(),
                    category: "General".into(),
                    blocks,
                    ..Default::default()
                }),
            }
        } else {
            s.building_blocks.insert(name.to_string(), frag);
        }
    }
    if let Some(name) = p::str(v, "delete")
        && (in_document || s.building_blocks.remove(name).is_none())
        && let Some(i) = document_entry(&s.doc, name, gallery)
    {
        s.doc.glossary.remove(i);
    }
    if let Some(name) = p::str(v, "insert") {
        let frag = match s.building_blocks.get(name) {
            Some(f) => f.clone(),
            None => {
                let e = document_entry(&s.doc, name, gallery).and_then(|i| s.doc.glossary.get(i));
                let e = e.ok_or_else(|| CmdError::Params(format!("no Quick Part `{name}`")))?;
                wordcraft_doc::edit::Fragment { blocks: e.blocks.iter().map(|b| (**b).clone()).collect(), parts: Default::default() }
            }
        };
        let at = delete_selection(s)?;
        let end = s.doc.insert_fragment(&at, &frag)?;
        s.sel = Selection::caret(end);
    }
    let document: Vec<&str> = s.doc.glossary.iter().filter(|e| e.gallery == gallery).map(|e| e.name.as_str()).collect();
    let mut parts: Vec<&str> = s.building_blocks.keys().map(String::as_str).collect();
    let mut seen: std::collections::HashSet<&str> = parts.iter().copied().collect();
    parts.extend(document.iter().copied().filter(|n| seen.insert(n)));
    Ok(json!({"parts": parts, "document": document}))
}

/// The index of the document's building block `name`, in `gallery` if there is one there.
fn document_entry(doc: &Document, name: &str, gallery: &str) -> Option<usize> {
    let g = &doc.glossary;
    g.iter().position(|e| e.name == name && e.gallery == gallery).or_else(|| g.iter().position(|e| e.name == name))
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
        s.push_ui_request(json!({"open": "customizeKeyboard"}));
    }
    Ok(Value::Object(out))
}

/// `tools.search`: the commands whose label, location or id hold every word of `query`, or,
/// with no query, the Search box (`ui_requests` → the UI's command search).
fn search_commands(s: &mut Session, v: &Value) -> CmdResult {
    let Some(query) = p::str(v, "query") else {
        s.push_ui_request(json!({"open": "commands"}));
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
        s.push_ui_request(json!({"open": "customizeRibbon"}));
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

    /// A document's body as compared: text, runs (without revision marks), objects (pictures by
    /// their bytes, not their media key), paragraph style and list definition, tables cell by cell.
    fn shape(doc: &Document) -> Value {
        fn blocks(doc: &Document, bl: &[Arc<Block>]) -> Value {
            Value::Array(bl.iter().map(|b| block(doc, b)).collect())
        }
        fn block(doc: &Document, b: &Block) -> Value {
            match b {
                Block::Para(p) => {
                    let runs: Vec<Value> = p
                        .runs
                        .iter()
                        .map(|r| {
                            json!([r.len, serde_json::to_value(CharProps { ins: None, del: None, fmt_change: None, ..r.props.clone() }).unwrap()])
                        })
                        .collect();
                    let objects: Vec<Value> = p
                        .objects
                        .iter()
                        .map(|o| {
                            let mut v = serde_json::to_value(o).unwrap();
                            if let InlineObject::Image { media, .. } = o {
                                v["media"] = json!(doc.media.get(media).map(|d| d.to_vec()));
                            }
                            v
                        })
                        .collect();
                    let list = p
                        .props
                        .numbering
                        .as_ref()
                        .map(|n| json!([n.level, doc.numbering.abstract_of(n.num).map(|a| serde_json::to_value(&a.levels).unwrap())]));
                    json!({"text": p.text, "runs": runs, "objects": objects, "style": p.props.style, "list": list})
                }
                Block::Table(t) => {
                    Value::Array(t.rows.iter().map(|r| Value::Array(r.cells.iter().map(|c| blocks(doc, &c.blocks)).collect())).collect())
                }
            }
        }
        blocks(doc, &doc.body)
    }

    /// Compare `original` with `revised`, then check Accept All gives the revised document and
    /// Reject All the original. Returns the compare result and a session on the compared document.
    fn compare_round_trip(original: &Document, revised: &Document) -> (Value, Session) {
        let mut s = Session::new(original.clone());
        let r = compare_with(&mut s, revised.clone(), &json!({"author": "Cmp"})).unwrap();
        let result = s.doc.clone();
        s.run("review.acceptAll", &json!({})).unwrap();
        assert_eq!(shape(&s.doc), shape(revised), "Accept All gives the revised document");
        let mut s = Session::new(result.clone());
        s.run("review.rejectAll", &json!({})).unwrap();
        assert_eq!(shape(&s.doc), shape(original), "Reject All gives the original document");
        (r, Session::new(result))
    }

    fn picture(media: &str) -> InlineObject {
        InlineObject::Image { media: media.into(), w: 20.0, h: 10.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None }
    }

    fn para_with_object(text: &str, obj: InlineObject) -> Arc<Block> {
        let mut p = Paragraph::with_text(text, CharProps::default());
        let end = p.len();
        p.insert_object(end, obj, &CharProps::default()).unwrap();
        para_block(p)
    }

    fn para_mut(b: &mut Arc<Block>) -> &mut Paragraph {
        match Arc::make_mut(b) {
            Block::Para(p) => p,
            Block::Table(_) => panic!("not a paragraph"),
        }
    }

    /// #1317: a picture the revised document stores under a key the original uses for another
    /// picture shows the revised picture where it is inserted, the original one where kept.
    #[test]
    fn compare_imports_pictures_whose_keys_collide() {
        let mut original = Document::from_text("Intro\nOutro");
        original.media.insert("image1.png".into(), Arc::new(vec![1, 1, 1]));
        original.body.insert(1, para_with_object("Kept ", picture("image1.png")));
        let mut revised = Document::from_text("Intro\nOutro");
        revised.media.insert("image2.png".into(), Arc::new(vec![1, 1, 1]));
        revised.media.insert("image1.png".into(), Arc::new(vec![2, 2, 2]));
        revised.body.insert(1, para_with_object("Kept ", picture("image2.png")));
        revised.body.insert(2, para_with_object("New ", picture("image1.png")));
        let (_, s) = compare_round_trip(&original, &revised);
        let bytes = |b: &Block| match b.as_para().and_then(|p| p.objects.first()) {
            Some(InlineObject::Image { media, .. }) => s.doc.media.get(media).map(|d| d.to_vec()),
            _ => None,
        };
        assert_eq!(bytes(&s.doc.body[1]), Some(vec![1, 1, 1]), "the kept picture is the original's");
        assert_eq!(bytes(&s.doc.body[2]), Some(vec![2, 2, 2]), "the inserted picture is the revised one");
    }

    /// #1317: one changed word in a paragraph with a picture, a field and a bold run: only the
    /// word is marked; the objects and the formatting survive.
    #[test]
    fn compare_changed_paragraph_keeps_objects_and_formatting() {
        let build = |word: &str| {
            let mut d = Document::from_text("");
            d.media.insert("image1.png".into(), Arc::new(vec![7, 7]));
            let mut p = Paragraph::with_text("Hello ", CharProps::default());
            let n = p.len();
            p.insert_text(n, "brave", &CharProps { bold: Some(true), ..Default::default() }).unwrap();
            let n = p.len();
            p.insert_text(n, &format!(" {word} "), &CharProps::default()).unwrap();
            let n = p.len();
            p.insert_object(n, picture("image1.png"), &CharProps::default()).unwrap();
            let n = p.len();
            p.insert_text(n, " page ", &CharProps::default()).unwrap();
            let n = p.len();
            let field =
                InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() };
            p.insert_object(n, field, &CharProps::default()).unwrap();
            d.body = vec![para_block(p)];
            d
        };
        let (_, s) = compare_round_trip(&build("world"), &build("planet"));
        let p = s.doc.body[0].as_para().unwrap();
        let marked = |f: fn(&CharProps) -> bool| -> String { p.run_ranges().filter(|(_, c)| f(c)).map(|(r, _)| p.text[r].to_string()).collect() };
        assert_eq!(marked(|c| c.del.is_some()), "world ");
        assert_eq!(marked(|c| c.ins.is_some()), "planet ");
        assert_eq!(p.objects.len(), 2, "the picture and the field are kept");
        assert_eq!(marked(|c| c.bold == Some(true)), "brave");
    }

    /// #1317: whole paragraphs and tables added or removed are tracked with their paragraph
    /// marks and rows, so resolving leaves no empty paragraphs or tables behind.
    #[test]
    fn compare_tracks_whole_paragraphs_and_tables() {
        let table = || {
            let mut t = wordcraft_doc::Table::new(2, 2, 200.0);
            for (i, c) in t.rows.iter_mut().flat_map(|r| r.cells.iter_mut()).enumerate() {
                *c = wordcraft_doc::table::Cell::with_text(&format!("cell {i}"));
            }
            Arc::new(Block::Table(t))
        };
        let mut original = Document::from_text("Intro\nRemoved paragraph\nMiddle\nOutro");
        original.body.insert(2, table());
        let revised = Document::from_text("Intro\nMiddle\nAdded paragraph\nOutro\nTail paragraph");
        compare_round_trip(&original, &revised);
        // The other way round: a paragraph removed at the end, a table added.
        let mut revised = Document::from_text("Intro\nMiddle");
        revised.body.insert(1, table());
        let original = Document::from_text("Intro\nMiddle\nLast one");
        compare_round_trip(&original, &revised);
    }

    /// #1317: styles and lists only the revised document defines come with its content, and a
    /// list id the original uses for another list doesn't take over.
    #[test]
    fn compare_imports_styles_and_lists() {
        use wordcraft_doc::numbering::ListKind;
        let mut original = Document::from_text("Intro\nItem one");
        let mine = original.numbering.add_list(ListKind::Bullet);
        para_mut(&mut original.body[1]).props.numbering = Some(NumRef { num: mine, level: 0 });
        let mut revised = Document::from_text("Intro\nItem one\nFancy text");
        let theirs = revised.numbering.add_list(ListKind::Numbered);
        assert_eq!(mine, theirs, "the two lists share an id");
        revised.styles.upsert(wordcraft_doc::styles::Style {
            id: "Fancy".into(),
            name: "Fancy".into(),
            kind: wordcraft_doc::styles::StyleKind::Paragraph,
            chr: CharProps { italic: Some(true), ..Default::default() },
            ..Default::default()
        });
        para_mut(&mut revised.body[1]).props.numbering = Some(NumRef { num: theirs, level: 0 });
        let fancy = para_mut(&mut revised.body[2]);
        fancy.props.style = Some("Fancy".into());
        fancy.props.numbering = Some(NumRef { num: theirs, level: 0 });
        let (_, s) = compare_round_trip(&original, &revised);
        assert_eq!(s.doc.styles.get("Fancy").and_then(|st| st.chr.italic), Some(true));
    }

    /// #1317: a footnote the revised document adds comes with its own story; a reference to a
    /// note the revised document lacks is left out and reported, never pointed at one of ours.
    #[test]
    fn compare_imports_notes_and_reports_what_it_drops() {
        use wordcraft_doc::{PartKind, para::NoteKind};
        let mut original = Document::from_text("Intro");
        let ours = original.try_add_part(PartKind::Footnote, vec![para_block(Paragraph::with_text("Our note", CharProps::default()))]).unwrap();
        let mut revised = Document::from_text("Intro");
        let theirs = revised.try_add_part(PartKind::Footnote, vec![para_block(Paragraph::with_text("Their note", CharProps::default()))]).unwrap();
        assert_eq!(ours, theirs, "the two notes share an id");
        revised.body.push(para_with_object("Noted", InlineObject::NoteRef { kind: NoteKind::Footnote, id: theirs, custom: String::new() }));
        revised.body.push(para_with_object("Lost", InlineObject::NoteRef { kind: NoteKind::Footnote, id: 999, custom: String::new() }));
        let mut s = Session::new(original);
        let r = compare_with(&mut s, revised, &json!({})).unwrap();
        let note = s.doc.body[1].as_para().and_then(|p| match p.objects.first() {
            Some(InlineObject::NoteRef { id, .. }) => Some(*id),
            _ => None,
        });
        let note = note.expect("the note reference is kept");
        assert_ne!(note, ours);
        assert_eq!(s.doc.plain_text(StoryRef::Part(note)), "Their note");
        assert_eq!(s.doc.plain_text(StoryRef::Part(ours)), "Our note");
        assert!(s.doc.body[2].as_para().is_some_and(|p| p.objects.is_empty() && p.text == "Lost"));
        assert_eq!(r["dropped"][0]["what"], "noteReference");
        assert_eq!(r["dropped"][0]["count"], 1);
    }

    /// A document with a footnote, a comment and a text box, and a first paragraph to edit.
    fn rich_doc(first: &str) -> Document {
        use wordcraft_doc::{Comment, PartKind, para::NoteKind};
        let plain = CharProps::default();
        let mut d = Document::from_text(&format!("{first}\nOutro"));
        let note = d.try_add_part(PartKind::Footnote, vec![para_block(Paragraph::with_text("A note", plain.clone()))]).unwrap();
        d.body.insert(1, para_with_object("Noted", InlineObject::NoteRef { kind: NoteKind::Footnote, id: note, custom: String::new() }));
        let part = d.try_add_part(PartKind::Comment, vec![para_block(Paragraph::with_text("A comment", plain.clone()))]).unwrap();
        d.comments.insert(0, Comment { author: "Ann".into(), part, ..Default::default() });
        let mut p = Paragraph::new();
        p.insert_object(0, InlineObject::CommentStart { id: 0 }, &plain).unwrap();
        let n = p.len();
        p.insert_text(n, "Commented", &plain).unwrap();
        let n = p.len();
        p.insert_object(n, InlineObject::CommentEnd { id: 0 }, &plain).unwrap();
        d.body.insert(2, para_block(p));
        let story = d.try_add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text("Boxed", plain))]).unwrap();
        let shape = InlineObject::Shape {
            kind: Default::default(),
            w: 50.0,
            h: 20.0,
            fill: None,
            stroke: None,
            stroke_width: 1.0,
            float: Default::default(),
            story: Some(story),
            freeform: None,
            effects: Default::default(),
            extra: Default::default(),
        };
        d.body.insert(3, para_with_object("Box ", shape));
        d
    }

    /// Text marked inserted or deleted in the body (with paragraph marks as "¶").
    fn tracked(doc: &Document, ins: bool) -> String {
        let mut out = String::new();
        for path in doc.para_paths(StoryRef::Body) {
            let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
            for (r, c) in p.run_ranges() {
                if (ins && c.ins.is_some()) || (!ins && c.del.is_some()) {
                    out.push_str(&p.text[r]);
                }
            }
            if (ins && p.mark.ins.is_some()) || (!ins && p.mark.del.is_some()) {
                out.push('¶');
            }
        }
        out
    }

    /// #1317 review: footnotes, comments and text boxes the two documents share are no change,
    /// and comparing doesn't leave copies of them behind.
    #[test]
    fn compare_shared_notes_comments_and_text_boxes_are_unchanged() {
        let original = rich_doc("Intro words here");
        let mut s = Session::new(original.clone());
        let r = compare_with(&mut s, original.clone(), &json!({})).unwrap();
        assert_eq!(r["changes"], 0);
        assert_eq!((tracked(&s.doc, true), tracked(&s.doc, false)), (String::new(), String::new()));
        assert_eq!((s.doc.parts.len(), s.doc.comments.len()), (original.parts.len(), 1));
        // One word changed elsewhere: only that word is marked.
        let mut s = Session::new(original.clone());
        let r = compare_with(&mut s, rich_doc("Intro lines here"), &json!({})).unwrap();
        assert_eq!(r["changes"], 1);
        assert_eq!((tracked(&s.doc, true), tracked(&s.doc, false)), ("lines ".to_string(), "words ".to_string()));
        assert_eq!((s.doc.parts.len(), s.doc.comments.len()), (original.parts.len(), 1));
    }

    fn cells_table() -> Arc<Block> {
        let mut t = wordcraft_doc::Table::new(2, 2, 200.0);
        for (i, c) in t.rows.iter_mut().flat_map(|r| r.cells.iter_mut()).enumerate() {
            *c = wordcraft_doc::table::Cell::with_text(&format!("cell {i}"));
        }
        Arc::new(Block::Table(t))
    }

    /// Select the whole table at body index `i` (2 × 2).
    fn select_table(s: &mut Session, i: u32) {
        let end = s.doc.para(StoryRef::Body, &wordcraft_doc::Path(vec![i, 1, 1, 0])).map(|p| p.len()).unwrap();
        s.sel = Selection {
            anchor: Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![i, 0, 0, 0]), off: 0 },
            focus: Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![i, 1, 1, 0]), off: end },
        };
    }

    /// #1317 review: a table deletion rejected (or an insertion accepted) by selection stays
    /// resolved, so a later Accept All (Reject All) keeps the table.
    #[test]
    fn row_marks_resolved_by_selection_survive_resolve_all() {
        let mut original = Document::from_text("Intro\nOutro");
        original.body.insert(1, cells_table());
        let mut s = Session::new(original);
        compare_with(&mut s, Document::from_text("Intro\nOutro"), &json!({})).unwrap();
        select_table(&mut s, 1);
        s.run("review.reject", &json!({})).unwrap();
        s.run("review.acceptAll", &json!({})).unwrap();
        assert!(matches!(s.doc.body.get(1).map(|b| &**b), Some(Block::Table(_))), "the kept table stays");
        let mut revised = Document::from_text("Intro\nOutro");
        revised.body.insert(1, cells_table());
        let mut s = Session::new(Document::from_text("Intro\nOutro"));
        compare_with(&mut s, revised, &json!({})).unwrap();
        select_table(&mut s, 1);
        s.run("review.accept", &json!({})).unwrap();
        s.run("review.rejectAll", &json!({})).unwrap();
        assert!(matches!(s.doc.body.get(1).map(|b| &**b), Some(Block::Table(_))), "the accepted table stays");
    }

    /// #1317 review: a table whose text is the same but whose formatting changed is tracked, so
    /// Reject All gives back the original table.
    #[test]
    fn compare_tracks_a_table_whose_formatting_changed() {
        let mut original = Document::from_text("Intro\nOutro");
        original.body.insert(1, cells_table());
        let mut revised = original.clone();
        if let Block::Table(t) = Arc::make_mut(&mut revised.body[1]) {
            let p = match Arc::make_mut(&mut t.rows[0].cells[0].blocks[0]) {
                Block::Para(p) => p,
                Block::Table(_) => panic!("not a paragraph"),
            };
            let n = p.len();
            p.format(0, n, &|c| c.bold = Some(true)).unwrap();
        }
        compare_round_trip(&original, &revised);
    }

    /// #1317 review: a chart from the revised document whose package part has the same name as a
    /// different part of the original's can't be brought over: its kept source is left out and
    /// reported, never pointed at the original's part.
    #[test]
    fn compare_drops_embedded_parts_whose_names_collide() {
        use wordcraft_doc::graphic::{Embedded, EmbeddedRef, Graphic};
        let chart = |items: usize| InlineObject::Graphic {
            w: 10.0,
            h: 10.0,
            alt: String::new(),
            float: Default::default(),
            graphic: Arc::new(Graphic {
                w: items as f32,
                source: Some(Arc::new(Embedded {
                    xml: "<a:graphic/>".into(),
                    refs: vec![EmbeddedRef { id: "rId1".into(), kind: "chart".into(), target: "word/charts/chart1.xml".into(), external: false }],
                })),
                ..Default::default()
            }),
        };
        let mut original = Document::from_text("Intro\nOutro");
        original.passthrough.insert("word/charts/chart1.xml".into(), Arc::new(b"A".to_vec()));
        original.body.insert(1, para_with_object("Old chart ", chart(1)));
        let mut revised = Document::from_text("Intro\nOutro");
        revised.passthrough.insert("word/charts/chart1.xml".into(), Arc::new(b"B".to_vec()));
        revised.body.insert(1, para_with_object("Old chart ", chart(1)));
        revised.body.insert(2, para_with_object("New chart ", chart(2)));
        let mut s = Session::new(original);
        let r = compare_with(&mut s, revised, &json!({})).unwrap();
        let source = |i: usize| match s.doc.body[i].as_para().and_then(|p| p.objects.first()) {
            Some(InlineObject::Graphic { graphic, .. }) => graphic.source.is_some(),
            _ => panic!("no chart"),
        };
        assert!(source(1), "the original's chart keeps its part");
        assert!(!source(2), "the inserted chart doesn't point at the original's part");
        assert!(r["dropped"].as_array().unwrap().iter().any(|d| d["what"] == "embeddedObject"), "{r}");
    }

    /// #1317 review (#11): a comment id at the top of the range in the original isn't overwritten
    /// by a comment the revised document brings.
    #[test]
    fn compare_never_overwrites_a_comment_at_the_last_id() {
        let mut original = Document::from_text("Intro");
        let part =
            original.try_add_part(wordcraft_doc::PartKind::Comment, vec![para_block(Paragraph::with_text("Mine", CharProps::default()))]).unwrap();
        original.comments.insert(u32::MAX, wordcraft_doc::Comment { author: "Me".into(), part, ..Default::default() });
        let mut s = Session::new(original);
        compare_with(&mut s, rich_doc("Intro"), &json!({})).unwrap();
        assert_eq!(s.doc.comments.get(&u32::MAX).map(|c| c.author.as_str()), Some("Me"));
    }

    /// #1317 (N1): a comment whose range grew stays one comment, its start and end both there,
    /// after Accept All.
    #[test]
    fn compare_keeps_a_comment_whose_range_grew_one_comment() {
        use wordcraft_doc::{Comment, PartKind};
        let build = |grown: bool| {
            let plain = CharProps::default();
            let mut d = Document::from_text("Intro");
            let part = d.try_add_part(PartKind::Comment, vec![para_block(Paragraph::with_text("Remark", plain.clone()))]).unwrap();
            d.comments.insert(0, Comment { author: "Ann".into(), part, ..Default::default() });
            let mut p = Paragraph::with_text("A ", plain.clone());
            let items: Vec<(Option<InlineObject>, &str)> = if grown {
                vec![(Some(InlineObject::CommentStart { id: 0 }), "B C"), (Some(InlineObject::CommentEnd { id: 0 }), "")]
            } else {
                vec![(Some(InlineObject::CommentStart { id: 0 }), "B "), (Some(InlineObject::CommentEnd { id: 0 }), "C")]
            };
            for (obj, text) in items {
                if let Some(o) = obj {
                    let n = p.len();
                    p.insert_object(n, o, &plain).unwrap();
                }
                let n = p.len();
                p.insert_text(n, text, &plain).unwrap();
            }
            d.body.push(para_block(p));
            d
        };
        let mut s = Session::new(build(false));
        compare_with(&mut s, build(true), &json!({})).unwrap();
        s.run("review.acceptAll", &json!({})).unwrap();
        assert_eq!(s.doc.comments.len(), 1, "one comment");
        let id = *s.doc.comments.keys().next().unwrap();
        let p = s.doc.body[1].as_para().unwrap();
        let anchors: Vec<&InlineObject> =
            p.objects.iter().filter(|o| matches!(o, InlineObject::CommentStart { .. } | InlineObject::CommentEnd { .. })).collect();
        assert_eq!(anchors, [&InlineObject::CommentStart { id }, &InlineObject::CommentEnd { id }]);
        assert_eq!(p.plain_text(), "A B C");
    }

    /// #1317 (N2): rejecting the insertion of a row that holds a nested table resolves that row
    /// once: the next row and its text stay.
    #[test]
    fn rejecting_a_row_with_a_nested_table_keeps_the_next_row() {
        let mut outer = wordcraft_doc::Table::new(2, 2, 200.0);
        for (i, c) in outer.rows.iter_mut().flat_map(|r| r.cells.iter_mut()).enumerate() {
            *c = wordcraft_doc::table::Cell::with_text(&format!("cell {i}"));
        }
        let mut inner = wordcraft_doc::Table::new(1, 1, 50.0);
        inner.rows[0].cells[0] = wordcraft_doc::table::Cell::with_text("nested");
        outer.rows[0].cells[1].blocks.insert(0, Arc::new(Block::Table(inner)));
        let mut revised = Document::from_text("Intro\nOutro");
        revised.body.insert(1, Arc::new(Block::Table(outer)));
        let mut s = Session::new(Document::from_text("Intro\nOutro"));
        compare_with(&mut s, revised, &json!({})).unwrap();
        let end = s.doc.para(StoryRef::Body, &wordcraft_doc::Path(vec![1, 0, 1, 1])).map(|p| p.len()).unwrap();
        s.sel = Selection {
            anchor: Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![1, 0, 0, 0]), off: 0 },
            focus: Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![1, 0, 1, 1]), off: end },
        };
        s.run("review.reject", &json!({})).unwrap();
        let t = s.doc.body.get(1).and_then(|b| b.as_table()).expect("the table stays");
        assert_eq!(t.rows.len(), 1);
        assert_eq!(block_text(&s.doc.body[1], 0), "cell 2\tcell 3");
    }

    /// #1317 (N4): a row inserted or deleted whole is listed as a change, even with empty cells.
    #[test]
    fn row_insertions_are_listed_as_changes() {
        let mut revised = Document::from_text("Intro\nOutro");
        revised.body.insert(1, Arc::new(Block::Table(wordcraft_doc::Table::new(1, 2, 200.0))));
        let mut s = Session::new(Document::from_text("Intro\nOutro"));
        compare_with(&mut s, revised, &json!({})).unwrap();
        let list = s.run("review.changes", &json!({})).unwrap();
        assert!(list.as_array().unwrap().iter().any(|c| c["kind"] == "insert"), "{list}");
    }

    /// #1998: a compare result's row marks survive save → open as DOCX, so Accept All and
    /// Reject All on the reopened file give the revised and the original tables (no empty rows).
    #[test]
    fn compared_row_marks_survive_a_docx_round_trip() {
        let mut original = Document::from_text("Intro\nOutro");
        original.body.insert(1, cells_table());
        let revised = Document::from_text("Intro\nOutro");
        for (original, revised) in [(original.clone(), revised.clone()), (revised, original)] {
            let mut s = Session::new(original.clone());
            compare_with(&mut s, revised.clone(), &json!({"author": "Cmp"})).unwrap();
            let bytes = wordcraft_docx::write(&s.doc).unwrap();
            let reopened = wordcraft_docx::read(&bytes).unwrap();
            let mut s = Session::new(reopened.clone());
            s.run("review.rejectAll", &json!({})).unwrap();
            assert_eq!(shape(&s.doc), shape(&original), "Reject All on the reopened file gives the original");
            let mut s = Session::new(reopened);
            s.run("review.acceptAll", &json!({})).unwrap();
            assert_eq!(shape(&s.doc), shape(&revised), "Accept All on the reopened file gives the revised document");
        }
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
        // Two dates that happen to share digits in a different order ("01.09.2026" vs
        // "19.10.2026") must come out as one clean replacement, not a scrambled chain of
        // single-character deletions and insertions wherever a digit coincidentally recurs.
        let mut s = Session::new(Document::from_text("zawarta w dniu 01.09.2026 pomiędzy:"));
        s.run("review.compare", &json!({"text": "zawarta w dniu 19.10.2026 pomiędzy:", "level": "character"})).unwrap();
        let p = s.doc.body[0].as_para().unwrap();
        let deleted: String = p.run_ranges().filter(|(_, c)| c.del.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
        let inserted: String = p.run_ranges().filter(|(_, c)| c.ins.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
        assert_eq!(deleted, "01.09");
        assert_eq!(inserted, "19.10");
        assert_eq!(p.run_ranges().filter(|(_, c)| c.del.is_some()).count(), 1, "one clean deleted run, not several scattered ones");
        assert_eq!(p.run_ranges().filter(|(_, c)| c.ins.is_some()).count(), 1, "one clean inserted run, not several scattered ones");
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
    fn collapse_noise_matches_merges_short_interior_runs_but_keeps_prefix_suffix() {
        // "01.09" vs "19.10": a pure LCS finds a scrambled chain of single-character matches
        // ('1', '.', '0') threaded through what a reader expects to be one clean replacement.
        let ops = lcs_ops(&['0', '1', '.', '0', '9'], &['1', '9', '.', '1', '0']);
        assert!(ops.contains(&Op::Same), "the raw LCS does find spurious matches here, or this test proves nothing");
        assert_eq!(collapse_noise_matches(&ops), vec![Op::Del, Op::Del, Op::Del, Op::Del, Op::Del, Op::Ins, Op::Ins, Op::Ins, Op::Ins, Op::Ins]);
        // A plain single-character replacement flanked by a real common prefix and suffix has no
        // interior match to begin with (the DP's middle is pure delete/insert), so it's untouched.
        let ops = lcs_ops(&['a', 'b', 'c', 'x', 'd', 'e', 'f'], &['a', 'b', 'c', 'y', 'd', 'e', 'f']);
        assert_eq!(collapse_noise_matches(&ops), ops);
    }

    #[test]
    fn compare_at_word_level_still_matches_a_whole_unchanged_word() {
        // `collapse_noise_matches` only runs for character-level compares (wired up in
        // `word_diff`): a single matched *word* between two edits is a real, meaningful match
        // (unlike a single matched character), and must never be folded into a replacement.
        let mut s = Session::new(Document::from_text("the cat sat"));
        s.run("review.compare", &json!({"text": "a cat ran"})).unwrap();
        let p = s.doc.body[0].as_para().unwrap();
        assert!(
            p.run_ranges().any(|(r, c)| c.del.is_none() && c.ins.is_none() && p.text[r].trim() == "cat"),
            "the unchanged word must survive as plain text"
        );
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
        // The "none" mode is the shorthand for stopping the protection, and it doesn't stop a
        // protected document: only the explicit stop does (#1327 follow-up).
        assert!(s.run("review.restrict", &json!({"mode": "none"})).is_err());
        assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
        s.run("review.restrict", &json!({"stop": true})).unwrap();
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

    /// The inspector's removals delete straight from the document, so they aren't tracked: on a
    /// document that tracks changes the text would go with no revision to show for it, and the
    /// revision trail would be broken behind the user's back. The removal is refused instead,
    /// and the document is left as it was.
    #[test]
    fn inspect_removals_are_refused_while_the_document_tracks_changes() {
        let mut s = Session::new(Document::from_text("Visible text.\nHidden text."));
        s.run("select.text", &json!({"text": "Hidden"})).unwrap();
        s.run("format.hidden", &json!({"value": true})).unwrap();
        // Tracking without Restrict Editing isolates the Inspector guard from the protected
        // document check added separately in #3285.
        s.run("review.trackChanges", &json!({"value": true})).unwrap();
        assert!(s.doc.settings.track_changes, "the document doesn't track changes");
        let before = s.doc.clone();
        let err = s.run("file.inspect", &json!({"remove": ["hidden"]})).unwrap_err();
        assert!(matches!(err, CmdError::Disabled(_)), "{err}");
        assert_eq!(s.doc, before, "the document changed despite the refusal");
        // Reading the document stays allowed.
        let found = s.run("file.inspect", &json!({})).unwrap();
        assert_eq!(found["found"]["hiddenText"], true, "{found}");
    }

    /// A document that doesn't track changes is inspected as before: the hidden text goes.
    #[test]
    fn inspect_removals_work_when_the_document_does_not_track_changes() {
        let mut s = Session::new(Document::from_text("Visible text.\nHidden text."));
        s.run("select.text", &json!({"text": "Hidden"})).unwrap();
        s.run("format.hidden", &json!({"value": true})).unwrap();
        assert!(!s.doc.settings.track_changes);
        let out = s.run("file.inspect", &json!({"remove": ["hidden"]})).unwrap();
        assert_eq!(out["residual"], json!([]), "{out}");
        let text = s.doc.plain_text(StoryRef::Body);
        assert!(!text.contains("Hidden"), "the hidden text is still there: {text}");
        assert!(text.contains("Visible text."), "{text}");
    }
}
