//! The caret inside a ligature (#1974): a glyph that draws several letters (fi, ffi, lam-alef)
//! gives each letter an even share of its advance, so the caret, clicks and selection edges can
//! land between the letters. A single grapheme cluster (letter plus combining mark) stays atomic.

use super::*;
use wordcraft_doc::{Pos, StoryRef};

fn lay(doc: &Document) -> DocLayout {
    let mut c = LayoutCache::new();
    layout(doc, &mut c, &LayoutOptions::default())
}

fn para_layout(l: &DocLayout) -> &ParaLayout {
    for p in &l.pages {
        for it in &p.items {
            if let Placed::Lines { story: StoryRef::Body, para, .. } = it {
                return para;
            }
        }
    }
    panic!("no paragraph laid out");
}

/// `text` in a bundled font that draws fi and ffi as ligatures, so the tests don't depend on the
/// installed fonts (Liberation Sans, the usual Linux stand-in for the default font, draws none).
fn doc(text: &str) -> Document {
    let mut d = Document::from_text(text);
    let end = d.body.first().and_then(|b| b.as_para()).map_or(0, |p| p.text.len());
    let _ = d.format_range(&Pos::body(0, 0), &Pos::body(0, end), &|c| c.font = Some(LIGATURE_FONT.into()));
    d
}

const LIGATURE_FONT: &str = "Source Serif 4";

fn caret_x(l: &DocLayout, off: usize) -> f32 {
    l.caret(&Pos::body(0, off)).map(|c| c.x).unwrap_or(f32::NAN)
}

/// Does the default font draw `fi` as one glyph? (The tests below need a ligature to mean anything.)
fn has_fi_ligature(l: &DocLayout) -> bool {
    let pl = para_layout(l);
    pl.clusters.len() >= 2 && pl.clusters.first().is_some_and(|c| c.g1 > c.g0) && pl.glyphs.len() < pl.clusters.len()
}

#[test]
fn caret_between_the_letters_of_a_ligature_is_inside_it() {
    let l = lay(&doc("fi"));
    assert!(has_fi_ligature(&l), "the default font should draw fi as a ligature");
    let (a, b, c) = (caret_x(&l, 0), caret_x(&l, 1), caret_x(&l, 2));
    assert!(a < b && b < c, "caret x before, inside and after fi: {a} {b} {c}");
    assert!((b - a - (c - b)).abs() < 0.05, "an even split: {a} {b} {c}");
}

/// Width of "fi" (both letters) with character spacing `sp` points per letter.
fn fi_width_with_spacing(sp: Option<f32>) -> (DocLayout, f32) {
    let mut d = doc("fi");
    d.format_range(&Pos::body(0, 0), &Pos::body(0, 2), &|c| c.spacing = sp).unwrap();
    let l = lay(&d);
    let w = caret_x(&l, 2) - caret_x(&l, 0);
    (l, w)
}

#[test]
fn character_spacing_inside_a_ligature_is_kept() {
    let (l0, plain) = fi_width_with_spacing(None);
    assert!(has_fi_ligature(&l0), "the default font should draw fi as a ligature");
    for sp in [3.0_f32, -0.5] {
        let (l, w) = fi_width_with_spacing(Some(sp));
        assert!(has_fi_ligature(&l), "fi stays a ligature with spacing {sp}");
        assert!((w - (plain + 2.0 * sp)).abs() < 0.05, "spacing {sp}: width {w}, expected {}", plain + 2.0 * sp);
        let (a, b, c) = (caret_x(&l, 0), caret_x(&l, 1), caret_x(&l, 2));
        assert!(a < b && b < c, "caret inside the ligature with spacing {sp}: {a} {b} {c}");
    }
}

#[test]
fn clicks_inside_a_ligature_land_between_its_letters() {
    let l = lay(&doc("fi"));
    let (a, b, c) = (caret_x(&l, 0), caret_x(&l, 1), caret_x(&l, 2));
    let cr = l.caret(&Pos::body(0, 1)).unwrap();
    let y = cr.top + cr.height / 2.0;
    let hit = |x: f32| l.hit(cr.page, x, y, StoryRef::Body).map(|p| p.off);
    assert_eq!(hit(b + 0.2), Some(1));
    assert_eq!(hit(b - 0.2), Some(1));
    assert_eq!(hit(a + 0.2), Some(0));
    assert_eq!(hit(c - 0.2), Some(2));
}

#[test]
fn every_letter_of_a_three_letter_ligature_has_its_own_caret() {
    let l = lay(&doc("ffi"));
    assert!(has_fi_ligature(&l), "the default font should draw ffi as a ligature");
    let xs: Vec<f32> = (0..=3).map(|o| caret_x(&l, o)).collect();
    assert!(xs.windows(2).all(|w| w[0] < w[1]), "{xs:?}");
}

#[test]
fn a_combining_mark_stays_in_its_letters_cluster() {
    let l = lay(&doc("fi\u{301}"));
    let pl = para_layout(&l);
    let offs = pl.line_offsets(0);
    assert!(!offs.contains(&2), "no caret between i and its accent: {offs:?}");
}

#[test]
fn selection_edges_inside_a_ligature_use_the_same_positions() {
    let d = doc("fi");
    let l = lay(&d);
    let rects = l.selection_rects(&d, &Pos::body(0, 0), &Pos::body(0, 1), 0);
    let r = rects.first().map(|(_, r)| *r).unwrap();
    let (a, b, c) = (caret_x(&l, 0), caret_x(&l, 1), caret_x(&l, 2));
    assert!((r.x - a).abs() < 0.05 && (r.x + r.w - b).abs() < 0.05, "{r:?} {a} {b}");
    assert!(r.x + r.w < c - 0.05, "the highlight of f stops inside the ligature: {r:?} {c}");
}

/// "لا" laid out in an installed Arabic font that draws lam-alef as one glyph (Noto Naskh Arabic,
/// the usual fallback, draws two), or `None` when no such font is installed.
fn lam_alef_ligature() -> Option<DocLayout> {
    ["Noto Sans Arabic", "Noto Kufi Arabic", "Amiri", "Scheherazade New", "DejaVu Sans", "Arial", "Geeza Pro", "Tahoma"].into_iter().find_map(
        |family| {
            let mut d = Document::from_text("لا");
            d.format_range(&Pos::body(0, 0), &Pos::body(0, 4), &|c| {
                c.font = Some(family.into());
                c.font_cs = Some(family.into());
            })
            .ok()?;
            let l = lay(&d);
            let pl = para_layout(&l);
            let glyphs: Vec<u32> = pl.clusters.iter().map(|c| c.g1.saturating_sub(c.g0)).collect();
            (glyphs == [1, 0] && pl.styles.iter().any(|s| s.face.family == family)).then_some(l)
        },
    )
}

#[test]
fn a_right_to_left_ligature_splits_mirrored() {
    let Some(l) = lam_alef_ligature() else {
        eprintln!("skipped: no lam-alef ligature font");
        return;
    };
    // Logical offsets 0 (before lam), 2 (between lam and alef), 4 (after alef); the text runs leftward.
    let (a, b, c) = (caret_x(&l, 0), caret_x(&l, 2), caret_x(&l, 4));
    assert!(a > b && b > c, "caret x runs right to left through the ligature: {a} {b} {c}");
    assert!(((a - b) - (b - c)).abs() < 0.05, "an even split: {a} {b} {c}");
}

#[test]
fn a_right_to_left_ligature_is_drawn_from_its_left_edge() {
    // The glyph stays on lam, the rightmost letter; it must still start at alef's left edge.
    let Some(l) = lam_alef_ligature() else {
        eprintln!("skipped: no lam-alef ligature font");
        return;
    };
    let pl = para_layout(&l);
    let line = pl.lines.first().unwrap();
    let (k, head) = pl.clusters.iter().enumerate().find(|(_, c)| c.g1 > c.g0).unwrap();
    let glyph = pl.glyphs.get(head.g0 as usize).unwrap();
    let left = (line.c0..line.c1).filter_map(|k| line.cl_left(k)).fold(f32::INFINITY, f32::min);
    let drawn = line.cl_left(k).unwrap() + glyph.dx;
    assert!((drawn - left).abs() < 0.05, "glyph drawn at {drawn}, ligature starts at {left}");
}

#[test]
fn an_emoji_zwj_sequence_stays_one_cluster() {
    // 👩‍💻: woman, zero-width joiner, laptop — one grapheme, one caret stop on each side.
    let t = "a\u{1F469}\u{200D}\u{1F4BB}b";
    let l = lay(&Document::from_text(t));
    let offs = para_layout(&l).line_offsets(0);
    assert!(!offs.contains(&5) && !offs.contains(&8), "no caret inside the emoji: {offs:?}");
    let xs: Vec<f32> = [0, 1, 12, 13].iter().map(|&o| caret_x(&l, o)).collect();
    assert!(xs.windows(2).all(|w| w[0] < w[1]), "{xs:?}");
}

/// Font/style fragments still draw independently, but every user-facing stop and emergency
/// wrap must be at a whole paragraph grapheme boundary, in LTR and bidirectional lines alike.
fn assert_atomic_drawing_group(d: &Document, legal: &[usize], start: usize, inside: usize, end: usize) {
    let l = lay(d);
    let pl = para_layout(&l);
    assert!(pl.clusters.iter().any(|c| c.start == inside), "fixture must actually split the drawing inside the grapheme");
    let li = pl.line_of(start);
    assert_eq!(pl.x_of(li, inside), pl.x_of(li, start), "an explicit internal offset uses the grapheme's leading edge");
    let mut reachable: Vec<_> = (0..pl.lines.len()).flat_map(|i| pl.line_offsets(i)).collect();
    reachable.sort_unstable();
    reachable.dedup();
    assert_eq!(reachable, legal, "every expected grapheme boundary remains reachable");
    if d.last_section.text_width() <= 18.01 {
        assert!(pl.lines.len() > 1, "the narrow fixture must actually exercise wrapping");
    }
    for (li, line) in pl.lines.iter().enumerate() {
        assert!(legal.contains(&line.start) && legal.contains(&line.stop), "wrap cannot split a grapheme: {}..{}", line.start, line.stop);
        assert!(pl.line_offsets(li).iter().all(|o| legal.contains(o)), "only legal keyboard stops");
        let left = (line.c0..line.c1).filter_map(|k| line.cl_left(k)).fold(line.left, f32::min);
        let right = (line.c0..line.c1).filter_map(|k| line.cl_right(k)).fold(line.right, f32::max);
        for sample in 0..=128 {
            let x = left + (right - left) * sample as f32 / 128.0;
            let off = pl.off_at_x(li, x);
            assert!(legal.contains(&off), "click at {x} reached internal offset {off}");
        }
    }
    // The whole group's selection covers every differently drawn component without gaps.
    let rects = l.selection_rects(d, &Pos::body(0, start), &Pos::body(0, end), 0);
    let width: f32 = rects.iter().map(|(_, r)| r.w).sum();
    let expected = (pl.x_of(li, end).unwrap() - pl.x_of(li, start).unwrap()).abs();
    assert!(width > 0.0 && (width - expected).abs() < 0.05, "whole-grapheme selection: {width}, expected {expected}");
}

/// A mixed-size emoji ZWJ grapheme keeps both drawing sizes and gives caret hit testing,
/// selection and wrapping atomic boundaries in LTR and bidi at several widths.
#[test]
fn a_mixed_style_emoji_grapheme_has_atomic_caret_hit_and_wrap_boundaries() {
    for bidi in [false, true] {
        let mut d = Document::from_text("x\u{1F469}\u{200D}\u{1F4BB}y");
        d.format_range(&Pos::body(0, 0), &Pos::body(0, 13), &|c| {
            c.font = Some("Source Sans 3".into());
            c.size = Some(24.0);
        })
        .unwrap();
        d.format_range(&Pos::body(0, 8), &Pos::body(0, 12), &|c| c.size = Some(48.0)).unwrap();
        d.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().props.bidi = Some(bidi);
        let wide = lay(&d);
        let pl = para_layout(&wide);
        if bidi {
            assert!(pl.lines.iter().any(|line| !line.vis.is_empty()), "exercise the bidi hit/caret path");
        }
        let styles: Vec<_> = pl.clusters.iter().filter(|c| c.start >= 1 && c.end <= 12).map(|c| pl.styles[c.style as usize].size).collect();
        assert!(styles.contains(&24.0) && styles.contains(&48.0), "both drawing sizes must survive: {styles:?}");
        assert_atomic_drawing_group(&d, &[0, 1, 12, 13], 1, 8, 12);
        for width in [18.0, 32.0, 48.0, 96.0] {
            d.last_section.page_w = 144.0 + width;
            assert_atomic_drawing_group(&d, &[0, 1, 12, 13], 1, 8, 12);
        }
    }
}

/// A base letter and combining mark using different bundled fonts keep their own faces and
/// glyphs, with atomic hit, selection and wrap boundaries across widths and bidi.
#[test]
fn a_mixed_bundled_font_combining_grapheme_preserves_drawing_and_atomic_boundaries() {
    for bidi in [false, true] {
        let mut d = Document::from_text("xa\u{301}b");
        d.format_range(&Pos::body(0, 0), &Pos::body(0, 5), &|c| {
            c.font = Some("Source Sans 3".into());
            c.size = Some(24.0);
        })
        .unwrap();
        d.format_range(&Pos::body(0, 2), &Pos::body(0, 4), &|c| {
            c.font = Some("Source Serif 4".into());
            c.size = Some(48.0);
        })
        .unwrap();
        d.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().props.bidi = Some(bidi);
        let wide = lay(&d);
        let pl = para_layout(&wide);
        if bidi {
            assert!(pl.lines.iter().any(|line| !line.vis.is_empty()), "exercise the bidi hit/caret path");
        }
        let base = pl.clusters.iter().find(|c| c.start == 1).unwrap();
        let mark = pl.clusters.iter().find(|c| c.start == 2).unwrap();
        assert_eq!(pl.styles[base.style as usize].face.family, "Source Sans 3");
        assert_eq!(pl.styles[mark.style as usize].face.family, "Source Serif 4");
        assert!(base.g1 > base.g0 && mark.g1 > mark.g0, "both independently shaped pieces keep their glyphs");
        assert_atomic_drawing_group(&d, &[0, 1, 4, 5], 1, 2, 4);
        for width in [18.0, 32.0, 48.0, 96.0] {
            d.last_section.page_w = 144.0 + width;
            assert_atomic_drawing_group(&d, &[0, 1, 4, 5], 1, 2, 4);
        }
    }
}

#[test]
fn an_indic_conjunct_stays_one_cluster() {
    // क्ष: ka, virama, ssa — one conjunct; the caret never stops inside it.
    let l = lay(&Document::from_text("\u{915}\u{94D}\u{937}"));
    let offs = para_layout(&l).line_offsets(0);
    assert!(!offs.contains(&3) && !offs.contains(&6), "no caret inside the conjunct: {offs:?}");
}

#[test]
fn a_long_word_never_wraps_inside_a_ligature() {
    // A narrow page forces an emergency break in the unbroken word; whatever the offset, a line
    // never ends between a ligature's glyph and its glyphless letters.
    let mut seen_ligature = false;
    for k in 0..24 {
        let mut d = doc(&format!("{}ffi", "x".repeat(k)));
        d.last_section.margin_left = 250.0;
        d.last_section.margin_right = 250.0;
        let l = lay(&d);
        let pl = para_layout(&l);
        seen_ligature |= pl.glyphs.len() < pl.clusters.len();
        for line in &pl.lines {
            if let Some(next) = pl.clusters.get(line.c1) {
                let inside = next.g1 <= next.g0 && next.kind == crate::para::ClKind::Text && line.c1 > line.c0;
                assert!(!inside, "k={k}: line ends at {} inside a ligature", line.c1);
            }
        }
    }
    assert!(seen_ligature, "the default font should draw ffi as a ligature");
}

#[test]
fn a_hyphenation_point_never_splits_a_ligature() {
    // Soft hyphens sit between the letters of "ffi"; a line never ends between a ligature's glyph
    // and its glyphless letters, whether or not the soft hyphen leaves the ligature intact.
    for k in 0..24 {
        let mut d = doc(&format!("{}f\u{ad}f\u{ad}i", "x".repeat(k)));
        d.last_section.margin_left = 250.0;
        d.last_section.margin_right = 250.0;
        let l = lay(&d);
        let pl = para_layout(&l);
        for line in &pl.lines {
            if let Some(next) = pl.clusters.get(line.c1) {
                let inside = next.g1 <= next.g0 && next.kind == crate::para::ClKind::Text && line.c1 > line.c0;
                assert!(!inside, "k={k}: line ends at {} inside a ligature", line.c1);
            }
        }
    }
}

/// The paragraph's text without its inline-object markers (`U+FFFC`), and for every character
/// start of that text the byte offset it has in the paragraph (plus the end).
fn without_markers(p: &wordcraft_doc::Paragraph) -> (String, Vec<(usize, usize)>) {
    let mut text = String::new();
    let mut at = Vec::new();
    for (i, c) in p.text.char_indices() {
        if c == wordcraft_doc::para::OBJ {
            continue;
        }
        at.push((text.len(), i));
        text.push(c);
    }
    at.push((text.len(), p.text.len()));
    (text, at)
}

/// "x👩‍💻y" with the emoji's components at two sizes, justified; optionally with a zero-width
/// marker at each end of the grapheme and of the paragraph. Every marker sits on a grapheme
/// boundary: a marker *inside* one is a `U+FFFC` in the text and splits it in the document model.
fn marked_grapheme_document(marks: bool, bidi: bool) -> Document {
    let mut d = Document::from_text("x\u{1F469}\u{200D}\u{1F4BB}y");
    d.format_range(&Pos::body(0, 0), &Pos::body(0, 13), &|c| {
        c.font = Some("Source Sans 3".into());
        c.size = Some(24.0);
    })
    .unwrap();
    d.format_range(&Pos::body(0, 8), &Pos::body(0, 12), &|c| c.size = Some(48.0)).unwrap();
    {
        let p = d.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
        p.props.bidi = Some(bidi);
        p.props.align = Some(wordcraft_doc::props::Align::Justify);
    }
    if marks {
        // From the paragraph's end, so the earlier insertions keep their offsets.
        let marks = [
            (13, wordcraft_doc::para::InlineObject::BookmarkStart { name: "g".into() }),
            (12, wordcraft_doc::para::InlineObject::CommentStart { id: 7 }),
            (1, wordcraft_doc::para::InlineObject::ControlStart { control: Box::default() }),
            (0, wordcraft_doc::para::InlineObject::FieldStart { instr: "PAGE".into(), locked: false, dirty: false }),
        ];
        for (at, mark) in marks {
            d.insert_object(&Pos::body(0, at), mark, &Default::default()).unwrap();
        }
    }
    d
}

/// Text clusters on each line, not counting zero-width markers: a marker may not move text
/// between lines.
fn text_cluster_counts(pl: &ParaLayout) -> Vec<usize> {
    pl.lines.iter().map(|l| pl.clusters.get(l.c0..l.c1).map_or(0, |cs| cs.iter().filter(|c| c.kind != crate::para::ClKind::Marker).count())).collect()
}

/// A bookmark, comment, content-control and field marker at the edges of a mixed-style emoji
/// grapheme (and of the paragraph) add no line of their own and move no break into the grapheme,
/// at narrow and wide widths, LTR and bidi. Their line-break skip and the grapheme's atomic
/// caret stops must agree.
#[test]
fn markers_at_a_mixed_style_graphemes_edges_keep_it_atomic() {
    const START: usize = 1; // the emoji's first byte in the text without markers
    const INSIDE: usize = 8; // its size change: a drawing segment start inside the grapheme
    for bidi in [false, true] {
        for width in [18.0, 32.0, 48.0, 180.0] {
            let mut plain_doc = marked_grapheme_document(false, bidi);
            plain_doc.last_section.page_w = 144.0 + width;
            let plain = lay(&plain_doc);
            let plain_counts = text_cluster_counts(para_layout(&plain));
            if width == 18.0 {
                assert!(plain_counts.len() > 1, "the narrow fixture must exercise wrapping");
            }

            let mut d = marked_grapheme_document(true, bidi);
            d.last_section.page_w = 144.0 + width;
            let l = lay(&d);
            let pl = para_layout(&l);
            let p = d.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
            let (clean, at) = without_markers(p);
            assert_eq!(clean, "x\u{1F469}\u{200D}\u{1F4BB}y", "{bidi}/{width}: markers are not text");
            let para_off = |clean_off: usize| at.binary_search_by_key(&clean_off, |e| e.0).ok().and_then(|k| at.get(k)).map(|e| e.1);
            let clean_starts = wordcraft_doc::cluster::cluster_starts(&clean);
            let mut boundaries: Vec<usize> = clean_starts.iter().filter_map(|&o| para_off(o)).collect();
            // The paragraph's end is a caret stop on the last line but not a grapheme start.
            boundaries.extend(para_off(clean.len()));
            let markers: Vec<usize> = p.text.char_indices().filter(|&(_, c)| c == wordcraft_doc::para::OBJ).map(|(i, _)| i).collect();
            assert_eq!(markers.len(), 4, "{bidi}/{width}: the fixture carries all four zero-width markers");
            for &marker in &markers {
                let clean_offset = p.text[..marker].chars().filter(|&c| c != wordcraft_doc::para::OBJ).map(char::len_utf8).sum::<usize>();
                assert!(clean_starts.contains(&clean_offset) || clean_offset == clean.len(), "marker must sit on a clean grapheme edge");
            }
            assert_eq!(pl.clusters.iter().filter(|c| c.kind == crate::para::ClKind::Marker).count(), 4);
            let start = para_off(START).unwrap();
            let inside = para_off(INSIDE).unwrap();
            let interior: Vec<usize> = clean.char_indices().map(|(i, _)| i).filter(|o| !clean_starts.contains(o)).filter_map(&para_off).collect();
            assert!(interior.contains(&inside), "{bidi}/{width}: {inside} must be inside the grapheme: {interior:?}");
            let allowed = |off: usize| boundaries.contains(&off) || markers.contains(&off);

            assert_eq!(text_cluster_counts(pl), plain_counts, "{bidi}/{width}: markers must not move text between lines");
            assert!(pl.clusters.iter().any(|c| c.start == inside), "{bidi}/{width}: the size change must still split the drawing");
            for (li, line) in pl.lines.iter().enumerate() {
                assert!(
                    pl.clusters.get(line.c0..line.c1).is_some_and(|cs| cs.iter().any(|c| c.kind != crate::para::ClKind::Marker)),
                    "{bidi}/{width}: line {li} holds only markers: {line:?}"
                );
                for off in [line.start, line.stop] {
                    assert!(!interior.contains(&off), "{bidi}/{width}: line {li} ends or starts inside the grapheme at {off}: {line:?}");
                    assert!(allowed(off), "{bidi}/{width}: line {li} boundary {off} is not a grapheme boundary: {line:?}");
                }
                for off in pl.line_offsets(li) {
                    assert!(!interior.contains(&off), "{bidi}/{width}: caret stop {off} inside the grapheme");
                    assert!(allowed(off), "{bidi}/{width}: caret stop {off} is not a grapheme boundary");
                }
                let left = (line.c0..line.c1).filter_map(|k| line.cl_left(k)).fold(line.left, f32::min);
                let right = (line.c0..line.c1).filter_map(|k| line.cl_right(k)).fold(line.right, f32::max);
                for sample in 0..=64 {
                    let off = pl.off_at_x(li, left + (right - left) * sample as f32 / 64.0);
                    assert!(!interior.contains(&off), "{bidi}/{width}: a click on line {li} reached {off} inside the grapheme");
                }
            }
            let li = pl.line_of(start);
            assert_eq!(
                pl.x_of(li, inside).expect("interior offset resolves"),
                pl.x_of(li, start).expect("grapheme start resolves"),
                "{bidi}/{width}: an explicit interior offset uses the grapheme's leading edge"
            );
        }
    }
}
