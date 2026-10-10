//! Right-to-left and bidirectional layout: Persian paragraphs, mixed Persian/Latin lines,
//! numbers and punctuation, caret and selection geometry. Right-to-left tables (#66) show the
//! first logical column on the right; RTL sections (#66) flow columns right to left with the
//! gutter on the binding (right) side.

use super::*;
use crate::para::{bidi_levels, visual_order};
use wordcraft_doc::props::{Align, ParaProps};
use wordcraft_doc::{Block, Paragraph, Pos, StoryRef, Table, para_block};

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
        if let display::Draw::Glyphs { glyphs, text, ranges, .. } = it {
            for (k, (_, x, _)) in glyphs.iter().enumerate() {
                // A wrapped line's trailing spaces hang past its end edge: the left margin here.
                if ranges.get(k).and_then(|r| text.get(r.clone())).is_some_and(|t| t.trim().is_empty()) {
                    continue;
                }
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

#[test]
fn character_border_boxes_a_whole_rtl_word() {
    let s = "کتاب خوب";
    let mut d = doc_of(&[(s, true)]);
    let b = wordcraft_doc::props::Border { space: 0.0, ..wordcraft_doc::props::Border::single(0.5) };
    d.format_range(&Pos::body(0, 0), &Pos::body(0, at(s, 4)), &|c| c.border = Some(b)).unwrap();
    let l = lay(&d);
    let (pl, x) = first_line(&l, 0);
    let line = &pl.lines[0];
    let word: Vec<usize> = (line.c0..line.c1).filter(|&k| pl.clusters[k].end <= at(s, 4)).collect();
    let lo = word.iter().filter_map(|&k| line.cl_left(k)).fold(f32::MAX, f32::min) + x;
    let hi = word.iter().filter_map(|&k| line.cl_right(k)).fold(f32::MIN, f32::max) + x;
    // The box's top edge spans the whole word, which reads right to left.
    let top = display::page_display(&d, &l.pages[0], &Default::default())
        .into_iter()
        .filter_map(|it| match it {
            display::Draw::Line { x0, y0, x1, y1, .. } if (y0 - y1).abs() < 0.01 && (x1 - x0).abs() > 1.0 => Some((x0.min(x1), x0.max(x1))),
            _ => None,
        })
        .next()
        .expect("a border");
    assert!((top.0 - lo).abs() < 0.5 && (top.1 - hi).abs() < 0.5, "box {top:?} vs word {lo}..{hi}");
}

#[test]
fn inline_picture_in_rtl_text_sits_in_its_place() {
    // "متن [picture] بیشتر": in a right-to-left paragraph the picture is drawn where its cluster
    // is on the line (left of the first word), not at its logical x.
    let s = "متن  بیشتر";
    let mut d = doc_of(&[(s, true)]);
    let off = at(s, 4);
    let pic =
        wordcraft_doc::InlineObject::Image { media: "m".into(), w: 30.0, h: 20.0, alt: String::new(), float: Default::default(), crop: [0.0; 4] };
    d.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().insert_object(off, pic, &Default::default()).unwrap();
    let l = lay(&d);
    let (pl, x) = first_line(&l, 0);
    let line = &pl.lines[0];
    let k = (line.c0..line.c1).find(|&k| matches!(pl.clusters[k].kind, para::ClKind::Object(_))).unwrap();
    let (a, b) = (x + line.cl_left(k).unwrap(), x + line.cl_right(k).unwrap());
    let drawn: Vec<Rect> = display::page_display(&d, &l.pages[0], &Default::default())
        .into_iter()
        .filter_map(|it| if let display::Draw::Image { rect, .. } = it { Some(rect) } else { None })
        .collect();
    assert_eq!(drawn.len(), 1);
    assert!((drawn[0].x - a).abs() < 0.5 && (drawn[0].x + drawn[0].w - b).abs() < 0.5, "picture {:?} vs its slot {a}..{b}", drawn[0]);
    // The object placed for selection and dragging is at the same spot.
    let placed: Vec<Rect> = l.pages[0].items.iter().filter_map(|it| if let Placed::Object { rect, .. } = it { Some(*rect) } else { None }).collect();
    assert!(placed.iter().any(|r| (r.x - a).abs() < 0.5), "placed {placed:?} vs {a}");
    assert!(a < 540.0 - 20.0, "after the first word, so left of the right margin");
}

/// A one-row two-column table, right to left or not, with `left`/`right` cell text.
fn two_col_table(rtl: bool) -> Document {
    let mut t = Table::new(1, 2, 200.0);
    t.props.rtl = rtl;
    for (c, text) in ["left", "right"].iter().enumerate() {
        t.rows[0].cells[c].blocks = vec![para_block(Paragraph::with_text(text, Default::default()))];
    }
    let mut d = Document::from_text("before");
    d.insert_block(StoryRef::Body, &wordcraft_doc::Path::top(1), Block::Table(t)).unwrap();
    d
}

/// Page x of the caret at the start of cell `(r, c)` of table block 1.
fn cell_caret_x(l: &DocLayout, r: u32, c: u32) -> f32 {
    let pos = Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![1, r, c, 0]), off: 0 };
    l.caret(&pos).unwrap().x
}

#[test]
fn rtl_tables_put_the_first_logical_column_on_the_right() {
    let ltr = lay(&two_col_table(false));
    assert!(cell_caret_x(&ltr, 0, 0) < cell_caret_x(&ltr, 0, 1), "LTR: logical order runs left to right");
    let rtl = lay(&two_col_table(true));
    let (c0, c1) = (cell_caret_x(&rtl, 0, 0), cell_caret_x(&rtl, 0, 1));
    assert!(c0 > c1, "RTL: the first logical column is right of the second ({c0} vs {c1})");
    // The mirror is exact within each table, and the default placement follows the direction:
    // an LTR table starts at the text's left edge, an RTL table ends at its right edge.
    let (l0, l1) = (cell_caret_x(&ltr, 0, 0), cell_caret_x(&ltr, 0, 1));
    let (xl, xr) = (l0 - 72.0 - 5.4, c1 - 72.0 - 5.4);
    assert!(xl.abs() < 1.0, "default LTR table starts at the left edge: {xl}");
    assert!((xr + 200.0 - 468.0).abs() < 1.0, "default RTL table ends at the right edge: {xr}");
    assert!(
        (c0 - (72.0 + xr + 100.0 + 5.4)).abs() < 0.5 && (l1 - (72.0 + xl + 100.0 + 5.4)).abs() < 0.5,
        "columns are exact mirrors: rtl ({c0}, {c1}) vs ltr ({l0}, {l1})"
    );
    // Clicking the visual-right cell selects the logical-first cell, and typing there keeps
    // logical document order: the stored cell array is never reversed.
    let top = ltr.caret(&Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![1, 0, 0, 0]), off: 0 }).unwrap().top;
    assert_eq!(rtl.cell_at(0, c0, top + 2.0).map(|(_, _, r, c)| (r, c)), Some((0, 0)));
    assert_eq!(rtl.cell_at(0, c1, top + 2.0).map(|(_, _, r, c)| (r, c)), Some((0, 1)));
    let d = two_col_table(true);
    let t = d.table(StoryRef::Body, &wordcraft_doc::Path::top(1)).unwrap();
    let texts: Vec<String> = t.rows[0].cells.iter().map(|c| c.blocks[0].as_para().unwrap().plain_text()).collect();
    assert_eq!(texts, ["left", "right"]);
}

#[test]
fn rtl_table_indent_is_measured_from_the_right() {
    let mut d = two_col_table(false);
    d.table_mut(StoryRef::Body, &wordcraft_doc::Path::top(1)).unwrap().props.indent = Some(36.0);
    let ltr = lay(&d);
    let mut d = two_col_table(true);
    d.table_mut(StoryRef::Body, &wordcraft_doc::Path::top(1)).unwrap().props.indent = Some(36.0);
    let rtl = lay(&d);
    // The default text column is 468 wide starting at x=72; cells are 100 wide with 5.4 margins.
    let l0 = cell_caret_x(&ltr, 0, 0);
    assert!((l0 - (72.0 + 36.0 + 5.4)).abs() < 1.0, "LTR table starts 36pt in: {l0}");
    let (r0, r1) = (cell_caret_x(&rtl, 0, 0), cell_caret_x(&rtl, 0, 1));
    assert!((r0 - (72.0 + (468.0 - 200.0 - 36.0) + 100.0 + 5.4)).abs() < 1.5, "RTL first cell starts 36pt from the right: {r0}");
    assert!(r1 < r0, "columns still mirrored with an indent");
}

#[test]
fn rtl_tables_mirror_merged_cells_and_unequal_widths() {
    let mut t = Table::new(2, 3, 300.0);
    t.props.rtl = true;
    t.grid = vec![50.0, 100.0, 150.0];
    // A horizontal merge in the first row: logical columns 1..3 in one cell.
    t.merge(0, 0, 1, 2);
    for r in 0..2 {
        for (c, text) in ["a", "b", "c"].iter().enumerate() {
            if let Some(cell) = t.rows[r].cells.get_mut(c) {
                cell.blocks = vec![para_block(Paragraph::with_text(text, Default::default()))];
            }
        }
    }
    let mut d = Document::from_text("before");
    d.insert_block(StoryRef::Body, &wordcraft_doc::Path::top(1), Block::Table(t)).unwrap();
    let l = lay(&d);
    // Second row: three unequal cells run right to left in logical order.
    let xs: Vec<f32> = (0..3).map(|c| cell_caret_x(&l, 1, c as u32)).collect();
    assert!(xs[0] > xs[1] && xs[1] > xs[2], "unequal columns mirrored: {xs:?}");
    // The merged cell spans the two visual-left columns (logical 1..3).
    let merged = cell_caret_x(&l, 0, 1);
    assert!(merged < xs[0] - 40.0, "merged cell sits left of the first column: {merged} vs {xs:?}");
    // Splitting across pages keeps the mirror: a tall RTL table lays out without panics.
    let mut tall = Table::new(80, 2, 300.0);
    tall.props.rtl = true;
    let mut d2 = Document::from_text("before");
    d2.insert_block(StoryRef::Body, &wordcraft_doc::Path::top(1), Block::Table(tall)).unwrap();
    let l2 = lay(&d2);
    assert!(l2.pages.len() > 1, "tall table paginates");
}

#[test]
fn nested_tables_keep_their_own_direction() {
    let mut inner = Table::new(1, 2, 100.0);
    for (c, text) in ["i0", "i1"].iter().enumerate() {
        inner.rows[0].cells[c].blocks = vec![para_block(Paragraph::with_text(text, Default::default()))];
    }
    let mut outer = Table::new(1, 2, 300.0);
    outer.props.rtl = true;
    outer.rows[0].cells[0].blocks = vec![para_block(Paragraph::with_text("o0", Default::default())), Block::Table(inner).into()];
    outer.rows[0].cells[1].blocks = vec![para_block(Paragraph::with_text("o1", Default::default()))];
    let mut d = Document::from_text("before");
    d.insert_block(StoryRef::Body, &wordcraft_doc::Path::top(1), Block::Table(outer)).unwrap();
    let l = lay(&d);
    // Outer columns mirrored, inner columns in logical (LTR) order.
    assert!(cell_caret_x(&l, 0, 0) > cell_caret_x(&l, 0, 1), "outer RTL table mirrored");
    let inner = |c: u32| {
        let pos = Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![1, 0, 0, 1, 0, c, 0]), off: 0 };
        l.caret(&pos).unwrap().x
    };
    assert!(inner(0) < inner(1), "inner LTR table keeps logical order");
}

#[test]
fn hostile_rtl_tables_never_panic() {
    // Empty grid, absurd indent and asymmetric margins: layout must finish with finite geometry.
    let mut t = Table::new(1, 2, 200.0);
    t.props.rtl = true;
    t.props.indent = Some(1e20);
    t.grid.clear();
    t.rows[0].cells[0].props.margins = Some([0.0, 40.0, 0.0, 1.0]);
    let mut d = Document::from_text("before");
    d.insert_block(StoryRef::Body, &wordcraft_doc::Path::top(1), Block::Table(t)).unwrap();
    let l = lay(&d);
    for p in &l.pages {
        for it in &p.items {
            if let Placed::Cell { rect, .. } = it {
                assert!(rect.x.is_finite() && rect.w.is_finite(), "{rect:?}");
            }
        }
    }
}

/// Page x of every body line on a page, in placement order.
fn body_line_xs_on(l: &DocLayout, page: usize) -> Vec<f32> {
    l.pages[page].items.iter().filter_map(|it| if let Placed::Lines { story: StoryRef::Body, x, .. } = it { Some(*x) } else { None }).collect()
}

#[test]
fn rtl_shapes_floats_headers_and_notes() {
    use wordcraft_doc::para::{Anchor, Float, InlineObject, ShapeKind, Wrap};
    // A square-wrapped shape anchored in an RTL paragraph: text flows around it, joined intact.
    let mut d = doc_of(&[("كلمات كثيرة تتدفق حول الصورة هنا بشكل طبيعي في الفقرة الطويلة. ".repeat(3).trim_end(), true)]);
    let float = Float { wrap: Wrap::Square, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: 0.0, y: 0.0, dist: 9.0, ..Default::default() };
    let shape =
        InlineObject::Shape { kind: ShapeKind::Rectangle, w: 144.0, h: 60.0, fill: None, stroke: None, stroke_width: 1.0, float, story: None };
    d.insert_object(&Pos::body(0, 0), shape, &Default::default()).unwrap();
    let l = lay(&d);
    assert!(l.pages[0].items.iter().any(|i| matches!(i, Placed::Shape { .. })), "shape placed");
    let lines: usize = l.pages[0]
        .items
        .iter()
        .filter_map(|it| if let Placed::Lines { story: StoryRef::Body, l0, l1, .. } = it { Some(l1 - l0) } else { None })
        .sum();
    assert!(lines > 2, "wrapped RTL lines laid out");
    // Arabic header and footer stories attach to the page.
    let hp = wordcraft_doc::para_block(Paragraph::with_text("رأس الصفحة", Default::default()));
    let fp = wordcraft_doc::para_block(Paragraph::with_text("تذييل الصفحة", Default::default()));
    let mut h = Document::from_text("body");
    let (hid, fid) = (h.add_part(wordcraft_doc::PartKind::Header, vec![hp]), h.add_part(wordcraft_doc::PartKind::Footer, vec![fp]));
    h.last_section.headers.default = Some(hid);
    h.last_section.footers.default = Some(fid);
    let l = lay(&h);
    assert_eq!(l.pages[0].header_story, Some(hid));
    assert_eq!(l.pages[0].footer_story, Some(fid));
    let hg =
        l.pages[0].header.iter().filter_map(|it| if let Placed::Lines { para, .. } = it { Some(para.glyphs.len()) } else { None }).sum::<usize>();
    assert!(hg > 0, "the Arabic header shapes into glyphs");
    // An Arabic footnote is numbered and placed at the page bottom.
    let mut n = Document::from_text(&"Body text line.\n".repeat(10));
    let note = wordcraft_doc::Paragraph::with_text("ملاحظة عربية.", Default::default());
    let id = n.add_part(wordcraft_doc::PartKind::Footnote, vec![wordcraft_doc::para_block(note)]);
    n.insert_object(
        &Pos::body(3, 4),
        InlineObject::NoteRef { kind: wordcraft_doc::para::NoteKind::Footnote, id, custom: String::new() },
        &Default::default(),
    )
    .unwrap();
    let l = lay(&n);
    let c = l.caret_on(&Pos { story: StoryRef::Part(id), path: wordcraft_doc::Path::top(0), off: 0 }, 0).unwrap();
    assert!(c.top > 500.0, "note at the bottom: {c:?}");
}

#[test]
fn kashida_justification_keeps_joining_marks_and_mappings() {
    use wordcraft_doc::props::Kashida;
    // Arabic with diacritics and a superscript alef, one paragraph over many lines.
    let text = "بِسْمِ اللَّهِ الرَّحْمَنِ الرَّحِيمِ طٰه ".repeat(30);
    assert!(!text.contains('\u{640}'), "no tatweel in the stored text");
    let mut counts = Vec::new();
    for mode in [None, Some(Kashida::Low), Some(Kashida::Medium), Some(Kashida::High)] {
        let mut d = Document::from_text(&text);
        let at = Pos::body(0, 0);
        d.format_paragraphs(&at, &at, &|p: &mut ParaProps| {
            p.bidi = Some(true);
            p.align = Some(Align::Justify);
            p.kashida = mode;
        })
        .unwrap();
        let l = lay(&d);
        // Every wrapped line is full width: the slack went to inter-word spaces, never into words.
        let mut lines = 0;
        for p in &l.pages {
            for it in &p.items {
                if let Placed::Lines { story: StoryRef::Body, path, para, l0, l1, x, .. } = it
                    && path.0 == vec![0]
                {
                    for li in *l0..*l1 {
                        lines += 1;
                        let line = &para.lines[li];
                        if li + 1 == para.lines.len() && *l1 == para.lines.len() {
                            continue; // the last line is never justified
                        }
                        let content = text[line.start..line.stop].trim_end().len();
                        let (a, b) = (para.x_of(li, line.start).unwrap() + x, para.x_of(li, line.start + content).unwrap() + x);
                        assert!((a - 540.0).abs() < 1.5 && (b - 72.0).abs() < 1.5, "full line {mode:?}: {a}..{b}");
                    }
                }
            }
        }
        assert!(lines > 3, "several wrapped lines");
        // Clusters cover every byte exactly once: mappings stay logical through justification.
        let pl = l
            .pages
            .iter()
            .flat_map(|p| p.items.iter())
            .find_map(|it| if let Placed::Lines { story: StoryRef::Body, para, .. } = it { Some(para.clone()) } else { None });
        let pl = pl.expect("laid out");
        let mut spans: Vec<(usize, usize)> = pl.clusters.iter().map(|c| (c.start, c.end)).collect();
        spans.sort();
        let mut next = 0;
        for (a, b) in &spans {
            assert_eq!(*a, next, "no gap or overlap in cluster mappings ({mode:?})");
            next = *b;
        }
        assert_eq!(next, text.len());
        counts.push(pl.clusters.len());
        // Diacritics and the superscript alef are covered by the chosen Arabic face, if any.
        if arabic_face().is_some() {
            assert!(pl.glyphs.iter().all(|g| g.gid != 0), "no .notdef marks ({mode:?})");
        }
    }
    assert!(counts.windows(2).all(|w| w[0] == w[1]), "justification never reshapes: {counts:?}");
}

#[test]
fn rtl_sections_fill_columns_right_to_left() {
    let text = "line\n".repeat(100);
    let mut d = Document::from_text(&text);
    d.last_section.columns.count = 2;
    let ltr = lay(&d);
    let xs = body_line_xs_on(&ltr, 0);
    assert!((xs[0] - 72.0).abs() < 1.0, "LTR reading starts in the left column: {xs:?}");
    let right = xs.iter().position(|x| (*x - (72.0 + 252.0)).abs() < 1.0).expect("LTR overflow reaches the right column");
    assert!(xs[..right].iter().all(|x| (*x - 72.0).abs() < 1.0), "left column first, then right");
    let mut d = Document::from_text(&text);
    d.last_section.columns.count = 2;
    d.last_section.rtl = true;
    let rtl = lay(&d);
    let xs = body_line_xs_on(&rtl, 0);
    // Two equal columns with a 36pt gap in a 468pt text area: the right column starts at 72+252.
    assert!((xs[0] - (72.0 + 252.0)).abs() < 1.0, "RTL reading starts in the right column: {xs:?}");
    assert!(xs.iter().any(|x| (*x - 72.0).abs() < 1.0), "overflow continues in the left column: {xs:?}");
    let right = xs.iter().position(|x| (*x - 72.0).abs() < 1.0).unwrap();
    assert!(xs[..right].iter().all(|x| (*x - (72.0 + 252.0)).abs() < 1.0), "right column first, then left");
}

#[test]
fn rtl_section_gutter_sits_on_the_right() {
    let mut d = Document::from_text("body");
    d.last_section.gutter = 36.0;
    let ltr = lay(&d);
    assert!((ltr.pages[0].body.x - 108.0).abs() < 0.5, "LTR gutter after the left margin");
    let mut d = Document::from_text("body");
    d.last_section.gutter = 36.0;
    d.last_section.rtl = true;
    let rtl = lay(&d);
    // Text width 432 (612 − 72 − 72 − 36); the binding side is the right: 612 − 72 − 36 − 432.
    assert!((rtl.pages[0].body.x - 72.0).abs() < 0.5, "RTL gutter before the right margin: {}", rtl.pages[0].body.x);
    assert!((rtl.pages[0].body.w - 432.0).abs() < 0.5);
    // Headers hang from the same edge as the body.
    let hp = wordcraft_doc::para_block(Paragraph::with_text("head", Default::default()));
    let mut h = Document::from_text("body");
    h.last_section.gutter = 36.0;
    h.last_section.rtl = true;
    let hid = h.add_part(wordcraft_doc::PartKind::Header, vec![hp]);
    h.last_section.headers.default = Some(hid);
    let l = lay(&h);
    let hx = l.pages[0].header.iter().find_map(|it| if let Placed::Lines { x, .. } = it { Some(*x) } else { None }).expect("header laid out");
    assert!((hx - 72.0).abs() < 1.0, "header starts where the RTL body starts: {hx}");
}

#[test]
fn mixed_direction_sections_keep_their_geometry_and_headers() {
    use wordcraft_doc::section::{Columns, SectionProps, SectionStart};
    let h1 = wordcraft_doc::para_block(Paragraph::with_text("H1", Default::default()));
    let h2 = wordcraft_doc::para_block(Paragraph::with_text("H2 first", Default::default()));
    let f1 = wordcraft_doc::para_block(Paragraph::with_text("F1", Default::default()));
    let mut d = Document::from_text(&("row\n".repeat(120)));
    let (i1, i2, i3) = (
        d.add_part(wordcraft_doc::PartKind::Header, vec![h1]),
        d.add_part(wordcraft_doc::PartKind::Header, vec![h2]),
        d.add_part(wordcraft_doc::PartKind::Footer, vec![f1]),
    );
    let mut s1 = SectionProps::default();
    s1.headers.default = Some(i1);
    let mut s2 = SectionProps { start: SectionStart::NextPage, rtl: true, title_page: true, ..Default::default() };
    s2.columns = Columns { count: 2, space: 24.0, separator: false, widths: Vec::new() };
    s2.headers.first = Some(i2);
    s2.footers.default = Some(i3);
    d.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().section = Some(Box::new(s1));
    d.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(60)).unwrap().section = Some(Box::new(s2));
    let l = lay(&d);
    let secs: Vec<usize> = l.pages.iter().map(|p| p.section).collect();
    assert_eq!(secs[0], 0, "starts in section 0: {secs:?}");
    let s2page = secs.iter().position(|s| *s == 1).expect("section 1 has pages");
    assert!(secs[s2page..].iter().all(|s| *s == 1 || *s == 2), "sections run in order: {secs:?}");
    assert!(secs.contains(&2), "the default last section closes the document: {secs:?}");
    // Section 0 is LTR at the left margin; section 1 is RTL with two columns.
    assert!((l.pages[0].body.x - 72.0).abs() < 0.5);
    assert_eq!(l.pages[0].header_story, Some(i1));
    // A two-column 468pt area with a 24pt gap: columns are 222 wide; RTL starts right.
    let first = body_line_xs_on(&l, s2page);
    assert!((first[0] - (l.pages[s2page].body.x + 246.0)).abs() < 1.0, "RTL section starts right: {first:?}");
    assert_eq!(l.pages[s2page].header_story, Some(i2), "title page uses the first header");
    if l.pages.len() > s2page + 1 {
        assert_eq!(l.pages[s2page + 1].header_story, Some(i1), "link to previous inherits section 0's header");
        assert_eq!(l.pages[s2page + 1].footer_story, Some(i3));
    }
}

#[test]
fn rtl_footnote_separator_starts_from_the_right() {
    use wordcraft_doc::para::{InlineObject, NoteKind};
    let doc = || {
        let mut d = Document::from_text(&"Body text line.\n".repeat(30));
        let note = wordcraft_doc::Paragraph::with_text("The note text.", Default::default());
        let id = d.add_part(wordcraft_doc::PartKind::Footnote, vec![wordcraft_doc::para_block(note)]);
        d.insert_object(&Pos::body(3, 4), InlineObject::NoteRef { kind: NoteKind::Footnote, id, custom: String::new() }, &Default::default())
            .unwrap();
        d
    };
    let ltr = lay(&doc());
    let ltr_sep = horizontal_rules(&ltr.pages[0]);
    assert_eq!(ltr_sep.len(), 1);
    assert!((ltr_sep[0].0 - 72.0).abs() < 0.5, "LTR separator from the left: {ltr_sep:?}");
    let mut d = doc();
    d.last_section.rtl = true;
    let rtl = lay(&d);
    let rtl_sep = horizontal_rules(&rtl.pages[0]);
    assert_eq!(rtl_sep.len(), 1);
    assert!((rtl_sep[0].1 - (72.0 + 468.0)).abs() < 0.5, "RTL separator to the right edge: {rtl_sep:?}");
}

/// Long horizontal rules on a page: (x0, x1).
fn horizontal_rules(p: &crate::Page) -> Vec<(f32, f32)> {
    p.items
        .iter()
        .filter_map(|it| {
            if let Placed::Rule { x0, y0, x1, y1, .. } = it
                && (y0 - y1).abs() < 0.01
                && (x1 - x0).abs() > 100.0
            {
                Some((*x0, *x1))
            } else {
                None
            }
        })
        .collect()
}
