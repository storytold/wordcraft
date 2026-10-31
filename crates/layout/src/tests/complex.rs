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

#[test]
fn thai_issue_1548_breaks_at_expected_word_boundaries() {
    let text = "[A] การประชุมคณะกรรมการบริหารโครงการพัฒนาระบบสารสนเทศเพื่อการบริหารจัดการทรัพยากรบุคคลขององค์กรปกครองส่วนท้องถิ่นได้มีมติเห็นชอบให้ดำเนินการจัดซื้อจัดจ้างเครื่องคอมพิวเตอร์แม่ข่ายพร้อมอุปกรณ์จัดเก็บข้อมูลสำรองภายในปีงบประมาณพุทธศักราชสองพันห้าร้อยหกสิบเก้าเพื่อรองรับการขยายตัวของระบบงานและลดความเสี่ยงจากการสูญหายของข้อมูลสำคัญซึ่งอาจส่งผลกระทบต่อการให้บริการประชาชน";
    let l = lay(&Document::from_text(text));
    let pl = para_layout(&l, 0);
    let first = pl.lines.first().and_then(|line| text.get(line.start..line.stop)).unwrap_or("");
    assert!(first.starts_with("[A] ") && first.len() > "[A] ".len(), "the [A] label must share its line with Thai text: {first:?}");
    let resource = text.find("บุคคลของ").unwrap_or(text.len());
    let server = text.find("พร้อมอุปกรณ์").unwrap_or(text.len());
    assert!(pl.clusters.iter().any(|c| c.end == resource && c.break_after), "expected the reported ทรัพยากร | บุคคล boundary at byte {resource}");
    assert!(pl.clusters.iter().any(|c| c.end == server && c.break_after), "expected the reported แม่ข่าย | พร้อม boundary at byte {server}");
    for word in ["ทรัพยากร", "อุปกรณ์", "ประชาชน"] {
        let start = text.find(word).unwrap_or(text.len());
        assert!(!pl.lines.iter().skip(1).any(|line| start < line.start && line.start < start + word.len()), "a line starts inside {word}");
    }
}

#[test]
fn thai_mixed_scripts_digits_and_punctuation_keep_unicode_boundaries() {
    let sample = "ไทย123 ภาษาไทย ๐๑๒๓, ไทย! ";
    let text = sample.repeat(55);
    let l = lay(&Document::from_text(&text));
    let pl = para_layout(&l, 0);
    assert!(pl.lines.len() > 1, "the mixed-script paragraph wraps");
    for word in ["123", "๐๑๒๓"] {
        let start = text.find(word).unwrap_or(text.len());
        assert!(!pl.lines.iter().skip(1).any(|line| start < line.start && line.start < start + word.len()), "a line starts inside {word}");
    }
    for marker in ["ภาษาไทย", "ไทย!"] {
        let boundary = text.find(marker).unwrap_or(text.len());
        assert!(
            pl.clusters.iter().any(|c| c.end == boundary && c.break_after),
            "the existing space/punctuation boundary before {marker:?} remains usable"
        );
    }
    for line in pl.lines.iter().skip(1) {
        assert!(crate::kinsoku::is_safe_line_break(&text, line.start), "mixed-script line starts at unsafe byte {}", line.start);
    }
}

#[test]
fn thai_wrap_off_and_emergency_breaks_keep_safe_units() {
    let text = "เก่งกำลัง";
    for (sample, off) in [("เก", 3), ("กิ่", 3), ("กิ่", 6), ("กำ", 3)] {
        assert!(!crate::kinsoku::is_safe_line_break(sample, off), "Thai grapheme/prefix-vowel boundary at {off} in {sample:?} is unsafe");
    }
    for sample in ["กิ่", "กำ"] {
        assert!(crate::kinsoku::is_safe_line_break(sample, sample.len()), "the complete Thai unit is safe");
    }
    let mut opportunities = Vec::new();
    crate::kinsoku::apply(text, &mut opportunities, true, false);
    // UTF-8 offsets: after เ (3) and ก before its tone mark (15) are unsafe; after each complete
    // grapheme is allowed, including after the sara am in กำ.
    assert!(!opportunities.contains(&3), "do not separate a preposed vowel from its consonant: {opportunities:?}");
    assert!(!opportunities.contains(&6), "do not separate a consonant from its tone mark: {opportunities:?}");
    assert!(opportunities.contains(&12), "the boundary after เก่ง is safe: {opportunities:?}");
    assert!(!opportunities.contains(&15), "do not separate ก from sara am: {opportunities:?}");
    assert!(opportunities.contains(&18), "the boundary after sara am is safe: {opportunities:?}");

    let text = "เก่งกำลังดี".repeat(16);
    for size in [11.0, 32.0] {
        for word_wrap in [true, false] {
            let mut d = Document::from_text("");
            let mut p = wordcraft_doc::Paragraph::with_text(&text, Default::default());
            p.props.word_wrap = Some(word_wrap);
            d.body = vec![wordcraft_doc::para_block(p)];
            d.last_section.margin_left = 285.0;
            d.last_section.margin_right = 285.0;
            d.format_range(&Pos::body(0, 0), &Pos::body(0, text.len()), &|c| c.size = Some(size)).unwrap();
            // Force run boundaries inside a Thai shaping unit, as can happen with imported formatting.
            d.format_range(&Pos::body(0, 6), &Pos::body(0, 9), &|c| {
                c.bold = Some(true);
                c.font = Some("WordCraft Thai Boundary Test".to_string());
            })
            .unwrap();
            let l = lay(&d);
            let pl = para_layout(&l, 0);
            assert!(pl.lines.len() > 1, "the narrow column should wrap at {size} pt");
            assert_eq!(pl.lines.last().map(|line| line.stop), Some(text.len()), "wrapping makes progress through the full paragraph");
            for line in pl.lines.iter().skip(1) {
                assert!(
                    crate::kinsoku::is_safe_line_break(&text, line.start),
                    "line starts at unsafe byte {} (word_wrap={word_wrap}, size={size})",
                    line.start
                );
            }
        }
    }
}

#[test]
fn thai_safe_unit_wider_than_column_overflows_and_makes_progress() {
    let text = "กำก";
    let mut d = Document::from_text(text);
    d.last_section.margin_left = 297.0;
    d.last_section.margin_right = 297.0;
    d.format_range(&Pos::body(0, 0), &Pos::body(0, text.len()), &|c| c.size = Some(72.0)).unwrap();
    let l = lay(&d);
    let pl = para_layout(&l, 0);
    let first = pl.clusters.first().map(|c| c.adv).unwrap_or(0.0);
    assert!(first > 18.0, "the first safe Thai unit must exceed the minimum text column: {first}");
    assert_eq!(pl.lines.first().map(|line| line.stop), Some("กำ".len()));
    assert_eq!(pl.lines.get(1).map(|line| line.start), Some("กำ".len()));
    assert_eq!(pl.lines.last().map(|line| line.stop), Some(text.len()), "overflow still advances through all text");
}

#[test]
fn thai_emergency_wrapping_preserves_latin_ligatures() {
    let text = "ffiffiffi ภาษาไทย";
    let mut d = Document::from_text(text);
    d.format_range(&Pos::body(0, 0), &Pos::body(0, 9), &|c| c.font = Some("Source Serif 4".into())).unwrap();
    d.last_section.margin_left = 297.0;
    d.last_section.margin_right = 297.0;
    let l = lay(&d);
    let pl = para_layout(&l, 0);
    assert!(pl.clusters.iter().any(|c| c.start < 9 && c.g1 <= c.g0), "the Latin prefix contains ligatures");
    for line in &pl.lines {
        if let Some(next) = pl.clusters.get(line.c1).filter(|c| c.start < 9) {
            assert!(next.g1 > next.g0, "Thai emergency wrapping must not start a line inside a Latin ligature");
        }
    }
    assert_eq!(pl.lines.last().map(|line| line.stop), Some(text.len()), "overwide ligatures still make progress");
}

#[test]
fn thai_dictionary_context_skips_hidden_text_and_tracked_deletions() {
    let text = "การประชุXmคณะกรรมการบริหารโครงการ";
    let hidden = text.find('X').unwrap_or(text.len());
    let boundary = text.find("กรรมการ").unwrap_or(text.len());
    for deleted in [false, true] {
        let mut d = Document::from_text(text);
        d.format_range(&Pos::body(0, hidden), &Pos::body(0, hidden + 1), &|c| {
            if deleted {
                c.del = Some(0);
            } else {
                c.hidden = Some(true);
            }
        })
        .unwrap();
        let opts = LayoutOptions { hide_deleted: deleted, ..Default::default() };
        let mut cache = LayoutCache::new();
        let l = layout(&d, &mut cache, &opts);
        let pl = para_layout(&l, 0);
        assert!(
            pl.clusters.iter().any(|c| c.end == boundary && c.break_after),
            "visible Thai context maps its word boundary back to source byte {boundary} (deleted={deleted})"
        );
    }
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
