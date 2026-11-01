#[cfg(not(target_arch = "wasm32"))]
use super::face::HINT_STRIKE;
use super::face::{BUNDLED, FALLBACK_FAMILY, FontBytes, FontFace, NEXT_ID};
use super::{Arc, FileRef, Location, MetadataProvider, Ordering, Size, StringId};

/// Is `f` an East Asian (Chinese, Japanese or Korean) font: do its OS/2 code pages say so
/// (Shift-JIS, GB2312, Wansung, Big5, Johab), or, without them, does it map common ideographs?
pub(super) fn east_asian_font(f: &skrifa::FontRef<'_>) -> bool {
    use skrifa::raw::TableProvider;
    const CJK_CODE_PAGES: u32 = 0b1_1111 << 17;
    match f.os2().ok().and_then(|t| t.ul_code_page_range_1()) {
        Some(pages) if pages != 0 => pages & CJK_CODE_PAGES != 0,
        _ => ['中', '国', '日'].iter().all(|c| f.charmap().map(*c).is_some()),
    }
}

/// A family name in another language, from a font's `name` table.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalName {
    /// BCP 47 language tag, lowercase (`zh-hans`, `zh-tw`, `ja-jp`).
    pub lang: String,
    /// The name in that language (宋体).
    pub name: String,
    /// The English family name it stands for (SimSun): the face's typographic or legacy family.
    pub english: String,
}

/// Does a `name` table language tag (`zh-Hans`, `zh-TW`, `ja-JP`) suit the interface language
/// `ui` (`zh-hans`, `zh-hant`, `ja`)? Chinese matches by script: Simplified for mainland China and
/// Singapore, Traditional for Taiwan, Hong Kong and Macao.
pub fn name_language_matches(ui: &str, tag: &str) -> bool {
    let parts = |s: &str| s.split(['-', '_']).map(str::to_ascii_lowercase).collect::<Vec<_>>();
    let (u, t) = (parts(ui), parts(tag));
    let (Some(up), Some(tp)) = (u.first(), t.first()) else { return false };
    if up.is_empty() || up != tp {
        return false;
    }
    if up != "zh" {
        return true;
    }
    let traditional = |p: &[String]| p.iter().any(|x| matches!(x.as_str(), "hant" | "tw" | "hk" | "mo"));
    let simplified = |p: &[String]| p.iter().any(|x| matches!(x.as_str(), "hans" | "cn" | "sg"));
    // Plain `zh` is Simplified Chinese, as in the interface languages.
    if traditional(&u) { traditional(&t) } else { simplified(&t) }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
pub(super) struct CatalogEntry {
    pub(super) family: String,
    pub(super) style: String,
    pub(super) legacy: String,
    pub(super) full_name: Option<String>,
    pub(super) postscript: Option<String>,
    pub(super) local: Vec<LocalName>,
    /// A Chinese, Japanese or Korean font ([`FontFace::is_east_asian`]).
    pub(super) east_asian: bool,
    pub(super) path: std::path::PathBuf,
}

/// The longest name a document's font table gives an embedded font that faces take on.
pub(super) const MAX_FAMILY_NAME: usize = 128;

pub(super) fn name(font: &skrifa::FontRef<'_>, ids: &[StringId]) -> Option<String> {
    ids.iter().find_map(|id| font.localized_strings(*id).english_or_first().map(|s| s.to_string()).filter(|s| !s.is_empty()))
}

/// The family's names in other languages: localized typographic family names (name ID 16) stand
/// for `family`, localized legacy family names (name ID 1) for `id1` (or for `family` when the
/// font has no typographic names). Bounded: a damaged font can list any number of records.
fn local_names(f: &skrifa::FontRef<'_>, family: &str, id1: Option<&str>) -> Vec<LocalName> {
    const MAX_NAMES: usize = 64;
    const MAX_CHARS: usize = 128;
    let typographic = f.localized_strings(StringId::TYPOGRAPHIC_FAMILY_NAME).next().is_some();
    let legacy = if typographic { id1 } else { Some(family) };
    let mut out: Vec<LocalName> = Vec::new();
    for (id, english) in [(StringId::TYPOGRAPHIC_FAMILY_NAME, Some(family)), (StringId::FAMILY_NAME, legacy)] {
        let Some(english) = english else { continue };
        for s in f.localized_strings(id) {
            let Some(lang) = s.language().map(str::to_ascii_lowercase) else { continue };
            if lang == "en" || lang.starts_with("en-") {
                continue;
            }
            let name: String = s.chars().take(MAX_CHARS).collect::<String>().trim().to_string();
            if name.is_empty() || name.eq_ignore_ascii_case(english) || out.iter().any(|n| n.lang == lang && n.name == name) {
                continue;
            }
            if out.len() >= MAX_NAMES {
                return out;
            }
            out.push(LocalName { lang, name, english: english.to_string() });
        }
    }
    out
}

/// A face's family and style names (the default instance's style for a variable font), and its
/// legacy family name (name ID 1) when it has one.
pub(crate) fn face_names(f: &skrifa::FontRef<'_>) -> Option<(String, String, Option<String>)> {
    let family = name(f, &[StringId::TYPOGRAPHIC_FAMILY_NAME, StringId::FAMILY_NAME])?;
    let style = name(f, &[StringId::TYPOGRAPHIC_SUBFAMILY_NAME, StringId::SUBFAMILY_NAME]).unwrap_or_else(|| "Regular".into());
    Some((family, style, name(f, &[StringId::FAMILY_NAME])))
}

/// The family name Word uses for a face that has no legacy name (name ID 1) of its own, such as a
/// variable font's named instance: the family plus the style's words other than Regular, Bold and
/// Italic, which stay styles within it (OpenType `name` table, IDs 1/2 vs 16/17). "Roboto" +
/// "Light Italic" → "Roboto Light"; "Roboto" + "Bold Italic" → "Roboto".
pub(crate) fn legacy_family(family: &str, style: &str) -> String {
    const RIBBI: &[&str] = &["regular", "normal", "bold", "italic", "oblique"];
    let extra: Vec<&str> = base_style(style).split_whitespace().filter(|w| !RIBBI.iter().any(|r| w.eq_ignore_ascii_case(r))).collect();
    if extra.is_empty() { family.to_string() } else { format!("{family} {}", extra.join(" ")) }
}

/// The color bitmap tables the renderer draws glyphs from ([`FontFace::color_glyph`]).
#[cfg(not(target_arch = "wasm32"))]
const COLOR_BITMAP_TABLES: [&[u8; 4]; 2] = [b"sbix", b"CBDT"];

/// The names of every face in the font file at `path`, reading only its table directories and
/// `name` and `fvar` tables: a scan opens hundreds of font files, many of them megabytes long
/// (CJK collections run to 20 MB and more). A variable font yields one entry per named instance,
/// as it does loaded. Faces with none of the [`OUTLINE_TABLES`](crate::ui_fallback::OUTLINE_TABLES)
/// and [`COLOR_BITMAP_TABLES`] are left out.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn file_face_names(path: &std::path::Path) -> Vec<Found> {
    use crate::ui_fallback::OUTLINE_TABLES;
    use std::io::{Read, Seek, SeekFrom};
    /// Caps on what a (possibly damaged) file can make the scan read.
    const MAX_FACES: u32 = 256;
    const MAX_TABLE: u32 = 1 << 20;
    let Ok(mut file) = std::fs::File::open(path) else { return vec![] };
    let mut read_at = |offset: u64, len: usize| -> Option<Vec<u8>> {
        let mut buf = vec![0; len];
        file.seek(SeekFrom::Start(offset)).ok()?;
        file.read_exact(&mut buf).ok()?;
        Some(buf)
    };
    let be32 = |b: &[u8], at: usize| b.get(at..at + 4).and_then(|s| s.try_into().ok()).map(u32::from_be_bytes);
    let Some(head) = read_at(0, 12) else { return vec![] };
    // A collection lists where each face's table directory starts.
    let starts: Vec<u32> = if head.starts_with(b"ttcf") {
        let n = be32(&head, 8).unwrap_or(0).min(MAX_FACES) as usize;
        read_at(12, n * 4).map(|b| b.as_chunks::<4>().0.iter().map(|c| u32::from_be_bytes(*c)).collect()).unwrap_or_default()
    } else {
        vec![0]
    };
    starts
        .into_iter()
        .filter_map(|start| {
            let dir = read_at(start.into(), 12)?;
            let tables = u16::from_be_bytes(dir.get(4..6)?.try_into().ok()?) as usize;
            let records = read_at(u64::from(start) + 12, tables * 16)?;
            if !records.as_chunks::<16>().0.iter().any(|r| OUTLINE_TABLES.iter().chain(&COLOR_BITMAP_TABLES).any(|t| r.starts_with(*t))) {
                return None;
            }
            let mut table = |tag: &[u8; 4]| -> Option<Vec<u8>> {
                let rec = records.as_chunks::<16>().0.iter().find(|r| r.starts_with(tag))?;
                let (offset, len) = (be32(rec, 8)?, be32(rec, 12)?);
                if len > MAX_TABLE {
                    return None;
                }
                read_at(offset.into(), len as usize)
            };
            let name = table(b"name")?;
            // Variable fonts: the named instances; OS/2: the code pages (East Asian fonts).
            let mut tables: Vec<([u8; 4], Vec<u8>)> = table(b"OS/2").map(|t| (*b"OS/2", t)).into_iter().collect();
            tables.extend(table(b"fvar").map(|t| (*b"fvar", t)));
            tables.push((*b"name", name));
            // A font holding just those tables, to read them as the font itself would.
            let font = mini_font(&tables);
            Some(font_faces(&skrifa::FontRef::new(&font).ok()?, 0))
        })
        .flatten()
        .collect()
}

/// A font file holding just `tables` (sorted by tag), each padded to four bytes.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn mini_font(tables: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut font = Vec::new();
    font.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
    // numTables, then searchRange, entrySelector and rangeShift (which readers don't need).
    font.extend_from_slice(&u16::try_from(tables.len()).unwrap_or(0).to_be_bytes());
    font.extend_from_slice(&[0; 6]);
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in tables {
        font.extend_from_slice(tag);
        font.extend_from_slice(&[0; 4]);
        for v in [offset, data.len()] {
            font.extend_from_slice(&u32::try_from(v).unwrap_or(0).to_be_bytes());
        }
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in tables {
        font.extend_from_slice(data);
        font.resize(font.len().next_multiple_of(4), 0);
    }
    font
}

/// The platform's font folders (the system's and the user's), scanned by [`FontDb::global`].
/// Empty on wasm.
pub fn system_font_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    if cfg!(target_arch = "wasm32") {
        return dirs;
    }
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    if cfg!(target_os = "macos") {
        dirs.extend(["/System/Library/Fonts", "/Library/Fonts"].map(Into::into));
        if let Some(h) = &home {
            dirs.push(h.join("Library/Fonts"));
        }
        // Fonts macOS ships as assets (PingFang, Yu Mincho, …): com_apple_MobileAsset_Font<N>
        // (`Assets` before macOS 11, `AssetsV2` since).
        for parent in ["/System/Library/AssetsV2", "/System/Library/AssetsV2/PreinstalledAssetsV2/InstallWithOs", "/System/Library/Assets"] {
            let Ok(entries) = std::fs::read_dir(parent) else { continue };
            for e in entries.flatten() {
                if e.file_name().to_string_lossy().starts_with("com_apple_MobileAsset_Font") {
                    dirs.push(e.path());
                }
            }
        }
    } else if cfg!(windows) {
        let root = std::env::var_os("WINDIR").map(std::path::PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
        dirs.push(root.join("Fonts"));
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(std::path::PathBuf::from(l).join("Microsoft\\Windows\\Fonts"));
        }
    } else {
        dirs.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(Into::into));
        if let Some(h) = &home {
            dirs.push(h.join(".fonts"));
            dirs.push(h.join(".local/share/fonts"));
        }
        // Flatpak (#1937): the folders above are the runtime's, and the host's system, local and
        // user fonts are mounted here instead (no filesystem permission needed). Flatpak after
        // 1.19.2 also mounts ~/.fonts, at user-fonts-legacy when the XDG user font folder exists too.
        dirs.extend(["/run/host/fonts", "/run/host/local-fonts", "/run/host/user-fonts", "/run/host/user-fonts-legacy"].map(Into::into));
    }
    dirs
}

/// One face found in a font file.
pub(super) struct Found {
    /// Face index within the file.
    pub(super) index: u32,
    pub(super) family: String,
    pub(super) style: String,
    /// The legacy family name ([`FontFace::legacy_family`]).
    pub(super) legacy: String,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(super) full_name: Option<String>,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(super) postscript: Option<String>,
    /// The family's names in other languages ([`FontFace::local_names`]).
    pub(super) local: Vec<LocalName>,
    /// A Chinese, Japanese or Korean font ([`FontFace::is_east_asian`]); the scan's catalog keeps it.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(super) east_asian: bool,
    /// Variable fonts: the named instance's axis settings.
    pub(super) coords: Vec<([u8; 4], f32)>,
}

/// The faces of `f` (face `index` of its file): one per named instance of a variable font
/// (InDesign lists them as styles), otherwise the font itself.
pub(super) fn font_faces(f: &skrifa::FontRef<'_>, index: u32) -> Vec<Found> {
    let Some((family, style, id1)) = face_names(f) else { return vec![] };
    let local = local_names(f, &family, id1.as_deref());
    let east_asian = east_asian_font(f);
    let axes = f.axes();
    let mut named: Vec<Found> = Vec::new();
    // At most one more than a file may add, so a caller can tell there are more (#3716).
    for ni in f.named_instances().iter().take(MAX_FILE_FACES + 1) {
        let Some(style) = name(f, &[ni.subfamily_name_id()]) else { continue };
        if named.iter().any(|n| n.style.eq_ignore_ascii_case(&style)) {
            continue;
        }
        let coords: Vec<([u8; 4], f32)> = axes.iter().zip(ni.user_coords()).map(|(a, v)| (a.tag().to_be_bytes(), v)).collect();
        // Name ID 1 names the default instance only.
        let legacy = legacy_family(&family, &style);
        // Name ID 1 names the default instance only: keep the names of the whole family.
        let local = local.iter().filter(|n| n.english.eq_ignore_ascii_case(&family)).cloned().collect();
        named.push(Found { index, family: family.clone(), style, legacy, full_name: None, postscript: None, local, east_asian, coords });
    }
    if !named.is_empty() {
        return named;
    }
    let legacy = id1.filter(|n| !n.eq_ignore_ascii_case(&family)).unwrap_or_else(|| legacy_family(&family, &style));
    vec![Found {
        index,
        family,
        style,
        legacy,
        full_name: name(f, &[StringId::FULL_NAME]),
        postscript: name(f, &[StringId::POSTSCRIPT_NAME]),
        local,
        east_asian,
        coords: Vec::new(),
    }]
}

/// Most faces one font file adds (collection members, variable fonts' named instances).
pub(super) const MAX_FILE_FACES: usize = 256;

/// Parse the faces in `data` (a font file or collection), at most `limit` of them; a variable
/// font yields one face per named instance. A face with the family and style of an earlier one
/// is left out (a collection can list one font many times, #3716).
pub(super) fn enumerate_faces(data: &[u8], limit: usize) -> Vec<Found> {
    let count = match FileRef::new(data) {
        Ok(FileRef::Font(_)) => 1,
        Ok(FileRef::Collection(c)) => c.len().min(u32::try_from(limit).unwrap_or(u32::MAX)),
        Err(_) => 0,
    };
    let mut seen = std::collections::HashSet::new();
    (0..count)
        .filter_map(|i| skrifa::FontRef::from_index(data, i).ok().map(|f| font_faces(&f, i)))
        .flatten()
        .filter(|f| seen.insert((f.family.to_ascii_lowercase(), f.style.to_ascii_lowercase())))
        .take(limit)
        .collect()
}

/// The compiled-in Source Sans 3 Regular, for a database that somehow has no faces at all.
pub(crate) fn last_resort_face() -> Arc<FontFace> {
    static FACE: std::sync::OnceLock<Arc<FontFace>> = std::sync::OnceLock::new();
    FACE.get_or_init(|| {
        // The font is compiled in (`include_bytes!`), so parsing it can't depend on input; the
        // `last_resort_face_parses` test proves it on every run.
        #[allow(clippy::expect_used)]
        let face = make_face(FontBytes::Static(BUNDLED[0]), 0, FALLBACK_FAMILY.into(), "Regular".into(), FALLBACK_FAMILY.into(), Vec::new())
            .expect("the compiled-in Source Sans 3 Regular parses");
        Arc::new(face)
    })
    .clone()
}

pub(super) fn make_face(
    bytes: FontBytes,
    index: u32,
    family: String,
    style: String,
    legacy_family: String,
    coords: Vec<([u8; 4], f32)>,
) -> Option<FontFace> {
    let data: &[u8] = match &bytes {
        FontBytes::Static(b) => b,
        FontBytes::Owned(v) => v.as_slice(),
    };
    let f = skrifa::FontRef::from_index(data, index).ok()?;
    let settings: Vec<(skrifa::Tag, f32)> = coords.iter().map(|(t, v)| (skrifa::Tag::new(t), *v)).collect();
    let location = if coords.is_empty() { Location::default() } else { f.axes().location(settings.iter().copied()) };
    let m = f.metrics(Size::unscaled(), &location);
    let a = f.attributes();
    let hb = harfrust::FontRef::from_index(data, index).ok()?;
    let shaper = harfrust::ShaperData::new(&hb);
    let instance = (!coords.is_empty())
        .then(|| harfrust::ShaperInstance::from_variations(&hb, settings.iter().map(|(t, v)| harfrust::Variation { tag: *t, value: *v })));
    let axis = |tag: &[u8; 4]| coords.iter().find(|(t, _)| t == tag).map(|(_, v)| *v);
    // Old releases of heavy faces (Arial Black in the core fonts) claim weight 400 where current
    // ones say 900, which Word doesn't embolden: a family named for its weight is that heavy.
    let weight = axis(b"wght").unwrap_or(if coords.is_empty() { static_weight(a.weight.value(), &legacy_family) } else { style_weight(&style) });
    let italic = axis(b"ital")
        .map(|v| v >= 0.5)
        .or(axis(b"slnt").map(|v| v.abs() > 0.1))
        .unwrap_or(!matches!(a.style, skrifa::attribute::Style::Normal) || style_italic(&style));
    Some(FontFace {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        family,
        style,
        legacy_family,
        local_names: Vec::new(),
        full_name: coords.is_empty().then(|| name(&f, &[StringId::FULL_NAME])).flatten(),
        postscript: coords.is_empty().then(|| name(&f, &[StringId::POSTSCRIPT_NAME])).flatten(),
        weight,
        italic,
        upem: m.units_per_em.max(1) as f64,
        ascent: m.ascent as f64,
        descent: -(m.descent as f64),
        cap_height: m.cap_height.map(|v| v as f64).filter(|v| *v > 0.0).unwrap_or(m.ascent as f64 * 0.72),
        x_height: m.x_height.map(|v| v as f64).filter(|v| *v > 0.0).unwrap_or(m.ascent as f64 * 0.5),
        shaper,
        coords,
        location,
        instance,
        bytes,
        index,
        bmp: std::sync::OnceLock::new(),
        outline_hinter: std::sync::OnceLock::new(),
        hint_draws: Default::default(),
        hint_slow: Default::default(),
        hinting_off: Default::default(),
        #[cfg(not(target_arch = "wasm32"))]
        hint_strike: HINT_STRIKE,
    })
}

/// The named style part of a style with axis settings (`Bold {wght:650}` → `Bold`).
pub fn base_style(style: &str) -> &str {
    style.split('{').next().unwrap_or(style).trim()
}

pub(super) fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Weight implied by a style name.
pub(super) fn style_weight(style: &str) -> f32 {
    let s = norm(style);
    const TABLE: &[(&str, f32)] = &[
        ("extralight", 200.0),
        ("ultralight", 200.0),
        ("semibold", 600.0),
        ("demibold", 600.0),
        ("extrabold", 800.0),
        ("ultrabold", 800.0),
        ("hairline", 100.0),
        ("thin", 100.0),
        ("light", 300.0),
        ("medium", 500.0),
        ("bold", 700.0),
        ("black", 900.0),
        ("heavy", 900.0),
    ];
    TABLE.iter().find(|(k, _)| s.contains(k)).map(|(_, w)| *w).unwrap_or(400.0)
}

/// A static face's weight: the file's, or the weight the last word of its family name stands
/// for (`Arial Black`, `Franklin Gothic Heavy`) when the file says less than semibold.
fn static_weight(file: f32, family: &str) -> f32 {
    if file >= 600.0 {
        return file;
    }
    match family.split_whitespace().next_back().unwrap_or("").to_ascii_lowercase().as_str() {
        "black" | "heavy" => 900.0,
        "extrabold" | "ultrabold" => 800.0,
        _ => file,
    }
}

pub(super) fn style_italic(style: &str) -> bool {
    let s = norm(style);
    s.contains("italic") || s.contains("oblique") || s == "it"
}
