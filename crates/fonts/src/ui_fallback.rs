//! An installed CJK font for the interface, after the embedded craft-fonts faces, when they leave
//! characters of the interface text without a glyph: a build without craft-fonts, one whose
//! craft-fonts checkout lacks the Chinese face (#241), or one whose faces lack characters the
//! catalogs use (#486) would otherwise draw Chinese menus with boxes and `?`. Native only; wasm
//! has no system fonts to read.
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

/// Is `c` drawn from a CJK font: Han ideographs, kana, bopomofo, Hangul, CJK punctuation or a
/// full-width form? (Latin, Cyrillic and symbols come from the interface's Latin faces.)
pub fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{2E80}'..='\u{2FDF}' // radicals
        | '\u{3000}'..='\u{33FF}' // punctuation, kana, bopomofo, compatibility
        | '\u{3400}'..='\u{4DBF}' // extension A
        | '\u{4E00}'..='\u{9FFF}' // unified ideographs
        | '\u{AC00}'..='\u{D7AF}' // Hangul syllables
        | '\u{F900}'..='\u{FAFF}' // compatibility ideographs
        | '\u{FE30}'..='\u{FE4F}' // vertical forms
        | '\u{FF00}'..='\u{FFEF}' // full-width forms
        | '\u{20000}'..='\u{3FFFF}' // extensions B and later
    )
}

/// The distinct CJK characters of `texts` (see [`is_cjk`]) that none of the `embedded` faces
/// maps, in code point order. A craft-fonts face may lack characters the interface catalogs use
/// (an older or subset file), and those would draw as boxes or `?` (#486).
pub fn ui_uncovered_cjk(texts: &[&str], embedded: &[&CraftFont]) -> Vec<char> {
    let faces: Vec<_> = embedded.iter().filter_map(|f| skrifa::FontRef::from_index(f.bytes, 0).ok()).map(|f| f.charmap()).collect();
    let chars: std::collections::BTreeSet<char> = texts.iter().flat_map(|t| t.chars()).filter(|c| is_cjk(*c)).collect();
    chars.into_iter().filter(|c| !faces.iter().any(|m| m.map(*c).is_some())).collect()
}

/// Do the embedded interface faces (`ui_cjk_fonts`) leave CJK characters of `texts` (the
/// interface catalogs and language names that can be on screen) without a glyph? Then an
/// installed CJK font goes after them. No embedded face (a build without craft-fonts) leaves
/// every CJK character uncovered; a Japanese face alone lacks the simplified-only hanzi (页, 选,
/// 删 …, #241) and some traditional ones (啟, 內 …, #486).
pub fn ui_needs_system_cjk(texts: &[&str], embedded: &[&CraftFont]) -> bool {
    !ui_uncovered_cjk(texts, embedded).is_empty()
}

/// Files larger than this are skipped (the big system CJK collections are around 20–80 MB).
const MAX_FILE: u64 = 128 << 20;
/// Faces looked at in one collection.
const MAX_FACES: u32 = 64;
/// Entries looked at in one macOS font asset folder (each font is one `<hash>.asset` folder).
const MAX_ASSETS: usize = 8192;
/// Characters a face's coverage is measured on.
const MAX_NEEDED: usize = 4096;
/// The tables skrifa reads glyph outlines from: TrueType, CFF, CFF2 and VARC. A face with
/// outlines only in Apple's `hvgl` table has none of them, and the interface draws its characters
/// as blank space.
const OUTLINE_TABLES: [&[u8; 4]; 4] = [b"glyf", b"CFF ", b"CFF2", b"VARC"];

/// A character only a font for the language has: a simplified-only hanzi, kana, or Hangul.
fn probe(hans: bool, korean: bool) -> char {
    if korean {
        '한'
    } else if hans {
        '页'
    } else {
        'の'
    }
}

/// Font files to look for, relative to a font folder (or absolute), best first.
fn candidates(hans: bool, korean: bool) -> Vec<&'static str> {
    let (zh, ja, ko): (&[&str], &[&str], &[&str]) = if cfg!(windows) {
        (
            &["Deng.ttf", "msyh.ttc", "simhei.ttf", "simsun.ttc", "NotoSansSC-VF.ttf"],
            &["YuGothR.ttc", "meiryo.ttc", "msgothic.ttc"],
            &["malgun.ttf", "malgunsl.ttf", "gulim.ttc", "batang.ttc"],
        )
    } else if cfg!(target_os = "macos") {
        (
            &[
                // An asset on macOS 10.15 and later (see `with_asset_data`).
                "PingFang.ttc",
                "Hiragino Sans GB.ttc",
                "STHeiti Light.ttc",
                "STHeiti Medium.ttc",
                // The system interface's own copy, present when the PingFang asset isn't.
                "/System/Library/PrivateFrameworks/FontServices.framework/Resources/Reserved/PingFangUI.ttc",
            ],
            &["ヒラギノ角ゴシック W3.ttc", "Hiragino Sans W3.ttc"],
            &["/System/Library/Fonts/AppleSDGothicNeo.ttc", "/System/Library/Fonts/Supplemental/AppleGothic.ttf"],
        )
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
            &[
                "noto-cjk/NotoSansCJKkr-Regular.otf",
                "opentype/noto/NotoSansCJKkr-Regular.otf",
                "google-noto-sans-cjk-fonts/NotoSansCJKkr-Regular.otf",
                "adobe-source-han-sans/SourceHanSansK-Regular.otf",
                "opentype/adobe-source-han-sans/SourceHanSansK-Regular.otf",
                "truetype/nanum/NanumGothic.ttf",
            ],
        )
    };
    if korean {
        ko.iter().chain(ja).chain(zh).copied().collect()
    } else if hans {
        zh.to_vec()
    } else {
        ja.iter().chain(zh).copied().collect()
    }
}

/// The installed CJK font for a Chinese (`hans`) or other interface that best covers `needed`
/// (the characters the embedded faces lack; empty: a character typical of the language), if one
/// is found. Never panics on damaged files.
pub fn system_cjk_ui_font(hans: bool, needed: &[char]) -> Option<SystemUiFont> {
    system_cjk_ui_font_for(hans, false, needed)
}

/// The installed CJK font for a Chinese, Korean or Japanese interface.
pub fn system_cjk_ui_font_for(hans: bool, korean: bool, needed: &[char]) -> Option<SystemUiFont> {
    let probe = [probe(hans, korean)];
    let needed = if needed.is_empty() { &probe[..] } else { needed.get(..MAX_NEEDED).unwrap_or(needed) };
    find_ui_font(&with_asset_data(crate::system_font_dirs()), &candidates(hans, korean), needed, hans)
}

/// Is `c` an Arabic-script letter, mark or digit (Arabic, Arabic Supplement, Extended-A/B and the
/// presentation forms)? The interface's Latin faces have none of them.
pub fn is_arabic(c: char) -> bool {
    matches!(c, '\u{0600}'..='\u{06FF}' | '\u{0750}'..='\u{077F}' | '\u{0870}'..='\u{08FF}' | '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}')
}

/// Installed fonts with Arabic letters, by platform, in preference order (interface sans faces
/// first). Paths as in [`candidates`]: a bare name is looked for in every system font folder.
fn arabic_candidates() -> &'static [&'static str] {
    if cfg!(windows) {
        &["segoeui.ttf", "tahoma.ttf", "arial.ttf"]
    } else if cfg!(target_os = "macos") {
        &[
            "/System/Library/Fonts/SFArabic.ttf",
            "/System/Library/Fonts/GeezaPro.ttc",
            "/System/Library/Fonts/Supplemental/GeezaPro.ttc",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
        ]
    } else {
        &[
            "truetype/noto/NotoSansArabic-Regular.ttf",
            "noto/NotoSansArabic-Regular.ttf",
            "google-noto/NotoSansArabic-Regular.ttf",
            "google-noto-sans-arabic-fonts/NotoSansArabic-Regular.ttf",
            "truetype/noto/NotoSansArabicUI-Regular.ttf",
            "truetype/noto/NotoNaskhArabic-Regular.ttf",
            "truetype/dejavu/DejaVuSans.ttf",
            "dejavu/DejaVuSans.ttf",
            "dejavu-sans-fonts/DejaVuSans.ttf",
        ]
    }
}

/// The installed font with Arabic letters that best covers `needed` (empty: a typical Arabic
/// letter), for the Arabic interface catalog and the language list's «العربية» when the build has
/// no craft-fonts Arabic face. Never panics on damaged files.
pub fn system_arabic_ui_font(needed: &[char]) -> Option<SystemUiFont> {
    let probe = ['ب'];
    let needed = if needed.is_empty() { &probe[..] } else { needed.get(..MAX_NEEDED).unwrap_or(needed) };
    find_ui_font(&with_asset_data(crate::system_font_dirs()), arabic_candidates(), needed, false)
}

/// `dirs` and, for each macOS font asset folder among them (`com_apple_MobileAsset_Font<N>`),
/// the `AssetData` folder of every `<hash>.asset` in it: macOS ships PingFang there, e.g.
/// `/System/Library/AssetsV2/com_apple_MobileAsset_Font7/<hash>.asset/AssetData/PingFang.ttc`
/// (#486). One level, at most [`MAX_ASSETS`] entries per folder.
pub(crate) fn with_asset_data(dirs: Vec<std::path::PathBuf>) -> Vec<std::path::PathBuf> {
    let mut out = Vec::with_capacity(dirs.len());
    for d in dirs {
        let assets = d.file_name().is_some_and(|n| n.to_string_lossy().starts_with("com_apple_MobileAsset_Font"));
        if assets && let Ok(entries) = std::fs::read_dir(&d) {
            let mut found: Vec<_> =
                entries.flatten().take(MAX_ASSETS).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "asset")).collect();
            found.sort();
            out.extend(found.into_iter().map(|p| p.join("AssetData")));
        }
        out.push(d);
    }
    out
}

/// The font file among `names` (tried in order, each in every folder of `dirs`) whose best face
/// maps all of `needed`, or failing that the one mapping the most of them (the earliest on a
/// tie). A file that maps none of them is never chosen.
pub(crate) fn find_ui_font(dirs: &[std::path::PathBuf], names: &[&str], needed: &[char], hans: bool) -> Option<SystemUiFont> {
    let mut tried = std::collections::HashSet::new();
    let mut best: Option<(usize, SystemUiFont)> = None;
    for path in names.iter().flat_map(|n| dirs.iter().map(move |d| d.join(n))) {
        if !tried.insert(path.clone()) {
            continue;
        }
        let Some(size) = std::fs::metadata(&path).ok().filter(|m| m.is_file()).map(|m| m.len()) else { continue };
        if size > MAX_FILE {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Some((index, family, covered)) = pick_face(&bytes, needed, hans) else { continue };
        if best.as_ref().is_some_and(|(b, _)| *b >= covered) {
            continue;
        }
        log::debug!("interface CJK fallback: {family} ({}), {covered} of {} characters", path.display(), needed.len());
        best = Some((covered, SystemUiFont { family, bytes, index }));
        if covered >= needed.len() {
            break;
        }
    }
    best.map(|(_, f)| f)
}

/// The face of a font file (or collection) to use and how many of `needed` it maps: the one
/// mapping the most, then preferring a Regular style and, for Chinese, a Simplified Chinese
/// family (`… SC`, `… GB`, as in a pan-CJK collection). Faces with none of the
/// [`OUTLINE_TABLES`] are skipped. `None` if the header isn't a font's or no face with outlines
/// maps any of `needed`.
pub(crate) fn pick_face(data: &[u8], needed: &[char], hans: bool) -> Option<(u32, String, usize)> {
    let magic = data.get(..4)?;
    if ![&[0, 1, 0, 0][..], b"OTTO", b"ttcf"].contains(&magic) {
        return None;
    }
    let count = match FileRef::new(data).ok()? {
        FileRef::Font(_) => 1,
        FileRef::Collection(c) => c.len().min(MAX_FACES),
    };
    let mut best: Option<((usize, bool, bool), u32, String)> = None;
    for i in 0..count {
        let Ok(f) = skrifa::FontRef::from_index(data, i) else { continue };
        if !OUTLINE_TABLES.iter().any(|t| f.table_data(skrifa::Tag::new(t)).is_some()) {
            continue;
        }
        let charmap = f.charmap();
        let covered = needed.iter().filter(|c| charmap.map(**c).is_some()).count();
        if covered == 0 {
            continue;
        }
        let Some((family, style, _)) = crate::fontdb::face_names(&f) else { continue };
        let regional = !hans || family.ends_with(" SC") || family.ends_with(" GB") || family.contains("YaHei");
        let score = (covered, style.eq_ignore_ascii_case("Regular"), regional);
        if best.as_ref().is_none_or(|(s, ..)| score > *s) {
            best = Some((score, i, family));
        }
    }
    best.map(|((covered, ..), i, family)| (i, family, covered))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn korean_interface_uses_korean_first_system_font_candidates() {
        let names = candidates(false, true);
        assert_eq!(
            names.first(),
            Some(&if cfg!(windows) {
                "malgun.ttf"
            } else if cfg!(target_os = "macos") {
                "/System/Library/Fonts/AppleSDGothicNeo.ttc"
            } else {
                "noto-cjk/NotoSansCJKkr-Regular.otf"
            })
        );
        assert_eq!(probe(false, true), '한');
    }

    #[test]
    fn needs_system_cjk_when_an_embedded_face_lacks_an_interface_character() {
        // A face without CJK glyphs stands in for a craft-fonts face that lacks some (#486).
        let latin = CraftFont { family: "Source Sans 3", style: "Regular", scripts: &["Hans"], bytes: crate::bundled()[0] };
        assert_eq!(ui_uncovered_cjk(&["选项 Options", "選項：选项"], &[&latin]), ['选', '選', '項', '项', '：']);
        assert!(ui_needs_system_cjk(&["简体中文"], &[]));
        assert!(ui_needs_system_cjk(&["Save", "开"], &[&latin]));
        // Latin, Cyrillic and symbols are the Latin faces' business.
        assert!(!ui_needs_system_cjk(&["Українська Español → ¶"], &[]));
        // A damaged embedded face counts as covering nothing; it never panics.
        let broken = CraftFont { family: "x", style: "Regular", scripts: &["Hans"], bytes: b"OTTO" };
        assert!(ui_needs_system_cjk(&["页"], &[&broken]));
        // The real faces, when built with craft-fonts: the Chinese face covers simplified and
        // traditional hanzi the Japanese one lacks (#241, #486).
        let embedded = crate::ui_cjk_fonts(true);
        if crate::chinese_fonts().next().is_some() {
            assert!(!ui_needs_system_cjk(&["页选项另存为啟內"], &embedded));
        }
    }

    #[test]
    fn finds_the_best_covering_font_also_in_macos_asset_folders() {
        let dir = std::env::temp_dir().join(format!("wc-uifallback-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let assets = dir.join("com_apple_MobileAsset_Font7");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::create_dir_all(assets.join("0a1b.asset/AssetData")).unwrap();
        std::fs::create_dir_all(assets.join("not-an-asset/AssetData")).unwrap();
        let sans = crate::bundled()[0];
        // Missing, damaged and non-font files are skipped without panicking.
        std::fs::write(dir.join("Deng.ttf"), b"ttcf\0\x01\0\0\xff\xff\xff\xff").unwrap();
        std::fs::write(dir.join("msyh.ttc"), b"<html>not a font</html>").unwrap();
        std::fs::write(dir.join("sub/good.ttf"), sans).unwrap();
        let names = ["missing.ttf", "Deng.ttf", "msyh.ttc", "sub/good.ttf"];
        let found = find_ui_font(std::slice::from_ref(&dir), &names, &['A'], true).unwrap();
        assert_eq!((found.family.as_str(), found.index, found.bytes.len()), ("Source Sans 3", 0, sans.len()));
        // No candidate maps a CJK character: nothing is chosen.
        assert!(find_ui_font(std::slice::from_ref(&dir), &names, &['页'], true).is_none());
        assert!(pick_face(b"OTT", &['A'], false).is_none());
        // macOS 12 keeps PingFang in `AssetsV2/com_apple_MobileAsset_Font7/<hash>.asset/AssetData`.
        std::fs::write(assets.join("0a1b.asset/AssetData/PingFang.ttc"), sans).unwrap();
        std::fs::write(assets.join("not-an-asset/AssetData/Hidden.ttc"), sans).unwrap();
        let dirs = with_asset_data(vec![dir.clone(), assets.clone()]);
        assert_eq!(dirs, [dir.clone(), assets.join("0a1b.asset/AssetData"), assets.clone()]);
        let found = find_ui_font(&dirs, &["PingFang.ttc"], &['A'], true).unwrap();
        assert_eq!(found.family, "Source Sans 3");
        assert!(find_ui_font(&dirs, &["Hidden.ttc"], &['A'], true).is_none());
        // No file maps every needed character: the one mapping the most is used.
        let found = find_ui_font(&dirs, &["sub/good.ttf", "PingFang.ttc"], &['A', '页'], true).unwrap();
        assert_eq!(found.family, "Source Sans 3");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn skips_faces_without_outlines() {
        // Renaming the `glyf` table to `hvgl` gives a face like those of `PingFangUI.ttc` on
        // macOS 27.0.1. Its cmap maps characters, and skrifa reads no outlines from it.
        let sans = crate::bundled()[0];
        let mut hvgl = sans.to_vec();
        let tables = u16::from_be_bytes([hvgl[4], hvgl[5]]) as usize;
        let glyf = (0..tables).map(|r| 12 + r * 16).find(|&at| &hvgl[at..at + 4] == b"glyf").unwrap();
        hvgl[glyf..glyf + 4].copy_from_slice(b"hvgl");
        assert!(skrifa::FontRef::new(&hvgl).unwrap().charmap().map('A').is_some());
        assert!(pick_face(&hvgl, &['A'], true).is_none());
        assert_eq!(pick_face(sans, &['A'], true), Some((0, "Source Sans 3".to_string(), 1)));
        // `find_ui_font` uses the next file, and returns `None` when no other file is listed.
        let dir = std::env::temp_dir().join(format!("wc-uifallback-outlines-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("hvgl.ttf"), &hvgl).unwrap();
        std::fs::write(dir.join("glyf.ttf"), sans).unwrap();
        let found = find_ui_font(std::slice::from_ref(&dir), &["hvgl.ttf", "glyf.ttf"], &['A'], true).unwrap();
        assert!(found.bytes == sans);
        assert!(find_ui_font(std::slice::from_ref(&dir), &["hvgl.ttf"], &['A'], true).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
