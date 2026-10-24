//! File tab (Backstage): new, open, save, export, print, properties; document inspection.

use serde_json::{Value, json};
use wordcraft_doc::{Block, Document, StoryRef};

use super::{pos_json, sel_result};
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// Longest user name kept, in characters; longer names are cut, not refused.
/// `format` of the save commands for an `.xml` file: Word 2003 XML Document (WordprocessingML 2003).
pub const FORMAT_WORD_2003: &str = "wordml2003";
/// `format` for an `.xml` file as a Word XML Document (Flat OPC), the default.
pub const FORMAT_FLAT_OPC: &str = "flatopc";

pub const MAX_AUTHOR_CHARS: usize = 255;

/// The user name as it is stored: trimmed, stripped of control characters (they can't go into
/// the saved XML) and cut to [`MAX_AUTHOR_CHARS`]; a blank name is refused so revisions always
/// carry an author.
pub fn normalize_author(name: &str) -> Result<String, CmdError> {
    let name: String = name.trim().chars().filter(|c| !c.is_control()).take(MAX_AUTHOR_CHARS).collect();
    let name = name.trim_end();
    if name.is_empty() {
        return Err(CmdError::Params("`name` must not be blank".into()));
    }
    Ok(name.to_string())
}

/// Sets the user name that new tracked changes and comments are recorded under. The name is
/// normalised by [`normalize_author`].
/// A protected document refuses it: the name is the identity its exceptions are checked against.
pub(crate) fn set_author(s: &mut Session, v: &Value) -> CmdResult {
    super::protect::check_user_name(s)?;
    s.author = normalize_author(p::req_str(v, "name")?)?;
    let mut r = sel_result(s)?;
    if let Some(o) = r.as_object_mut() {
        o.insert("name".into(), json!(s.author));
    }
    Ok(r)
}

/// Rename the current file in its directory, or name a document before its first save.
fn rename(s: &mut Session, v: &Value) -> CmdResult {
    let name = p::req_str(v, "name")?.trim();
    if name.is_empty()
        || name.chars().count() > 200
        || name.ends_with(['.', ' '])
        || name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        || matches!(name, "." | "..")
    {
        return Err(CmdError::Params("Enter a valid document name (up to 200 characters, without path separators).".into()));
    }
    if let Some(path) = &s.path {
        let mut destination = path.with_file_name(name);
        // The field edits the stem; retain the original format, including macro/template files.
        // A name that already ends in that extension is taken as the whole file name, since a
        // script or an agent passes one ("renamed.docx" became "renamed.docx.docx", #2983). The
        // extension alone (".docx") is a stem, not a file name, so it still gets one.
        if let Some(ext) = path.extension() {
            let ext = ext.to_string_lossy();
            // Split at the name's own last dot rather than counting bytes back from its end: a
            // name whose characters are more than one byte each ("résumé.docm") would be cut
            // inside one of them, and slicing there panics (#3128 review).
            let given = name.rsplit_once('.').is_some_and(|(stem, suffix)| !stem.is_empty() && suffix.eq_ignore_ascii_case(ext.as_ref()));
            destination.set_file_name(if given { name.to_string() } else { format!("{name}.{ext}") });
        }
        if destination != *path {
            #[cfg(not(target_arch = "wasm32"))]
            {
                crate::io::rename_file(path, &destination).map_err(CmdError::Failed)?;
            }
            s.path = Some(destination);
            s.relayout();
        }
    } else {
        s.file_name = Some(name.to_string());
        s.relayout();
    }
    Ok(json!({"name": name, "path": s.path}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("file.rename", "Rename Document", "File › Info", rename).params(r#"{"name": string (document name; its extension is kept, so the name may be given with or without it)}"#).pure(),
        CommandSpec::new("file.new", "New", "File", new).key("Mod+N").params(r#"{"template"?: "blank|sample|letter|resume|report"}"#).pure(),
        CommandSpec::new("file.open", "Open", "File", open)
            .key("Mod+O / Mod+F12 / Mod+Alt+F2")
            .params(r#"{"path": string, "data"?: base64 (the file's bytes; `path` then only names it), "password"?: string (for a password-protected Word document)} → {path, paragraphs, words, encrypted, "incomplete"?: [{part, kind, reason}] (Word embedded objects — charts, diagrams, OLE objects, kept parts — the file doesn't hold whole; kind: missingPart|partLimit|relationshipLimit|notCarried|truncated)}"#)
            .pure(),
        CommandSpec::new("file.save", "Save", "File", save)
            .key("Mod+S / Shift+F12 / Alt+Shift+F2")
            .params(r#"{"path"?: string, "password"?: string (encrypt this and later saves with it, as file.encrypt does), "markup"?: bool (Print Markup for a PDF, PNG, HTML, Markdown or text copy: comments shown; default file.printMarkup), "format"?: "wordml2003" (an .xml file as a Word 2003 XML Document) | "flatopc" (as a Word XML Document, the default; saving over a Word 2003 XML file keeps that format)} → {saved, path, encrypted, "dropped"?: [{part, kind, reason}] (what a Word package couldn't carry of embedded objects; kind: missingPart|partLimit|relationshipLimit|notCarried|leftOut|danglingReference|truncated)}"#)
            .pure(),
        CommandSpec::new("file.saveAs", "Save As", "File", save_as).key(super::mac_or("F12 / Mod+Shift+S", "F12 / Alt+F2")).params(r#"{"path": string, "password"?: string, "markup"?: bool, "format"?: string (as file.save)} → as file.save, "dropped" included"#).pure(),
        // Pure: the password must not land in Repeat or a recorded macro, and it isn't an edit
        // to undo; it marks the document as changed itself.
        CommandSpec::new("file.encrypt", "Encrypt with Password", "File › Info › Protect Document", encrypt)
            .params(r#"{"password": string (Word documents are saved encrypted with it) | null or "" (saved unencrypted)}"#)
            .pure(),
        CommandSpec::new("file.exportPdf", "Export PDF", "File › Export", |s, v| {
            // No path (the command search, say): the app asks where to save, as File › Export
            // does, instead of failing on the required parameter (#2313).
            let Some(path) = p::str(v, "path") else {
                s.push_ui_request(json!({"open": "exportPdf"}));
                return Ok(json!({"exported": false}));
            };
            let path = if path.to_ascii_lowercase().ends_with(".pdf") { path.to_string() } else { format!("{path}.pdf") };
            let opts = export_options(s, v);
            crate::io::save_path_with(std::path::Path::new(&path), &s.doc, None, &opts).map_err(CmdError::Failed)?;
            Ok(json!({"path": path, "markup": opts.markup}))
        })
        .params(r#"{"path": string, "markup"?: bool (Print Markup: tracked changes marked up and each page scaled down beside the comments in balloons; default file.printMarkup)}"#)
        .pure(),
        CommandSpec::new("file.exportPng", "Export Page as PNG", "File › Export", export_png)
            .params(r#"{"path": string, "page"?: n (1-based), "scale"?: px per pt, "markup"?: bool (the page with its comments in balloons beside it; default file.printMarkup)}"#)
            .pure(),
        // A print setting, like Word's: not part of the document, so no undo step.
        CommandSpec::new("file.printMarkup", "Print Markup", "File › Print", |s, v| {
            s.prefs.print_markup = p::bool(v, "value").unwrap_or(!s.prefs.print_markup);
            Ok(json!({"value": s.prefs.print_markup}))
        })
        .params(r#"{"value"?: bool (omit to toggle)} — PDFs, prints and copies show comments (balloons beside each page; a list at the end of HTML, Markdown and text) and tracked changes"#)
        .pure(),
        CommandSpec::new("file.printPdf", "Print to PDF", "File › Print", crate::print::print_pdf_to_file)
            .params(
                r#"{"path": string, "range"?: "all" | "current" | "selection" (the pages the selection touches) | "custom", "pages"?: "1-3,5" (implies custom), "parity"?: "all" | "odd" | "even", "copies"?: 1-999, "collate"?: bool (default true), "paper"?: "none" | "Letter" | "Legal" | "A4" | … (a Layout › Size name: scale every page to this paper, centred; the document keeps its size)}"#,
            )
            .pure(),
        CommandSpec::new("file.print", "Print", "File", |s, _| {
            s.push_ui_request(json!({"open": "print"}));
            sel_result(s)
        })
        .key("Mod+P / Mod+Shift+F12 / Mod+F2 / Mod+Alt+I")
        .pure(),
        CommandSpec::new("file.close", "Close", "File", |s, _| {
            s.push_ui_request(json!({"close": true}));
            sel_result(s)
        })
        .key("Mod+W / Mod+F4")
        .params(r#"{"force"?: bool (desktop app's control channel and MCP bridge only: without it a document with unsaved changes is an error and stays open; with it the changes are discarded. Headless sessions never ask)}"#)
        .pure(),
        CommandSpec::new("file.properties", "Properties", "File › Info", properties)
            .params(r#"{"title"?, "subject"?, "author"?, "keywords"?, "comments"?, "category"?, "custom"?: {name: string | null (removes)}}"#),
        CommandSpec::new("file.info", "Info", "File", info).pure(),
        CommandSpec::new("file.setAuthor", "User Name", "File › Options › General", set_author)
            .params(r#"{"name": string (author of new tracked changes and comments)}"#)
            .pure(),
        // Trust Center › Privacy Options: the document's `w:removePersonalInformation` and
        // `w:removeDateAndTime` settings, honoured when it is saved as .docx (#1062).
        // Changing them is a document change (undoable, refused on a protected document); reading
        // them is `file.privacy`, which changes nothing.
        CommandSpec::new("file.privacyOptions", "Privacy Options", "File › Options › Save", |s, v| {
            let personal = p::bool(v, "removePersonalInfo");
            let date_time = p::bool(v, "removeDateTime");
            if personal.is_none() && date_time.is_none() {
                return Err(CmdError::Params("give removePersonalInfo or removeDateTime (file.privacy reads them)".into()));
            }
            if let Some(on) = personal {
                s.doc.settings.remove_personal_info = on;
            }
            if let Some(on) = date_time {
                s.doc.settings.remove_date_time = on;
            }
            Ok(privacy(s))
        })
        .params(r#"{"removePersonalInfo"?: bool (save comment and revision authors as "Author", no creator or last-modified-by), "removeDateTime"?: bool (save comments and tracked changes without dates)} — at least one; the open document keeps its real names and dates"#),
        CommandSpec::new("file.privacy", "Privacy Settings", "File › Options › Save", |s, _| Ok(privacy(s)))
            .params(r#"{} → {"removePersonalInfo": bool, "removeDateTime": bool} (file.privacyOptions changes them)"#)
            .pure(),
        CommandSpec::new("document.inspect", "Inspect Document", "Agents", inspect).params(r#"{"text"?: bool}"#).pure(),
        CommandSpec::new("document.text", "Document Text", "Agents", |s, v| {
            let story = super::story_param(s, v);
            Ok(json!({"text": s.doc.final_text(story)}))
        })
        .pure(),
        CommandSpec::new("document.paragraph", "Paragraph Details", "Agents", paragraph).params(r#"{"path": [n], "story"?: Story}"#).pure(),
        CommandSpec::new("document.selection", "Selection", "Agents", |s, _| {
            let mut r = sel_result(s)?;
            if let Some(o) = r.as_object_mut() {
                o.insert("text".into(), json!(s.selected_text()));
                o.insert("format".into(), super::format::state(s));
            }
            Ok(r)
        })
        .pure(),
        CommandSpec::new("document.layout", "Layout Summary", "Agents", layout_summary).pure(),
        CommandSpec::new("document.setText", "Replace Document Text", "Agents", |s, v| {
            let text = p::req_str(v, "text")?;
            let d = Document::from_text(text);
            s.doc.body = d.body;
            s.sel = crate::Selection::caret(s.doc.start_of(StoryRef::Body));
            sel_result(s)
        })
        .params(r#"{"text": string}"#),
    ]
}

/// The document's privacy settings, as `file.privacy` and `file.privacyOptions` return them.
fn privacy(s: &Session) -> Value {
    json!({"removePersonalInfo": s.doc.settings.remove_personal_info, "removeDateTime": s.doc.settings.remove_date_time})
}

/// Create a blank document in the saved proofing language, or use the requested template.
fn new(s: &mut Session, v: &Value) -> CmdResult {
    let doc = match p::str(v, "template").unwrap_or("blank") {
        "sample" => crate::sample::sample_document(),
        "letter" => crate::sample::letter(),
        "resume" => crate::sample::resume(),
        "report" => crate::sample::report(),
        _ => normal_document(s),
    };
    s.set_document(doc);
    s.path = None;
    sel_result(s)
}

/// A blank document on the Normal template: the region's paper (#838), written in the editing
/// language (the templates' text is English).
fn normal_document(s: &Session) -> Document {
    let mut d = s.blank_document();
    if let Some(lang) = s.prefs.default_language.as_ref().or(s.editing_lang.as_ref()) {
        d.styles.default_chr.lang = Some(lang.clone());
    }
    d
}

/// Open a document, using the saved proofing default only for plain text and Markdown.
fn open(s: &mut Session, v: &Value) -> CmdResult {
    let Some(path) = p::str(v, "path") else {
        s.push_ui_request(json!({"open": "openFile"}));
        return sel_result(s);
    };
    let password = p::str(v, "password");
    let (mut doc, encrypted) = if let Some(data) = p::str(v, "data") {
        let bytes = super::insert::base64_decode(data).ok_or_else(|| CmdError::Params("bad base64".into()))?;
        // An empty file (File Explorer's New › Microsoft Word Document makes one) opens as a new
        // blank document that saves back to it (#1384).
        if bytes.is_empty() {
            (normal_document(s), false)
        } else {
            crate::io::open_bytes_in(path, &bytes, password, &s.region()).map_err(CmdError::Failed)?
        }
    } else if crate::io::is_empty_file(std::path::Path::new(path)) {
        (normal_document(s), false)
    } else {
        crate::io::open_path_in(std::path::Path::new(path), password, &s.region()).map_err(CmdError::Failed)?
    };
    // Plain text and Markdown carry no language: they are in the editing language.
    let ext = std::path::Path::new(path).extension().and_then(std::ffi::OsStr::to_str).unwrap_or("").to_ascii_lowercase();
    if let Some(lang) = s.prefs.default_language.as_ref().or(s.editing_lang.as_ref())
        && matches!(ext.as_str(), "txt" | "text" | "md" | "markdown")
    {
        doc.styles.default_chr.lang = Some(lang.clone());
    }
    // Embedded objects the file doesn't hold whole (#1343).
    let incomplete = wordcraft_docx::incomplete_objects(&doc);
    s.set_document(doc);
    if let Some(z) = s.doc.settings.zoom {
        super::view::restore_zoom(s, z);
    }
    s.path = Some(path.into());
    // Like Word, a document opened with a password is saved with it again.
    if encrypted {
        s.password = password.map(crate::io::Password::new);
    }
    let mut out = json!({"path": path, "paragraphs": s.doc.paragraph_count(), "words": s.doc.word_count(), "encrypted": encrypted});
    if !incomplete.is_empty() {
        out["incomplete"] = losses_json(&incomplete);
    }
    Ok(out)
}

/// `[{"part", "kind", "reason"}…]` for what a Word package lacked or lost (`kind`: see
/// [`wordcraft_docx::LossKind::as_str`]).
pub(crate) fn losses_json(losses: &[wordcraft_docx::Loss]) -> Value {
    losses.iter().map(|l| json!({"part": l.part, "kind": l.kind.as_str(), "reason": l.reason})).collect()
}

/// File › Info › Protect Document › Encrypt with Password: the password Word documents are saved
/// with from now on, or none.
fn encrypt(s: &mut Session, v: &Value) -> CmdResult {
    let password = match v.get("password") {
        Some(Value::Null) => None,
        Some(Value::String(pw)) if pw.is_empty() => None,
        Some(Value::String(pw)) => {
            wordcraft_docx::check_password(pw).map_err(|e| CmdError::Params(e.to_string()))?;
            Some(crate::io::Password::new(pw))
        }
        _ => return Err(CmdError::Params("`password` must be a string, or null to remove the password".into())),
    };
    if password != s.password {
        if password.is_some() {
            s.clear_recovery_before_password()?;
        }
        s.password = password;
        s.dirty = true;
    }
    Ok(json!({"encrypted": s.password.is_some()}))
}

fn save(s: &mut Session, v: &Value) -> CmdResult {
    let path = match p::str(v, "path").map(std::path::PathBuf::from).or_else(|| s.path.clone()) {
        Some(p) => p,
        None => {
            s.push_ui_request(json!({"open": "saveAs"}));
            return Ok(json!({"saved": false}));
        }
    };
    if let Some(pw) = v.get("password").filter(|pw| !pw.is_null()) {
        let Some(pw) = pw.as_str().filter(|pw| !pw.is_empty()) else {
            return Err(CmdError::Params("`password` must be a non-empty string (file.encrypt removes a password)".into()));
        };
        if !crate::io::can_encrypt(&path.to_string_lossy()) {
            return Err(CmdError::Params("only Word documents (.docx, .docm, .dotx, .dotm) can be saved with a password".into()));
        }
        wordcraft_docx::check_password(pw).map_err(|e| CmdError::Params(e.to_string()))?;
        s.clear_recovery_before_password()?;
        s.password = Some(crate::io::Password::new(pw));
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let mut opts = export_options(s, v);
    match v.get("format").filter(|f| !f.is_null()) {
        None => {
            // Saving over a Word 2003 XML file (one opened as such, say) keeps its format.
            opts.word2003 = ext == "xml" && is_word_2003_file(&path);
        }
        Some(f) if matches!(f.as_str(), Some(FORMAT_WORD_2003 | FORMAT_FLAT_OPC)) && ext == "xml" => {}
        Some(_) => {
            return Err(CmdError::Params(format!("`format` is \"{FORMAT_WORD_2003}\" or \"{FORMAT_FLAT_OPC}\", and only for an .xml file")));
        }
    }
    // Undone below if the write fails: a failed save (no permission, disk full, a bad path)
    // must not leave the document's own save stamps — zoom, modified/created, author, revision —
    // changed, or a later save would persist them for a write that never actually happened.
    let before_stamp =
        (s.doc.settings.zoom, s.doc.core.last_modified_by.clone(), s.doc.core.created.clone(), s.doc.core.modified.clone(), s.doc.core.revision);
    s.stamp_save();
    let password = s.password.as_ref().map(crate::io::Password::as_str);
    // A header/footer just made and left empty isn't saved (#960).
    let copy = super::insert::saved_copy(s);
    let losses = match crate::io::save_path_report(&path, copy.as_ref().unwrap_or(&s.doc), password, &opts) {
        Ok(losses) => losses,
        Err(e) => {
            (s.doc.settings.zoom, s.doc.core.last_modified_by, s.doc.core.created, s.doc.core.modified, s.doc.core.revision) = before_stamp;
            return Err(CmdError::Failed(e));
        }
    };
    let encrypted = password.is_some() && crate::io::can_encrypt(&path.to_string_lossy());
    // A save in a format that doesn't keep everything (plain text, Markdown, a page image) is a
    // copy, except over the document's own file: a note.txt opened and saved is saved (#2376).
    let own_file = s.path.as_deref() == Some(path.as_path());
    if own_file || ["docx", "docm", "dotx", "dotm", "odt", "rtf", "json", "xml"].contains(&ext.as_str()) {
        s.path = Some(path.clone());
        s.dirty = false;
        s.relayout();
    }
    let mut out = json!({"saved": true, "path": path.to_string_lossy(), "encrypted": encrypted});
    // What a Word package couldn't carry of the embedded objects (#1343).
    if !losses.is_empty() {
        out["dropped"] = losses_json(&losses);
    }
    Ok(out)
}

/// Does `path` hold a Word 2003 XML Document already (so a plain save keeps that format)?
#[cfg(not(target_arch = "wasm32"))]
fn is_word_2003_file(path: &std::path::Path) -> bool {
    // The root element and its namespaces come first.
    let Ok(head) = crate::io::read_file_head(&path.to_string_lossy(), 64 * 1024) else { return false };
    wordcraft_docx::sniff_xml(&head) == wordcraft_docx::XmlKind::WordMl2003
}

#[cfg(target_arch = "wasm32")]
fn is_word_2003_file(_path: &std::path::Path) -> bool {
    false
}

fn save_as(s: &mut Session, v: &Value) -> CmdResult {
    if p::str(v, "path").is_none() {
        s.push_ui_request(json!({"open": "saveAs"}));
        return Ok(json!({"saved": false}));
    }
    save(s, v)
}

/// How a copy is written: Print Markup from the command's `markup`, else the File › Print
/// setting (`file.printMarkup`); comments as Track Changes Options show them; digits as the
/// Numeral setting shows them on screen.
pub fn export_options(s: &Session, v: &Value) -> crate::io::ExportOptions {
    let markup = p::bool(v, "markup").unwrap_or(s.prefs.print_markup);
    crate::io::ExportOptions {
        markup,
        comments: markup && s.prefs.markup.comments,
        word2003: p::str(v, "format") == Some(FORMAT_WORD_2003),
        numeral: s.view.numeral.resolve(crate::session::system_locale()),
    }
}

fn export_png(s: &mut Session, v: &Value) -> CmdResult {
    // No path: the app asks where to save, as file.exportPdf does (#2313).
    let Some(path) = p::str(v, "path") else {
        s.push_ui_request(json!({"open": "exportPng"}));
        return Ok(json!({"exported": false}));
    };
    let page = p::u64(v, "page").unwrap_or(1).max(1) as usize - 1;
    let scale = p::f32(v, "scale").unwrap_or(2.0).clamp(0.1, 8.0);
    let l = s.export_layout_with(export_options(s, v).markup);
    let pg = l.pages.get(page).ok_or_else(|| CmdError::Params(format!("no page {}", page + 1)))?;
    // A page image is the printed page: headers and footers at full strength. Dimming them is
    // the on-screen cue while editing the body, not part of the page.
    let mut opts = wordcraft_render::RenderOptions::default();
    opts.display.dim_header = false;
    if !path.to_ascii_lowercase().ends_with(".png") {
        return Err(CmdError::Params("path must end in .png".into()));
    }
    let img = wordcraft_render::render_page(&s.doc, pg, scale, &opts);
    let png = img.to_png();
    crate::io::write_file(path, &png).map_err(CmdError::Failed)?;
    Ok(json!({"path": path, "width": img.width, "height": img.height, "bytes": png.len()}))
}

fn properties(s: &mut Session, v: &Value) -> CmdResult {
    let c = &mut s.doc.core;
    let set = |dst: &mut String, k: &str| {
        if let Some(x) = p::str(v, k) {
            *dst = x.to_string();
        }
    };
    set(&mut c.title, "title");
    set(&mut c.subject, "subject");
    set(&mut c.creator, "author");
    set(&mut c.keywords, "keywords");
    set(&mut c.description, "comments");
    set(&mut c.category, "category");
    if let Some(custom) = v.get("custom") {
        let Some(m) = custom.as_object() else {
            return Err(CmdError::Params("`custom` must be an object of name → string or null".into()));
        };
        for (name, val) in m {
            if name.trim().is_empty() || name.chars().count() > MAX_CUSTOM_NAME {
                return Err(CmdError::Params(format!("custom property names are 1–{MAX_CUSTOM_NAME} characters")));
            }
            match val {
                Value::Null => {
                    s.doc.remove_custom_prop(name);
                }
                Value::String(x) if x.len() <= MAX_CUSTOM_VALUE => s.doc.set_custom_prop(name, x),
                _ => return Err(CmdError::Params(format!("custom property `{name}` must be a string (≤ {MAX_CUSTOM_VALUE} bytes) or null"))),
            }
        }
    }
    let mut out = serde_json::to_value(&s.doc.core).unwrap_or(Value::Null);
    if let Some(o) = out.as_object_mut() {
        o.insert("custom".into(), serde_json::to_value(&s.doc.custom_props).unwrap_or(Value::Null));
    }
    Ok(out)
}

/// Longest custom property name (Word's limit).
const MAX_CUSTOM_NAME: usize = 255;
/// Largest custom property value we accept from a command.
const MAX_CUSTOM_VALUE: usize = 1024 * 1024;

fn info(s: &mut Session, _: &Value) -> CmdResult {
    let l = s.layout();
    Ok(json!({
        "path": s.path.as_ref().map(|p| p.to_string_lossy().to_string()),
        "dirty": s.dirty,
        "pages": l.pages.len(),
        "words": s.doc.word_count(),
        "paragraphs": s.doc.paragraph_count(),
        "properties": s.doc.core,
        "trackChanges": s.doc.settings.track_changes,
        "comments": s.doc.comments.len(),
        "sections": s.doc.sections().len(),
        "encrypted": s.password.is_some(),
        "layoutMs": l.ms,
    }))
}

/// Structure for agents: blocks with text and formatting summary, selection, stats.
fn inspect(s: &mut Session, v: &Value) -> CmdResult {
    let with_text = p::bool(v, "text").unwrap_or(true);
    let mut blocks = Vec::new();
    for (i, b) in s.doc.body.iter().enumerate().take(2000) {
        match &**b {
            Block::Para(p) => {
                let style = p.props.style.clone().unwrap_or_else(|| "Normal".into());
                let mut o = json!({"index": i, "type": "paragraph", "style": style, "len": p.len()});
                if with_text {
                    o["text"] = json!(p.plain_text());
                }
                if let Some(n) = p.props.numbering.filter(|n| n.num != 0) {
                    o["list"] = json!({"num": n.num, "level": n.level});
                }
                if !p.props.is_empty() {
                    o["props"] = serde_json::to_value(&p.props).unwrap_or(Value::Null);
                }
                let runs: Vec<Value> = p
                    .run_ranges()
                    .filter(|(_, c)| !c.is_empty())
                    .map(|(r, c)| json!({"start": r.start, "end": r.end, "props": c}))
                    .take(50)
                    .collect();
                if !runs.is_empty() {
                    o["runs"] = json!(runs);
                }
                if !p.objects.is_empty() {
                    o["objects"] = serde_json::to_value(&p.objects).unwrap_or(Value::Null);
                }
                if p.section.is_some() {
                    o["sectionBreak"] = json!(true);
                }
                blocks.push(o);
            }
            Block::Table(t) => {
                let cells: Vec<Vec<String>> = t
                    .rows
                    .iter()
                    .map(|r| {
                        r.cells
                            .iter()
                            .map(|c| c.blocks.iter().filter_map(|b| b.as_para().map(|p| p.plain_text())).collect::<Vec<_>>().join("\n"))
                            .collect()
                    })
                    .collect();
                blocks.push(json!({"index": i, "type": "table", "rows": t.rows.len(), "cols": t.cols(), "style": t.props.style, "cells": cells}));
            }
        }
    }
    let l = s.layout();
    let parts: Vec<Value> =
        s.doc.parts.iter().map(|(id, p)| json!({"id": id, "kind": p.kind, "text": s.doc.plain_text(StoryRef::Part(*id))})).collect();
    Ok(json!({
        "blocks": blocks,
        "parts": parts,
        "selection": {"anchor": pos_json(&s.sel.anchor), "focus": pos_json(&s.sel.focus), "text": s.selected_text()},
        "pages": l.pages.len(),
        "words": s.doc.word_count(),
        "sections": s.doc.sections().iter().map(|(end, sp)| json!({"endBlock": end, "props": sp})).collect::<Vec<_>>(),
        "comments": s.doc.comments.len(),
        "trackChanges": s.doc.settings.track_changes,
        "properties": s.doc.core,
        "canUndo": s.can_undo(),
        "canRedo": s.can_redo(),
    }))
}

fn paragraph(s: &mut Session, v: &Value) -> CmdResult {
    let path: Vec<u32> = serde_json::from_value(v.get("path").cloned().unwrap_or(json!([0]))).map_err(|e| CmdError::Params(e.to_string()))?;
    let story = super::story_param(s, v);
    let p = s.doc.para(story, &wordcraft_doc::Path(path)).ok_or_else(|| CmdError::Params("no paragraph there".into()))?;
    let resolved = s.doc.styles.resolve_para(&p.props);
    Ok(json!({"paragraph": p, "resolved": resolved}))
}

fn layout_summary(s: &mut Session, _: &Value) -> CmdResult {
    let l = s.layout();
    let pages: Vec<Value> = l
        .pages
        .iter()
        .map(|p| {
            let lines: usize = p.items.iter().map(|it| if let wordcraft_layout::Placed::Lines { l0, l1, .. } = it { l1 - l0 } else { 0 }).sum();
            json!({"number": p.number, "w": p.w, "h": p.h, "section": p.section, "lines": lines, "firstBlock": p.first_block, "header": p.header_story, "footer": p.footer_story})
        })
        .collect();
    let caret = l.caret_on(&s.sel.focus, s.page_hint).map(|c| json!({"page": c.page, "x": c.x, "y": c.top, "h": c.height}));
    Ok(json!({"pages": pages, "caret": caret, "ms": l.ms}))
}
