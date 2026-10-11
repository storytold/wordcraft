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
    ("1/2", "½"),
    ("1/4", "¼"),
    ("3/4", "¾"),
];

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
    /// Capitalize the first letter of sentences.
    pub cap_sentences: bool,
    /// Capitalize the first letter of table cells.
    pub cap_cells: bool,
    /// Capitalize names of days.
    pub cap_days: bool,
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
}

impl Default for AutoCorrectPrefs {
    fn default() -> Self {
        AutoCorrectPrefs {
            enabled: true,
            replace_text: true,
            cap_sentences: true,
            cap_cells: true,
            cap_days: true,
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
        for list in [&mut self.removed, &mut self.exceptions, &mut self.exceptions_removed] {
            let mut seen = std::collections::HashSet::new();
            list.retain(|a| ok(a, MAX_FROM) && seen.insert(a.clone()));
            list.truncate(MAX_ENTRIES);
        }
        self
    }

    /// The replacement for `word`, if any: the user's entry, else a built-in one not deleted.
    /// Built-in lowercase entries also match a capitalised word ("Teh" → "The").
    pub fn replacement(&self, word: &str) -> Option<String> {
        if let Some((_, b)) = self.entries.iter().find(|(a, _)| a == word) {
            return Some(b.clone());
        }
        AUTOCORRECT
            .iter()
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
        ("capSentences", &mut ac.cap_sentences),
        ("capCells", &mut ac.cap_cells),
        ("capDays", &mut ac.cap_days),
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
    let ac = &s.prefs.autocorrect;
    Ok(json!({
        "enabled": ac.enabled,
        "replaceText": ac.replace_text,
        "capSentences": ac.cap_sentences,
        "capCells": ac.cap_cells,
        "capDays": ac.cap_days,
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
            .params(r#"{"run"?: name, "define"?: {"name": string, "steps": [{"command", "params"}]}, "delete"?: name}"#)
            .pure(),
        CommandSpec::new("tools.autocorrect", "AutoCorrect Options", "File › Options › Proofing", autocorrect_options)
            .params(
                r#"{"enabled"?: bool (all of AutoCorrect), "replaceText"|"capSentences"|"capCells"|"capDays"|"smartQuotes"|"fractions"|"ordinals"|"dashes"|"links"|"bullets"|"numbering"|"borderLines"?: bool, "add"?: {"from": string, "to": string} (adds or replaces an entry), "delete"?: string (an entry's "from", built in or added), "addException"?: string, "deleteException"?: string} → options, entries and exceptions"#,
            )
            .pure(),
        CommandSpec::new("review.compare", "Compare", "Review › Compare", compare).params(COMPARE_PARAMS),
        CommandSpec::new("review.combine", "Combine", "Review › Compare", compare).params(COMPARE_PARAMS),
        CommandSpec::new("file.accessibility", "Check Accessibility", "Review › Accessibility", accessibility).pure(),
        CommandSpec::new("file.inspect", "Inspect Document", "File › Info", inspect_doc)
            .params(r#"{"remove"?: ["comments", "revisions", "properties", "hidden", "headers"]}"#),
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
        CommandSpec::new("file.recover", "Recover Unsaved Documents", "File › Info", |s, _| {
            Ok(Value::Array(s.versions.iter().enumerate().map(|(i, (l, d, _))| json!({"index": i, "label": l, "date": d})).collect()))
        })
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
            if s.doc.settings.watermark.is_some() {
                issues.push("Watermarks are kept in WordCraft documents but not yet written to .docx.");
            }
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
        CommandSpec::new("insert.autoText", "AutoText", "Insert › Text › Quick Parts", quick_parts).params(r#"{"save"?: name, "insert"?: name}"#),
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
            let end = s.doc.insert_object(&at, InlineObject::Field { instr, result: val, locked: false }, &props)?;
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
            let id = s.doc.numbering.add_list(wordcraft_doc::ListKind::Numbered);
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
        CommandSpec::new("review.language", "Language", "Review › Language", |s, v| {
            let lang = p::str(v, "lang").unwrap_or("en-US").to_string();
            let no_proof = p::bool(v, "noProof");
            super::format::apply(s, &|c| {
                c.lang = Some(lang.clone());
                if let Some(np) = no_proof {
                    c.no_proof = Some(np);
                }
            })
        })
        .params(r#"{"lang": "en-US|en-GB|fr-FR|…", "noProof"?: bool}"#),
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
            s.run(id, &json!({}))
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
            s.view.focus_mode = s.view.read_mode;
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
            s.view.fit = "multiplePages".into();
            Ok(json!({"pageMovement": "sideToSide"}))
        })
        .pure(),
    ]
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
    let f = s.sel.focus.clone();
    let Some(text) = s.doc.para_at(&f).map(|p| p.text.clone()) else { return Ok(()) };
    let Some(before) = text.get(..f.off) else { return Ok(()) };
    let Some(last) = before.chars().next_back() else { return Ok(()) };
    if !on_enter && !(last == ' ' || ",.;:!?".contains(last)) {
        return Ok(());
    }
    let trigger = if on_enter { 0 } else { last.len_utf8() };
    let Some(body) = before.get(..before.len() - trigger) else { return Ok(()) };
    let start = body.rfind(char::is_whitespace).map(|i| i + 1).unwrap_or(0);
    let Some(word) = body.get(start..).map(str::to_string) else { return Ok(()) };
    if word.is_empty() {
        return Ok(());
    }
    // A whole-word replacement, else "wait..." → "wait…".
    let found = if ac.replace_text { ac.replacement(&word) } else { None };
    let (from, rep) = match found {
        Some(r) => (start, Some(r)),
        None if ac.replace_text && word.len() > 3 && word.ends_with("...") => (start + word.len() - 3, Some("…".to_string())),
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
    s.doc.revisions.push(wordcraft_doc::Revision { kind: RevisionKind::Insert, author: author.clone(), date: date.clone() });
    let ins = (s.doc.revisions.len() - 1) as u32;
    s.doc.revisions.push(wordcraft_doc::Revision { kind: RevisionKind::Delete, author: author.clone(), date: date.clone() });
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
    let mut issues = Vec::new();
    let mut last_level: Option<u8> = None;
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        for o in &p.objects {
            if let InlineObject::Image { alt, .. } = o
                && alt.trim().is_empty()
            {
                issues.push(json!({"kind": "error", "issue": "Missing alternative text", "where": path.0, "fix": "Select the picture and run picture.altText"}));
            }
            // Charts and diagrams can't be given alt text yet (see objects.rs), so only report them.
            if let InlineObject::Graphic { alt, .. } = o
                && alt.trim().is_empty()
            {
                issues.push(json!({"kind": "error", "issue": "Chart or diagram has no alternative text", "where": path.0}));
            }
        }
        if let Some(l) = s.doc.styles.resolve_para(&p.props).outline_level {
            if let Some(prev) = last_level
                && l > prev + 1
            {
                issues.push(json!({"kind": "warning", "issue": format!("Heading level skipped (Heading {} after Heading {})", l + 1, prev + 1), "where": path.0}));
            }
            last_level = Some(l);
        }
        if p.runs.iter().any(|r| r.props.link.is_some()) {
            let txt = p.plain_text().to_lowercase();
            if txt.contains("click here") {
                issues.push(json!({"kind": "tip", "issue": "Link text \"click here\" isn't descriptive", "where": path.0}));
            }
        }
    }
    for (i, b) in s.doc.body.iter().enumerate() {
        if let Block::Table(t) = &**b
            && !t.rows.first().is_some_and(|r| r.props.header)
        {
            issues.push(json!({"kind": "warning", "issue": "Table has no header row", "where": [i], "fix": "table.repeatHeader on the first row"}));
        }
    }
    if s.doc.core.title.is_empty() {
        issues.push(json!({"kind": "tip", "issue": "Document has no title", "fix": "file.properties {title}"}));
    }
    Ok(json!({"issues": issues, "ok": issues.is_empty()}))
}

fn inspect_doc(s: &mut Session, v: &Value) -> CmdResult {
    let remove: Vec<String> =
        v.get("remove").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let has_hidden = s
        .doc
        .para_paths(StoryRef::Body)
        .iter()
        .any(|p| s.doc.para(StoryRef::Body, p).is_some_and(|x| x.runs.iter().any(|r| r.props.hidden == Some(true))));
    let report = json!({
        "comments": s.doc.comments.len(),
        "revisions": s.doc.para_paths(StoryRef::Body).iter().filter(|p| s.doc.para(StoryRef::Body, p).is_some_and(|x| x.runs.iter().any(|r| r.props.ins.is_some() || r.props.del.is_some()))).count(),
        "properties": !s.doc.core.creator.is_empty() || !s.doc.core.last_modified_by.is_empty(),
        "hiddenText": has_hidden,
        "headers": s.doc.parts.values().filter(|p| matches!(p.kind, wordcraft_doc::PartKind::Header | wordcraft_doc::PartKind::Footer)).count(),
    });
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
            "hidden" => {
                for path in s.doc.para_paths(StoryRef::Body) {
                    let ranges: Vec<(usize, usize)> = s
                        .doc
                        .para(StoryRef::Body, &path)
                        .map(|p| p.run_ranges().filter(|(_, c)| c.hidden == Some(true)).map(|(r, _)| (r.start, r.end)).collect())
                        .unwrap_or_default();
                    let para = s.doc.para_mut(StoryRef::Body, &path)?;
                    for (a, b) in ranges.into_iter().rev() {
                        para.delete(a, b)?;
                    }
                }
            }
            "headers" => {
                s.run("insert.removeHeader", &json!({}))?;
                s.run("insert.removeFooter", &json!({}))?;
            }
            _ => {}
        }
    }
    s.clamp_selection();
    Ok(json!({"found": report, "removed": remove}))
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
}
