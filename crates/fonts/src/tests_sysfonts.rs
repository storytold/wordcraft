//! Installed system fonts: found by family name by every lookup, whatever ran before it (#22).

use std::path::{Path, PathBuf};

use super::*;

const FAMILY: &str = "Sysfont Sans3";

/// A bundled Source Sans 3 file renamed [`FAMILY`] (as long as the original name).
fn renamed(file: &str) -> Vec<u8> {
    renamed_to(file, FAMILY)
}

/// A bundled Source Sans 3 file renamed `family` (13 characters, as long as the original name).
fn renamed_to(file: &str, family: &str) -> Vec<u8> {
    assert_eq!(family.len(), "Source Sans 3".len());
    let mut data = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts").join(file)).unwrap();
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    for (from, to) in [(utf16("Source Sans 3"), utf16(family)), (b"Source Sans 3".to_vec(), family.as_bytes().to_vec())] {
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

#[test]
fn fallback_reads_each_installed_font_file_once_however_many_characters_miss() {
    // #605: every distinct character no font covers read every installed font file again.
    let dir = font_dir("coverage");
    // The installed Regular also maps U+30000 (no bundled font does): its last cmap group, the
    // single character U+1F916, moved there (still the highest, so the groups stay sorted).
    let mut regular = renamed("SourceSans3-Regular.ttf");
    let (from, to) = ([0x0001_F916_u32.to_be_bytes(); 2].concat(), [0x0003_0000_u32.to_be_bytes(); 2].concat());
    let mut patched = 0;
    while let Some(at) = regular.windows(from.len()).position(|w| w == from) {
        regular[at..at + to.len()].copy_from_slice(&to);
        patched += 1;
    }
    assert!(patched > 0);
    std::fs::write(dir.join("Sysfont-Regular.ttf"), regular).unwrap();
    let db = FontDb::with_font_dirs(vec![dir]);
    let latin = db.face(FALLBACK_FAMILY, "Regular");
    let found = db.fallback_for('\u{30000}', latin.id()).expect("the installed font maps it");
    assert_eq!((found.family.as_str(), found.style.as_str()), (FAMILY, "Regular"));
    // 2,000 distinct characters nothing maps (Supplementary Private Use Area-A: no font here,
    // bundled or craft-fonts, maps it; CJK Extension G after U+30000 is partly in Noto Sans CJK
    // SC): the index answers, no file is read again.
    for cp in 0xF_0001..0xF_0001 + 2000 {
        let c = char::from_u32(cp).unwrap();
        assert!(db.fallback_for(c, latin.id()).is_none());
    }
    let index = db.coverage.lock().unwrap();
    assert_eq!(index.files.len(), 2, "Regular and Bold are cataloged");
    assert_eq!(index.reads, 2, "each file read once");
}

#[test]
fn coverage_ranges_hold_exactly_the_mapped_characters() {
    let data = renamed("SourceSans3-Regular.ttf");
    let ranges = coverage_ranges(&data);
    assert!(ranges.windows(2).all(|w| w[0].1 + 1 < w[1].0), "sorted, disjoint and merged");
    let font = skrifa::FontRef::new(&data).unwrap();
    let mapped: Vec<u32> = font.charmap().mappings().map(|(cp, _)| cp).collect();
    assert!(ranges.len() < mapped.len() / 4, "ranges, not a set of characters");
    assert!(mapped.iter().all(|cp| ranges_hold(&ranges, *cp)));
    for cp in [0x0, 0x9, 0x0E01, 0x17000, 0x10_FFFF] {
        assert_eq!(ranges_hold(&ranges, cp), font.charmap().map(cp).is_some(), "U+{cp:04X}");
    }
    // Damaged data indexes as nothing.
    assert!(coverage_ranges(b"ttcf\0\x01\0\0\xff\xff\xff\xff").is_empty());
    assert!(coverage_ranges(&data[..100]).is_empty());
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

/// `data` with its OS/2 code page bits (`ulCodePageRange1`) set to `pages`.
fn with_code_pages(data: &[u8], pages: u32) -> Vec<u8> {
    let font = skrifa::FontRef::new(data).unwrap();
    let tables: Vec<([u8; 4], Vec<u8>)> = font
        .table_directory()
        .table_records()
        .iter()
        .map(|r| {
            let mut t = font.table_data(r.tag()).unwrap().as_bytes().to_vec();
            if r.tag() == skrifa::Tag::new(b"OS/2") {
                t[78..82].copy_from_slice(&pages.to_be_bytes());
            }
            (r.tag().to_be_bytes(), t)
        })
        .collect();
    mini_font(&tables)
}

#[test]
fn east_asian_fonts_are_listed_for_the_asian_text_font_box() {
    // #505: the Font dialog's Asian text font box lists the Chinese, Japanese and Korean fonts,
    // found by their OS/2 code pages (here GB2312) during the scan, without loading them.
    let dir = font_dir("eastasian");
    let song = with_family_names("SourceSans3-Regular.ttf", &[(0x0409, "Songti Test"), (0x0804, "测试宋体")]);
    std::fs::write(dir.join("song.ttf"), with_code_pages(&song, 1 << 18)).unwrap();
    let db = FontDb::with_font_dirs(vec![dir]);
    let listed = |db: &FontDb| db.east_asian_families().iter().filter(|f| *f == "Songti Test").count();
    assert_eq!(listed(&db), 1);
    assert!(!has(&db.east_asian_families(), FAMILY), "Latin fonts are left out");
    // Loaded, it is still listed once.
    assert!(db.face("Songti Test", "Regular").is_east_asian());
    assert_eq!(listed(&db), 1);
}

/// #848: browsing the font list drew each family's name in its own face, loading every family
/// shown for good (tens of megabytes per trip through the Font box). Preview faces are read for
/// the preview alone: however many families are previewed, the database keeps no font data, and
/// each preview face is freed when dropped.
#[test]
fn font_list_previews_load_nothing_for_good() {
    let dir = std::env::temp_dir().join(format!("dc-sysfonts-{}-previews", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let families: Vec<String> = (0..40).map(|i| format!("Preview Fam{i:02}")).collect();
    for (i, f) in families.iter().enumerate() {
        std::fs::write(dir.join(format!("p{i}-Regular.ttf")), renamed_to("SourceSans3-Regular.ttf", f)).unwrap();
        std::fs::write(dir.join(format!("p{i}-Bold.ttf")), renamed_to("SourceSans3-Bold.ttf", f)).unwrap();
    }
    let db = FontDb::with_font_dirs(vec![dir.clone()]);
    let (faces, bytes) = (db.read_faces().len(), db.loaded_font_bytes());
    // Every family previewed many times over, as reopening the list does.
    for _ in 0..5 {
        for f in &families {
            let face = db.preview_face(f).expect("installed family previews");
            assert_eq!(face.family, *f);
            assert_eq!(face.style, "Regular", "the preview shows the Regular face");
            assert!(face.covers('P'));
            assert_eq!(Arc::strong_count(&face), 1, "nothing else keeps the preview face (or its data)");
        }
    }
    assert_eq!(db.read_faces().len(), faces, "previews load no faces");
    assert_eq!(db.loaded_font_bytes(), bytes, "previews keep no font data");
    assert!(db.preview_face("No Such Family").is_none());

    // Using a font loads it once; previewing it again shares the loaded face.
    assert_eq!(db.face(&families[0], "Regular").family, families[0]);
    let used = db.loaded_font_bytes();
    assert!(used > bytes, "a used family's data is counted");
    let shared = db.preview_face(&families[0]).unwrap();
    assert!(Arc::strong_count(&shared) > 1, "a loaded family's preview is the shared face");
    for _ in 0..3 {
        for f in &families[1..] {
            drop(db.preview_face(f));
        }
    }
    assert_eq!(db.loaded_font_bytes(), used, "memory stays bounded across previews");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Give an OFL test face a complete synthetic name table without adding a font asset.
fn named_face(file: &str, family: &str) -> Vec<u8> {
    let mut data = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts").join(file)).unwrap();
    let style = if file.contains("Bold") { "Bold" } else { "Regular" };
    let names =
        [(1, family.to_string()), (2, style.to_string()), (4, format!("{family} {style}")), (6, format!("{}-{style}", family.replace(' ', "")))];
    let mut table = Vec::new();
    table.extend_from_slice(&0u16.to_be_bytes());
    table.extend_from_slice(&(names.len() as u16).to_be_bytes());
    table.extend_from_slice(&(6u16 + 12 * names.len() as u16).to_be_bytes());
    let mut strings = Vec::new();
    for (id, value) in names {
        let bytes: Vec<_> = value.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for n in [3, 1, 0x409, id, bytes.len() as u16, strings.len() as u16] {
            table.extend_from_slice(&n.to_be_bytes());
        }
        strings.extend_from_slice(&bytes);
    }
    table.extend_from_slice(&strings);
    data.resize(data.len().next_multiple_of(4), 0);
    let offset = data.len() as u32;
    let count = u16::from_be_bytes(data[4..6].try_into().unwrap()) as usize;
    let entry = (0..count).map(|i| 12 + i * 16).find(|i| &data[*i..*i + 4] == b"name").unwrap();
    data[entry + 8..entry + 12].copy_from_slice(&offset.to_be_bytes());
    data[entry + 12..entry + 16].copy_from_slice(&(table.len() as u32).to_be_bytes());
    data.extend_from_slice(&table);
    data
}

#[test]
fn circle_uses_installed_face() {
    let dir = font_dir("circle-fallback");
    std::fs::write(dir.join("Text-Regular.ttf"), named_face("SourceSans3-Regular.ttf", "Times New Roman")).unwrap();
    std::fs::write(dir.join("Text-Bold.ttf"), named_face("SourceSans3-Bold.ttf", "Times New Roman")).unwrap();
    let db = FontDb::with_font_dirs(vec![dir]);
    let primary = db.face("Source Serif 4", "Regular");
    assert!(!primary.covers('●'), "the requested face lacks the marker");
    assert!(!db.is_loaded("Times New Roman"), "fallback must find a cataloged face");
    db.set_system_fallback(false);
    assert_eq!(db.fallback_for('●', primary.id()).unwrap().family, FALLBACK_FAMILY);
    assert!(!db.is_loaded("Times New Roman"));
    db.set_system_fallback(true);
    let found = db.fallback_for('●', primary.id()).unwrap();
    assert_eq!(found.family, "Times New Roman");
    assert!(found.covers('●'));
    assert_ne!(db.fallback_for('●', found.id()).unwrap().id(), found.id(), "the excluded face cannot be reused");
    assert_eq!(db.fallback_for('●', primary.id()).unwrap().id(), found.id());
    let bold = db.fallback_styled('●', primary.id(), true, false).unwrap();
    assert_eq!(bold.family, "Times New Roman");
    assert!(bold.weight >= 600.0 && bold.covers('●'));
    assert_eq!(db.fallback_for('A', primary.id()).unwrap().family, FALLBACK_FAMILY, "ordinary character fallback is unchanged");
}

#[test]
fn circle_without_text_face() {
    let db = FontDb::with_font_dirs(Vec::new());
    db.set_system_fallback(false);
    let primary = db.face("Source Serif 4", "Regular");
    let found = db.fallback_for('●', primary.id()).unwrap();
    assert_eq!(found.family, FALLBACK_FAMILY);
    assert!(found.covers('●'));
}

#[test]
fn circle_checks_face_coverage() {
    let db = FontDb::with_font_dirs(Vec::new());
    db.set_system_fallback(false);
    db.add_document_fonts(vec![named_face("SourceSerif4-Regular.ttf", "Times New Roman")]);
    let preferred = db.face("Times New Roman", "Regular");
    assert!(!preferred.covers('●'));
    let primary = db.face("Source Serif 4", "Regular");
    assert_eq!(db.fallback_for('●', primary.id()).unwrap().family, FALLBACK_FAMILY);
}
