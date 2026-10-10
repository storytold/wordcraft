//! Right-to-left and bidirectional layout: Persian paragraphs, mixed Persian/Latin lines,
//! numbers and punctuation, caret and selection geometry.

use super::*;
use crate::para::{bidi_levels, visual_order};
use wordcraft_doc::props::{Align, ParaProps};
use wordcraft_doc::{Pos, StoryRef};

fn lay(doc: &Document) -> DocLayout {
    let mut c = LayoutCache::new();
    layout(doc, &mut c, &LayoutOptions::default())
}

/// A document of `paras`, each right to left when its flag is set.
fn doc_of(paras: &[(&str, bool)]) -> Document {
    let text: Vec<&str> = paras.iter().map(|p| p.0).collect();
    let mut d = Document::from_text(&text.join("\n"));
    for (i, (_, rtl)) in paras.iter().enumerate() {
        if *rtl {
            let at = Pos::body(i, 0);
            d.format_paragraphs(&at, &at, &|p: &mut ParaProps| p.bidi = Some(true)).unwrap();
        }
    }
    d
}

/// The first line of body paragraph `i` with its x offset on the page.
fn first_line(l: &DocLayout, i: u32) -> (&ParaLayout, f32) {
    for p in &l.pages {
        for it in &p.items {
            if let Placed::Lines { story: StoryRef::Body, path, para, x, .. } = it
                && path.0 == vec![i]
            {
                return (para, *x);
            }
        }
    }
    panic!("paragraph {i} not laid out");
}

/// Page x of the caret at `off` in paragraph `i`.
fn caret_x(l: &DocLayout, i: usize, off: usize) -> f32 {
    l.caret(&Pos::body(i, off)).unwrap().x
}

/// Left edge (page x) of the cluster starting at byte `off` of paragraph `i`'s first line.
fn cluster_x(l: &DocLayout, i: u32, off: usize) -> f32 {
    let (pl, x) = first_line(l, i);
    let line = &pl.lines[0];
    let k = (line.c0..line.c1).find(|&k| pl.clusters[k].start == off).unwrap_or_else(|| panic!("no cluster at {off}"));
    x + line.cl_left(k).unwrap()
}

/// Byte offset of the `n`th char of `s`.
fn at(s: &str, n: usize) -> usize {
    s.char_indices().nth(n).map(|(i, _)| i).unwrap_or(s.len())
}

#[test]
fn visual_order_follows_uax9_rule_l2() {
    // The example from UAX #9 / unicode-bidi: levels 0 0 0 1 1 1 2 2.
    assert_eq!(visual_order(&[0, 0, 0, 1, 1, 1, 2, 2]), vec![0, 1, 2, 6, 7, 5, 4, 3]);
    assert_eq!(visual_order(&[1, 1, 1]), vec![2, 1, 0]);
    assert_eq!(visual_order(&[0, 0]), vec![0, 1]);
    assert_eq!(visual_order(&[]), Vec::<usize>::new());
    // Hostile levels never panic.
    assert_eq!(visual_order(&[125, 0, 125]).len(), 3);
}

#[test]
fn levels_for_persian_with_latin_and_numbers() {
    assert!(bidi_levels("plain English", false).is_empty(), "pure LTR paragraphs skip the algorithm");
    let s = "سلام Word ۱۴۰۳";
    let lv = bidi_levels(s, true);
    assert_eq!(lv.len(), s.len());
    assert_eq!(lv[0] % 2, 1, "Persian letters are right to left");
    assert_eq!(lv[at(s, 5)], 2, "Latin inside a right-to-left paragraph is embedded at level 2");
    assert_eq!(lv[at(s, 10)], 2, "Persian digits are numbers: left to right inside right-to-left text");
    // In a left-to-right paragraph Persian is embedded at level 1.
    let lv = bidi_levels("Hello سلام", false);
    assert_eq!(lv[0], 0);
    assert_eq!(lv[at("Hello سلام", 6)], 1);
}

#[test]
fn rtl_paragraph_is_right_aligned_and_reads_right_to_left() {
    let s = "سلام دنیا";
    let d = doc_of(&[(s, true)]);
    let l = lay(&d);
    let right = 612.0 - 72.0;
    // The caret at the start of a right-to-left paragraph is at the right margin.
    assert!((caret_x(&l, 0, 0) - right).abs() < 0.5, "start caret at {}", caret_x(&l, 0, 0));
    // Each next character sits further left.
    let xs: Vec<f32> = (0..s.chars().count()).map(|n| caret_x(&l, 0, at(s, n))).collect();
    assert!(xs.windows(2).all(|w| w[1] < w[0] + 0.01), "caret moves leftwards: {xs:?}");
    assert!(caret_x(&l, 0, s.len()) < xs[xs.len() - 1]);
    // Glyph boxes: the first letter is the rightmost.
    assert!(cluster_x(&l, 0, 0) > cluster_x(&l, 0, at(s, 5)));
}

#[test]
fn ltr_paragraph_with_embedded_persian_word() {
    let s = "Hello سلام world";
    let d = doc_of(&[(s, false)]);
    let l = lay(&d);
    let (h, sin, mim, w) = (cluster_x(&l, 0, 0), cluster_x(&l, 0, at(s, 6)), cluster_x(&l, 0, at(s, 9)), cluster_x(&l, 0, at(s, 11)));
    assert!((h - 72.0).abs() < 0.5, "left-to-right paragraphs still start at the left margin");
    assert!(h < mim && mim < sin && sin < w, "the Persian word reads right to left between the English ones: {h} {mim} {sin} {w}");
}

#[test]
fn numbers_and_latin_inside_persian_keep_their_order() {
    // "Price 250 toman" in Persian, with a Latin product name and Persian punctuation.
    let s = "قیمت 250 تومان برای Word، امسال ۱۴۰۳.";
    let d = doc_of(&[(s, true)]);
    let l = lay(&d);
    let x = |n: usize| cluster_x(&l, 0, at(s, n));
    // Digits run left to right.
    assert!(x(5) < x(6) && x(6) < x(7), "250 reads left to right: {} {} {}", x(5), x(6), x(7));
    assert!(x(32) < x(33) && x(33) < x(34) && x(34) < x(35), "۱۴۰۳ reads left to right");
    // "Word" reads left to right, and the words around it right to left.
    assert!(x(20) < x(21) && x(21) < x(22) && x(22) < x(23), "Word reads left to right");
    assert!(x(0) > x(9), "قیمت comes before تومان, so it is further right");
    assert!(x(9) > x(20), "تومان is right of Word");
    // The final full stop is at the paragraph's end: the far left.
    let dot = x(s.chars().count() - 1);
    for n in 0..s.chars().count() - 1 {
        assert!(dot <= x(n) + 0.01, "the full stop is leftmost (char {n})");
    }
}

#[test]
fn caret_and_hit_agree_in_bidi_text() {
    for (s, rtl) in [("سلام world ۱۲۳", true), ("Hello سلام دنیا end", false), ("کتاب «WordCraft» (نسخه ۲)", true)] {
        let d = doc_of(&[(s, rtl)]);
        let l = lay(&d);
        let (pl, _) = first_line(&l, 0);
        let offs = pl.line_offsets(0);
        let mut seen: Vec<(usize, f32)> = Vec::new();
        for off in offs {
            let c = l.caret(&Pos::body(0, off)).unwrap();
            let back = l.hit(c.page, c.x, c.top + c.height / 2.0, StoryRef::Body).unwrap();
            // A click on a caret position finds an offset drawn at the same place.
            let bx = l.caret(&back).unwrap().x;
            assert!((bx - c.x).abs() < 0.01, "{s:?}: offset {off} at {} came back as {} at {bx}", c.x, back.off);
            seen.push((off, c.x));
        }
        assert!(seen.len() > 5);
    }
}

#[test]
fn visual_steps_cover_the_line_in_order() {
    let s = "سلام world";
    let d = doc_of(&[(s, true)]);
    let l = lay(&d);
    // From the logical start (right edge), stepping left visits every position once.
    let mut pos = Pos::body(0, 0);
    let mut xs = vec![l.caret(&pos).unwrap().x];
    for _ in 0..40 {
        match l.visual_step(&pos, true, 0).unwrap() {
            hit::VisualStep::Moved(p) => {
                pos = p;
                xs.push(l.caret(&pos).unwrap().x);
            }
            hit::VisualStep::Edge { rtl, .. } => {
                assert!(rtl);
                break;
            }
        }
    }
    assert!(xs.windows(2).all(|w| w[1] < w[0]), "each step moves left: {xs:?}");
    // Every distinct caret position is visited (a lam-alef ligature, لا in سلام, puts two
    // offsets at one spot).
    let (pl, _) = first_line(&l, 0);
    let mut distinct: Vec<f32> = pl.line_offsets(0).iter().filter_map(|o| pl.x_of(0, *o)).collect();
    distinct.sort_by(f32::total_cmp);
    distinct.dedup_by(|a, b| (*a - *b).abs() < 0.01);
    assert_eq!(xs.len(), distinct.len(), "every caret position on the line is visited");
}

#[test]
fn selection_of_mixed_text_can_be_two_pieces() {
    // Logical range "lo wo" of "Hello سلام world"? Select across the Persian word's start:
    // "lo سل" is two visual pieces (the end of Hello, and the right part of سلام).
    let s = "Hello سلام world";
    let d = doc_of(&[(s, false)]);
    let l = lay(&d);
    let rects = l.selection_rects(&d, &Pos::body(0, at(s, 3)), &Pos::body(0, at(s, 8)), 0);
    assert_eq!(rects.len(), 2, "{rects:?}");
    // A whole right-to-left word is one piece.
    let rects = l.selection_rects(&d, &Pos::body(0, at(s, 6)), &Pos::body(0, at(s, 10)), 0);
    assert_eq!(rects.len(), 1, "{rects:?}");
}

#[test]
fn rtl_indents_and_alignment_are_measured_from_the_right() {
    let s = "متن فارسی";
    let mut d = doc_of(&[(s, true), (s, true), (s, true)]);
    let p0 = Pos::body(0, 0);
    d.format_paragraphs(&p0, &p0, &|p: &mut ParaProps| p.indent_left = Some(36.0)).unwrap();
    let p1 = Pos::body(1, 0);
    // `Align::Right` is the end edge: the left in a right-to-left paragraph.
    d.format_paragraphs(&p1, &p1, &|p: &mut ParaProps| p.align = Some(Align::Right)).unwrap();
    let p2 = Pos::body(2, 0);
    d.format_paragraphs(&p2, &p2, &|p: &mut ParaProps| p.align = Some(Align::Center)).unwrap();
    let l = lay(&d);
    assert!((caret_x(&l, 0, 0) - (540.0 - 36.0)).abs() < 0.5, "start indent on the right: {}", caret_x(&l, 0, 0));
    assert!((caret_x(&l, 1, s.len()) - 72.0).abs() < 0.5, "end-aligned text ends at the left margin: {}", caret_x(&l, 1, s.len()));
    let mid = (caret_x(&l, 2, 0) + caret_x(&l, 2, s.len())) / 2.0;
    assert!((mid - 306.0).abs() < 1.0, "centred: {mid}");
}

#[test]
fn rtl_list_label_sits_on_the_right() {
    let mut d = doc_of(&[("مورد اول", true), ("مورد دوم", true)]);
    let id = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    d.format_paragraphs(&Pos::body(0, 0), &Pos::body(1, 0), &|p: &mut ParaProps| {
        p.numbering = Some(wordcraft_doc::props::NumRef { num: id, level: 0 })
    })
    .unwrap();
    let l = lay(&d);
    let (pl, x) = first_line(&l, 0);
    let lab = pl.label.as_ref().expect("numbered");
    let text_right = caret_x(&l, 0, 0);
    assert!(x + lab.x >= text_right - 0.5, "label {} is right of the text start {text_right}", x + lab.x);
    assert!(x + lab.x + lab.width <= 540.5);
    // "1." reads right to left: the full stop is drawn left of the number.
    let st = &pl.styles[lab.style as usize];
    let (one, dot) = (st.face.glyph_for('1'), st.face.glyph_for('.'));
    let gx = |gid: u32| lab.glyphs.iter().find(|g| g.gid == gid).map(|g| g.dx).unwrap();
    assert!(gx(dot) < gx(one), "label shows .1");
}

#[test]
fn rtl_empty_and_trailing_lines_mirror() {
    let d = doc_of(&[("", true)]);
    let l = lay(&d);
    assert!((caret_x(&l, 0, 0) - 540.0).abs() < 0.5, "empty right-to-left paragraph: caret at the right margin");
    // A paragraph ending in a line break gets an empty last line, also on the right.
    let mut d = doc_of(&[("سطر", true)]);
    let end = "سطر".len();
    d.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().insert_text(end, "\n", &Default::default()).unwrap();
    let l = lay(&d);
    assert!((caret_x(&l, 0, end + 1) - 540.0).abs() < 0.5, "{}", caret_x(&l, 0, end + 1));
}

#[test]
fn bidi_display_draws_every_glyph_inside_the_line() {
    let s = "این یک متن آزمایشی با English و عدد ۱۲۳ است. ".repeat(12);
    let d = doc_of(&[(&s, true)]);
    let l = lay(&d);
    let items = display::page_display(&d, &l.pages[0], &Default::default());
    let mut n = 0;
    for it in &items {
        if let display::Draw::Glyphs { glyphs, .. } = it {
            for (_, x, _) in glyphs {
                assert!(*x >= 72.0 - 1.0 && *x <= 540.0 + 1.0, "glyph at {x} outside the text column");
                n += 1;
            }
        }
    }
    assert!(n > 100);
    // Lines wrap and every line but the last is filled to the right margin.
    let (pl, x) = first_line(&l, 0);
    assert!(pl.lines.len() > 3);
    assert!((x + pl.lines[0].vis.iter().map(|v| v.x + v.w).fold(0.0f32, f32::max) - 540.0).abs() < 1.0);
}

#[test]
fn hostile_bidi_text_never_panics() {
    // Explicit embeddings, isolates, unmatched pops, marks with no base, lone surrogates' stand-ins.
    let junk = [
        "\u{202B}\u{202B}\u{202C}\u{202C}\u{202C}abc\u{2067}سلام\u{2069}\u{2069}",
        "\u{064E}\u{064E}\u{200C}\u{200D}",
        "\u{200F}\u{200E}\t\u{200F}\n\u{200E}",
        "(((((((((سلام]]]]]]]",
        &"\u{202E}".repeat(200),
    ];
    for s in junk {
        for rtl in [false, true] {
            let d = doc_of(&[(s, rtl)]);
            let l = lay(&d);
            let _ = display::page_display(&d, &l.pages[0], &Default::default());
            for off in 0..=s.len() {
                if s.is_char_boundary(off) {
                    let _ = l.caret(&Pos::body(0, off));
                    let _ = l.visual_step(&Pos::body(0, off), true, 0);
                    let _ = l.visual_step(&Pos::body(0, off), false, 0);
                }
            }
            let _ = l.selection_rects(&d, &Pos::body(0, 0), &Pos::body(0, s.len()), 0);
        }
    }
}

/// A face that has Arabic letters (installed system fonts), or `None` to skip.
fn arabic_face() -> Option<wordcraft_fonts::FaceRef> {
    wordcraft_fonts::FontDb::global().fallback_for('ب', 0).map(|f| wordcraft_fonts::FaceRef::of(&f)).filter(|f| f.covers('پ'))
}

#[test]
fn persian_letters_join_and_brackets_mirror() {
    let Some(face) = arabic_face() else {
        eprintln!("skipped: no font with Persian letters installed");
        return;
    };
    let isolated = wordcraft_fonts::shape_run(&face, "ب", &[], |c| c, true);
    let joined = wordcraft_fonts::shape_run(&face, "ببب", &[], |c| c, true);
    assert_eq!(joined.len(), 3);
    assert!(joined.iter().all(|g| g.gid != 0), "the font has the letters");
    // Initial, medial and final forms differ from the isolated letter.
    assert!(joined.iter().any(|g| g.gid != isolated[0].gid), "cursive joining picks contextual forms");
    // Clusters come back in logical order.
    assert!(joined.windows(2).all(|w| w[0].cluster <= w[1].cluster));
    // ZWNJ (نیم‌فاصله) breaks the join: "می‌خواهم" keeps می separate.
    let with_zwnj = wordcraft_fonts::shape_run(&face, "ب\u{200C}ب", &[], |c| c, true);
    let first = with_zwnj.first().map(|g| g.gid);
    assert_eq!(first, Some(isolated[0].gid), "a letter before ZWNJ takes its isolated form");
    // In right-to-left text, "(" is drawn with the mirrored glyph.
    let open = wordcraft_fonts::shape_run(&face, "(", &[], |c| c, true);
    let close = wordcraft_fonts::shape_run(&face, ")", &[], |c| c, false);
    if let (Some(o), Some(c)) = (open.first(), close.first())
        && c.gid != 0
    {
        assert_eq!(o.gid, c.gid, "bidi mirroring");
    }
}

#[test]
fn persian_text_in_a_latin_font_stays_in_one_fallback_face() {
    if arabic_face().is_none() {
        eprintln!("skipped: no font with Persian letters installed");
        return;
    }
    // Calibri (substituted) has no Persian: the whole word must come from one fallback face so
    // its letters join, including the Persian-only letters and a diacritic.
    let s = "پژوهشگرِ گرامی";
    let d = doc_of(&[(s, true)]);
    let l = lay(&d);
    let (pl, _) = first_line(&l, 0);
    let word: Vec<u32> = pl.clusters.iter().filter(|c| c.end <= at(s, 8)).map(|c| pl.styles[c.style as usize].face.id()).collect();
    assert!(word.windows(2).all(|w| w[0] == w[1]), "one face for the word: {word:?}");
    // No .notdef glyphs.
    assert!(pl.glyphs.iter().all(|g| g.gid != 0));
}

#[test]
fn glyph_runs_carry_their_text_for_pdf_export() {
    let s = "سلام WordCraft می‌خواهم";
    let d = doc_of(&[(s, true)]);
    let l = lay(&d);
    let mut all = String::new();
    for it in display::page_display(&d, &l.pages[0], &Default::default()) {
        if let display::Draw::Glyphs { glyphs, text, ranges, .. } = it {
            assert_eq!(ranges.len(), glyphs.len(), "one text range per glyph");
            for r in &ranges {
                assert!(text.get(r.clone()).is_some_and(|t| !t.is_empty()), "{r:?} in {text:?}");
            }
            // A ligature glyph (لا) shows both its letters (when a Persian font draws it).
            for r in ranges.iter().filter(|_| arabic_face().is_some()) {
                if text.get(r.clone()) == Some("ل") {
                    panic!("the lam of لا is drawn as one glyph with the alef: {text:?} {ranges:?}");
                }
            }
            all.push_str(&text);
        }
    }
    assert_eq!(all.replace(' ', ""), s.replace(' ', ""), "every character is accounted for, in logical order");
}
