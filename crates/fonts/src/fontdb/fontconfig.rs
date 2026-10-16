//! The font folders named in the fontconfig configuration (Linux and the BSDs), so the system
//! font scan finds what `fc-list` lists (#2499): packages add their own folders through
//! `conf.d` (TeX Live's `/usr/share/texmf/fonts/opentype/...`), and users through
//! `~/.config/fontconfig/fonts.conf`.
//!
//! Read from the documented configuration format (fonts-conf(5)): `<dir>` elements, with their
//! `prefix` (`xdg`: the user's data folder; `relative`: the configuration file's folder) and a
//! leading `~` for the home folder, and the files and folders `<include>` pulls in (a folder's
//! `[0-9]*.conf` files, in name order). Everything else in the configuration is left alone.
//! Bounded whatever the files say: how many files are read, how large and how deep includes go.

use std::path::{Path, PathBuf};

/// Most configuration files read (a distribution has a few dozen).
const MAX_FILES: usize = 512;
/// Largest configuration file read.
const MAX_FILE_BYTES: u64 = 1 << 20;
/// Deepest chain of includes followed.
const MAX_DEPTH: usize = 16;
/// Most folders taken from the configuration.
const MAX_DIRS: usize = 256;

/// Where the configuration is, and the folders its relative names start from.
pub(crate) struct Env {
    /// The main configuration file (`/etc/fonts/fonts.conf`).
    pub(crate) config: PathBuf,
    pub(crate) home: Option<PathBuf>,
    /// `$XDG_DATA_HOME` (`~/.local/share`): where `prefix="xdg"` folders are.
    pub(crate) data_home: Option<PathBuf>,
    /// `$XDG_CONFIG_HOME` (`~/.config`): where `prefix="xdg"` includes are.
    pub(crate) config_home: Option<PathBuf>,
}

impl Env {
    /// This process's: `$FONTCONFIG_FILE` or `$FONTCONFIG_PATH/fonts.conf` before
    /// `/etc/fonts/fonts.conf`, and the XDG folders with their defaults.
    pub(crate) fn from_process() -> Env {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        let home = var("HOME");
        let config = var("FONTCONFIG_FILE")
            .or_else(|| var("FONTCONFIG_PATH").map(|d| d.join("fonts.conf")))
            .unwrap_or_else(|| PathBuf::from("/etc/fonts/fonts.conf"));
        let under_home = |p: &str| home.as_ref().map(|h| h.join(p));
        Env {
            config,
            data_home: var("XDG_DATA_HOME").filter(|p| p.is_absolute()).or_else(|| under_home(".local/share")),
            config_home: var("XDG_CONFIG_HOME").filter(|p| p.is_absolute()).or_else(|| under_home(".config")),
            home,
        }
    }
}

/// The font folders the configuration names, in the order it names them, without repeats.
pub(crate) fn dirs(env: &Env) -> Vec<PathBuf> {
    let mut r = Reader { env, files: 0, seen: Vec::new(), dirs: Vec::new() };
    r.file(&env.config, 0);
    r.dirs
}

struct Reader<'a> {
    env: &'a Env,
    files: usize,
    /// Files already read (an include of an include of itself ends there).
    seen: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
}

impl Reader<'_> {
    fn file(&mut self, path: &Path, depth: usize) {
        if depth > MAX_DEPTH || self.files >= MAX_FILES {
            return;
        }
        let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if self.seen.contains(&key) {
            return;
        }
        self.seen.push(key);
        let Some(text) = read_capped(path) else { return };
        self.files += 1;
        let here = path.parent().map(Path::to_path_buf).unwrap_or_default();
        for el in elements(&text) {
            match el.name {
                "dir" => {
                    if let Some(d) = self.resolve(&el.text, el.prefix.as_deref(), &here, false)
                        && self.dirs.len() < MAX_DIRS
                        && !self.dirs.contains(&d)
                    {
                        self.dirs.push(d);
                    }
                }
                "include" => {
                    if let Some(p) = self.resolve(&el.text, el.prefix.as_deref(), &here, true) {
                        self.include(&p, depth + 1);
                    }
                }
                _ => {}
            }
        }
    }

    /// An included file, or a folder's `[0-9]*.conf` files in name order.
    fn include(&mut self, path: &Path, depth: usize) {
        if path.is_dir() {
            let Ok(rd) = std::fs::read_dir(path) else { return };
            let mut names: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    name.starts_with(|c: char| c.is_ascii_digit()) && name.ends_with(".conf") && p.is_file()
                })
                .take(MAX_FILES)
                .collect();
            names.sort();
            for n in names {
                self.file(&n, depth);
            }
        } else {
            self.file(path, depth);
        }
    }

    /// A `<dir>` or (`include`) `<include>` name as a path: `~` is the home folder,
    /// `prefix="xdg"` starts from the user's data folder (a `<dir>`) or configuration folder (an
    /// `<include>`), `prefix="relative"` and a relative include from the file's folder. A
    /// relative `<dir>` without a prefix would start from the working folder, which means nothing
    /// here: it is left out.
    fn resolve(&self, name: &str, prefix: Option<&str>, here: &Path, include: bool) -> Option<PathBuf> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        if let Some(rest) = name.strip_prefix('~') {
            let rest = rest.trim_start_matches('/');
            return self.env.home.as_ref().map(|h| if rest.is_empty() { h.clone() } else { h.join(rest) });
        }
        let p = Path::new(name);
        if p.is_absolute() {
            return Some(p.to_path_buf());
        }
        let xdg = if include { &self.env.config_home } else { &self.env.data_home };
        match prefix {
            Some("xdg") => xdg.as_ref().map(|x| x.join(p)),
            Some("relative") => Some(here.join(p)),
            // An include without a prefix starts from the including file's folder ("conf.d").
            None if include => Some(here.join(p)),
            _ => None,
        }
    }
}

/// A configuration file's text, or None when it's missing, unreadable or too large.
fn read_capped(path: &Path) -> Option<String> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    if !f.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    f.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// A `<dir>` or `<include>` element: its name, `prefix` and text.
struct Element<'a> {
    name: &'a str,
    prefix: Option<String>,
    text: String,
}

/// The `<dir>` and `<include>` elements of a configuration file, in order. Comments,
/// declarations and processing instructions are skipped; other elements are stepped over.
fn elements(text: &str) -> Vec<Element<'static>> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find('<') {
        rest = rest.get(at..).unwrap_or("");
        if let Some(r) = rest.strip_prefix("<!--") {
            rest = r.find("-->").and_then(|i| r.get(i + 3..)).unwrap_or("");
            continue;
        }
        if rest.starts_with("<?") || rest.starts_with("<!") {
            rest = rest.find('>').and_then(|i| rest.get(i + 1..)).unwrap_or("");
            continue;
        }
        let Some(close) = rest.find('>') else { break };
        let tag = rest.get(1..close).unwrap_or("");
        rest = rest.get(close + 1..).unwrap_or("");
        let name_end = tag.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(tag.len());
        let name = match tag.get(..name_end) {
            Some("dir") => "dir",
            Some("include") => "include",
            _ => continue,
        };
        if tag.ends_with('/') {
            continue;
        }
        let end = format!("</{name}>");
        let Some(stop) = rest.find(&end) else { break };
        let body = rest.get(..stop).unwrap_or("");
        rest = rest.get(stop + end.len()..).unwrap_or("");
        out.push(Element { name, prefix: attr(tag, "prefix"), text: unescape(body) });
    }
    out
}

/// Attribute `name`'s value in a start tag's text.
fn attr(tag: &str, name: &str) -> Option<String> {
    let mut rest = tag;
    while let Some(at) = rest.find(name) {
        let before = rest.get(..at).and_then(|b| b.chars().last());
        let after = rest.get(at + name.len()..).unwrap_or("").trim_start();
        rest = rest.get(at + name.len()..).unwrap_or("");
        if !before.is_some_and(char::is_whitespace) {
            continue;
        }
        let Some(v) = after.strip_prefix('=') else { continue };
        let v = v.trim_start();
        let q = v.chars().next()?;
        if q != '"' && q != '\'' {
            return None;
        }
        let v = v.get(1..)?;
        return v.find(q).and_then(|i| v.get(..i)).map(unescape);
    }
    None
}

/// The five predefined XML entities; anything else is kept as written.
fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wc-fontconfig-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn env(root: &Path) -> Env {
        Env {
            config: root.join("etc/fonts.conf"),
            home: Some(root.join("home")),
            data_home: Some(root.join("home/.local/share")),
            config_home: Some(root.join("home/.config")),
        }
    }

    /// Folders from the main file, its `conf.d` (numbered files only, in name order) and the
    /// user's configuration, with each kind of name: absolute, `~`, `prefix="xdg"`,
    /// `prefix="relative"`. A `<cachedir>`, a commented-out `<dir>`, a relative `<dir>` without a
    /// prefix and a repeated folder are left out; a missing include is fine.
    #[test]
    fn the_configuration_names_the_folders() {
        let root = tmp("names");
        let etc = root.join("etc");
        std::fs::create_dir_all(etc.join("conf.d")).unwrap();
        std::fs::create_dir_all(root.join("home/.config/fontconfig")).unwrap();
        std::fs::write(
            etc.join("fonts.conf"),
            r#"<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <!-- <dir>/commented/out</dir> -->
  <dir>/usr/share/fonts</dir>
  <dir prefix="xdg">fonts</dir>
  <dir>~/.fonts</dir>
  <dir>relative/without/prefix</dir>
  <cachedir>/var/cache/fontconfig</cachedir>
  <include ignore_missing="yes">conf.d</include>
  <include ignore_missing="yes" prefix="xdg">fontconfig/fonts.conf</include>
  <include ignore_missing="yes">/no/such/file.conf</include>
  <match target="pattern"><test name="family"><string>mono</string></test></match>
</fontconfig>"#,
        )
        .unwrap();
        std::fs::write(etc.join("conf.d/65-fonts-lmodern.conf"), "<fontconfig><dir>/usr/share/texmf/fonts/opentype/public/lm</dir></fontconfig>")
            .unwrap();
        std::fs::write(etc.join("conf.d/09-early.conf"), "<fontconfig><dir prefix='relative'>../extra &amp; more</dir></fontconfig>").unwrap();
        std::fs::write(etc.join("conf.d/README"), "<fontconfig><dir>/not/a/conf/file</dir></fontconfig>").unwrap();
        std::fs::write(etc.join("conf.d/70-again.conf"), "<fontconfig><dir>/usr/share/fonts</dir></fontconfig>").unwrap();
        std::fs::write(root.join("home/.config/fontconfig/fonts.conf"), "<fontconfig><dir>/opt/my fonts</dir></fontconfig>").unwrap();
        let got = dirs(&env(&root));
        assert_eq!(
            got,
            vec![
                PathBuf::from("/usr/share/fonts"),
                root.join("home/.local/share/fonts"),
                root.join("home/.fonts"),
                etc.join("conf.d/../extra & more"),
                PathBuf::from("/usr/share/texmf/fonts/opentype/public/lm"),
                PathBuf::from("/opt/my fonts"),
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Includes that lead back to themselves, a file larger than the cap and a configuration
    /// that isn't there all end quietly.
    #[test]
    fn hostile_configurations_stay_bounded() {
        let root = tmp("hostile");
        let etc = root.join("etc");
        std::fs::create_dir_all(&etc).unwrap();
        std::fs::write(
            etc.join("fonts.conf"),
            "<fontconfig><dir>/a</dir><include>fonts.conf</include><include>loop.conf</include><include>big.conf</include></fontconfig>",
        )
        .unwrap();
        std::fs::write(etc.join("loop.conf"), "<fontconfig><dir>/b</dir><include>fonts.conf</include><include>loop.conf</include></fontconfig>")
            .unwrap();
        let mut big = String::from("<fontconfig><dir>/too/big</dir>");
        big.push_str(&" ".repeat(MAX_FILE_BYTES as usize));
        big.push_str("</fontconfig>");
        std::fs::write(etc.join("big.conf"), big).unwrap();
        assert_eq!(dirs(&env(&root)), vec![PathBuf::from("/a"), PathBuf::from("/b")]);
        let mut missing = env(&root);
        missing.config = root.join("nowhere/fonts.conf");
        assert!(dirs(&missing).is_empty());
        // Unclosed and malformed elements.
        let els = elements("<fontconfig><dir prefix=xdg>fonts</dir><dir>/ok</dir><include>never closed");
        assert_eq!(
            els.iter().map(|e| (e.name, e.prefix.clone(), e.text.as_str())).collect::<Vec<_>>(),
            vec![("dir", None, "fonts"), ("dir", None, "/ok")]
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
