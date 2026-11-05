#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod sysfonts;

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn occasional_slow_hinted_glyphs_do_not_turn_hinting_off() {
    let off = |pattern: &[bool]| {
        let face = super::make_face(super::FontBytes::Static(super::BUNDLED[0]), 0, "T".into(), "Regular".into(), "T".into(), Vec::new()).unwrap();
        pattern.iter().position(|&slow| face.count_hinted(slow))
    };
    // A face slow throughout stops at the third strike.
    assert_eq!(off(&[true; 5]), Some(2));
    // Three draws slowed by a busy machine among many: hinting stays on.
    let mut busy = vec![false; 100];
    busy.extend([true; 3]);
    assert_eq!(off(&busy), None);
    // Slow glyphs spread among fast ones still stop it once they are one in 16.
    let spread: Vec<bool> = (0..64).map(|i| i % 8 == 7).collect();
    assert_eq!(off(&spread), Some(23));
}
#[test]
#[ignore = "requires a locally licensed kaiu.ttf; set WORDCRAFT_TEST_KAI_FONT"]
fn kai_contours_include_complete_character_body() {
    use kurbo::Shape;
    let path = std::env::var_os("WORDCRAFT_TEST_KAI_FONT").expect("set WORDCRAFT_TEST_KAI_FONT");
    let db = super::FontDb::with_font_dirs(vec![]);
    db.set_system_fallback(false);
    assert!(db.add_font(std::fs::read(path).unwrap()) > 0);
    let face = db.face("DFKai-SB", "Regular");
    assert_eq!(face.family, "DFKai-SB");
    for ch in ['中', '文', '契', '約'] {
        let outline = face.glyph_outline(face.glyph_for(ch));
        let bounds = outline.bounding_box();
        eprintln!("{ch}: {bounds:?}, upem={}", face.upem);
        assert!(bounds.width() > face.upem * 0.5, "{ch}: missing horizontal strokes");
        assert!(bounds.height() > face.upem * 0.5, "{ch}: missing vertical strokes");
        assert!(std::sync::Arc::ptr_eq(&db.outline(&face, face.glyph_for(ch)), &db.outline(&face, face.glyph_for(ch))));
    }
}

use super::*;

#[test]
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
fn system_font_dirs_include_the_flatpak_host_folders() {
    // Inside a Flatpak, /usr/share/fonts is the runtime's: the host's fonts are under /run/host (#1937).
    let dirs = system_font_dirs();
    for d in ["/usr/share/fonts", "/run/host/fonts", "/run/host/local-fonts", "/run/host/user-fonts", "/run/host/user-fonts-legacy"] {
        assert!(dirs.contains(&std::path::PathBuf::from(d)), "{d} missing from {dirs:?}");
    }
}

#[test]
fn a_capped_outline_is_refused_while_it_is_drawn() {
    // #3716 review: a colour glyph's clip outline stops growing at the payload left.
    let db = FontDb::with_font_dirs(Vec::new());
    db.set_system_fallback(false);
    let face = db.face(FALLBACK_FAMILY, "Regular");
    let gid = face.glyph_for('g');
    let whole = face.glyph_outline(gid);
    let n = whole.elements().len();
    assert!(n > 4, "{n}");
    assert_eq!(face.glyph_outline_capped(gid, n).as_ref().map(BezPath::elements), Some(whole.elements()));
    assert!(face.glyph_outline_capped(gid, n - 1).is_none());
    assert!(face.glyph_outline_capped(gid, 0).is_none());
}

#[test]
fn run_fallback_is_cached_per_char_set() {
    let db = FontDb::with_font_dirs(Vec::new());
    db.set_system_fallback(false);
    let latin = db.face(FALLBACK_FAMILY, "Regular");
    let a = db.fallback_for_run(&['分', '类', '分'], latin.id(), false, false).map(|f| f.id());
    // The same set in another order (and without the repeat) is the same entry.
    assert_eq!(db.fallback_for_run(&['类', '分'], latin.id(), false, false).map(|f| f.id()), a);
    {
        let cache = db.run_fallbacks.read().unwrap();
        assert_eq!(cache.faces, db.read_faces().len());
        assert_eq!(cache.map.len(), 1);
        assert!(cache.map.contains_key(&(vec!['分', '类'], latin.id(), false, false)));
    }
    // Long hostile runs keep the cache bounded.
    let many: Vec<char> = (0x4E00u32..0x4E00 + 50_000).filter_map(char::from_u32).collect();
    for i in 0..30 {
        let _ = db.fallback_for_run(many.get(i..).unwrap_or(&[]), latin.id(), false, false);
    }
    let cache = db.run_fallbacks.read().unwrap();
    assert!(cache.chars <= RUN_FALLBACK_CACHE_CHARS, "{}", cache.chars);
    // Turning the system fallback on or off forgets remembered misses.
    drop(cache);
    db.set_system_fallback(false);
    assert!(db.run_fallbacks.read().unwrap().map.is_empty());
}

/// The issue's run (#1549): Thai letters and digits, Latin letters, digits and the baht sign.
const THAI_RUN: &str = "[D] น้ำ ค่ำ ปั้น ฝั่ง ปู่ ๑๒๓๔๕๖๗๘๙๐ ฿1,250.50";

#[test]
fn a_thai_run_with_latin_falls_back_to_one_face() {
    let db = FontDb::global();
    let chars: Vec<char> = THAI_RUN.chars().filter(|c| !c.is_whitespace()).collect();
    // Needs a Thai font with Latin letters and digits (Tahoma, Leelawadee UI, Thonburi, the
    // TLWG fonts…): Noto Sans Thai alone has none, so no face covers the run.
    if !chars.iter().all(|c| crate::word::resolve("TH Sarabun New", false, false).face.covers(*c)) {
        eprintln!("skipped: no Thai font with Latin letters installed");
        return;
    }
    let latin = db.face(FALLBACK_FAMILY, "Regular");
    let f = db.fallback_for_run(&chars, latin.id(), false, false).expect("one face covers the run");
    assert_ne!(f.id, latin.id);
    for c in &chars {
        assert!(f.covers(*c), "{} covers {c}", f.family);
    }
    // A Thai font with OpenType tables, as the per-character fallback prefers.
    assert!(f.has_layout_tables(), "{} shapes Thai", f.family);
    // Without system fonts and without craft-fonts' Thai face (built in since craft-fonts
    // 3d9ab3a1) no bundled face covers Thai: the caller keeps the per-character path.
    let bundled = FontDb::with_font_dirs(Vec::new());
    bundled.set_system_fallback(false);
    let latin = bundled.face(FALLBACK_FAMILY, "Regular");
    // Expected from the embedded font list, not from the database under test.
    let thai_bundled = crate::CRAFT_FONTS.iter().any(|f| f.scripts.contains(&"Thai"));
    assert_eq!(bundled.fallback_for('น', 0).is_some(), thai_bundled);
    assert_eq!(bundled.fallback_for_run(&chars, latin.id(), false, false).is_some(), thai_bundled);
}

/// With only bundled Latin faces and no system fallback, a Thai run has no single covering face,
/// so run-level fallback declines. This test asserts that `fallback_for_run` returns `None`; the
/// caller's per-character path is not exercised here.
#[test]
fn a_run_without_a_covering_face_keeps_the_per_character_path() {
    // Use only this repository's Latin faces. Optional craft-fonts inputs can contain a
    // complete Thai face even when system font loading is disabled.
    let bundled = FontDb::with_font_dirs(Vec::new());
    bundled.faces.write().unwrap().retain(|face| BUNDLED.iter().any(|bytes| std::ptr::eq(face.data(), *bytes)));
    bundled.set_system_fallback(false);
    let latin = bundled.face(FALLBACK_FAMILY, "Regular");
    let chars: Vec<char> = THAI_RUN.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(!bundled.faces.read().unwrap().is_empty(), "the negative fixture retains real Latin faces");
    assert!(bundled.faces.read().unwrap().iter().all(|face| !face.covers('น')));
    assert!(bundled.fallback_for_run(&chars, latin.id(), false, false).is_none());
}

#[test]
fn a_family_named_for_a_heavy_weight_is_that_heavy() {
    let face = |bytes: &'static [u8], family: &str| {
        make_face(FontBytes::Static(bytes), 0, family.into(), "Regular".into(), family.into(), Vec::new()).unwrap()
    };
    // The file says 400, as the core-fonts Arial Black does: bold must not embolden it again.
    assert_eq!(face(BUNDLED[0], "Arial Black").weight, 900.0);
    assert_eq!(face(BUNDLED[0], "Franklin Gothic Heavy ").weight, 900.0);
    assert_eq!(face(BUNDLED[0], "Segoe UI ExtraBold").weight, 800.0);
    assert_eq!(face(BUNDLED[0], "Blackadder ITC").weight, 400.0);
    assert_eq!(face(BUNDLED[0], "Arial").weight, 400.0);
    // A file that already says it is heavy keeps its own weight (Lato Heavy is 800, not 900).
    assert_eq!(face(BUNDLED[1], "Source Sans 3 Heavy").weight, 600.0);
}

#[test]
fn legacy_family_keeps_the_style_words_that_are_not_regular_bold_or_italic() {
    for (family, style, legacy) in [
        ("Roboto", "Regular", "Roboto"),
        ("Roboto", "Bold Italic", "Roboto"),
        ("Roboto", "italic", "Roboto"),
        ("Roboto", "Light", "Roboto Light"),
        ("Roboto", "Light Italic", "Roboto Light"),
        ("Roboto", "Condensed Bold", "Roboto Condensed"),
        ("Inter", "SemiBold", "Inter SemiBold"),
        ("Inter", "ExtraBold  Italic", "Inter ExtraBold"),
        ("Inter", "Medium {wght:520}", "Inter Medium"),
        ("Inter", "", "Inter"),
    ] {
        assert_eq!(legacy_family(family, style), legacy, "{family} {style}");
    }
}

#[test]
fn faces_are_found_by_their_legacy_family_name() {
    // Bundled static fonts whose name ID 1 differs from the typographic family (#219).
    let db = FontDb::with_font_dirs(vec![]);
    let families = db.families();
    for name in ["Inter", "Inter Medium", "Inter SemiBold", "Source Sans 3 Semibold", "Source Serif 4 Semibold"] {
        assert!(families.iter().any(|f| f == name), "{name} in {families:?}");
        assert!(db.has_family(name));
    }
    let semibold = db.face("Source Serif 4 Semibold", "Regular");
    assert_eq!((semibold.family.as_str(), semibold.style.as_str()), ("Source Serif 4", "Semibold"));
    assert_eq!(db.face("inter medium", "Regular").style, "Medium");
    assert_eq!(db.face("Inter SemiBold", "Regular").style, "SemiBold");
    // Bold and italic choose within the faces the legacy name covers; the family's own Bold
    // isn't one of them.
    assert_eq!(db.face("Source Serif 4 Semibold", "Bold").style, "Semibold");
    assert_eq!(db.face("Source Serif 4 Semibold", "Italic").style, "Semibold");
    assert_eq!(db.styles("Source Sans 3 Semibold"), ["Semibold"]);
    // The typographic family is unchanged.
    assert_eq!(db.face("Source Serif 4", "Bold").style, "Bold");
    assert_eq!(db.face("Source Serif 4", "Regular").style, "Regular");
    assert_eq!(db.face("Source Serif 4", "Regular").legacy_family, "Source Serif 4");
    // Not a name any face has.
    assert!(!db.has_family("Source Serif 4 Light"));
    assert_eq!(db.face("Source Serif 4 Light", "Regular").family, FALLBACK_FAMILY);
}

#[test]
fn a_legacy_named_instance_with_axis_settings_is_made_once() {
    let db = FontDb::with_font_dirs(vec![]);
    let a = db.face("Inter Medium", "Medium {wght:520}");
    let b = db.face("Inter Medium", "Medium {wght:520}");
    assert_eq!(a.id(), b.id());
    assert_eq!(a.legacy_family, "Inter Medium");
}

#[test]
fn full_names_select_exact_faces() {
    let db = FontDb::with_font_dirs(vec![]);
    for style in ["Regular", "Bold", "Italic", "Bold Italic"] {
        let expected = db.face("Source Serif 4", style);
        let f = expected.skrifa().unwrap();
        let full = name(&f, &[StringId::FULL_NAME]).unwrap();
        assert!(db.has_family(&full), "{full}");
        assert_eq!(db.face(&full, "Regular").id(), expected.id());
        let bold = if full == expected.family { db.face(&full, "Bold") } else { expected.clone() };
        assert_eq!(db.face(&full.to_ascii_lowercase(), "Bold").id(), bold.id());
    }
    assert!(!db.has_family("NotAFont Bold"));
    assert_eq!(db.face("Source Serif 4", "Regular").style, "Regular");
}

#[test]
fn postscript_names_select_faces() {
    let db = FontDb::with_font_dirs(vec![]);
    for style in ["Regular", "Bold", "Italic", "Bold Italic"] {
        let expected = db.face("Source Serif 4", style);
        let f = expected.skrifa().unwrap();
        let ps = name(&f, &[StringId::POSTSCRIPT_NAME]).unwrap();
        assert!(db.has_family(&ps), "{ps}");
        let actual = db.face(&ps, "Regular");
        assert_eq!(actual.id(), expected.id(), "{ps}");
        assert_eq!(db.face(&ps.to_ascii_lowercase(), "Bold").id(), expected.id());
    }
    assert!(!db.has_family("NotAFont-Bold"));
    assert_eq!(db.face("Source Serif 4", "Regular").style, "Regular");
}

/// A variable font holding just `name` and `fvar` tables: family "Var Test", one `wght` axis
/// and `instances` named instances, each with its own style name.
fn variable_font(instances: u16) -> Vec<u8> {
    variable_font_named("Var Test", &[], instances)
}

/// [`variable_font`] with family `family`, and family names in other languages `local`
/// (Windows language ids).
fn variable_font_named(family: &str, local: &[(u16, &str)], instances: u16) -> Vec<u8> {
    let mut names: Vec<(u16, u16, String)> = vec![(0x409, 1, family.into()), (0x409, 2, "Regular".into())];
    names.extend(local.iter().map(|(lang, n)| (*lang, 1, n.to_string())));
    names.extend((0..instances).map(|i| (0x409, 256 + i, format!("Style {i}"))));
    // Records sorted by language, then name id.
    names.sort_by_key(|(lang, id, _)| (*lang, *id));
    let mut name = vec![0, 0];
    name.extend_from_slice(&(names.len() as u16).to_be_bytes());
    name.extend_from_slice(&(6 + 12 * names.len() as u16).to_be_bytes());
    let mut strings = Vec::new();
    for (lang, id, text) in &names {
        let utf16: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for v in [3, 1, *lang, *id, utf16.len() as u16, strings.len() as u16] {
            name.extend_from_slice(&v.to_be_bytes());
        }
        strings.extend(utf16);
    }
    name.extend(strings);
    let mut fvar = Vec::new();
    for v in [1u16, 0, 16, 2, 1, 20, instances, 8] {
        fvar.extend_from_slice(&v.to_be_bytes());
    }
    fvar.extend_from_slice(b"wght");
    for v in [100u32 << 16, 400 << 16, 900 << 16] {
        fvar.extend_from_slice(&v.to_be_bytes());
    }
    fvar.extend_from_slice(&[0, 0, 0, 2]); // flags, axisNameId
    for i in 0..instances {
        fvar.extend_from_slice(&(256 + i).to_be_bytes());
        fvar.extend_from_slice(&[0, 0]);
        fvar.extend_from_slice(&(400u32 << 16).to_be_bytes());
    }
    mini_font(&[(*b"fvar", fvar), (*b"name", name)])
}

/// A collection listing `font` `members` times.
fn collection_of(font: &[u8], members: u32) -> Vec<u8> {
    let start = 12 + 4 * members;
    let mut out = b"ttcf\0\x01\0\0".to_vec();
    out.extend_from_slice(&members.to_be_bytes());
    for _ in 0..members {
        out.extend_from_slice(&start.to_be_bytes());
    }
    let mut font = font.to_vec();
    let tables = u16::from_be_bytes([font[4], font[5]]) as usize;
    for i in 0..tables {
        let at = 12 + 16 * i + 8;
        let offset = u32::from_be_bytes(font[at..at + 4].try_into().unwrap()) + start;
        font[at..at + 4].copy_from_slice(&offset.to_be_bytes());
    }
    out.extend(font);
    out
}

/// JetBrains Mono renamed "MingLiU JBMono" (a name the interpreter is required for), with its
/// `maxp` twilight-zone size set to `twilight`.
fn hint_reliant_font(twilight: u16) -> Vec<u8> {
    let mut data = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/JetBrainsMono-Regular.ttf")).unwrap();
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    for (from, to) in [(utf16("JetBrains Mono"), utf16("MingLiU JBMono")), (b"JetBrains Mono".to_vec(), b"MingLiU JBMono".to_vec())] {
        while let Some(at) = data.windows(from.len()).position(|w| w == from) {
            data[at..at + to.len()].copy_from_slice(&to);
        }
    }
    let tables = u16::from_be_bytes([data[4], data[5]]) as usize;
    let entry = (0..tables).map(|i| 12 + 16 * i).find(|e| &data[*e..e + 4] == b"maxp").unwrap();
    let maxp = u32::from_be_bytes(data[entry + 8..entry + 12].try_into().unwrap()) as usize;
    data[maxp + 16..maxp + 18].copy_from_slice(&twilight.to_be_bytes());
    data
}

#[test]
fn a_font_asking_for_huge_hinting_zones_is_drawn_unhinted() {
    // #3692: skrifa's budget counts instructions, not the points each one moves.
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(hint_reliant_font(660));
    let face = db.face("MingLiU JBMono", "Regular");
    assert_eq!(face.family, "MingLiU JBMono");
    assert!(face.hinter().is_some(), "a real font's zones are hinted");
    assert!(!face.glyph_outline(face.glyph_for('a')).elements().is_empty());
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(hint_reliant_font(u16::MAX));
    let face = db.face("MingLiU JBMono", "Regular");
    assert!(face.hinter().is_none(), "a huge twilight zone is not hinted");
    assert!(!face.glyph_outline(face.glyph_for('a')).elements().is_empty(), "drawn unhinted");
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn the_draw_that_turns_hinting_off_is_redrawn_unhinted_for_the_outline_cache() {
    // #3692: the glyph whose slow hinted draw trips the breaker may be misassembled; it must not be
    // what the outline cache keeps.
    let data = hint_reliant_font(660);
    let mut face = super::make_face(
        super::FontBytes::Owned(std::sync::Arc::new(data)),
        0,
        "MingLiU JBMono".into(),
        "Regular".into(),
        "MingLiU JBMono".into(),
        Vec::new(),
    )
    .unwrap();
    assert!(face.hinter().is_some());
    face.hint_strike = std::time::Duration::ZERO; // every hinted draw is slow
    let gid = face.glyph_for('a');
    assert!(face.draw_outline(gid, usize::MAX).hinted, "hinted while the breaker is closed");
    assert!(face.draw_outline(gid, usize::MAX).hinted);
    let tripping = face.draw_outline(gid, usize::MAX);
    assert!(face.hinting_off.load(std::sync::atomic::Ordering::Relaxed), "the third slow draw trips the breaker");
    assert!(!tripping.hinted, "the tripping draw is returned unhinted");
    assert!(!tripping.path.elements().is_empty());
    // And what the cache keeps is that outline.
    let db = FontDb::with_font_dirs(vec![]);
    assert_eq!(db.outline(&face, gid).elements(), tripping.path.elements());
}

#[test]
fn long_names_are_cut_before_faces_copy_them() {
    // A name record can hold 64K characters, and every named instance copies the family and its
    // localized names: the names are cut when read, so the copies stay small.
    let long = "W".repeat(14_000);
    let local = "Ж".repeat(14_000);
    let font = variable_font_named(&long, &[(0x419, &local)], 32);
    let faces = font_faces(&skrifa::FontRef::new(&font).unwrap(), 0);
    assert_eq!(faces.len(), 32);
    for f in &faces {
        assert_eq!(f.family.chars().count(), MAX_NAME_CHARS);
        assert!(f.legacy.chars().count() <= MAX_NAME_CHARS + " Style 31".len(), "{}", f.legacy.len());
        assert_eq!(f.local.len(), 1);
        assert_eq!(f.local[0].name.chars().count(), MAX_NAME_CHARS);
        assert_eq!(f.local[0].english, f.family);
    }
    // Short names are kept whole.
    let faces = font_faces(&skrifa::FontRef::new(&variable_font_named("Var Test", &[(0x419, "Вар Тест")], 2)).unwrap(), 0);
    assert_eq!((faces[0].family.as_str(), faces[0].local[0].name.as_str()), ("Var Test", "Вар Тест"));
}

#[test]
fn named_instances_and_collection_members_are_capped() {
    // #3716: a file's faces are capped before they are built, however the file multiplies them.
    let font = variable_font(300);
    let faces = font_faces(&skrifa::FontRef::new(&font).unwrap(), 0);
    assert_eq!(faces.len(), MAX_FILE_FACES + 1, "one more than the cap, so a caller can tell");
    assert_eq!(faces[0].style, "Style 0");
    assert_eq!(enumerate_faces(&font, MAX_FILE_FACES).len(), MAX_FILE_FACES);
    // A million members, all the same font: read once, and no more than the cap.
    let started = std::time::Instant::now();
    let ttc = collection_of(BUNDLED[0], 1_000_000);
    let found = enumerate_faces(&ttc, MAX_FILE_FACES);
    assert_eq!(found.iter().map(|f| (f.family.as_str(), f.style.as_str())).collect::<Vec<_>>(), [(FALLBACK_FAMILY, "Regular")]);
    let db = FontDb::with_font_dirs(vec![]);
    assert_eq!(db.add_font(collection_of(&font, 1_000_000)), MAX_FILE_FACES);
    // A font the user adds is refused instead.
    assert!(matches!(db.install_font("v.ttf", &variable_font(300)), Err(user::UserFontError::Unusable(e)) if e.contains("more than 256")));
    // The face counts above are what tell the old code (a face per member) apart; the time
    // (under 0.1 s on a loaded machine) only backs up "read once", with room for a busy CI.
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
}

/// A system Khmer font that draws a triisap above an upper vowel (Khmer Sangam MN for អ) shapes it
/// as the U-form, like Khmer OS System, which does so itself. Fonts that already do (Khmer OS
/// System, Khmer Busra, which dotted-circles a typed ុ + ិ) are left alone. Skipped without the fonts.
#[test]
fn khmer_triisap_takes_the_u_form_like_khmer_os_system() {
    let face = |path: &std::path::Path| {
        let data = std::fs::read(path).ok()?;
        super::make_face(super::FontBytes::Owned(data.into()), 0, "Khmer".into(), "Regular".into(), "Khmer".into(), Vec::new())
    };
    let gids = |f: &FontFace, t: &str| crate::shape(f, t, &[], |c| c).into_iter().map(|g| g.gid).collect::<Vec<_>>();
    if let Some(f) = face(std::path::Path::new("/System/Library/Fonts/Supplemental/Khmer Sangam MN.ttf")) {
        // The probe decides per base: អ is rewritten (a font may already draw the others below, its own way).
        let _ = gids(&f, "អ៊ិ");
        let needs = f.triisap_u.get().copied().unwrap_or([false; 3]);
        assert!(needs[0], "the font leaves the triisap of អ above");
        for (need, (tri, u)) in needs.into_iter().zip([("អ៊ិ", "អុិ"), ("ស៊ី", "សុី"), ("ហ៊ឹ", "ហុឹ")]) {
            if need {
                assert_eq!(gids(&f, tri), gids(&f, u), "{tri}");
            }
        }
        // Other bases keep their triisap, and clusters index the original string.
        assert_ne!(gids(&f, "ប៊ិ"), gids(&f, "បុិ"));
        assert!(crate::shape(&f, "អ៊ិ", &[], |c| c).iter().all(|g| g.cluster == 0));
    }
    // The user's font folder, read at run time: no home directory (some Windows runners) skips these.
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(std::path::PathBuf::from);
    for font in ["!KhmerOSSystem.ttf", "Busra-R.ttf"] {
        if let Some(f) = home.as_ref().and_then(|h| face(&h.join("Library/Fonts").join(font))) {
            let _ = gids(&f, "អ៊ិ");
            assert_eq!(f.triisap_u.get().copied(), Some([false; 3]), "{font} already does the U-form");
        }
    }
}

/// A font holding only what shaping Khmer syllables with a triisap needs: glyphs for ប អ ស ហ, the
/// vowels ិ ី, the vowel U and the triisap (bases 500 wide, marks 0), and no layout tables. Nothing
/// moves the triisap below its base, so it stays above, as in the fonts the U-form fallback is for.
#[cfg(not(target_arch = "wasm32"))]
fn khmer_triisap_font() -> Vec<u8> {
    let chars = ['\u{1794}', '\u{179F}', '\u{17A0}', '\u{17A2}', '\u{17B7}', '\u{17B8}', '\u{17BB}', '\u{17CA}'];
    let glyphs = chars.len() as u16 + 1; // glyph 0 is .notdef
    let be16 = |vals: &[i32]| vals.iter().flat_map(|v| (*v as u16).to_be_bytes()).collect::<Vec<u8>>();
    let be32 = |vals: &[u32]| vals.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let mut head = be32(&[0x0001_0000, 0, 0, 0x5F0F_3CF5]);
    head.extend(be16(&[0, 1000]));
    head.extend([0u8; 16]); // created, modified
    head.extend(be16(&[0, 0, 0, 0, 0, 8, 2, 0, 0])); // bounds, macStyle, lowestRecPPEM, direction, loca format, glyph data format
    let mut hhea = be32(&[0x0001_0000]);
    hhea.extend(be16(&[800, -200, 0, 500, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, glyphs as i32]));
    let mut maxp = be32(&[0x0000_5000]);
    maxp.extend(be16(&[glyphs as i32]));
    // .notdef and the bases and vowel signs have an advance of 500, the combining marks none.
    let mut hmtx = Vec::new();
    for g in 0..glyphs {
        let mark = g >= 5; // the vowels, U and the triisap
        hmtx.extend(be16(&[if g == 0 || !mark { 500 } else { 0 }, 0]));
    }
    let mut cmap = be16(&[0, 1, 3, 10]);
    cmap.extend(be32(&[12]));
    cmap.extend(be16(&[12, 0]));
    cmap.extend(be32(&[16 + 12 * chars.len() as u32, 0, chars.len() as u32]));
    let mut sorted = chars.to_vec();
    sorted.sort();
    for c in sorted {
        let gid = 1 + chars.iter().position(|x| *x == c).unwrap_or(0) as u32;
        cmap.extend(be32(&[c as u32, c as u32, gid]));
    }
    mini_font(&[(*b"cmap", cmap), (*b"head", head), (*b"hhea", hhea), (*b"hmtx", hmtx), (*b"maxp", maxp)])
}

/// The U-form fallback through the real shaping path, with a face that leaves the triisap above
/// its base: no installed font is needed.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn khmer_triisap_is_shaped_as_the_u_form_for_a_face_that_leaves_it_above() {
    let face = make_face(
        FontBytes::Owned(std::sync::Arc::new(khmer_triisap_font())),
        0,
        "Khmer Test".into(),
        "Regular".into(),
        "Khmer Test".into(),
        Vec::new(),
    )
    .unwrap();
    let gids = |t: &str| crate::shape(&face, t, &[], |c| c).into_iter().map(|g| g.gid).collect::<Vec<_>>();
    // The probe finds the triisap above for every base, so each is shaped as the vowel U.
    assert_eq!(gids("អ៊ិ"), gids("អុិ"));
    assert_eq!(face.triisap_u.get().copied(), Some([true; 3]));
    assert_eq!(gids("ស៊ី"), gids("សុី"));
    assert_eq!(gids("ហ៊ិ"), gids("ហុិ"));
    // ប keeps its triisap, and so does a triisap with no upper vowel after it.
    assert_ne!(gids("ប៊ិ"), gids("បុិ"));
    assert_ne!(gids("អ៊"), gids("អុ"));
    // The text is untouched: the syllable is still one cluster of the original string.
    assert!(crate::shape(&face, "អ៊ិ", &[], |c| c).iter().all(|g| g.cluster == 0));
}
