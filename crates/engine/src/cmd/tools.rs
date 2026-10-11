//! Tools and the long tail: Repeat, macros, AutoCorrect, Compare, accessibility checker,
//! document inspector, restrict editing, versions, templates, Quick Parts, and more.

use serde_json::{Value, json};
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
        CommandSpec::new("tools.autocorrect", "AutoCorrect Options", "File › Options › Proofing", |s, v| {
            if let Some(b) = p::bool(v, "enabled") {
                s.autocorrect_on = b;
            }
            if let (Some(f), Some(t)) =
                (v.get("add").and_then(|a| a.get("from")).and_then(Value::as_str), v.get("add").and_then(|a| a.get("to")).and_then(Value::as_str))
            {
                s.autocorrect_user.retain(|(a, _)| a != f);
                s.autocorrect_user.push((f.to_string(), t.to_string()));
            }
            Ok(json!({"enabled": s.autocorrect_on, "entries": AUTOCORRECT.len() + s.autocorrect_user.len(), "user": s.autocorrect_user}))
        })
        .params(r#"{"enabled"?: bool, "add"?: {"from": string, "to": string}}"#)
        .pure(),
        CommandSpec::new("review.compare", "Compare", "Review › Compare", compare).params(r#"{"path"?: string, "text"?: string (revised version)}"#),
        CommandSpec::new("review.combine", "Combine", "Review › Compare", compare).params(r#"{"path"?: string}"#),
        CommandSpec::new("file.accessibility", "Check Accessibility", "Review › Accessibility", accessibility).pure(),
        CommandSpec::new("file.inspect", "Inspect Document", "File › Info", inspect_doc)
            .params(r#"{"remove"?: ["comments", "revisions", "properties", "hidden", "headers"]}"#),
        CommandSpec::new("review.restrict", "Restrict Editing", "Review › Protect", |s, v| {
            let mode = p::str(v, "mode").unwrap_or("readOnly").to_string();
            s.doc.settings.protection = if mode == "none" { None } else { Some(mode.clone()) };
            if mode == "trackedChanges" {
                s.doc.settings.track_changes = true;
            }
            Ok(json!({"protection": s.doc.settings.protection}))
        })
        .params(r#"{"mode": "none|readOnly|comments|trackedChanges|forms"}"#),
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
        CommandSpec::new("review.language", "Language", "Review › Language", language).params(
            r#"{"lang"?: "en-US|en-GB|fr-FR|…", "noProof"?: bool, "detect"?: bool (detect language automatically, a preference), "default"?: bool (make `lang` the document's default language instead)}"#,
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

/// Set Proofing Language: the selection's language and "do not check spelling or grammar",
/// or (`default`) the document's default language; `detect` is the per-user preference.
/// Nothing at all sets US English, as before.
fn language(s: &mut Session, v: &Value) -> CmdResult {
    let lang = match p::str(v, "lang") {
        Some(l) if !l.is_empty() && l.len() <= 35 && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') => Some(l.to_string()),
        Some(l) => return Err(CmdError::Params(format!("`{l}` is not a language tag"))),
        None => None,
    };
    let no_proof = p::bool(v, "noProof");
    let detect = p::bool(v, "detect");
    if let Some(d) = detect {
        s.prefs.detect_language = d;
    }
    if p::bool(v, "default") == Some(true) {
        let l = lang.ok_or_else(|| CmdError::Params("`lang` is required with `default`".into()))?;
        s.doc.styles.default_chr.lang = Some(l.clone());
        // A Normal style that names its own language follows the new default.
        if let Some(n) = s.doc.styles.styles.iter_mut().find(|x| x.id == "Normal")
            && n.chr.lang.is_some()
        {
            n.chr.lang = Some(l.clone());
        }
        return Ok(json!({"default": l, "detect": s.prefs.detect_language}));
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
    if !s.autocorrect_on {
        return Ok(());
    }
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
    let user = s.autocorrect_user.iter().find(|(a, _)| *a == word).map(|(_, b)| b.clone());
    let builtin = AUTOCORRECT
        .iter()
        .find(|(a, _)| {
            *a == word || (a.chars().all(|c| c.is_lowercase()) && word.to_lowercase() == *a && word.chars().next().is_some_and(char::is_uppercase))
        })
        .map(|(a, b)| {
            if *a != word && b.chars().next().is_some_and(char::is_lowercase) {
                let mut c = b.chars();
                c.next().map(|x| x.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
            } else {
                b.to_string()
            }
        });
    // A whole-word replacement, else "wait..." → "wait…".
    let (from, rep) = match user.or(builtin) {
        Some(r) => (start, Some(r)),
        None if word.len() > 3 && word.ends_with("...") => (start + word.len() - 3, Some("…".to_string())),
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
    if let Some(n) = ordinal_suffix(&word) {
        s.doc.format_range(&at(end - n), &at(end), &|c| c.vert_align = Some(wordcraft_doc::props::VertAlign::Superscript))?;
        return Ok(());
    }
    // Web addresses become links (trailing punctuation stays outside).
    let lower = word.to_ascii_lowercase();
    if (lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("www.")) && word.len() > 8 {
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
    if starts_sentence(text.get(..start).unwrap_or("")) && should_capitalize(&word, if on_enter { '\n' } else { last }) {
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

/// Does a word typed after `before` (the paragraph text up to the word) begin a sentence?
fn starts_sentence(before: &str) -> bool {
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
    !initial && !NOT_SENTENCE_END.contains(&lower.as_str())
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

/// Compare the current document (original) with a revised one: the result shows the
/// differences as tracked insertions and deletions.
fn compare(s: &mut Session, v: &Value) -> CmdResult {
    let revised = if let Some(t) = p::str(v, "text") {
        Document::from_text(t)
    } else if let Some(path) = p::str(v, "path") {
        crate::io::open_path(std::path::Path::new(path)).map_err(CmdError::Failed)?
    } else {
        return Err(CmdError::Params("`path` or `text` of the revised document is required".into()));
    };
    let old: Vec<Paragraph> = s.doc.body.iter().filter_map(|b| b.as_para().cloned()).collect();
    let new: Vec<Paragraph> = revised.body.iter().filter_map(|b| b.as_para().cloned()).collect();
    let author = "Compare".to_string();
    let date = super::now_iso();
    s.doc.revisions.push(wordcraft_doc::Revision { kind: RevisionKind::Insert, author: author.clone(), date: date.clone() });
    let ins = (s.doc.revisions.len() - 1) as u32;
    s.doc.revisions.push(wordcraft_doc::Revision { kind: RevisionKind::Delete, author, date });
    let del = (s.doc.revisions.len() - 1) as u32;
    let ops = lcs_ops(&old.iter().map(|p| p.plain_text()).collect::<Vec<_>>(), &new.iter().map(|p| p.plain_text()).collect::<Vec<_>>());
    let mut out: Vec<Paragraph> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    let mut changes = 0usize;
    let mut k = 0;
    while k < ops.len() {
        match ops.get(k) {
            Some(Op::Same) => {
                if let Some(p) = old.get(i) {
                    out.push(p.clone());
                }
                i += 1;
                j += 1;
                k += 1;
            }
            Some(Op::Del) if matches!(ops.get(k + 1), Some(Op::Ins)) => {
                // A changed paragraph: diff its words.
                let (a, b) = (old.get(i).cloned().unwrap_or_default(), new.get(j).cloned().unwrap_or_default());
                out.push(word_diff(&a, &b, ins, del));
                changes += 1;
                i += 1;
                j += 1;
                k += 2;
            }
            Some(Op::Del) => {
                let mut p = old.get(i).cloned().unwrap_or_default();
                let len = p.len();
                let _ = p.format(0, len, &|c| c.del = Some(del));
                out.push(p);
                changes += 1;
                i += 1;
                k += 1;
            }
            Some(Op::Ins) => {
                let mut p = new.get(j).cloned().unwrap_or_default();
                let len = p.len();
                let _ = p.format(0, len, &|c| c.ins = Some(ins));
                out.push(p);
                changes += 1;
                j += 1;
                k += 1;
            }
            None => break,
        }
    }
    s.doc.body = out.into_iter().map(para_block).collect();
    s.doc.ensure_nonempty();
    s.sel = Selection::caret(s.doc.start_of(StoryRef::Body));
    Ok(json!({"changes": changes}))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Same,
    Del,
    Ins,
}

/// LCS edit script between two sequences (bounded size).
fn lcs_ops<T: PartialEq>(a: &[T], b: &[T]) -> Vec<Op> {
    let (n, m) = (a.len().min(4000), b.len().min(4000));
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
    let mut ops = Vec::new();
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
    ops.extend(std::iter::repeat_n(Op::Del, n - i));
    ops.extend(std::iter::repeat_n(Op::Ins, m - j));
    ops
}

fn tokens(s: &str) -> Vec<String> {
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

fn word_diff(a: &Paragraph, b: &Paragraph, ins: u32, del: u32) -> Paragraph {
    let (ta, tb) = (tokens(&a.plain_text()), tokens(&b.plain_text()));
    let ops = lcs_ops(&ta, &tb);
    let mut p = Paragraph::new();
    p.props = b.props.clone();
    let base = a.runs.first().map(|r| r.props.clone()).unwrap_or_default();
    let (mut i, mut j) = (0, 0);
    for op in ops {
        let at = p.len();
        match op {
            Op::Same => {
                let _ = p.insert_text(at, ta.get(i).map(String::as_str).unwrap_or(""), &base);
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
