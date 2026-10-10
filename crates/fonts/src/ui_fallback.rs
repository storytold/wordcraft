//! An installed CJK font for the interface when no embedded craft-fonts face covers the interface
//! language (#241): a build without craft-fonts, or one whose craft-fonts checkout lacks the
//! Chinese face, would otherwise draw Chinese menus as boxes. Native only; wasm has no system
//! fonts to read.
//!
//! It looks for a short list of well-known files in the platform's font folders
//! ([`system_font_dirs`](crate::system_font_dirs)) rather than waiting for the full system font
//! scan, since the interface fonts are installed on the first frame.

use skrifa::MetadataProvider;
use skrifa::raw::FileRef;

use crate::CraftFont;

/// An installed font file chosen for the interface.
pub struct SystemUiFont {
    /// The face's family name, e.g. `Microsoft YaHei`.
    pub family: String,
    /// The whole font file.
    pub bytes: Vec<u8>,
    /// The face's index in the file (0 unless it is a collection).
    pub index: u32,
}

/// Do the embedded interface faces (`ui_cjk_fonts`) leave the interface without CJK glyphs it
/// needs? A Chinese interface (`prefer_hans`) needs a Chinese (`Hans`) face: the Japanese one
/// lacks the simplified-only hanzi (页, 选, 删 …). Any other interface still needs some CJK face,
/// for the language names in the Options menu and CJK file names.
pub fn ui_needs_system_cjk(prefer_hans: bool, embedded: &[&CraftFont]) -> bool {
    if prefer_hans { !embedded.iter().any(|f| f.scripts.contains(&"Hans")) } else { embedded.is_empty() }
}

/// Files larger than this are skipped (the big system CJK collections are around 20–80 MB).
const MAX_FILE: u64 = 128 << 20;
/// Faces looked at in one collection.
const MAX_FACES: u32 = 64;

/// A character only a font for the language has: a simplified-only hanzi, or kana.
fn probe(hans: bool) -> char {
    if hans { '页' } else { 'の' }
}

/// Font files to look for, relative to a font folder, best first. Chinese faces cover kana too,
/// so the Japanese list ends with the Chinese one.
fn candidates(hans: bool) -> Vec<&'static str> {
    let (zh, ja): (&[&str], &[&str]) = if cfg!(windows) {
        (&["Deng.ttf", "msyh.ttc", "simhei.ttf", "simsun.ttc", "NotoSansSC-VF.ttf"], &["YuGothR.ttc", "meiryo.ttc", "msgothic.ttc"])
    } else if cfg!(target_os = "macos") {
        (&["PingFang.ttc", "Hiragino Sans GB.ttc", "STHeiti Light.ttc", "STHeiti Medium.ttc"], &["ヒラギノ角ゴシック W3.ttc", "Hiragino Sans W3.ttc"])
    } else {
        (
            &[
                "noto-cjk/NotoSansCJK-Regular.ttc",
                "opentype/noto/NotoSansCJK-Regular.ttc",
                "google-noto-sans-cjk-fonts/NotoSansCJK-Regular.ttc",
                "google-noto-cjk/NotoSansCJK-Regular.ttc",
                "noto-cjk/NotoSansCJKsc-Regular.otf",
                "opentype/noto/NotoSansCJKsc-Regular.otf",
                "adobe-source-han-sans/SourceHanSansCN-Regular.otf",
                "wenquanyi/wqy-microhei/wqy-microhei.ttc",
                "truetype/wqy/wqy-microhei.ttc",
                "wenquanyi/wqy-zenhei/wqy-zenhei.ttc",
                "truetype/wqy/wqy-zenhei.ttc",
                "truetype/droid/DroidSansFallbackFull.ttf",
            ],
            &["noto-cjk/NotoSansCJKjp-Regular.otf", "opentype/noto/NotoSansCJKjp-Regular.otf"],
        )
    };
    if hans { zh.to_vec() } else { ja.iter().chain(zh).copied().collect() }
}

/// The installed CJK font for a Chinese (`hans`) or other interface, if one is found. Reads at
/// most one font file per candidate; never panics on damaged files.
pub fn system_cjk_ui_font(hans: bool) -> Option<SystemUiFont> {
    find_ui_font(&crate::system_font_dirs(), &candidates(hans), probe(hans), hans)
}

/// The first of `names` (tried in order, each in every folder of `dirs`) that is a font file
/// with a face mapping `probe`.
pub(crate) fn find_ui_font(dirs: &[std::path::PathBuf], names: &[&str], probe: char, hans: bool) -> Option<SystemUiFont> {
    names.iter().flat_map(|n| dirs.iter().map(move |d| d.join(n))).find_map(|path| {
        let size = std::fs::metadata(&path).ok().filter(|m| m.is_file())?.len();
        if size > MAX_FILE {
            return None;
        }
        let bytes = std::fs::read(&path).ok()?;
        let (index, family) = pick_face(&bytes, probe, hans)?;
        log::debug!("interface CJK fallback: {family} ({})", path.display());
        Some(SystemUiFont { family, bytes, index })
    })
}

/// The face of a font file (or collection) to use: one that maps `probe`, preferring a Regular
/// style and, for Chinese, a Simplified Chinese family (`… SC`, `… GB`, as in a pan-CJK
/// collection). `None` if the header isn't a font's or no face qualifies.
pub(crate) fn pick_face(data: &[u8], probe: char, hans: bool) -> Option<(u32, String)> {
    let magic = data.get(..4)?;
    if ![&[0, 1, 0, 0][..], b"OTTO", b"ttcf"].contains(&magic) {
        return None;
    }
    let count = match FileRef::new(data).ok()? {
        FileRef::Font(_) => 1,
        FileRef::Collection(c) => c.len().min(MAX_FACES),
    };
    let mut best: Option<((bool, bool), u32, String)> = None;
    for i in 0..count {
        let Ok(f) = skrifa::FontRef::from_index(data, i) else { continue };
        if f.charmap().map(probe).is_none() {
            continue;
        }
        let Some((family, style, _)) = crate::fontdb::face_names(&f) else { continue };
        let regional = !hans || family.ends_with(" SC") || family.ends_with(" GB") || family.contains("YaHei");
        let score = (style.eq_ignore_ascii_case("Regular"), regional);
        if best.as_ref().is_none_or(|(s, ..)| score > *s) {
            best = Some((score, i, family));
        }
    }
    best.map(|(_, i, family)| (i, family))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_system_cjk_only_when_no_embedded_face_covers_the_language() {
        let jp = CraftFont { family: "BIZ UDPGothic", style: "Regular", scripts: &["Jpan", "Latn"], bytes: &[] };
        let sc = CraftFont { family: "Noto Sans CJK SC", style: "Regular", scripts: &["Hans", "Latn"], bytes: &[] };
        // #241: the release embedded only the Japanese faces, and the Chinese interface fell back to them.
        assert!(ui_needs_system_cjk(true, &[&jp]));
        assert!(!ui_needs_system_cjk(true, &[&sc, &jp]));
        assert!(!ui_needs_system_cjk(false, &[&jp]));
        assert!(ui_needs_system_cjk(false, &[]));
        assert!(ui_needs_system_cjk(true, &[]));
    }

    #[test]
    fn finds_the_first_valid_font_covering_the_probe() {
        let dir = std::env::temp_dir().join(format!("wc-uifallback-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let sans = crate::bundled()[0];
        // Missing, damaged and non-font files are skipped without panicking.
        std::fs::write(dir.join("Deng.ttf"), b"ttcf\0\x01\0\0\xff\xff\xff\xff").unwrap();
        std::fs::write(dir.join("msyh.ttc"), b"<html>not a font</html>").unwrap();
        std::fs::write(dir.join("sub/good.ttf"), sans).unwrap();
        let names = ["missing.ttf", "Deng.ttf", "msyh.ttc", "sub/good.ttf"];
        let found = find_ui_font(std::slice::from_ref(&dir), &names, 'A', true).unwrap();
        assert_eq!((found.family.as_str(), found.index, found.bytes.len()), ("Source Sans 3", 0, sans.len()));
        // No candidate maps a CJK character: nothing is chosen.
        assert!(find_ui_font(std::slice::from_ref(&dir), &names, '页', true).is_none());
        assert!(pick_face(b"OTT", 'A', false).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
