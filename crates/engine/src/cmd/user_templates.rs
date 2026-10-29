//! Personal templates (#2852): the `.dotx`/`.dotm` files in the user templates folder, which
//! File › New lists next to the built-in templates.
//!
//! The folder is the user's own (File › Options › Advanced › File locations, else the app's
//! `Templates` folder, which the front end passes as `dir`). As in Word, the templates at its top
//! level are listed, and each folder directly inside it is a category of the templates it holds;
//! deeper folders are not looked into. A template is named by its file name, as in Word's
//! gallery; its title (the core properties' `dc:title`), when it has one, comes along for search
//! (a template saved from a document often keeps that document's title, so it can't be the name).
//!
//! Everything read here comes from the disk, so it is bounded: the number of entries looked at,
//! of categories and of templates listed, the size of a template whose title is read and of the
//! part it is read from. A file that can't be read is listed by its name or skipped, never an
//! error. A network folder (`\\host\share`) is refused, as attached templates are
//! ([`super::templates`]), and the automation read root applies to the folder and each file.

use serde_json::{Value, json};

use crate::{CmdError, CmdResult, CommandSpec, Session};

/// Most templates one listing returns.
pub const MAX_TEMPLATES: usize = 500;
/// Most category folders looked into.
pub const MAX_CATEGORIES: usize = 64;
/// Most directory entries looked at in one folder.
#[cfg(not(target_arch = "wasm32"))]
const MAX_ENTRIES: usize = 10_000;
/// Longest folder path taken, in bytes.
const MAX_DIR: usize = 32 * 1024;
/// Largest template whose title is read (bigger ones are listed by file name).
#[cfg(not(target_arch = "wasm32"))]
const MAX_TITLE_FILE: u64 = 64 << 20;
/// Most bytes of the core properties part read for the title.
#[cfg(not(target_arch = "wasm32"))]
const MAX_CORE_XML: u64 = 64 << 10;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("file.listTemplates", "Personal Templates", "File › New", list)
            .params(r#"{"dir"?: string (the user templates folder; default: the one saved in File › Options)} → {"dir", "templates": [{"label" (the file name without its extension), "title": string|null (dc:title), "path", "category" ("" at the top level, else the subfolder's name), "modified": ISO-8601|null}] (.dotx and .dotm, at most 500, by category then label), "truncated": bool}; a network folder is refused"#)
            .pure(),
    ]
}

/// One template in the folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersonalTemplate {
    /// Its file name without the extension.
    pub label: String,
    /// The title in its core properties, if any.
    pub title: Option<String>,
    pub path: String,
    /// The subfolder it is in; empty at the top level.
    pub category: String,
    /// When the file last changed (ISO-8601, UTC), if the system says.
    pub modified: Option<String>,
}

/// What [`list_templates`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    pub templates: Vec<PersonalTemplate>,
    /// Some were left out (a cap was reached).
    pub truncated: bool,
}

fn list(s: &mut Session, v: &Value) -> CmdResult {
    let dir = match v.get("dir") {
        None | Some(Value::Null) => {
            s.prefs.user_templates_folder.clone().ok_or_else(|| CmdError::Params("give `dir`: no user templates folder is saved".into()))?
        }
        Some(Value::String(d)) => d.clone(),
        Some(_) => return Err(CmdError::Params("`dir` must be a folder path".into())),
    };
    let found = list_templates(&dir)?;
    Ok(json!({
        "dir": dir,
        "templates": found.templates.iter().map(|t| json!({"label": t.label, "title": t.title, "path": t.path, "category": t.category, "modified": t.modified})).collect::<Vec<_>>(),
        "truncated": found.truncated,
    }))
}

/// Check that `dir` may be listed: a full local path, not too long, not on another machine.
pub fn check_dir(dir: &str) -> Result<(), CmdError> {
    let d = dir.trim();
    if d.is_empty() {
        return Err(CmdError::Params("`dir` is empty".into()));
    }
    if d.len() > MAX_DIR || d.chars().any(char::is_control) {
        return Err(CmdError::Params("`dir` isn't a folder path".into()));
    }
    // A folder on another machine: listing it would make Windows authenticate to that host.
    if super::templates::is_network_path(d) {
        return Err(CmdError::Params("templates aren't listed from a network folder".into()));
    }
    if !std::path::Path::new(d).is_absolute() {
        return Err(CmdError::Params(format!("`dir`: {d:?} isn't a full path")));
    }
    Ok(())
}

/// The personal templates in `dir` (see the module docs).
#[cfg(not(target_arch = "wasm32"))]
pub fn list_templates(dir: &str) -> Result<Listing, CmdError> {
    use std::path::Path;
    check_dir(dir)?;
    let dir = dir.trim();
    crate::file_access::check_read(dir).map_err(CmdError::Failed)?;
    let root = Path::new(dir);
    if !root.is_dir() {
        return Err(CmdError::Failed(format!("no folder at {dir}")));
    }
    let mut out = Listing::default();
    let (files, folders, cut) = entries(root);
    out.truncated |= cut;
    let mut found: Vec<(String, crate::io::DirEntry)> = files.into_iter().map(|f| (String::new(), f)).collect();
    let mut folders = folders;
    folders.sort();
    if folders.len() > MAX_CATEGORIES {
        folders.truncate(MAX_CATEGORIES);
        out.truncated = true;
    }
    for folder in folders {
        let Some(category) = folder.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        if crate::file_access::check_read(&folder.to_string_lossy()).is_err() {
            continue;
        }
        let (files, _, cut) = entries(&folder);
        out.truncated |= cut;
        found.extend(files.into_iter().map(|f| (category.clone(), f)));
    }
    // The cap applies to what is listed; titles are read only for those.
    found.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()).then_with(|| a.1.path.cmp(&b.1.path)));
    if found.len() > MAX_TEMPLATES {
        found.truncate(MAX_TEMPLATES);
        out.truncated = true;
    }
    for (category, e) in found {
        let title = title_of(&e.path);
        let modified = e.modified.map(super::iso_from_unix_secs);
        out.templates.push(PersonalTemplate { label: label_of(&e.name), title, path: e.path.to_string_lossy().into_owned(), category, modified });
    }
    // Two templates of one category that would show the same name (`Memo.dotx`, `Memo.dotm`)
    // show their file names instead, so they can be told apart.
    let key = |t: &PersonalTemplate| (t.category.to_lowercase(), t.label.to_lowercase());
    let mut seen = std::collections::HashMap::<(String, String), usize>::new();
    for t in &out.templates {
        *seen.entry(key(t)).or_default() += 1;
    }
    for t in &mut out.templates {
        if seen.get(&key(t)).is_some_and(|n| *n > 1)
            && let Some(name) = std::path::Path::new(&t.path).file_name()
        {
            t.label = name.to_string_lossy().into_owned();
        }
    }
    // The top level first, then each category; by label within them.
    out.templates.sort_by_cached_key(|t| (!t.category.is_empty(), t.category.to_lowercase(), t.label.to_lowercase(), t.path.clone()));
    Ok(out)
}

#[cfg(target_arch = "wasm32")]
pub fn list_templates(dir: &str) -> Result<Listing, CmdError> {
    check_dir(dir)?;
    Err(CmdError::Failed("templates folders can't be listed on the web".into()))
}

/// The template files and the folders directly in `dir` (hidden ones left out), and whether the
/// entry cap cut the listing short. An unreadable folder has none.
#[cfg(not(target_arch = "wasm32"))]
fn entries(dir: &std::path::Path) -> (Vec<crate::io::DirEntry>, Vec<std::path::PathBuf>, bool) {
    let Ok((list, cut)) = crate::io::list_dir(dir, MAX_ENTRIES) else { return (Vec::new(), Vec::new(), false) };
    let (mut files, mut folders) = (Vec::new(), Vec::new());
    for e in list {
        if e.name.starts_with('.') || e.name.starts_with("~$") {
            continue;
        }
        if e.dir {
            folders.push(e.path);
        } else if is_template_name(&e.name) {
            files.push(e);
        }
    }
    (files, folders, cut)
}

/// The name a template shows: its file name without the template extension, every one of them
/// (`Contract.dotm.dotx`, as older Save as Template made, is `Contract`); the whole name if
/// nothing would be left.
#[cfg(not(target_arch = "wasm32"))]
fn label_of(name: &str) -> String {
    let mut label = name;
    while is_template_name(label) {
        let Some(stem) = label.get(..label.len().saturating_sub(5)) else { break };
        label = stem;
    }
    let label = label.trim_end();
    if label.is_empty() { name.to_string() } else { label.to_string() }
}

/// A Word template by its name: `.dotx` or `.dotm`, in any case.
pub fn is_template_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    (lower.ends_with(".dotx") || lower.ends_with(".dotm")) && lower.len() > 5
}

/// The template's title from its core properties, read from the package's directory and at most
/// [`MAX_CORE_XML`] bytes of the part; `None` for anything else.
#[cfg(not(target_arch = "wasm32"))]
fn title_of(path: &std::path::Path) -> Option<String> {
    let title = crate::io::package_title(path, MAX_TITLE_FILE, MAX_CORE_XML)?;
    // One line of a sensible length for a gallery caption.
    let title: String = title.chars().map(|c| if c.is_control() { ' ' } else { c }).take(200).collect();
    let title = title.trim();
    (!title.is_empty()).then(|| title.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("wc-personal-templates-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A real template written by the engine, with `title` as its core title (empty: none).
    fn template(path: &std::path::Path, text: &str, title: &str) {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("document.setText", &json!({"text": text})).unwrap();
        s.doc.core.title = title.into();
        crate::io::save_path(path, &s.doc).unwrap();
    }

    fn labels(r: &Value) -> Vec<(String, String)> {
        r["templates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| (t["category"].as_str().unwrap().to_string(), t["label"].as_str().unwrap().to_string()))
            .collect()
    }

    #[test]
    fn lists_dotx_and_dotm_with_subfolders_as_categories() {
        let d = scratch("list");
        template(&d.join("Weekly report.dotx"), "Report body", "");
        template(&d.join("letterhead.dotx"), "Letter body", "Company letterhead");
        template(&d.join("Macros.dotm"), "Macro body", "");
        template(&d.join("Not a template.docx"), "Doc body", "");
        std::fs::write(d.join("notes.txt"), "x").unwrap();
        std::fs::write(d.join("Broken.dotx"), b"this is no zip").unwrap();
        std::fs::write(d.join(".hidden.dotx"), b"x").unwrap();
        std::fs::create_dir_all(d.join("Legal/Deeper")).unwrap();
        template(&d.join("Legal/NDA.DOTX"), "NDA body", "");
        template(&d.join("Legal/Deeper/Too deep.dotx"), "x", "");
        std::fs::create_dir_all(d.join("Empty")).unwrap();

        let mut s = Session::new(wordcraft_doc::Document::new());
        let r = s.run("file.listTemplates", &json!({"dir": d.to_string_lossy()})).unwrap();
        assert_eq!(
            labels(&r),
            [
                ("".into(), "Broken".into()),
                ("".into(), "letterhead".into()),
                ("".into(), "Macros".into()),
                ("".into(), "Weekly report".into()),
                ("Legal".into(), "NDA".into()),
            ],
            "{r}"
        );
        assert_eq!(r["truncated"], json!(false));
        let first = &r["templates"][1];
        assert_eq!(first["title"], json!("Company letterhead"));
        assert_eq!(r["templates"][0]["title"], json!(null), "a broken file has no title");
        assert!(first["path"].as_str().unwrap().ends_with("letterhead.dotx"));
        assert!(first["modified"].as_str().is_some_and(|m| m.ends_with('Z')));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn refuses_network_relative_and_missing_folders() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        for dir in [r"\\host\share\Templates", "//host/share/Templates", "relative/Templates", "", "   "] {
            assert!(matches!(s.run("file.listTemplates", &json!({"dir": dir})), Err(CmdError::Params(_))), "{dir:?}");
        }
        let missing = std::env::temp_dir().join(format!("wc-no-such-templates-{}", std::process::id()));
        assert!(s.run("file.listTemplates", &json!({"dir": missing.to_string_lossy()})).is_err());
        // Without `dir` the saved folder is used; none saved is an error.
        assert!(matches!(s.run("file.listTemplates", &json!({})), Err(CmdError::Params(_))));
        for junk in [json!(null), json!({"dir": 5}), json!({"dir": []}), json!({"dir": "x".repeat(40_000)}), json!({"dir": "/a\u{0}b"})] {
            let _ = s.run("file.listTemplates", &junk);
        }
    }

    #[test]
    fn the_saved_folder_is_listed_without_dir() {
        let d = scratch("saved");
        template(&d.join("Saved.dotx"), "x", "");
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("file.options", &json!({"userTemplatesFolder": d.to_string_lossy()})).unwrap();
        let r = s.run("file.listTemplates", &json!({})).unwrap();
        assert_eq!(labels(&r), [("".to_string(), "Saved".to_string())]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_listing_is_capped() {
        let d = scratch("cap");
        for i in 0..MAX_TEMPLATES + 5 {
            std::fs::write(d.join(format!("t{i:04}.dotx")), b"").unwrap();
        }
        let found = list_templates(&d.to_string_lossy()).unwrap();
        assert_eq!(found.templates.len(), MAX_TEMPLATES);
        assert!(found.truncated);
        assert_eq!(found.templates.first().map(|t| t.label.as_str()), Some("t0000"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A document made from a personal template is untitled, unsaved and attached to the
    /// template, as in Word.
    #[test]
    fn new_from_template_is_untitled_and_attached() {
        let d = scratch("new");
        let path = d.join("Memo.dotx");
        template(&path, "Memo text", "");
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.path = Some(d.join("Earlier.docx"));
        s.run("file.newFromTemplate", &json!({"path": path.to_string_lossy()})).unwrap();
        assert_eq!(s.path, None);
        assert!(s.dirty);
        assert_eq!(s.doc.settings.attached_template.as_deref(), Some(path.to_string_lossy().as_ref()));
        assert!(s.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Memo text"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_templates_folder_option_is_saved_and_checked() {
        let d = scratch("option");
        let mut s = Session::new(wordcraft_doc::Document::new());
        for bad in [json!(r"\\host\share"), json!("//host/share"), json!("relative/x"), json!(7)] {
            assert!(s.run("file.options", &json!({"userTemplatesFolder": bad})).is_err(), "{bad}");
        }
        assert!(s.run("file.options", &json!({"userTemplatesFolder": d.join("missing").to_string_lossy()})).is_err());
        assert_eq!(s.prefs.user_templates_folder, None, "a refused folder changes nothing");
        let r = s.run("file.options", &json!({"userTemplatesFolder": d.to_string_lossy()})).unwrap();
        assert_eq!(r["userTemplatesFolder"], json!(d.to_string_lossy()));
        assert_eq!(serde_json::to_value(&s.prefs).unwrap()["userTemplatesFolder"], json!(d.to_string_lossy()));
        s.run("file.options", &json!({"userTemplatesFolder": "auto"})).unwrap();
        assert_eq!(s.prefs.user_templates_folder, None);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// .dotx and .dotm templates show their names the same way, without the extension; two that
    /// would look alike in one category show their file names.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn labels_drop_the_extension_alike_and_tell_twins_apart() {
        let d = scratch("labels");
        template(&d.join("Service contract.dotm"), "a", "");
        template(&d.join("Old contract.dotm.dotx"), "b", "");
        template(&d.join("Memo.dotx"), "c", "");
        template(&d.join("Memo.DOTM"), "d", "");
        std::fs::create_dir_all(d.join("Other")).unwrap();
        template(&d.join("Other/Memo.dotx"), "e", "");
        let found = list_templates(&d.to_string_lossy()).unwrap();
        let got: Vec<(&str, &str)> = found.templates.iter().map(|t| (t.category.as_str(), t.label.as_str())).collect();
        assert_eq!(got, [("", "Memo.DOTM"), ("", "Memo.dotx"), ("", "Old contract"), ("", "Service contract"), ("Other", "Memo")]);
        assert_eq!(label_of(".dotx.dotm"), ".dotx", "never empty");
        assert_eq!(label_of(" .dotx"), " .dotx", "nothing left: the whole name");
        assert_eq!(label_of("é.dotx"), "é");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Save as Template keeps a .dotm (any case) and makes anything else a .dotx.
    #[test]
    fn save_template_keeps_a_template_extension() {
        let d = scratch("save");
        let mut s = Session::new(wordcraft_doc::Document::new());
        for (asked, saved) in [("A.dotm", "A.dotm"), ("B.DOTX", "B.DOTX"), ("C", "C.dotx"), ("D.docx", "D.docx.dotx")] {
            let r = s.run("file.saveTemplate", &json!({"path": d.join(asked).to_string_lossy()})).unwrap();
            assert_eq!(r["path"], json!(d.join(saved).to_string_lossy()));
            assert!(d.join(saved).is_file(), "{saved}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn template_names() {
        assert!(is_template_name("a.dotx") && is_template_name("B.DOTM"));
        assert!(!is_template_name(".dotx") && !is_template_name("a.docx") && !is_template_name("a.dot"));
    }
}
