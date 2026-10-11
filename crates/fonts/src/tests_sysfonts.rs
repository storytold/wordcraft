//! Installed system fonts: found by family name by every lookup, whatever ran before it (#22).

use std::path::{Path, PathBuf};

use super::*;

const FAMILY: &str = "Sysfont Sans3";

/// A bundled Source Sans 3 file renamed [`FAMILY`] (as long as the original name).
fn renamed(file: &str) -> Vec<u8> {
    let mut data = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts").join(file)).unwrap();
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    for (from, to) in [(utf16("Source Sans 3"), utf16(FAMILY)), (b"Source Sans 3".to_vec(), FAMILY.as_bytes().to_vec())] {
        let mut i = 0;
        while let Some(at) = data[i..].windows(from.len()).position(|w| w == from) {
            data[i + at..i + at + to.len()].copy_from_slice(&to);
            i += at + to.len();
        }
    }
    data
}

/// The fonts in `faces` as one collection (TTC) file.
fn collection(faces: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"ttcf".to_vec();
    out.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
    out.extend_from_slice(&(faces.len() as u32).to_be_bytes());
    let mut base = 12 + 4 * faces.len();
    for f in faces {
        out.extend_from_slice(&(base as u32).to_be_bytes());
        base += f.len();
    }
    for f in faces {
        // Table offsets count from the start of the collection.
        let start = out.len() as u32;
        let mut f = f.clone();
        let tables = u16::from_be_bytes([f[4], f[5]]) as usize;
        for r in 0..tables {
            let at = 12 + r * 16 + 8;
            let off = u32::from_be_bytes(f[at..at + 4].try_into().unwrap()) + start;
            f[at..at + 4].copy_from_slice(&off.to_be_bytes());
        }
        out.extend_from_slice(&f);
    }
    out
}

/// A font folder (with a subfolder, a damaged font and a file that isn't a font) holding
/// [`FAMILY`] Regular and Bold.
fn font_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dc-sysfonts-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Sub")).unwrap();
    std::fs::write(dir.join("Sysfont-Regular.ttf"), renamed("SourceSans3-Regular.ttf")).unwrap();
    std::fs::write(dir.join("Sub/Sysfont-Bold.TTF"), renamed("SourceSans3-Bold.ttf")).unwrap();
    std::fs::write(dir.join("damaged.ttf"), b"ttcf\0\x01\0\0\xff\xff\xff\xff").unwrap();
    std::fs::write(dir.join("readme.txt"), FAMILY).unwrap();
    dir
}

/// (family, style, legacy family) of each face the scan finds in the file at `path`.
fn names(path: &Path) -> Vec<(String, String, String)> {
    file_face_names(path).into_iter().map(|f| (f.family, f.style, f.legacy)).collect()
}

fn has(list: &[String], family: &str) -> bool {
    list.iter().any(|f| f == family)
}

#[test]
fn every_lookup_by_name_finds_installed_fonts_in_a_fresh_database() {
    let dir = font_dir("fresh");
    // Each lookup is the first thing a new database is asked. A folder listed twice is read once.
    let db = || FontDb::with_font_dirs(vec![dir.clone(), dir.join("Sub/..")]);
    let f = db().face("sysfont sans3", "Bold");
    assert_eq!((f.family.as_str(), f.style.as_str()), (FAMILY, "Bold"));
    assert!(db().has_family(FAMILY));
    assert!(has(&db().families(), FAMILY));
    assert_eq!(db().styles(FAMILY), ["Regular", "Bold"]);
    assert_eq!(db().load_system_fonts(), 2);
    // Missing fonts stay missing.
    let db = db();
    assert!(!db.has_family("No Such Font"));
    assert_eq!(db.face("No Such Font", "Regular").family, FALLBACK_FAMILY);
}

#[test]
fn the_scan_reads_collections_and_loads_fonts_only_when_used() {
    let dir = font_dir("collection");
    std::fs::remove_file(dir.join("Sysfont-Regular.ttf")).unwrap();
    std::fs::remove_file(dir.join("Sub/Sysfont-Bold.TTF")).unwrap();
    std::fs::write(dir.join("Sysfont.ttc"), collection(&[renamed("SourceSans3-Regular.ttf"), renamed("SourceSans3-Bold.ttf")])).unwrap();
    let db = FontDb::with_font_dirs(vec![dir.clone()]);
    assert_eq!(db.styles(FAMILY), ["Regular", "Bold"]);
    // Cataloged, not loaded.
    assert!(!db.is_loaded(FAMILY));
    let bold = db.face(FAMILY, "Bold");
    assert_eq!((bold.style.as_str(), bold.index()), ("Bold", 1));
    assert_eq!(db.face(FAMILY, "Regular").index(), 0, "the collection's faces load together");
}

#[test]
fn concurrent_first_lookups_all_find_the_installed_font() {
    // Threads racing to load the same installed family used to lose: the one whose load added
    // nothing (another thread had just added it) fell back to the default font.
    let dir = font_dir("race");
    for _ in 0..20 {
        let db = FontDb::with_font_dirs(vec![dir.clone()]);
        assert!(db.has_family(FAMILY));
        std::thread::scope(|s| {
            let found: Vec<_> = (0..8).map(|_| s.spawn(|| db.face(FAMILY, "Regular").family.clone())).collect();
            for f in found {
                assert_eq!(f.join().unwrap(), FAMILY);
            }
        });
    }
}

#[test]
fn rescanning_finds_fonts_installed_since() {
    let dir = font_dir("rescan");
    let later = dir.join("Later");
    let db = FontDb::with_font_dirs(vec![later.clone()]);
    assert!(!db.has_family(FAMILY));
    std::fs::create_dir_all(&later).unwrap();
    std::fs::copy(dir.join("Sysfont-Regular.ttf"), later.join("Sysfont-Regular.ttf")).unwrap();
    assert!(!db.has_family(FAMILY), "the folders are scanned once, not on every miss");
    assert_eq!(db.load_system_fonts(), 1);
    assert!(db.has_family(FAMILY) && has(&db.families(), FAMILY));
}

#[test]
fn a_background_scan_serves_the_first_lookup() {
    let dir = font_dir("background");
    let db: &'static FontDb = Box::leak(Box::new(FontDb::with_font_dirs(vec![dir])));
    db.scan_in_background();
    // Waits for the scan when it is still running.
    assert!(has(&db.families(), FAMILY));
    assert_eq!(db.load_system_fonts(), 2, "a rescan catalogs the two faces again");
}

#[cfg(unix)]
#[test]
fn a_link_back_to_a_parent_folder_ends_the_scan() {
    let dir = font_dir("loop");
    std::os::unix::fs::symlink(&dir, dir.join("Sub/loop")).unwrap();
    let db = FontDb::with_font_dirs(vec![dir]);
    assert_eq!(db.load_system_fonts(), 2);
}

#[test]
fn fallback_lookups_are_remembered_until_fonts_load() {
    // Layout asks for a fallback once per character the font lacks; each search sorts every
    // loaded face, which froze Thai text once many fonts were loaded (#59, #121).
    let dir = font_dir("fallbacks");
    let db = FontDb::with_font_dirs(vec![dir]);
    db.set_system_fallback(false);
    let latin = db.face(FALLBACK_FAMILY, "Regular");
    let thai = 'ก';
    assert!(db.fallback_for(thai, latin.id()).is_none(), "no bundled Thai font");
    let a = db.fallback_for('A', latin.id()).unwrap();
    assert_eq!(db.fallback_for('A', latin.id()).unwrap().id(), a.id(), "the same answer from the cache");
    {
        let cache = db.fallbacks.read().unwrap();
        assert_eq!(cache.faces, db.read_faces().len());
        assert!(cache.map.contains_key(&(thai, latin.id())) && cache.map.contains_key(&('A', latin.id())));
    }
    // Loading fonts (here, the installed family) makes the remembered answers stale.
    assert_eq!(db.face(FAMILY, "Regular").family, FAMILY);
    assert_eq!(db.fallback_for('A', latin.id()).unwrap().id(), a.id());
    let cache = db.fallbacks.read().unwrap();
    assert_eq!(cache.faces, db.read_faces().len());
    assert!(!cache.map.contains_key(&(thai, latin.id())), "entries from before the fonts loaded are dropped");
}

#[test]
fn the_scan_reads_names_without_loading_the_font() {
    let dir = font_dir("names");
    assert_eq!(names(&dir.join("Sysfont-Regular.ttf")), [(FAMILY.to_string(), "Regular".to_string(), FAMILY.to_string())]);
    assert!(file_face_names(&dir.join("damaged.ttf")).is_empty());
    assert!(file_face_names(&dir.join("readme.txt")).is_empty());
    assert!(file_face_names(&dir.join("missing.ttf")).is_empty());
}

#[test]
fn installed_weights_are_found_by_their_legacy_family_name() {
    // Semibold is not one of Regular, Bold and Italic: the font names it "Sysfont Sans3 Semibold"
    // (name ID 1), the way Word lists and stores it (#219).
    let dir = font_dir("legacy");
    std::fs::write(dir.join("Sysfont-Semibold.ttf"), renamed("SourceSans3-Semibold.ttf")).unwrap();
    let semibold = format!("{FAMILY} Semibold");
    assert_eq!(names(&dir.join("Sysfont-Semibold.ttf")), [(FAMILY.to_string(), "Semibold".to_string(), semibold.clone())]);
    let db = || FontDb::with_font_dirs(vec![dir.clone()]);
    assert!(has(&db().families(), FAMILY) && has(&db().families(), &semibold));
    assert!(db().has_family(&semibold.to_uppercase()));
    assert_eq!(db().styles(&semibold), ["Semibold"]);
    assert_eq!(db().styles(FAMILY), ["Regular", "Semibold", "Bold"]);
    // The first lookup, by the legacy name, finds the face and loads the whole family with it.
    let db = db();
    let f = db.face(&semibold, "Regular");
    assert_eq!((f.family.as_str(), f.style.as_str(), f.legacy_family.as_str()), (FAMILY, "Semibold", semibold.as_str()));
    assert_eq!(db.face(FAMILY, "Regular").style, "Regular");
    assert_eq!(db.face(FAMILY, "Bold").style, "Bold");
}

#[test]
fn the_scan_lists_a_variable_fonts_named_instances_as_loading_it_does() {
    // Uses an installed variable font when there is one.
    let Some(path) = [
        "/System/Library/Fonts/Supplemental/Skia.ttf",
        "/System/Library/Fonts/NewYork.ttf",
        "/usr/share/fonts/cantarell/Cantarell-VF.otf",
        "/usr/share/fonts/Adwaita/AdwaitaSans-Regular.ttf",
        "C:\\Windows\\Fonts\\bahnschrift.ttf",
    ]
    .iter()
    .map(Path::new)
    .find(|p| p.exists()) else {
        eprintln!("no variable font installed: nothing to check");
        return;
    };
    let data = std::fs::read(path).unwrap();
    let loaded: Vec<(String, String, String)> = enumerate_faces(&data).into_iter().map(|f| (f.family, f.style, f.legacy)).collect();
    assert_eq!(names(path), loaded);
    if loaded.len() > 1 {
        // Instances other than Regular, Bold and Italic have names of their own.
        assert!(loaded.iter().any(|(family, _, legacy)| family != legacy), "{loaded:?}");
    }
}

#[test]
fn fallback_cache_is_dropped_when_a_face_loads_by_its_legacy_family_name() {
    // #121 with #219: a lookup by a legacy family name ("Sysfont Sans3 Semibold") loads faces,
    // so fallbacks remembered before it are searched again.
    let dir = font_dir("fallback-legacy");
    std::fs::write(dir.join("Sysfont-Semibold.ttf"), renamed("SourceSans3-Semibold.ttf")).unwrap();
    let db = FontDb::with_font_dirs(vec![dir]);
    db.set_system_fallback(false);
    let latin = db.face(FALLBACK_FAMILY, "Regular");
    let thai = 'ก';
    assert!(db.fallback_for(thai, latin.id()).is_none());
    let before = db.read_faces().len();
    let semibold = format!("{FAMILY} Semibold");
    let f = db.face(&semibold, "Regular");
    assert_eq!((f.style.as_str(), f.legacy_family.as_str()), ("Semibold", semibold.as_str()));
    assert!(db.read_faces().len() > before, "the installed family loaded");
    assert!(db.fallback_for('A', f.id()).is_some_and(|g| g.id() != f.id()));
    let cache = db.fallbacks.read().unwrap();
    assert_eq!(cache.faces, db.read_faces().len());
    assert!(!cache.map.contains_key(&(thai, latin.id())), "answers from before the load are dropped");
}

/// A bundled font renamed by a `name` table holding just its family names: (Windows language id,
/// name) pairs, English (0x0409) first, as SimSun's names 宋体 for Simplified Chinese.
fn with_family_names(file: &str, names: &[(u16, &str)]) -> Vec<u8> {
    let data = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts").join(file)).unwrap();
    let font = skrifa::FontRef::new(&data).unwrap();
    // Records sorted by language, then name ID: family (1) and subfamily (2, English only).
    let mut records: Vec<(u16, u16, Vec<u8>)> =
        names.iter().map(|(lang, n)| (*lang, 1, n.encode_utf16().flat_map(u16::to_be_bytes).collect())).collect();
    records.push((0x0409, 2, "Regular".encode_utf16().flat_map(u16::to_be_bytes).collect()));
    records.sort_by_key(|r| (r.0, r.1));
    let mut name = Vec::new();
    for v in [0u16, records.len() as u16, (6 + 12 * records.len()) as u16] {
        name.extend_from_slice(&v.to_be_bytes());
    }
    let mut offset = 0u16;
    for (lang, id, s) in &records {
        for v in [3u16, 1, *lang, *id, s.len() as u16, offset] {
            name.extend_from_slice(&v.to_be_bytes());
        }
        offset += s.len() as u16;
    }
    for (_, _, s) in &records {
        name.extend_from_slice(s);
    }
    let mut tables: Vec<([u8; 4], Vec<u8>)> = font
        .table_directory()
        .table_records()
        .iter()
        .filter(|r| r.tag() != skrifa::Tag::new(b"name"))
        .map(|r| (r.tag().to_be_bytes(), font.table_data(r.tag()).unwrap().as_bytes().to_vec()))
        .collect();
    tables.push((*b"name", name));
    tables.sort_by_key(|t| t.0);
    mini_font(&tables)
}

#[test]
fn chinese_fonts_are_listed_by_their_chinese_names_in_a_chinese_interface() {
    // #505: the font list showed fonts only by their English names (SimSun, never 宋体). Here a
    // Chinese font is the second face of a collection, as in simsun.ttc and msyh.ttc.
    let dir = font_dir("localized");
    let song = with_family_names("SourceSans3-Regular.ttf", &[(0x0409, "Songti Test"), (0x0804, "测试宋体"), (0x0404, "測試宋體")]);
    std::fs::write(dir.join("song.ttc"), collection(&[renamed("SourceSans3-Semibold.ttf"), song])).unwrap();
    let db = FontDb::with_font_dirs(vec![dir]);
    // English names in the English list; the Chinese names in place of them in a Chinese one.
    assert!(has(&db.families(), "Songti Test") && !has(&db.families(), "测试宋体"));
    assert!(has(&db.families_in("en"), "Songti Test"));
    let hans = db.families_in("zh-hans");
    assert!(has(&hans, "测试宋体") && !has(&hans, "Songti Test") && has(&hans, FAMILY), "{hans:?}");
    assert!(has(&db.families_in("zh-hant"), "測試宋體"));
    assert_eq!(db.localized_names("zh-hans"), [("Songti Test".to_string(), "测试宋体".to_string())]);
    // Either name finds the font, before and after it loads.
    assert!(db.has_family("测试宋体") && db.has_family("測試宋體"));
    assert_eq!(db.styles("测试宋体"), ["Regular"]);
    let f = db.face("测试宋体", "Regular");
    assert_eq!((f.family.as_str(), f.index()), ("Songti Test", 1));
    assert_eq!(db.face("測試宋體", "Bold").id(), f.id());
    assert_eq!(db.english_family("测试宋体").as_deref(), Some("Songti Test"));
    assert!(db.has_family("测试宋体"));
    // Name table languages against interface languages.
    for (ui, tag, ok) in [
        ("zh-hans", "zh-Hans", true),
        ("zh-hans", "zh-SG", true),
        ("zh-hans", "zh-TW", false),
        ("zh-hant", "zh-HK", true),
        ("zh-hant", "zh-Hans", false),
        ("ja", "ja-JP", true),
        ("ja", "ko-KR", false),
        ("en", "zh-Hans", false),
    ] {
        assert_eq!(name_language_matches(ui, tag), ok, "{ui} {tag}");
    }
}
