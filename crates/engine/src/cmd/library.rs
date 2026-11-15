//! Insert ▸ Library (#2868): pictures from the user's own folders, browsed in a side pane while
//! the document stays open for editing.
//!
//! The folders are a preference ([`crate::Prefs::library_folders`]); `library.addFolder` and
//! `library.removeFolder` change it, and the front end saves it with the other preferences.
//! Listing, searching and thumbnails only read: no undo step, no macro step. `library.insert` puts
//! a picture in the document as `insert.picture` does, in one undo step. The answers are the
//! Library core's the Craft apps share ([`craft_library`]), with WordCraft's nouns (`picture`,
//! `pictures`, `pictureCount`). Every folder and file goes through [`crate::io`], so the
//! automation read roots confine them, and every bound is in [`crate::library`]. The browser build
//! has no folders to read: there every `library.*` command says so.

use serde_json::{Value, json};

use crate::library::{self, Confined};
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// Every `library.*` command reads folders on this computer, which the browser build can't.
fn native(_: &Session) -> Option<&'static str> {
    cfg!(target_arch = "wasm32").then_some(library::NOT_IN_BROWSER)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("library.folders", "Library Folders", "Insert › Library", folders)
            .params(
                "{} → [{path, files, entries, truncated?, skippedDeep?, skippedLinks?, unreadable?, error?}]: the folders the Library browses and how many pictures each holds (an unreadable folder carries its `error`)",
            )
            .when(native)
            .pure(),
        CommandSpec::new("library.addFolder", "Add Library Folder", "Insert › Library", add_folder)
            .params(
                "{path} → {path, added, folders}: browse a folder of pictures (PNG, JPEG, GIF, WebP, BMP, TIFF, WMF, EMF) in the Library, kept with the preferences; at most 32 folders",
            )
            .when(native)
            .pure(),
        CommandSpec::new("library.removeFolder", "Remove Library Folder", "Insert › Library", remove_folder)
            .params("{path} → {path, folders}: stop browsing a Library folder (its files are not touched)")
            .when(native)
            .pure(),
        CommandSpec::new("library.list", "List Library", "Insert › Library", list)
            .params(
                "{folder?, file?, refresh?: bool} → [{file, folder, title, pictures: [{index, title, hidden}], pictureCount, error?}]: the picture files in the Library folders (or one folder, or one file), one picture each, listed from their names without reading them; a file over 256 MB carries its `error`, and one that isn't a picture says so when library.thumbnail or library.insert reads it; `refresh` looks again at the files that failed (the pane's Refresh)",
            )
            .when(native)
            .pure(),
        CommandSpec::new("library.search", "Search Library", "Insert › Library", search)
            .params(
                "{query: at most 256 characters, limit?: 1..=500 (default 100), folder?, file?} → [{file, picture, title, hidden, match: title|text|file}]: pictures whose file name or format (PNG, JPEG… WMF, EMF) holds `query`, any case; with `file`, only that Library file (the pane searches a few files a frame)",
            )
            .when(native)
            .pure(),
        CommandSpec::new("library.thumbnail", "Library Thumbnail", "Insert › Library", thumbnail)
            .params(
                "{file, picture: 0, width?: px 32..=512 (default 160)} → {png: base64, width, height}: a Library picture drawn small (a metafile drawn as vectors, the full width)",
            )
            .when(native)
            .pure(),
        CommandSpec::new("library.insert", "Insert from Library", "Insert › Library", insert)
            .params(
                "{file, picture?: 0, width?: pt, alt?: string} → {media, width, height, file}: put a Library picture at the selection as Insert › Pictures does (natural size at 96 ppi, a metafile's own frame; fitted to the text column; one undo step)",
            )
            .when(native),
    ]
}

fn failed(e: String) -> CmdError {
    CmdError::Failed(e)
}

fn folders(s: &mut Session, _v: &Value) -> CmdResult {
    Ok(s.library.folders(&Confined, &s.prefs.library_folders))
}

fn add_folder(s: &mut Session, v: &Value) -> CmdResult {
    library::Cache::add_folder(&Confined, &mut s.prefs.library_folders, v).map_err(failed)
}

fn remove_folder(s: &mut Session, v: &Value) -> CmdResult {
    library::Cache::remove_folder(&Confined, &mut s.prefs.library_folders, v).map_err(failed)
}

fn list(s: &mut Session, v: &Value) -> CmdResult {
    s.library.list(&Confined, &s.prefs.library_folders, v).map_err(failed)
}

fn search(s: &mut Session, v: &Value) -> CmdResult {
    s.library.search(&Confined, &s.prefs.library_folders, v).map_err(failed)
}

fn thumbnail(s: &mut Session, v: &Value) -> CmdResult {
    s.library.thumbnail(&Confined, &s.prefs.library_folders, v).map_err(failed)
}

/// `library.insert`: the picture file `file` names, which must be in a Library folder, put at the
/// selection through Insert › Pictures' own code from the bytes the Library read (not read again).
fn insert(s: &mut Session, v: &Value) -> CmdResult {
    let one = library::NOUNS.one;
    // A picture file holds one picture, so the index may be left out.
    let index = match v.get(one) {
        None | Some(Value::Null) => 0,
        Some(_) => s.library.item_param(v).map_err(CmdError::Params)?,
    };
    if index != 0 {
        return Err(CmdError::Params(format!("a picture file holds one picture: `{one}` is 0, not {index}")));
    }
    let (path, stamp, _) = s.library.file(&Confined, &s.prefs.library_folders, v).map_err(failed)?;
    let picture = s.library.doc(&Confined, &path, stamp).map_err(failed)?;
    let alt = p::str(v, "alt").unwrap_or("");
    let mut r = crate::cmd::insert::place_picture(s, picture.bytes.clone(), p::f32(v, "width"), alt)?;
    r["file"] = json!(path.to_string_lossy());
    Ok(r)
}

#[cfg(test)]
mod tests;
