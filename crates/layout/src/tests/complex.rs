//! Complex scripts that reorder and stack (#604): Myanmar, Khmer, Devanagari. Cluster and line
//! breaking tests don't depend on installed fonts (a missing font draws .notdef boxes, but the
//! clusters and breaks are the same); shaping tests use a system font when one is installed.

use crate::*;
use wordcraft_doc::{Pos, StoryRef};

fn lay(doc: &Document) -> DocLayout {
    let mut c = LayoutCache::new();
    layout(doc, &mut c, &LayoutOptions::default())
}

/// Body paragraph `i`'s layout.
fn para_layout(l: &DocLayout, i: u32) -> &ParaLayout {
    for p in &l.pages {
        for it in &p.items {
            if let Placed::Lines { story: StoryRef::Body, path, para, .. } = it
                && path.0 == vec![i]
            {
                return para;
            }
        }
    }
    panic!("paragraph {i} not laid out");
}

/// "မြန်မာစာ": the syllables မြ | န် | မာ | စာ.
const MYANMAR: &str = "\u{1019}\u{103C}\u{1014}\u{103A}\u{1019}\u{102C}\u{1005}\u{102C}";

#[test]
fn myanmar_syllables_are_single_clusters() {
    let l = lay(&Document::from_text(MYANMAR));
    let pl = para_layout(&l, 0);
    let starts: Vec<usize> = pl.clusters.iter().map(|c| c.start).collect();
    // Grapheme clusters would also start at the aa signs (bytes 15 and 21), splitting မာ and စာ.
    assert_eq!(starts, vec![0, 6, 12, 18]);
    // The caret can't land inside a syllable: every caret offset is a cluster start or the end.
    let offs = pl.line_offsets(0);
    assert!(!offs.contains(&15) && !offs.contains(&21), "{offs:?}");
    // Stacked consonants (virama) and Khmer subscripts (coeng) too.
    let l = lay(&Document::from_text("\u{1019}\u{1039}\u{1018}\u{102C} \u{1781}\u{17D2}\u{1798}\u{17C2}\u{179A}"));
    let pl = para_layout(&l, 0);
    let starts: Vec<usize> = pl.clusters.iter().map(|c| c.start).collect();
    assert_eq!(starts, vec![0, 12, 13, 25]);
}

#[test]
fn myanmar_without_spaces_wraps_between_syllables() {
    let text = MYANMAR.repeat(80);
    let l = lay(&Document::from_text(&text));
    let pl = para_layout(&l, 0);
    assert!(pl.lines.len() > 1, "a long Myanmar paragraph wraps");
    let breaks = wordcraft_doc::cluster::myanmar_breaks(&text);
    // A line may end before each syllable: before မာ and စာ, never before န် (a final).
    for c in &pl.clusters {
        assert_eq!(c.break_after, breaks.contains(&c.end), "break after {}", c.end);
    }
    assert!(pl.clusters.iter().any(|c| c.break_after && c.end == 12) && !pl.clusters.iter().any(|c| c.break_after && c.end == 6));
    for line in pl.lines.iter().skip(1) {
        let start = pl.clusters[line.c0].start;
        assert!(breaks.contains(&start), "line starts at syllable {start}");
    }
    // Lines are full: a syllable break, not an emergency break after one cluster.
    assert!(pl.lines[0].c1 - pl.lines[0].c0 > 10);
}

/// A face with Myanmar letters (an installed system font), or `None` to skip.
fn myanmar_face() -> Option<wordcraft_fonts::FaceRef> {
    wordcraft_fonts::FontDb::global().fallback_for('\u{1000}', 0).map(|f| wordcraft_fonts::FaceRef::of(&f)).filter(|f| f.has_layout_tables())
}

#[test]
fn myanmar_prevowel_is_drawn_before_its_consonant() {
    let Some(face) = myanmar_face() else {
        eprintln!("skipped: no Myanmar font with OpenType tables installed");
        return;
    };
    // ကေ: in memory consonant first, on screen the vowel sign ေ first.
    let ka = face.glyph_for('\u{1000}');
    let g = wordcraft_fonts::shape(&face, "\u{1000}\u{1031}", &[], |c| c);
    assert_eq!(g.len(), 2, "{g:?}");
    assert_eq!(g[1].gid, ka, "the consonant is drawn second: {g:?}");
    assert!(g.iter().all(|x| x.cluster == 0), "one cluster: {g:?}");
    // Itemised by script: the same syllable after Latin text in the same face shapes the same
    // (shaped as one Latin run, the vowel sign would stay after the consonant).
    if face.covers('H') {
        let mixed = wordcraft_fonts::shape(&face, "Hi \u{1000}\u{1031}", &[], |c| c);
        let tail: Vec<u32> = mixed.iter().filter(|x| x.cluster >= 3).map(|x| x.gid).collect();
        assert_eq!(tail, g.iter().map(|x| x.gid).collect::<Vec<_>>());
    }
}

#[test]
fn myanmar_in_a_latin_font_falls_back_to_a_regular_myanmar_face() {
    if myanmar_face().is_none() {
        eprintln!("skipped: no Myanmar font with OpenType tables installed");
        return;
    }
    let l = lay(&Document::from_text(MYANMAR));
    let pl = para_layout(&l, 0);
    let faces: Vec<_> = pl.clusters.iter().map(|c| pl.styles[c.style as usize].face).collect();
    assert!(faces.windows(2).all(|w| w[0].id() == w[1].id()), "one face for the word");
    let f = faces[0];
    assert!(f.has_layout_tables(), "{} can reorder Myanmar", f.family);
    assert!(!f.italic && (f.weight - 400.0).abs() <= 100.0, "a regular weight, not {} {}", f.family, f.style);
    assert!(pl.glyphs.iter().all(|g| g.gid != 0), "no missing glyphs");
    // The caret after the first syllable sits after its glyphs.
    let x6 = l.caret(&Pos::body(0, 6)).map(|c| c.x).unwrap_or(0.0);
    let x0 = l.caret(&Pos::body(0, 0)).map(|c| c.x).unwrap_or(0.0);
    assert!(x6 > x0);
}

/// The issue's run (#1549): Thai letters and digits, Latin letters, digits and the baht sign.
const THAI_RUN: &str = "[D] น้ำ ค่ำ ปั้น ฝั่ง ปู่ ๑๒๓๔๕๖๗๘๙๐ ฿1,250.50";

/// Is a font with Thai letters installed? `false` skips the test.
fn thai_font_installed() -> bool {
    wordcraft_fonts::FontDb::global().fallback_for('\u{0E19}', 0).is_some()
}

/// Is a Thai font with Latin letters and digits installed (Tahoma, Leelawadee UI, Thonburi, the
/// TLWG fonts…)? Noto Sans Thai alone has none, so no face covers [`THAI_RUN`]: `false` skips.
fn thai_latin_font_installed() -> bool {
    let face = wordcraft_fonts::word::resolve("TH Sarabun New", false, false).face;
    THAI_RUN.chars().filter(|c| !c.is_whitespace()).all(|c| face.covers(c))
}

/// Body paragraph 0 of `text`, its run set in `font` at 16 pt (`None` keeps the document's
/// default font), and the face of every cluster: (cluster start, face).
fn cluster_faces(text: &str, font: Option<&str>) -> Vec<(usize, wordcraft_fonts::FaceRef)> {
    let mut d = Document::from_text(text);
    let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p.runs[0].props.font = font.map(Into::into);
    p.runs[0].props.size = Some(16.0);
    let l = lay(&d);
    let pl = para_layout(&l, 0);
    pl.clusters.iter().map(|c| (c.start, pl.styles[c.style as usize].face)).collect()
}

#[test]
fn a_missing_thai_font_draws_the_whole_run_in_one_thai_face() {
    if !thai_latin_font_installed() {
        eprintln!("skipped: no Thai font with Latin letters installed");
        return;
    }
    // Word substitutes one font (Cordia New) for a missing Thai font, so the Latin letters, the
    // digits and the baht sign keep the Thai letters' face and size. A Thai name the lists don't
    // spell out ("TH Niramit AS") is matched by its "TH " prefix and substituted the same way.
    let db = wordcraft_fonts::FontDb::global();
    for font in ["TH Sarabun New", "Angsana New", "TH Niramit AS"] {
        if db.has_family(font) {
            continue;
        }
        let faces = cluster_faces(THAI_RUN, Some(font));
        let first = faces[0].1;
        let names: Vec<&str> = faces.iter().map(|(_, f)| f.family.as_str()).collect();
        assert!(faces.iter().all(|(_, f)| f.id() == first.id()), "{font}: one face for the run, not {names:?}");
        assert!(first.covers('\u{0E19}') && first.covers('D') && first.covers('1') && first.covers('\u{0E3F}'), "{font} → {}", first.family);
    }
}

#[test]
fn a_thai_substitute_without_latin_hands_the_run_to_one_face_covering_it() {
    if !thai_latin_font_installed() {
        eprintln!("skipped: no Thai font with Latin letters installed");
        return;
    }
    // When the substitute lacks the run's Thai letters (no listed Thai font is installed, so the
    // Latin fallback family stands in) or its Latin ones, the run moves to one face covering all.
    let latin = wordcraft_fonts::word::resolve("Source Sans 3", false, false).face;
    let face = crate::para::thai_run_face("TH Sarabun New", true, latin, THAI_RUN, false, false).expect("one face covers the run");
    assert_ne!(face.id(), latin.id());
    for c in THAI_RUN.chars().filter(|c| !c.is_whitespace()) {
        assert!(face.covers(c), "{} covers {c}", face.family);
    }
    // Only for a Thai family that was substituted.
    assert!(crate::para::thai_run_face("Calibri", true, latin, THAI_RUN, false, false).is_none());
    assert!(crate::para::thai_run_face("TH Sarabun New", false, latin, THAI_RUN, false, false).is_none());
    // A substitute that already covers the run keeps it.
    assert!(crate::para::thai_run_face("TH Sarabun New", true, face, THAI_RUN, false, false).is_none());
    // Text that is neither Thai nor Latin doesn't make the run move.
    assert!(crate::para::thai_run_face("TH Sarabun New", true, latin, "abc \u{2603}", false, false).is_none());
}

#[test]
fn latin_text_in_a_missing_latin_font_stays_in_its_substitute_beside_thai() {
    if !thai_font_installed() {
        eprintln!("skipped: no Thai font installed");
        return;
    }
    // Word keeps Latin text in the Latin font's substitute; only the Thai (or Myanmar) letters
    // fall back. A missing Latin family (and the default body font, Aptos, wherever it is
    // missing) must not move "Hello" into a Thai face.
    let myanmar = myanmar_face().is_some();
    for font in [Some("No Such Latin Font 1549"), None] {
        let requested = font.unwrap_or(wordcraft_doc::styles::BODY_FONT);
        let latin = wordcraft_fonts::word::resolve(requested, false, false).face;
        for text in ["Hello \u{0E2A}\u{0E27}\u{0E31}\u{0E2A}\u{0E14}\u{0E35}", "Hello \u{1019}\u{103C}\u{1014}\u{103A}\u{1019}\u{102C}"] {
            if text.contains('\u{1019}') && !myanmar {
                continue;
            }
            let faces = cluster_faces(text, font);
            let hello: Vec<_> = faces.iter().filter(|(at, _)| *at < 5).collect();
            assert_eq!(hello.len(), 5, "{faces:?}");
            assert!(
                hello.iter().all(|(_, f)| f.id() == latin.id()),
                "{requested}: Hello in {}, not {:?}",
                latin.family,
                hello.iter().map(|(_, f)| f.family.as_str()).collect::<Vec<_>>()
            );
            assert!(faces.iter().any(|(at, f)| *at >= 6 && f.id() != latin.id()), "the other script falls back");
        }
    }
}

#[test]
fn thai_in_an_installed_latin_font_keeps_the_latin_face_for_latin() {
    if !thai_font_installed() {
        eprintln!("skipped: no Thai font installed");
        return;
    }
    // The family exists, so nothing is substituted: Latin stays in it and only the Thai letters
    // fall back, character by character, as before.
    let faces = cluster_faces(THAI_RUN, Some("Source Sans 3"));
    let latin = wordcraft_fonts::word::resolve("Source Sans 3", false, false).face;
    assert!(faces.iter().any(|(_, f)| f.id() == latin.id()), "Latin clusters stay in Source Sans 3");
    assert!(faces.iter().any(|(_, f)| f.id() != latin.id() && f.covers('\u{0E19}')), "Thai clusters fall back to a Thai face");
    assert_eq!(faces[0].1.id(), latin.id(), "[D] opens the run in Source Sans 3");
}

#[test]
fn a_missing_latin_font_with_a_thai_complex_script_font_keeps_both() {
    if !thai_font_installed() {
        eprintln!("skipped: no Thai font installed");
        return;
    }
    // A run with its own complex-script font (`w:cs`) draws the Thai letters in that font's
    // substitute and the Latin letters in the Latin font's substitute, as Word does.
    let mut d = Document::from_text(THAI_RUN);
    let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p.runs[0].props.font = Some("No Such Latin Font 1549".into());
    p.runs[0].props.font_cs = Some("TH Sarabun New".into());
    let l = lay(&d);
    let pl = para_layout(&l, 0);
    let faces: Vec<_> = pl.clusters.iter().map(|c| pl.styles[c.style as usize].face).collect();
    let latin = wordcraft_fonts::word::resolve("No Such Latin Font 1549", false, false).face;
    let thai = wordcraft_fonts::word::resolve("TH Sarabun New", false, false).face;
    assert_eq!(faces[0].id(), latin.id(), "[D] in the Latin substitute");
    assert!(faces.iter().any(|f| f.id() == thai.id()), "Thai letters in the Thai substitute {}", thai.family);
}

#[test]
fn a_missing_roman_thai_font_is_not_drawn_in_cambria() {
    // Word draws a missing font the font table classes as roman in Cambria, but a missing Thai
    // font keeps its Thai substitute: the same face whether or not the table calls it roman.
    let db = wordcraft_fonts::FontDb::global();
    if db.has_family("TH Sarabun New") {
        eprintln!("skipped: TH Sarabun New is installed");
        return;
    }
    let face = |roman: bool| {
        let mut d = Document::from_text("\u{0E19}\u{0E49}\u{0E33} Thai");
        if roman {
            d.settings.roman_fonts = vec!["TH Sarabun New".into()];
        }
        let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
        p.runs[0].props.font = Some("TH Sarabun New".into());
        let l = lay(&d);
        let pl = para_layout(&l, 0);
        pl.styles[pl.clusters.last().expect("a cluster").style as usize].face
    };
    let cambria = wordcraft_fonts::word::resolve("Cambria", false, false).face;
    assert_ne!(face(true).id(), cambria.id());
    assert_eq!(face(true).id(), face(false).id());
}

/// `w14:ligatures` (Word writes `historicalDiscretional` on whole Khmer documents) turns `liga`
/// and `clig` off, which breaks Khmer OS syllables: Khmer text is shaped the same either way.
#[test]
fn khmer_ignores_the_ligature_setting() {
    use wordcraft_doc::props::{CharProps, Ligatures};
    let text = "\u{179F}\u{17CA}\u{17BE}\u{1794}\u{17A2}\u{1784}\u{17D2}\u{1780}\u{17C1}\u{178F} \u{1795}\u{17D2}\u{178F}\u{1793}\u{17D2}\u{1791}\u{17B6}\u{1791}\u{17C4}\u{179F}";
    let gids = |ligatures: Option<Ligatures>| {
        let chr = CharProps { font: Some("Khmer OS Siemreap".into()), size: Some(12.0), ligatures, ..Default::default() };
        let mut d = Document::new();
        d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(text, chr))];
        let l = lay(&d);
        para_layout(&l, 0).glyphs.iter().map(|g| g.gid).collect::<Vec<_>>()
    };
    let hd = Ligatures { historical: true, discretional: true, ..Default::default() };
    assert_eq!(gids(Some(hd)), gids(None));
    assert_eq!(gids(Ligatures::from_ooxml("none")), gids(None));
}

const KHMER_SENTENCE: &str = "ខ្ញុំស្រឡាញ់ភាសាខ្មែរ";

#[test]
fn khmer_dictionary_wraps_across_formatting_without_changing_text() {
    if !wordcraft_proof::khmer::data_available() {
        eprintln!("skipped: CRAFT_KHMER_DIR was not supplied at build time");
        return;
    }
    let text = KHMER_SENTENCE.repeat(40);
    let breaks = wordcraft_proof::khmer::line_breaks(&text);
    assert!(breaks.len() > 40, "fixture contains multiple dictionary words");
    let mut d = Document::from_text(&text);
    // A run boundary inside a word must neither create nor remove lexical opportunities.
    d.format_range(&Pos::body(0, 3), &Pos::body(0, 9), &|c| c.bold = Some(true)).unwrap();
    d.settings.auto_hyphenation = true;
    d.last_section.page_w = 344.0; // 200 pt of text; all fixture words fit.
    let before = serde_json::to_string(&d).unwrap();
    let l = lay(&d);
    let pl = para_layout(&l, 0);
    assert!(pl.lines.len() > 3);
    for line in &pl.lines {
        assert!(line.hyphen.is_none(), "Khmer dictionary breaks never insert hyphens");
        if line.start > 0 {
            assert!(breaks.contains(&line.start), "line starts at dictionary boundary {}", line.start);
        }
        for off in pl.line_offsets(pl.line_of(line.start)) {
            assert!(wordcraft_doc::cluster::is_cluster_boundary(&text, off));
        }
    }
    for cluster in &pl.clusters {
        if cluster.break_after {
            assert!(breaks.contains(&cluster.end), "no formatting-derived break at {}", cluster.end);
        }
    }
    assert_eq!(serde_json::to_string(&d).unwrap(), before, "layout preserves raw document and runs");
    assert_eq!(pl.text_len, text.len());
}

#[test]
fn khmer_emergency_wrap_keeps_coeng_and_formatted_clusters_whole() {
    // Deliberately not a dictionary word, and much wider than each line.
    let text = "ខ្មែ".repeat(50);
    let mut d = Document::from_text(&text);
    d.format_range(&Pos::body(0, 3), &Pos::body(0, 9), &|c| c.size = Some(30.0)).unwrap();
    for width in [1.0, 18.0, 35.0] {
        d.last_section.page_w = 144.0 + width;
        let l = lay(&d);
        let pl = para_layout(&l, 0);
        assert!(pl.lines.len() > 1);
        for line in &pl.lines {
            assert!(line.stop > line.start, "emergency wrapping makes progress");
            assert!(wordcraft_doc::cluster::is_cluster_boundary(&text, line.start));
            assert!(wordcraft_doc::cluster::is_cluster_boundary(&text, line.stop));
            assert!(line.hyphen.is_none());
        }
    }
}

#[test]
fn khmer_joiners_zwsp_and_mixed_punctuation_keep_original_offsets() {
    for text in ["ខ្មែរ\u{2060}ភាសា", "ខ្មែរ\u{200b}ភាសា", "Hello ខ្ញុំស្រឡាញ់ភាសាខ្មែរ។ Latin"]
    {
        let mut d = Document::from_text(text);
        d.last_section.page_w = 162.0; // force even emergency boundaries to be exercised
        let before = serde_json::to_string(&d).unwrap();
        let l = lay(&d);
        let pl = para_layout(&l, 0);
        for line in &pl.lines {
            assert!(text.is_char_boundary(line.start) && text.is_char_boundary(line.stop));
            assert!(wordcraft_doc::cluster::is_cluster_boundary(text, line.start));
            assert!(wordcraft_doc::cluster::is_cluster_boundary(text, line.stop));
            assert!(!text.get(..line.start).is_some_and(|s| s.ends_with('\u{2060}')));
            assert!(!text.get(line.start..).is_some_and(|s| s.starts_with('\u{2060}')));
            assert!(line.hyphen.is_none());
        }
        if let Some(at) = text.find('។') {
            assert!(!pl.clusters.iter().any(|c| c.end == at && c.break_after), "Khmer full stop stays with its word");
        }
        if let Some(at) = text.find('\u{200b}') {
            assert!(pl.clusters.iter().any(|c| c.end == at + '\u{200b}'.len_utf8() && c.break_after), "explicit ZWSP remains a break");
        }
        assert_eq!(serde_json::to_string(&d).unwrap(), before);
    }
}

#[test]
fn installed_khmer_fallback_shapes_without_missing_glyphs() {
    let Some(face) = wordcraft_fonts::FontDb::global().fallback_for('ខ', 0).filter(|f| f.has_layout_tables()) else {
        eprintln!("skipped: no Khmer system font with OpenType tables installed");
        return;
    };
    assert!(face.has_layout_tables(), "Khmer fallback requires OpenType shaping");
    let shaped = wordcraft_fonts::shape(&face, "ខ្មែ", &[], |c| c);
    assert!(!shaped.is_empty() && shaped.iter().all(|g| g.gid != 0));
    assert!(shaped.iter().all(|g| g.cluster == 0), "COENG syllable is one shaped cluster: {shaped:?}");
    let l = lay(&Document::from_text(KHMER_SENTENCE));
    let pl = para_layout(&l, 0);
    assert!(!pl.glyphs.is_empty());
    assert!(pl.glyphs.iter().all(|g| g.gid != 0), "no missing Khmer glyphs");
    assert!(pl.clusters.iter().all(|c| pl.styles[c.style as usize].face.has_layout_tables()));
}

#[test]
fn khmer_repetition_sign_stays_with_preceding_word_across_widths() {
    // Regression from the independent PR comparison: an ICU adapter allowed
    // the repetition sign to begin a line at 65 pt. Check final layout, not
    // merely candidate offsets, over a range of available widths.
    let text = "ខ្មែរអ្វីៗខ្មែរអ្វីៗខ្មែរ";
    for width in (40..=200).step_by(5) {
        let mut doc = Document::from_text(text);
        doc.last_section.page_w = width as f32 + 72.0;
        doc.last_section.margin_left = 36.0;
        doc.last_section.margin_right = 36.0;
        let p = doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
        for run in &mut p.runs {
            run.props.font = Some("Khmer Sangam MN".into());
            run.props.size = Some(14.0);
        }
        let before = serde_json::to_string(&doc).unwrap();
        let result = lay(&doc);
        let p = para_layout(&result, 0);
        assert!(!p.lines.is_empty());
        if width == 40 {
            assert!(p.lines.len() > 1, "fixture must exercise wrapping");
        }
        for line in &p.lines {
            assert!(!text.get(line.start..).is_some_and(|s| s.starts_with('ៗ')), "repetition sign at line start, width {width}");
            assert!(line.hyphen.is_none());
            assert!(wordcraft_doc::cluster::is_cluster_boundary(text, line.start));
        }
        assert_eq!(serde_json::to_string(&doc).unwrap(), before);
    }
}

#[test]
fn khmer_manual_zwsp_wraps_inside_a_model_word_across_formatting() {
    let text = "មូល\u{200b}ដ្ឋាន";
    let cut = "មូល\u{200b}".len();
    let mut doc = Document::from_text(text);
    doc.last_section.margin_left = 36.0;
    doc.last_section.margin_right = 36.0;
    doc.last_section.page_w = 1000.0;
    let p = doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    for run in &mut p.runs {
        run.props.font = Some("Khmer Sangam MN".into());
        run.props.size = Some(14.0);
    }
    // Keep the marker at a formatting edge; no text rewrite should be needed.
    doc.format_range(&Pos::body(0, cut), &Pos::body(0, text.len()), &|c| c.bold = Some(true)).unwrap();
    let wide = lay(&doc);
    let p = para_layout(&wide, 0);
    assert_eq!(p.lines.len(), 1, "ZWSP is optional, not a forced newline");
    let left: f32 = p.clusters.iter().filter(|c| c.end <= cut).map(|c| c.adv).sum();
    let right: f32 = p.clusters.iter().filter(|c| c.start >= cut).map(|c| c.adv).sum();
    assert!(left > 0.0 && right > 0.0);
    doc.last_section.page_w = 72.0 + left.max(right) + left.min(right) * 0.25;
    let saved = serde_json::to_string(&doc).unwrap();
    let narrow = lay(&doc);
    let p = para_layout(&narrow, 0);
    assert_eq!(p.lines.len(), 2);
    assert_eq!(p.lines[1].start, cut, "manual marker supplies the actual wrap");
    assert!(p.lines.iter().all(|l| l.hyphen.is_none()));
    assert_eq!(serde_json::to_string(&doc).unwrap(), saved);
}

#[test]
fn khmer_tabs_and_controls_keep_formatting_and_caret_offsets() {
    let text = "Label\tមូល\u{200b}ដ្ឋាន ខ្មែរ\u{2060}ភាសា ខ្ញុំស្រឡាញ់ភាសាខ្មែរ";
    let marker = text.find('\u{200b}').unwrap() + '\u{200b}'.len_utf8();
    let joined = crate::dictionary::joined_boundaries(text);
    for align in [wordcraft_doc::props::Align::Left, wordcraft_doc::props::Align::Justify, wordcraft_doc::props::Align::Distribute] {
        for width in [100.0, 150.0, 220.0] {
            let mut doc = Document::from_text(text);
            doc.last_section.page_w = 72.0 + width;
            doc.last_section.margin_left = 36.0;
            doc.last_section.margin_right = 36.0;
            doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().props.align = Some(align);
            doc.format_range(&Pos::body(0, marker), &Pos::body(0, text.len()), &|c| c.bold = Some(true)).unwrap();
            let saved = serde_json::to_value(&doc).unwrap();
            let result = lay(&doc);
            let p = para_layout(&result, 0);
            if width == 100.0 {
                assert!(p.lines.len() > 1, "fixture wraps in {align:?} at {width}");
            }
            assert!(p.clusters.iter().any(|c| c.end == marker && c.break_after), "formatting retains explicit ZWSP");
            let carets: Vec<usize> = (0..p.lines.len()).flat_map(|i| p.line_offsets(i)).collect();
            for boundary in &joined {
                assert!(carets.contains(boundary), "WJ remains editable: {boundary}");
            }
            for line in &p.lines {
                assert!(!joined.contains(&line.start), "WJ prevents emergency wrapping in {align:?} at {width}");
                assert!(wordcraft_doc::cluster::is_cluster_boundary(text, line.start));
                assert!(line.xs.iter().all(|x| x.is_finite()));
                assert!(line.hyphen.is_none());
            }
            // The tab's label is not moved by justification of the text after it.
            assert_eq!(p.x_of(0, 0), p.lines.first().map(|line| line.left));
            assert_eq!(serde_json::to_value(&doc).unwrap(), saved);
        }
    }
}
