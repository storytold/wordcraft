//! Custom theme colours, fonts and themes (Design › Colors › Customize Colors…, Fonts ›
//! Customize Fonts…, Themes › Save Current Theme… and Browse for Themes…, #1869).
//!
//! A custom colour set is 12 named theme colours (dark 1, light 1, dark 2, light 2, accents 1–6,
//! hyperlink, followed hyperlink); a custom font set is a heading and a body font; a custom theme
//! is an Office Theme file (`.thmx`, read and written by `wordcraft_docx`). They are kept in the
//! session ([`CustomThemes`]) and, when the host wires up a themes folder
//! ([`crate::Session::load_custom_themes`]), saved there: the colour and font sets in
//! [`CUSTOM_FILE`], each theme as `<name>.thmx`. Headless sessions, tests and the web build have
//! no folder, so their sets live in memory only and never touch the person's files.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use wordcraft_doc::props::Rgb;
use wordcraft_docx::Theme;

use super::design::{THEMES, set_accent, set_fonts};
use super::sel_result;
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// The colour and font sets' file in the themes folder.
pub const CUSTOM_FILE: &str = "custom.json";
/// Extension of a theme file.
pub const THEME_EXT: &str = "thmx";
/// Most custom sets of each kind kept (and theme files read from the folder).
pub const MAX_CUSTOM: usize = 200;
/// Longest custom set name, and longest font name.
pub const MAX_NAME: usize = 128;
/// Largest [`CUSTOM_FILE`] read.
const MAX_CUSTOM_FILE: u64 = 1024 * 1024;
/// Largest theme file read from the themes folder (a `.thmx` is a few kilobytes).
const MAX_THMX: u64 = 4 * 1024 * 1024;
/// Longest path taken from params.
const MAX_PATH: usize = 32 * 1024;

/// A named set of the 12 theme colours.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomColors {
    pub name: String,
    pub colors: [Rgb; 12],
}

/// A named heading and body font pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomFonts {
    pub name: String,
    pub heading: String,
    pub body: String,
}

/// The person's custom colour sets, font sets and themes (each theme's `name` is its gallery
/// name, the file's stem).
#[derive(Clone, Debug, Default)]
pub struct CustomThemes {
    pub colors: Vec<CustomColors>,
    pub fonts: Vec<CustomFonts>,
    pub themes: Vec<Theme>,
}

impl CustomThemes {
    pub fn find_colors(&self, name: &str) -> Option<&CustomColors> {
        self.colors.iter().find(|c| c.name.eq_ignore_ascii_case(name))
    }
    pub fn find_fonts(&self, name: &str) -> Option<&CustomFonts> {
        self.fonts.iter().find(|c| c.name.eq_ignore_ascii_case(name))
    }
    pub fn find_theme(&self, name: &str) -> Option<&Theme> {
        self.themes.iter().find(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// What `design.customThemes` returns.
    pub fn to_json(&self) -> Value {
        json!({
            "colors": self.colors.iter().map(|c| json!({"name": c.name, "colors": c.colors.iter().map(|x| x.hex()).collect::<Vec<_>>()})).collect::<Vec<_>>(),
            "fonts": self.fonts.iter().map(|f| json!({"name": f.name, "heading": f.heading, "body": f.body})).collect::<Vec<_>>(),
            "themes": self.themes.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        })
    }

    /// The colour and font sets as [`CUSTOM_FILE`] keeps them.
    fn file_json(&self) -> Value {
        let mut v = self.to_json();
        if let Some(o) = v.as_object_mut() {
            o.remove("themes");
        }
        v
    }

    /// Read a themes folder: [`CUSTOM_FILE`] and the `.thmx` files in it. Anything unreadable is
    /// skipped (logged), never fatal.
    pub fn load(dir: &Path) -> CustomThemes {
        let mut c = CustomThemes::default();
        let file = dir.join(CUSTOM_FILE);
        if let Some(bytes) = read_capped(&file, MAX_CUSTOM_FILE) {
            match serde_json::from_slice::<Value>(&bytes) {
                Ok(v) => c.read_file_json(&v),
                Err(e) => log::warn!("custom themes: ignoring unreadable {}: {e}", file.display()),
            }
        }
        let Ok(files) = crate::io::list_files(dir, MAX_CUSTOM * 4) else { return c };
        let mut paths: Vec<PathBuf> = files.into_iter().filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(THEME_EXT))).collect();
        paths.sort();
        for path in paths {
            if c.themes.len() >= MAX_CUSTOM {
                break;
            }
            let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().to_string()) else { continue };
            if check_name(&stem).is_err() || is_builtin(&stem) || c.find_theme(&stem).is_some() {
                continue;
            }
            let Some(bytes) = read_capped(&path, MAX_THMX) else { continue };
            match wordcraft_docx::read_theme(&bytes) {
                Ok(mut t) => {
                    t.name = stem;
                    c.themes.push(t);
                }
                Err(e) => log::warn!("custom themes: ignoring {}: {e}", path.display()),
            }
        }
        c
    }

    fn read_file_json(&mut self, v: &Value) {
        let s = |x: &Value, k: &str| x.get(k).and_then(Value::as_str).map(str::to_string);
        for x in v.get("colors").and_then(Value::as_array).into_iter().flatten().take(MAX_CUSTOM) {
            let (Some(name), Some(list)) = (s(x, "name"), x.get("colors")) else { continue };
            if let (Ok(name), Ok(colors)) = (check_name(&name), parse_colors(list))
                && self.find_colors(&name).is_none()
            {
                self.colors.push(CustomColors { name, colors });
            }
        }
        for x in v.get("fonts").and_then(Value::as_array).into_iter().flatten().take(MAX_CUSTOM) {
            let (Some(name), Some(heading), Some(body)) = (s(x, "name"), s(x, "heading"), s(x, "body")) else { continue };
            if let (Ok(name), Ok(heading), Ok(body)) = (check_name(&name), check_font(&heading), check_font(&body))
                && self.find_fonts(&name).is_none()
            {
                self.fonts.push(CustomFonts { name, heading, body });
            }
        }
    }
}

fn read_capped(path: &Path, cap: u64) -> Option<Vec<u8>> {
    crate::io::read_file_capped(&path.to_string_lossy(), cap, "theme file").ok()
}

/// A custom set's name: trimmed, not empty, at most [`MAX_NAME`] characters, and usable as a
/// file name on every system (a theme is saved as `<name>.thmx`): no path separators, reserved
/// characters, control characters, leading dot or trailing dot, at most 250 bytes (room for the
/// extension in a 255-byte file name), and not a Windows device name (`CON`, `NUL`, `COM1`…, with
/// or without an extension).
pub fn check_name(name: &str) -> Result<String, CmdError> {
    let n = name.trim();
    if n.is_empty() {
        return Err(CmdError::Params("the name is empty".into()));
    }
    if n.chars().count() > MAX_NAME || n.len() > 250 {
        return Err(CmdError::Params(format!("the name is longer than {MAX_NAME} characters (or 250 bytes)")));
    }
    let bad_char = |c: char| c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|');
    if n.starts_with('.') || n.ends_with('.') || n.chars().any(bad_char) || is_device_name(n) {
        return Err(CmdError::Params(format!("`{n}` can't be used as a name")));
    }
    Ok(n.to_string())
}

/// Windows' reserved device names, which can't be file names whatever the extension.
fn is_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    let upper = stem.to_ascii_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let (Some(head), Some(digit)) = (upper.get(..3), upper.get(3..)) else { return false };
    matches!(head, "COM" | "LPT") && digit.len() == 1 && digit.bytes().all(|b| b.is_ascii_digit())
}

fn check_font(name: &str) -> Result<String, CmdError> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > MAX_NAME || n.chars().any(char::is_control) {
        return Err(CmdError::Params(format!("`{}` is not a font name", n.chars().take(40).collect::<String>())));
    }
    Ok(n.to_string())
}

/// Exactly 12 `RRGGBB`/`#RRGGBB` colours.
fn parse_colors(v: &Value) -> Result<[Rgb; 12], CmdError> {
    let list = v.as_array().ok_or_else(|| CmdError::Params("`colors` must be a list of 12 colours".into()))?;
    if list.len() != 12 {
        return Err(CmdError::Params(format!("`colors` needs 12 colours, not {}", list.len())));
    }
    let mut out = wordcraft_doc::THEME_COLORS;
    for (slot, c) in out.iter_mut().zip(list) {
        *slot = c.as_str().and_then(Rgb::parse).ok_or_else(|| CmdError::Params(format!("`{c}` is not a colour (RRGGBB)")))?;
    }
    Ok(out)
}

pub fn is_builtin(name: &str) -> bool {
    THEMES.iter().any(|t| t.0.eq_ignore_ascii_case(name))
}

/// The file of the saved theme `name` in `dir`: an existing `.thmx` whose name matches whatever
/// its case (names are matched case-insensitively, file systems may not be), else `<name>.thmx`.
fn theme_file(dir: &Path, name: &str) -> PathBuf {
    let files = crate::io::list_files(dir, MAX_CUSTOM * 4).unwrap_or_default();
    let found = files.into_iter().find(|p| {
        p.extension().is_some_and(|e| e.eq_ignore_ascii_case(THEME_EXT))
            && p.file_stem().is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(name))
    });
    found.unwrap_or_else(|| dir.join(format!("{name}.{THEME_EXT}")))
}

pub fn specs() -> Vec<CommandSpec> {
    const LOC: &str = "Design › Document Formatting";
    vec![
        CommandSpec::new("design.customColors", "Customize Colors", LOC, custom_colors)
            .params(r#"{"name": string, "colors": [12 × "RRGGBB": dark 1, light 1, dark 2, light 2, accent 1–6, hyperlink, followed hyperlink], "replace"?: bool (update the set of that name), "renameFrom"?: string (a set to drop as this one is saved)} (from the ribbon, the app shows the dialog)"#),
        CommandSpec::new("design.customFonts", "Customize Fonts", LOC, custom_fonts)
            .params(r#"{"name": string, "heading": string, "body"?: string (default: heading), "replace"?: bool, "renameFrom"?: string} (from the ribbon, the app shows the dialog)"#),
        CommandSpec::new("design.deleteCustom", "Delete", LOC, delete_custom)
            .params(r#"{"kind": "colors|fonts|theme", "name": string}"#)
            .pure(),
        CommandSpec::new("design.customThemes", "Custom", LOC, |s, _| Ok(s.custom_themes.to_json()))
            .params(r#"{} → {"colors": [{"name", "colors": [12 × "RRGGBB"]}], "fonts": [{"name", "heading", "body"}], "themes": [name]}"#)
            .pure(),
        CommandSpec::new("design.saveTheme", "Save Current Theme", LOC, save_theme)
            .params(r#"{"path"?: string (a .thmx file), "name"?: string (no path: save to the themes folder under this name; neither: the app asks where)} → {"path", "name"}"#)
            .pure(),
        CommandSpec::new("design.openTheme", "Browse for Themes", LOC, open_theme).params(
            r#"{"path": string (a .thmx, or a .docx/.dotx whose theme to use), "data"?: base64 (the file's bytes; `path` then only names it)} (no path: the app asks for the file)"#,
        ),
    ]
}

/// Make these the document's 12 theme colours (headings take accent 1).
pub(crate) fn apply_colors(s: &mut Session, colors: &[Rgb; 12]) {
    s.doc.settings.theme_colors = colors.to_vec();
    set_accent(s, colors[4]);
}

/// Make this the document's theme: colours, fonts and name.
pub(crate) fn apply_theme(s: &mut Session, t: &Theme) {
    set_fonts(s, &t.major_font, &t.minor_font);
    apply_colors(s, &t.colors);
    s.doc.settings.theme_name = t.name.clone();
}

/// Save the colour and font sets to the themes folder, if there is one. A failed save keeps
/// them in memory and says so in the status bar; it never fails the command.
fn persist(s: &mut Session) {
    let Some(dir) = s.themes_dir.clone() else { return };
    let body = serde_json::to_vec_pretty(&s.custom_themes.file_json()).unwrap_or_default();
    let r = crate::io::create_dir(&dir).and_then(|_| crate::io::write_file(&dir.join(CUSTOM_FILE).to_string_lossy(), &body));
    if let Err(e) = r {
        log::warn!("custom themes: save to {} failed: {e}", dir.display());
        s.status = "Custom theme settings could not be saved.".into();
    }
}

fn custom_colors(s: &mut Session, v: &Value) -> CmdResult {
    let name = check_name(p::req_str(v, "name")?)?;
    let colors = parse_colors(v.get("colors").unwrap_or(&Value::Null))?;
    if is_builtin(&name) {
        return Err(CmdError::Params(format!("`{name}` is a built-in colour set")));
    }
    let replace = p::bool(v, "replace").unwrap_or(false);
    let list = &mut s.custom_themes.colors;
    drop_renamed(list, |c| &c.name, v, &name, replace, "colour set")?;
    let at = list.iter().position(|c| c.name.eq_ignore_ascii_case(&name));
    match at {
        Some(_) if !replace => return Err(CmdError::Params(format!("a colour set named `{name}` exists (pass `replace` to update it)"))),
        Some(i) => {
            if let Some(c) = list.get_mut(i) {
                *c = CustomColors { name: name.clone(), colors };
            }
        }
        None if list.len() >= MAX_CUSTOM => return Err(CmdError::Failed(format!("at most {MAX_CUSTOM} custom colour sets"))),
        None => list.push(CustomColors { name: name.clone(), colors }),
    }
    persist(s);
    apply_colors(s, &colors);
    sel_result(s)
}

fn custom_fonts(s: &mut Session, v: &Value) -> CmdResult {
    let name = check_name(p::req_str(v, "name")?)?;
    let heading = check_font(p::req_str(v, "heading")?)?;
    let body = check_font(p::str(v, "body").unwrap_or(&heading))?;
    if is_builtin(&name) {
        return Err(CmdError::Params(format!("`{name}` is a built-in font set")));
    }
    let replace = p::bool(v, "replace").unwrap_or(false);
    let set = CustomFonts { name: name.clone(), heading: heading.clone(), body: body.clone() };
    let list = &mut s.custom_themes.fonts;
    drop_renamed(list, |c| &c.name, v, &name, replace, "font set")?;
    let at = list.iter().position(|c| c.name.eq_ignore_ascii_case(&name));
    match at {
        Some(_) if !replace => return Err(CmdError::Params(format!("a font set named `{name}` exists (pass `replace` to update it)"))),
        Some(i) => {
            if let Some(c) = list.get_mut(i) {
                *c = set;
            }
        }
        None if list.len() >= MAX_CUSTOM => return Err(CmdError::Failed(format!("at most {MAX_CUSTOM} custom font sets"))),
        None => list.push(set),
    }
    persist(s);
    set_fonts(s, &heading, &body);
    sel_result(s)
}

fn delete_custom(s: &mut Session, v: &Value) -> CmdResult {
    let kind = p::req_str(v, "kind")?;
    let name = check_name(p::req_str(v, "name")?)?;
    let c = &s.custom_themes;
    let exists = match kind {
        "colors" => c.colors.iter().any(|x| x.name.eq_ignore_ascii_case(&name)),
        "fonts" => c.fonts.iter().any(|x| x.name.eq_ignore_ascii_case(&name)),
        "theme" => c.themes.iter().any(|x| x.name.eq_ignore_ascii_case(&name)),
        _ => return Err(CmdError::Params(format!("unknown `kind` `{kind}` (colors, fonts or theme)"))),
    };
    if !exists {
        return Err(CmdError::Params(format!("no custom {kind} named `{name}`")));
    }
    if kind == "theme" {
        // The file goes first: if it can't be removed the theme stays, so it doesn't come back
        // from the folder at the next start while looking deleted now.
        if let Some(dir) = s.themes_dir.clone() {
            let file = theme_file(&dir, &name);
            if file.exists() {
                crate::io::remove_file(&file).map_err(|e| CmdError::Failed(format!("couldn't delete the theme: {e}")))?;
            }
        }
        s.custom_themes.themes.retain(|x| !x.name.eq_ignore_ascii_case(&name));
    } else {
        match kind {
            "colors" => s.custom_themes.colors.retain(|x| !x.name.eq_ignore_ascii_case(&name)),
            _ => s.custom_themes.fonts.retain(|x| !x.name.eq_ignore_ascii_case(&name)),
        }
        persist(s);
    }
    Ok(json!({"deleted": name, "kind": kind}))
}

/// Renaming a set (`renameFrom`): the old set goes in the same step as the new one is written, so
/// `custom.json` is saved once and never holds both, or neither. Nothing changes unless it all can.
fn drop_renamed<T>(list: &mut Vec<T>, name_of: impl Fn(&T) -> &str, v: &Value, name: &str, replace: bool, what: &str) -> Result<(), CmdError> {
    let Some(from) = p::str(v, "renameFrom").map(check_name).transpose()? else { return Ok(()) };
    if from.eq_ignore_ascii_case(name) {
        return Ok(());
    }
    let Some(old) = list.iter().position(|c| name_of(c).eq_ignore_ascii_case(&from)) else {
        return Err(CmdError::Params(format!("no custom {what} named `{from}`")));
    };
    if !replace && list.iter().any(|c| name_of(c).eq_ignore_ascii_case(name)) {
        return Err(CmdError::Params(format!("a {what} named `{name}` exists (pass `replace` to update it)")));
    }
    list.remove(old);
    Ok(())
}

/// Whether `path` is a file directly inside `dir`, however it is spelled (`a/../themes/x`). A path
/// that climbs out of its start can't be placed, so it is an error rather than a guess.
fn directly_in(dir: &Path, path: &Path) -> Result<bool, CmdError> {
    use std::path::Component;
    fn tidy(p: &Path) -> Option<PathBuf> {
        let mut out = PathBuf::new();
        for c in p.components() {
            match c {
                Component::CurDir => {}
                Component::ParentDir => {
                    if !out.pop() {
                        return None;
                    }
                }
                c => out.push(c.as_os_str()),
            }
        }
        Some(out)
    }
    // The filesystem's answer first: it follows links before `..`, which spelling alone can't.
    let real = |p: &Path| crate::io::real_path(p);
    if let (Some(a), Some(b)) = (path.parent().and_then(real), real(dir)) {
        return Ok(a == b);
    }
    // Not there yet (or no filesystem): compare the spelling, but never guess about a `..`.
    let (Some(d), Some(f)) = (tidy(dir), tidy(path)) else {
        return Err(CmdError::Params("`path` climbs out of its folder".into()));
    };
    if path.components().any(|c| c == Component::ParentDir) {
        return Err(CmdError::Params("can't tell which folder `path` is in".into()));
    }
    Ok(f.parent() == Some(d.as_path()))
}

fn save_theme(s: &mut Session, v: &Value) -> CmdResult {
    let name = p::str(v, "name").map(check_name).transpose()?;
    if let Some(n) = name.as_deref().filter(|n| is_builtin(n)) {
        return Err(CmdError::Params(format!("`{n}` is a built-in theme")));
    }
    let mut theme = Theme::of(&s.doc);
    if let Some(n) = &name {
        theme.name = n.clone();
    }
    let path = match p::str(v, "path") {
        Some(path) => {
            let path = path.trim();
            if path.is_empty() || path.len() > MAX_PATH {
                return Err(CmdError::Params("`path` is empty or too long".into()));
            }
            PathBuf::from(path)
        }
        None => {
            let Some(n) = &name else {
                // From the ribbon: the UI asks where and runs this again with the path.
                s.ui_requests.push(json!({"open": "saveTheme"}));
                return Ok(json!({"pending": true}));
            };
            let dir = s.themes_dir.clone().ok_or_else(|| CmdError::Failed("there is no themes folder here: pass `path`".into()))?;
            crate::io::create_dir(&dir).map_err(CmdError::Failed)?;
            theme_file(&dir, n)
        }
    };
    let in_folder = s.themes_dir.as_deref().map(|d| directly_in(d, &path)).transpose()?.unwrap_or(false);
    if in_folder && !path.extension().is_some_and(|e| e.eq_ignore_ascii_case(THEME_EXT)) {
        return Err(CmdError::Params(format!("a theme in the themes folder must end in `.{THEME_EXT}`")));
    }
    let stem = path.file_stem().map(|x| x.to_string_lossy().to_string());
    // The gallery and startup skip built-in names in the themes folder, so don't write one there.
    if in_folder && let Some(stem) = stem.as_deref().filter(|n| is_builtin(n)) {
        return Err(CmdError::Params(format!("`{stem}` is a built-in theme")));
    }
    let bytes = wordcraft_docx::write_thmx(&theme).map_err(|e| CmdError::Failed(e.to_string()))?;
    crate::io::write_file(&path.to_string_lossy(), &bytes).map_err(CmdError::Failed)?;
    // Saved to the themes folder: it shows in the Themes gallery.
    if in_folder && let Some(stem) = stem {
        let mut t = theme.clone();
        t.name = stem.clone();
        let list = &mut s.custom_themes.themes;
        match list.iter_mut().find(|x| x.name.eq_ignore_ascii_case(&stem)) {
            Some(x) => *x = t,
            None => list.push(t),
        }
    }
    Ok(json!({"path": path.to_string_lossy(), "name": theme.name}))
}

fn open_theme(s: &mut Session, v: &Value) -> CmdResult {
    let Some(path) = p::str(v, "path") else {
        if v.get("data").is_some() {
            return Err(CmdError::Params("`path` (string) is required".into()));
        }
        // From the ribbon: the UI asks for the file and runs this again with it.
        s.ui_requests.push(json!({"open": "browseTheme"}));
        return Ok(json!({"pending": true}));
    };
    let path = path.trim();
    if path.is_empty() || path.len() > MAX_PATH {
        return Err(CmdError::Params("`path` is empty or too long".into()));
    }
    let bytes = match p::str(v, "data") {
        Some(d) => {
            // Refuse before decoding: the decoder allocates from the text's length.
            if d.len() > wordcraft_docx::MAX_THEME_FILE.div_ceil(3).saturating_mul(4) {
                return Err(CmdError::Params("theme data is too large".into()));
            }
            super::insert::base64_decode(d).ok_or_else(|| CmdError::Params("bad base64".into()))?
        }
        None => {
            let local = super::templates::local_path(path, None).ok_or_else(|| CmdError::Failed(format!("`{path}` is not a local file")))?;
            crate::io::read_file_capped(&local.to_string_lossy(), wordcraft_docx::MAX_THEME_FILE as u64, "theme file").map_err(CmdError::Failed)?
        }
    };
    let mut theme = wordcraft_docx::read_theme(&bytes).map_err(|e| CmdError::Failed(format!("`{path}` has no theme: {e}")))?;
    if theme.name.trim().is_empty() {
        theme.name = Path::new(path).file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
    }
    apply_theme(s, &theme);
    sel_result(s)
}
