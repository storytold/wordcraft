//! Fonts the site's admin publishes next to the app ("server fonts").
//!
//! At start-up the web shell fetches `fonts/manifest.json`, relative to the page like every other
//! URL (so it works at a domain root, under a prefix or from a CDN bucket), then downloads each
//! font it lists from the same folder and adds it to the font database like a font the user
//! picked with Add Font. No manifest (a 404) means no server fonts, silently. There is no server
//! side code: the admin uploads font files and edits the manifest. Format and hosting notes:
//! `packaging/web/README.md`; a sample: `packaging/web/fonts/manifest.json`.
//!
//! This file holds the manifest rules, which are plain Rust and tested natively; the fetching is
//! wasm only.

use std::collections::HashSet;

/// Where the manifest is, relative to the page. Listed font files are relative to its folder.
#[cfg(target_arch = "wasm32")]
pub const MANIFEST_URL: &str = "fonts/manifest.json";
/// Most fonts one manifest may list; the rest are ignored.
pub const MAX_ENTRIES: usize = 200;
/// Most font bytes loaded from one manifest (wasm memory is small).
#[cfg(target_arch = "wasm32")]
pub const MAX_TOTAL_BYTES: usize = 256 << 20;

/// Why a manifest couldn't be used.
#[derive(Debug, PartialEq, Eq)]
pub enum ManifestError {
    /// Not JSON, or not the shape described in the README.
    Invalid(String),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self::Invalid(why) = self;
        write!(f, "invalid server font manifest: {why}")
    }
}

/// The font files a manifest lists (paths relative to the manifest's folder), in order, without
/// duplicates and without entries that aren't plain relative font file paths. The skipped ones
/// come back in the second list with the reason, for the log.
///
/// The manifest is `{"fonts": [...]}`; each entry is a file name (`"Battambang.ttf"`) or an
/// object with a `file` member (`{"file": "Battambang.ttf"}`), which leaves room for more members.
pub fn parse_manifest(text: &str) -> Result<(Vec<String>, Vec<(String, String)>), ManifestError> {
    let json: serde_json::Value = serde_json::from_str(text).map_err(|e| ManifestError::Invalid(e.to_string()))?;
    let list = json.get("fonts").and_then(|f| f.as_array()).ok_or_else(|| ManifestError::Invalid("expected {\"fonts\": [...]}".into()))?;
    let (mut files, mut skipped, mut seen) = (Vec::new(), Vec::new(), HashSet::new());
    for entry in list {
        let name = match entry {
            serde_json::Value::String(s) => Some(s.as_str()),
            serde_json::Value::Object(o) => o.get("file").and_then(|f| f.as_str()),
            _ => None,
        };
        let Some(name) = name else {
            skipped.push((entry.to_string(), "not a file name or an object with \"file\"".to_string()));
            continue;
        };
        if let Err(why) = check_path(name) {
            skipped.push((name.to_string(), why.to_string()));
        } else if files.len() >= MAX_ENTRIES {
            skipped.push((name.to_string(), format!("more than {MAX_ENTRIES} fonts")));
        } else if seen.insert(name.to_string()) {
            files.push(name.to_string());
        }
    }
    Ok((files, skipped))
}

/// A listed file must stay inside the fonts folder: relative, no `..`, no scheme or query, and
/// named like a font file.
fn check_path(name: &str) -> Result<(), &'static str> {
    if name.is_empty() || name.starts_with('/') || name.contains('\\') || name.contains(':') || name.contains('?') || name.contains('#') {
        return Err("must be a relative path inside the fonts folder");
    }
    if name.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
        return Err("must not contain empty, . or .. parts");
    }
    if name.chars().any(char::is_control) {
        return Err("contains a control character");
    }
    if !wordcraft_fonts::is_font_file_name(name) {
        return Err("not a font file name (ttf, otf, ttc, otc, woff, woff2)");
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub use fetch::load_all;

#[cfg(target_arch = "wasm32")]
mod fetch {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::JsValue;

    use super::{MANIFEST_URL, MAX_TOTAL_BYTES, parse_manifest};

    /// GET `url` (relative to the page); `Ok(None)` for a 404.
    async fn get(url: &str) -> Result<Option<web_sys::Response>, String> {
        let window = web_sys::window().ok_or("no window")?;
        let response = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(url)).await.map_err(|e| format!("{e:?}"))?;
        let response: web_sys::Response = response.dyn_into().map_err(|_| "not a response".to_string())?;
        if response.status() == 404 {
            return Ok(None);
        }
        if !response.ok() {
            return Err(format!("HTTP {}", response.status()));
        }
        Ok(Some(response))
    }

    /// `path` with each part percent-encoded (file names may hold spaces).
    fn encode(path: &str) -> String {
        path.split('/').map(|part| String::from(js_sys::encode_uri_component(part))).collect::<Vec<_>>().join("/")
    }

    async fn bytes_of(response: &web_sys::Response) -> Result<Vec<u8>, JsValue> {
        let buffer = wasm_bindgen_futures::JsFuture::from(response.array_buffer()?).await?;
        Ok(js_sys::Uint8Array::new(&buffer).to_vec())
    }

    /// Fetch the manifest and add every font it lists to the font database (each checked like
    /// one the user picked; a font that fails is logged and skipped), repainting as fonts arrive:
    /// the app lays out again when fonts change.
    pub fn load_all(ctx: egui::Context) {
        wasm_bindgen_futures::spawn_local(async move {
            let text = match get(MANIFEST_URL).await {
                Ok(Some(response)) => match response.text() {
                    Ok(promise) => wasm_bindgen_futures::JsFuture::from(promise).await.ok().and_then(|t| t.as_string()),
                    Err(_) => None,
                },
                Ok(None) => return,
                Err(e) => {
                    log::warn!("server fonts: {MANIFEST_URL}: {e}");
                    return;
                }
            };
            let Some(text) = text else {
                log::warn!("server fonts: couldn't read {MANIFEST_URL}");
                return;
            };
            let (files, skipped) = match parse_manifest(&text) {
                Ok(parsed) => parsed,
                Err(e) => {
                    log::warn!("server fonts: {e}");
                    return;
                }
            };
            for (name, why) in skipped {
                log::warn!("server fonts: skipped {name:?}: {why}");
            }
            let folder = MANIFEST_URL.rsplit_once('/').map_or("", |(dir, _)| dir);
            let (mut total, mut added) = (0usize, 0usize);
            for file in files {
                let url = format!("{folder}/{}", encode(&file));
                let bytes = match get(&url).await {
                    Ok(Some(response)) => match bytes_of(&response).await {
                        Ok(bytes) => bytes,
                        Err(e) => {
                            log::warn!("server fonts: couldn't read {file:?}: {e:?}");
                            continue;
                        }
                    },
                    Ok(None) => {
                        log::warn!("server fonts: {file:?} is listed but not on the server");
                        continue;
                    }
                    Err(e) => {
                        log::warn!("server fonts: {file:?}: {e}");
                        continue;
                    }
                };
                if bytes.len() > wordcraft_fonts::MAX_FONT_FILE_BYTES {
                    log::warn!("server fonts: {file:?} is too large; skipped");
                    continue;
                }
                total = total.saturating_add(bytes.len());
                if total > MAX_TOTAL_BYTES {
                    log::warn!("server fonts: stopped at {file:?}: more than {} MB in all", MAX_TOTAL_BYTES >> 20);
                    break;
                }
                match wordcraft_fonts::FontDb::global().install_font(&file, &bytes) {
                    Ok(_) => {
                        added += 1;
                        ctx.request_repaint();
                    }
                    Err(wordcraft_fonts::UserFontError::AlreadyAvailable(_)) => {}
                    Err(e) => log::warn!("server fonts: {file:?} skipped: {e}"),
                }
            }
            log::info!("server fonts: {added} added");
            ctx.request_repaint();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(text: &str) -> Vec<String> {
        parse_manifest(text).map(|(f, _)| f).unwrap_or_default()
    }

    #[test]
    fn reads_names_and_objects_in_order_without_duplicates() {
        let text = r#"{"fonts": ["A.ttf", {"file": "sub/B.woff2", "note": "x"}, "A.ttf"]}"#;
        assert_eq!(files(text), ["A.ttf", "sub/B.woff2"]);
    }

    #[test]
    fn skips_paths_that_leave_the_folder() {
        let text = r#"{"fonts": ["../A.ttf", "/etc/A.ttf", "https://x.test/A.ttf", "a\\b.ttf", "a/../b.ttf", "a//b.ttf", "A.ttf?x", "A.txt", "", 7, {"nofile": 1}, "ok.otf"]}"#;
        let (good, skipped) = parse_manifest(text).unwrap_or_default();
        assert_eq!(good, ["ok.otf"]);
        assert_eq!(skipped.len(), 11);
    }

    #[test]
    fn caps_the_number_of_fonts() {
        let list: Vec<String> = (0..MAX_ENTRIES + 5).map(|i| format!("\"f{i}.ttf\"")).collect();
        let (good, skipped) = parse_manifest(&format!("{{\"fonts\": [{}]}}", list.join(","))).unwrap_or_default();
        assert_eq!((good.len(), skipped.len()), (MAX_ENTRIES, 5));
    }

    #[test]
    fn rejects_other_shapes() {
        for text in ["", "[]", "{}", r#"{"fonts": "A.ttf"}"#, "not json"] {
            assert!(parse_manifest(text).is_err(), "{text}");
        }
        assert_eq!(files(r#"{"fonts": []}"#), Vec::<String>::new());
    }
}
