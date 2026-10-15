//! Installing a plugin from its GitHub repository.
//!
//! `https://github.com/owner/repo` (or `…/releases/tag/v1.2`, `…/tree/branch`) resolves to a zip:
//! the release asset built for this platform (`…-macos-aarch64.zip`), else the release's source,
//! else the source of the default branch (or the named one). The folder holding `plugin.json` in
//! the zip is unpacked into the plugins folder as `<plugin id>/`, replacing an earlier install, with
//! `.craft-source.json` recording where it came from. Loading it is the caller's job.
//!
//! An add-in another one requires (manifest 1.3) is installed from the newest release whose tag
//! is a version in the required range ([`fetch`] with a range; [`crate::requires::pick_release`]).

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::RwLock;

use serde_json::{Value as Json, json};

use crate::version::Range;
use crate::{MANIFEST, Manifest};

/// Written next to `plugin.json` in an installed plugin.
pub const SOURCE_FILE: &str = ".craft-source.json";
/// Largest download.
const MAX_DOWNLOAD: u64 = 256 << 20;
/// Largest unpacked plugin, and most files in it.
const MAX_UNPACKED: u64 = 512 << 20;
const MAX_ENTRIES: usize = 20_000;

static INSTALL_DIR: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Sets where [`from_github`] installs plugins (the app: `plugins` in the settings folder).
pub fn set_install_dir(dir: Option<PathBuf>) {
    match INSTALL_DIR.write() {
        Ok(mut g) => *g = dir,
        Err(poisoned) => *poisoned.into_inner() = dir,
    }
}

/// The folder plugins are installed into, when the host has set one.
pub fn install_dir() -> Option<PathBuf> {
    match INSTALL_DIR.read() {
        Ok(g) => g.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

/// Which version of a repository to install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Version {
    /// The latest release, else the default branch.
    Latest,
    /// A release (`…/releases/tag/<tag>`).
    Release(String),
    /// A branch, tag or commit's source (`…/tree/<ref>`).
    Tree(String),
}

/// A GitHub repository and the version to install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    pub owner: String,
    pub name: String,
    pub version: Version,
}

fn valid_segment(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && s != "." && s != ".." && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Reads `https://github.com/owner/repo[.git][/releases/tag/T | /releases/latest | /tree/REF]`,
/// with or without the scheme or `www.`, or plain `owner/repo`.
pub fn parse_github_url(url: &str) -> Result<Repo, String> {
    let bad = || format!("`{url}` isn't a GitHub repository address (https://github.com/owner/repo)");
    let s = url.trim().trim_end_matches('/');
    let rest = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")).unwrap_or(s);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    let path = match rest.strip_prefix("github.com/") {
        Some(p) => p,
        None if !s.contains("://") && rest.split('/').count() == 2 => rest,
        None => return Err(bad()),
    };
    let path = path.split(['?', '#']).next().unwrap_or("");
    let mut parts = path.splitn(3, '/');
    let owner = parts.next().unwrap_or("");
    let name = parts.next().unwrap_or("");
    let name = name.strip_suffix(".git").unwrap_or(name);
    if !valid_segment(owner) || !valid_segment(name) {
        return Err(bad());
    }
    let version = match parts.next() {
        None | Some("") | Some("releases") | Some("releases/latest") => Version::Latest,
        Some(more) => {
            if let Some(tag) = more.strip_prefix("releases/tag/") {
                Version::Release(tag.to_string())
            } else if let Some(r) = more.strip_prefix("tree/") {
                Version::Tree(r.to_string())
            } else {
                return Err(bad());
            }
        }
    };
    if let Version::Release(r) | Version::Tree(r) = &version
        && (r.is_empty() || r.len() > 255 || r.contains("..") || r.chars().any(|c| c.is_whitespace() || c.is_control()))
    {
        return Err(bad());
    }
    Ok(Repo { owner: owner.to_string(), name: name.to_string(), version })
}

fn os_tokens(os: &str) -> &'static [&'static str] {
    match os {
        "macos" => &["macos", "darwin", "mac", "osx", "apple"],
        "linux" => &["linux"],
        "windows" => &["windows", "win", "win64", "win32"],
        "freebsd" => &["freebsd"],
        _ => &[],
    }
}

fn arch_tokens(arch: &str) -> &'static [&'static str] {
    match arch {
        "aarch64" => &["aarch64", "arm64"],
        "x86_64" => &["x86_64", "x64", "amd64", "x86-64"],
        _ => &[],
    }
}

const ALL_OS: [&str; 4] = ["macos", "linux", "windows", "freebsd"];
const ALL_ARCH: [&str; 2] = ["aarch64", "x86_64"];

/// The release asset to install on `os`/`arch` (as in `std::env::consts`): a `.zip` naming this
/// platform (with this architecture, `universal`, or no architecture), else a `.zip` naming no
/// platform at all. `None`: no asset fits (install the source instead).
pub fn pick_asset(names: &[String], os: &str, arch: &str) -> Option<usize> {
    let mut best: Option<(u8, usize)> = None;
    for (i, name) in names.iter().enumerate() {
        let lower = name.to_ascii_lowercase();
        let Some(stem) = lower.strip_suffix(".zip") else { continue };
        let tokens: Vec<&str> = stem.split(['-', '.', ' ', '+']).collect();
        let has = |words: &[&str]| words.iter().any(|w| tokens.contains(w) || (w.contains(['_', '-']) && stem.contains(w)));
        let names_os = ALL_OS.iter().any(|o| has(os_tokens(o)));
        let names_arch = ALL_ARCH.iter().any(|a| has(arch_tokens(a)));
        let score = if !names_os {
            1
        } else if !has(os_tokens(os)) {
            continue;
        } else if has(arch_tokens(arch)) {
            4
        } else if tokens.contains(&"universal") || !names_arch {
            3
        } else {
            continue; // This platform, another architecture.
        };
        if best.is_none_or(|(s, _)| score > s) {
            best = Some((score, i));
        }
    }
    best.map(|(_, i)| i)
}

/// Opens a plugin zip and reads its manifest: the shallowest `plugin.json` (a GitHub source zip
/// wraps everything in one folder). Returns the zip, the manifest and the folder holding it.
fn open_zip(zip_bytes: &[u8]) -> Result<(zip::ZipArchive<std::io::Cursor<&[u8]>>, Manifest, PathBuf), String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|e| format!("not a zip file: {e}"))?;
    if zip.len() > MAX_ENTRIES {
        return Err(format!("the zip holds more than {MAX_ENTRIES} files"));
    }
    // Find the manifest.
    let mut manifest_at: Option<(usize, PathBuf)> = None;
    for i in 0..zip.len() {
        let Ok(f) = zip.by_index(i) else { continue };
        let Some(p) = f.enclosed_name() else { continue };
        if p.file_name().is_some_and(|n| n == MANIFEST) && manifest_at.as_ref().is_none_or(|(_, q)| p.components().count() < q.components().count()) {
            manifest_at = Some((i, p));
        }
    }
    let (index, manifest_path) = manifest_at.ok_or_else(|| format!("no {MANIFEST} in the download: is this a plugin?"))?;
    let prefix = manifest_path.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut text = String::new();
    zip.by_index(index)
        .map_err(|e| e.to_string())?
        .take(crate::MAX_MANIFEST_BYTES)
        .read_to_string(&mut text)
        .map_err(|e| format!("{MANIFEST}: {e}"))?;
    let manifest = Manifest::parse(&text)?;
    Ok((zip, manifest, prefix))
}

/// The manifest of the plugin in a zip, without unpacking it.
pub fn zip_manifest(zip_bytes: &[u8]) -> Result<Manifest, String> {
    open_zip(zip_bytes).map(|(_, m, _)| m)
}

/// Unpacks the plugin in a zip into `root/<id>/` and returns that folder. The plugin is the
/// shallowest folder holding `plugin.json` (a GitHub source zip wraps everything in one folder).
pub fn unpack(zip_bytes: &[u8], root: &Path) -> Result<PathBuf, String> {
    let (mut zip, manifest, prefix) = open_zip(zip_bytes)?;
    std::fs::create_dir_all(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let staging = root.join(format!(".installing-{}-{}", manifest.id, std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    let unpacked = (|| -> Result<(), String> {
        let mut total = 0u64;
        for i in 0..zip.len() {
            let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
            let Some(path) = f.enclosed_name() else { continue };
            let Ok(rel) = path.strip_prefix(&prefix) else { continue };
            if rel.as_os_str().is_empty() || f.is_symlink() || rel.components().any(|c| !matches!(c, Component::Normal(_))) {
                continue;
            }
            let out = staging.join(rel);
            if f.is_dir() {
                std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
                continue;
            }
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            let mut file = std::fs::File::create(&out).map_err(|e| format!("{}: {e}", out.display()))?;
            let room = MAX_UNPACKED.saturating_sub(total);
            let n = std::io::copy(&mut (&mut f).take(room.saturating_add(1)), &mut file).map_err(|e| format!("{}: {e}", out.display()))?;
            total = total.saturating_add(n);
            if total > MAX_UNPACKED {
                return Err(format!("the plugin is larger than {} MiB unpacked", MAX_UNPACKED >> 20));
            }
            #[cfg(unix)]
            if let Some(mode) = f.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                // Keep programs runnable; nothing more than rwxr-xr-x.
                let mode = if mode & 0o111 != 0 { 0o755 } else { 0o644 };
                let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode));
            }
        }
        Ok(())
    })();
    if let Err(e) = unpacked {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    // Swap it in for an earlier install of the same plugin.
    let target = root.join(&manifest.id);
    let old = root.join(format!(".replaced-{}-{}", manifest.id, std::process::id()));
    if target.exists() {
        std::fs::rename(&target, &old).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    if let Err(e) = std::fs::rename(&staging, &target) {
        let _ = std::fs::rename(&old, &target);
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!("{}: {e}", target.display()));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(target)
}

/// Where an installed plugin came from (its `.craft-source.json`), if it was installed.
pub fn read_source(dir: &Path) -> Option<Json> {
    let f = std::fs::File::open(dir.join(SOURCE_FILE)).ok()?;
    let mut text = String::new();
    f.take(64 * 1024).read_to_string(&mut text).ok()?;
    serde_json::from_str(&text).ok()
}

/// A plugin downloaded from GitHub, not unpacked yet: its manifest can be checked first (does
/// the new version still suit the add-ins that need it?).
#[derive(Clone, Debug)]
pub struct Fetched {
    pub manifest: Manifest,
    /// The release's tag, when it came from a release.
    pub release: Option<String>,
    /// What `.craft-source.json` records.
    pub source: Json,
    bytes: Vec<u8>,
}

impl Fetched {
    /// Unpacks it into `root/<id>/` (replacing an earlier install) and records where it came from.
    pub fn unpack(&self, root: &Path) -> Result<PathBuf, String> {
        let dir = unpack(&self.bytes, root)?;
        let _ = std::fs::write(dir.join(SOURCE_FILE), serde_json::to_vec_pretty(&self.source).unwrap_or_default());
        Ok(dir)
    }
}

/// Downloads the plugin in a GitHub repository and unpacks it into `root` (see the module docs).
/// Returns its folder; load it with [`crate::load`].
pub fn from_github(url: &str, root: &Path) -> Result<PathBuf, String> {
    fetch(url, None)?.unpack(root)
}

/// Downloads the plugin in a GitHub repository (see the module docs) without unpacking it. With
/// `want` (a required add-in's range) and an address naming no release, the newest release
/// whose tag is a version in range is taken; with `*` that's simply the latest.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::disallowed_methods)] // The clock is fine off wasm.
pub fn fetch(url: &str, want: Option<&Range>) -> Result<Fetched, String> {
    let repo = parse_github_url(url)?;
    let api = format!("https://api.github.com/repos/{}/{}", repo.owner, repo.name);
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(15))
        .timeout_read(std::time::Duration::from_secs(60))
        .user_agent(&format!("WordCraft/{}", env!("CARGO_PKG_VERSION")))
        .build();
    let release = match (&repo.version, want.filter(|r| !r.is_any())) {
        (Version::Latest, Some(range)) => {
            // One page of the newest 100 releases.
            let list = get_json(&agent, &format!("{api}/releases?per_page=100"))?.unwrap_or(Json::Null);
            let list = list.as_array().cloned().unwrap_or_default();
            let (i, _) = crate::requires::pick_release(&list, range)
                .ok_or_else(|| format!("{}/{} has no release whose tag is a version in {range}", repo.owner, repo.name))?;
            list.get(i).cloned()
        }
        (Version::Latest, None) => get_json(&agent, &format!("{api}/releases/latest"))?,
        (Version::Release(tag), _) => Some(
            get_json(&agent, &format!("{api}/releases/tags/{tag}"))?.ok_or_else(|| format!("{}/{} has no release {tag}", repo.owner, repo.name))?,
        ),
        (Version::Tree(_), _) => None,
    };
    let (download, asset) = match &release {
        Some(r) => {
            let assets = r.get("assets").and_then(Json::as_array).cloned().unwrap_or_default();
            let names: Vec<String> = assets.iter().map(|a| a.get("name").and_then(Json::as_str).unwrap_or("").to_string()).collect();
            match pick_asset(&names, std::env::consts::OS, std::env::consts::ARCH)
                .and_then(|i| Some((assets.get(i)?.get("browser_download_url")?.as_str()?.to_string(), names.get(i)?.clone())))
            {
                Some((u, name)) => (u, Some(name)),
                None => (r.get("zipball_url").and_then(Json::as_str).ok_or("the release has no source zip")?.to_string(), None),
            }
        }
        None => match &repo.version {
            Version::Tree(r) => (format!("{api}/zipball/{r}"), None),
            _ => (format!("{api}/zipball"), None),
        },
    };
    log::info!("downloading plugin from {download}");
    let bytes = get_bytes(&agent, &download)?;
    let manifest = zip_manifest(&bytes)?;
    let tag = release.as_ref().and_then(|r| r.get("tag_name")).and_then(Json::as_str).map(str::to_string);
    let source = json!({
        "url": url.trim(),
        "repository": format!("https://github.com/{}/{}", repo.owner, repo.name),
        "release": tag,
        "asset": asset,
        "download": download,
        "version": manifest.version,
        "installed": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    });
    Ok(Fetched { manifest, release: tag, source, bytes })
}

#[cfg(target_arch = "wasm32")]
pub fn fetch(_url: &str, _want: Option<&Range>) -> Result<Fetched, String> {
    Err("plugins need the desktop app or the CLI".into())
}

/// A [`Fetched`] made from zip bytes at hand (tests, and installs from a file).
pub fn fetched_from_zip(bytes: Vec<u8>, source: Json) -> Result<Fetched, String> {
    let manifest = zip_manifest(&bytes)?;
    let release = source.get("release").and_then(Json::as_str).map(str::to_string);
    Ok(Fetched { manifest, release, source, bytes })
}

#[cfg(not(target_arch = "wasm32"))]
fn http_error(url: &str, e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(403 | 429, _) => {
            format!("GitHub refused the request ({url}); its limit for anonymous requests may be used up, try again later")
        }
        ureq::Error::Status(404, _) => format!("not found on GitHub ({url}): check the address; private repositories can't be installed"),
        ureq::Error::Status(code, _) => format!("{url}: HTTP {code}"),
        ureq::Error::Transport(t) => format!("{url}: {t}"),
    }
}

/// A GitHub API answer; `None` for 404.
#[cfg(not(target_arch = "wasm32"))]
fn get_json(agent: &ureq::Agent, url: &str) -> Result<Option<Json>, String> {
    match agent.get(url).set("Accept", "application/vnd.github+json").call() {
        Ok(r) => {
            let mut text = String::new();
            r.into_reader().take(8 << 20).read_to_string(&mut text).map_err(|e| format!("{url}: {e}"))?;
            serde_json::from_str(&text).map(Some).map_err(|e| format!("{url}: {e}"))
        }
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(e) => Err(http_error(url, e)),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn get_bytes(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, String> {
    let r = agent.get(url).call().map_err(|e| http_error(url, e))?;
    let mut bytes = Vec::new();
    r.into_reader().take(MAX_DOWNLOAD + 1).read_to_end(&mut bytes).map_err(|e| format!("{url}: {e}"))?;
    if bytes.len() as u64 > MAX_DOWNLOAD {
        return Err(format!("the download is larger than {} MiB", MAX_DOWNLOAD >> 20));
    }
    Ok(bytes)
}
