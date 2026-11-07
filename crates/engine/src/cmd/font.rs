//! Fonts the user adds without rebuilding (#582): `font.add`, `font.list`, `font.remove`; and
//! the fonts the document's text asks for that aren't installed, with the fonts drawn instead
//! (`font.missing`).
//!
//! An added font joins the process-wide font database on top of the bundled and system fonts
//! (never replacing a face already there), so it shows in the font lists, lays out, renders and
//! embeds in PDFs like any other. Where it is kept between runs is [`FontDb::load_user_fonts`]'s
//! folder (desktop); without one it lasts for the session (the web shell stores it itself).

use serde_json::{Value, json};
use wordcraft_doc::Document;
use wordcraft_fonts::{FontDb, UserFont};
use wordcraft_layout::DocLayout;

use crate::cmd::insert::base64_decode;
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("font.add", "Add Font…", "Home › Font", add)
            .params(r#"{"path"?: string, "data"?: base64, "name"?: string (file name for "data")} — a TTF, OTF, TTC, WOFF or WOFF2 file; none: pick a file"#)
            .pure(),
        CommandSpec::new("font.list", "Added Fonts", "Home › Font", list).params(r#"{"all"?: bool (also every available family)}"#).pure(),
        CommandSpec::new("font.remove", "Remove Added Font", "Home › Font", remove).params(r#"{"family": string}"#).pure(),
        CommandSpec::new("font.loadSystem", "Load System Installed Fonts…", "Home › Font", load_system).pure(),
        CommandSpec::new("font.missing", "Missing Fonts", "Status Bar", missing)
            .params(r#"{} → {"missing": [{"font": string, "shownAs": [string] (the families drawn instead), "metricCompatible": bool (its substitute measures like the font)}]} — the text the current view shows"#)
            .pure(),
    ]
}

fn load_system(s: &mut Session, _v: &Value) -> CmdResult {
    s.push_ui_request(json!({"open": "loadSystemFonts"}));
    Ok(json!({"pending": true}))
}

fn font_json(f: &UserFont) -> Value {
    json!({
        "file": f.file,
        "families": f.families(),
        "faces": f.faces.iter().map(|(family, style)| json!({"family": family, "style": style})).collect::<Vec<_>>(),
        "path": f.path.as_ref().map(|p| p.to_string_lossy().to_string()),
    })
}

/// Read a font file named by the user, refusing one larger than the cap before reading it.
#[cfg(not(target_arch = "wasm32"))]
fn read_font_file(path: &str) -> Result<Vec<u8>, CmdError> {
    // Through `io`, which keeps automation inside its read root (#769).
    crate::io::read_file_capped(path, wordcraft_fonts::MAX_FONT_FILE_BYTES as u64, "font file").map_err(CmdError::Failed)
}

/// Adding or removing a font writes to the user font folder: automation confined to a write root
/// (#769) may do that only when the folder is inside it.
fn check_font_store(dir: Option<&std::path::Path>) -> Result<(), CmdError> {
    match dir {
        Some(dir) => crate::file_access::check_write(&dir.to_string_lossy()).map_err(CmdError::Failed),
        None => Ok(()),
    }
}

fn add(s: &mut Session, v: &Value) -> CmdResult {
    if p::str(v, "url").is_some() {
        return Err(CmdError::Failed(
            "adding a font from a link isn't available yet: download the font file and add it with {\"path\"} or {\"data\"}".into(),
        ));
    }
    let (name, bytes) = if let Some(d) = p::str(v, "data") {
        // Base64 is a third larger than the file: refuse before decoding an oversized one.
        if d.len() / 4 * 3 > wordcraft_fonts::MAX_FONT_FILE_BYTES + 3 {
            return Err(CmdError::Failed(format!("the font file is larger than {} MB", wordcraft_fonts::MAX_FONT_FILE_BYTES >> 20)));
        }
        let bytes = base64_decode(d).ok_or_else(|| CmdError::Params("bad base64 data".into()))?;
        (p::str(v, "name").unwrap_or("font").to_string(), bytes)
    } else if let Some(path) = p::str(v, "path") {
        #[cfg(not(target_arch = "wasm32"))]
        {
            (path.to_string(), read_font_file(path)?)
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = path;
            return Err(CmdError::Failed("reading files by path isn't available on the web".into()));
        }
    } else {
        // From the ribbon: the UI shows a file picker and runs this again with the file.
        s.push_ui_request(json!({"open": "addFont"}));
        return Ok(json!({"pending": true}));
    };
    check_font_store(FontDb::global().user_font_dir().as_deref())?;
    let r = FontDb::global().install_font(&name, &bytes).map_err(|e| CmdError::Failed(e.to_string()))?;
    // Text already set in the family (shown in a fallback until now) takes the new font.
    s.relayout();
    let families = r.font.families().join(", ");
    s.status = match &r.warning {
        Some(w) => format!("Added {families} ({w})"),
        None => format!("Added {families}"),
    };
    let mut out = font_json(&r.font);
    if let Some(w) = r.warning {
        out["warning"] = json!(w);
    }
    Ok(out)
}

fn list(_: &mut Session, v: &Value) -> CmdResult {
    let db = FontDb::global();
    let mut out = json!({
        "fonts": db.user_fonts().iter().map(font_json).collect::<Vec<_>>(),
        "folder": db.user_font_dir().map(|p| p.to_string_lossy().to_string()),
    });
    if p::bool(v, "all") == Some(true) {
        out["families"] = json!(db.families());
    }
    Ok(out)
}

fn remove(s: &mut Session, v: &Value) -> CmdResult {
    let family = p::str(v, "family").filter(|f| !f.trim().is_empty()).ok_or_else(|| CmdError::Params("family is required".into()))?;
    check_font_store(FontDb::global().user_font_dir().as_deref())?;
    let gone = FontDb::global().remove_user_font(family.trim()).map_err(|e| CmdError::Failed(e.to_string()))?;
    s.relayout();
    s.status = format!("Removed {}", family.trim());
    Ok(json!({"removed": gone.iter().map(font_json).collect::<Vec<_>>()}))
}

/// A font the document's text asks for that isn't installed.
#[derive(Clone, Debug, PartialEq)]
pub struct MissingFont {
    /// The name the document asks for.
    pub font: String,
    /// The families its text is drawn in instead: the substitute, and fallback faces for
    /// characters the substitute lacks.
    pub shown_as: Vec<String>,
    /// The font's substitute measures like it (Carlito for Calibri…): line breaks don't move.
    /// Fallback faces for characters the substitute lacks don't count: the font itself might
    /// lack them too (an emoji in Calibri text).
    pub metric_compatible: bool,
}

/// The family text set in `font` is drawn in when `font` isn't installed (None when it is), as
/// layout picks it for `doc`.
pub fn shown_as(doc: &Document, font: &str) -> Option<String> {
    wordcraft_layout::para::substitute_face(doc, font).map(|f| f.get().family.clone())
}

/// The family Chinese, Japanese and Korean text is drawn in when its East Asian font `font_ea`
/// isn't installed (None when it is), the run's font being `font`, as layout picks it for `doc`.
pub fn east_asian_shown_as(doc: &Document, font_ea: &str, font: &str) -> Option<String> {
    wordcraft_layout::para::east_asian_substitute_face(doc, font_ea, font).map(|f| f.get().family.clone())
}

/// Whether text in `font` drawn in `substitute` is worth a warning: the substitute measures
/// differently, so line breaks can move, and the font isn't a symbol font (drawn as its
/// Unicode equivalents on purpose, which layout leaves out of [`missing_fonts`] too). Decides
/// [`MissingFont::metric_compatible`], which the font box and the status bar both read.
pub fn warns(font: &str, substitute: &str) -> bool {
    !wordcraft_doc::symbol_fonts::is_symbol_font(font) && !wordcraft_fonts::word::metric_compatible(font, substitute)
}

/// The fonts `doc`'s laid-out text asks for that aren't installed, from what layout drew
/// ([`DocLayout::substituted_fonts`]): only text the current view shows counts (no headers or
/// footers in Web, Draft or Read view), in every story on its pages, list labels and field
/// results included. Names differing only in case are one font.
pub fn missing_fonts(doc: &Document, layout: &DocLayout) -> Vec<MissingFont> {
    // By key: in name order, case variants merged.
    let mut out: std::collections::BTreeMap<String, MissingFont> = std::collections::BTreeMap::new();
    for (recorded, face) in layout.substituted_fonts() {
        let family = face.get().family.clone();
        let font = shown_name(&recorded);
        let m = out.entry(font_key(font)).or_insert_with(|| MissingFont {
            font: font.to_string(),
            shown_as: Vec::new(),
            // Judged by the name layout looked up, spaces and all: "Arial " isn't Arial to it, so
            // its text is in a substitute even where Arial is installed.
            metric_compatible: shown_as(doc, &recorded).is_some_and(|s| !warns(font, &s)),
        });
        if !m.shown_as.contains(&family) {
            m.shown_as.push(family);
        }
    }
    out.into_values().collect()
}

/// A font name as [`missing_fonts`] lists it: as layout records it (at most 256 characters),
/// trimmed.
fn shown_name(font: &str) -> &str {
    wordcraft_layout::para::reported_font_name(font).trim()
}

/// The key a font name is listed under in [`missing_fonts`]: two names with the same key are
/// one font. Look a document's font up in the list through this.
pub fn font_key(font: &str) -> String {
    shown_name(font).to_lowercase()
}

/// The status line's form of a font name: a hostile file's names are long.
fn short_name(font: &str) -> String {
    match font.char_indices().nth(40) {
        Some((i, _)) => format!("{}…", font.get(..i).unwrap_or(font)),
        None => font.to_string(),
    }
}

fn missing(s: &mut Session, _: &Value) -> CmdResult {
    let layout = s.layout();
    let found = missing_fonts(&s.doc, &layout);
    // From command search there's no dialog: the status line says what was found.
    // A hostile file can name thousands of fonts: name the first few.
    const NAMED: usize = 5;
    let changing: Vec<&str> = found.iter().filter(|f| !f.metric_compatible).map(|f| f.font.as_str()).collect();
    let named = changing.iter().take(NAMED).map(|f| short_name(f)).collect::<Vec<_>>().join(", ");
    s.status = match (found.is_empty(), changing.len()) {
        (true, _) => "Every font the document shows is installed".into(),
        (false, 0) => "Fonts that aren't installed are shown in fonts that measure the same: line breaks don't move".into(),
        (_, n) if n > NAMED => format!("Not installed: {named} and {} more", n - NAMED),
        _ => format!("Not installed: {named}"),
    };
    let fonts: Vec<Value> = found.iter().map(|f| json!({"font": f.font, "shownAs": f.shown_as, "metricCompatible": f.metric_compatible})).collect();
    Ok(json!({"missing": fonts}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confined_automation_changes_only_a_font_store_inside_its_write_root() {
        let root = std::env::temp_dir().join(format!("wordcraft-font-store-{}", std::process::id()));
        let outside = std::env::temp_dir().join(format!("wordcraft-font-store-out-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let write = crate::file_access::AutomationRoots::new(None, Some(&root)).unwrap();
        let read_only = crate::file_access::AutomationRoots::new(Some(&root), None).unwrap();
        crate::file_access::confine(write.as_ref(), || {
            assert!(check_font_store(None).is_ok(), "no folder: the font lasts for the session");
            assert!(check_font_store(Some(&root.join("fonts"))).is_ok());
            assert!(check_font_store(Some(&outside)).is_err());
        });
        crate::file_access::confine(read_only.as_ref(), || assert!(check_font_store(Some(&root)).is_err()));
        assert!(check_font_store(Some(&outside)).is_ok(), "unconfined: the person at the keyboard");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }
}
