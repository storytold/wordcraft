//! Automatic space between Asian text and Latin text or digits (`w:autoSpaceDE`, `w:autoSpaceDN`).
//! The room goes after the first of the two characters and takes that character's own size.
//!
//! - **In front of a Latin run** it is a quarter of the font size: Word's PDF of the #2358 file
//!   shows about that between 院 and 94 at 16 pt, as JIS X 4051 asks.
//! - **After a Latin run** it is half the Latin face's average character width — 0.1986 of the
//!   font size for Times New Roman, 0.2203 for Arial, 0.2575 for Calibri, 0.2980 for Courier New,
//!   0.3041 for Cambria. Measured in Word 16.0.17932.21056 over 26 faces, both orders, 10.5 to
//!   400 pt, Japanese and Chinese (#6583), from
//!   `Range.Information(wdHorizontalPositionRelativeToPage)` rather than a PDF, because
//!   `ExportAsFixedFormat` hung on the machine it was measured on. The room in front stayed a
//!   quarter of the size whatever face the Latin text used, and the room after a Latin run of
//!   several characters followed the last of them.
//!
//! Assertions compare a layout with the flags on to the same layout with them off, and read the
//! average character width from the face the layout actually drew with rather than naming a
//! figure (fonts differ between machines; CJK glyphs may be missing).

use crate::para::ClKind;
use crate::*;
use wordcraft_doc::props::Align;
use wordcraft_doc::section::{DocGrid, DocGridType};

fn lay(doc: &Document) -> DocLayout {
    layout(doc, &mut LayoutCache::new(), &LayoutOptions::default())
}

fn first_para(l: &DocLayout) -> Arc<ParaLayout> {
    l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, .. } = it { Some(para.clone()) } else { None }).unwrap()
}

fn para0(d: &mut Document) -> &mut Paragraph {
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap()
}

/// The room each cluster gains with the flags as `d` sets them, against both flags off, in points
/// rounded to a hundredth. The tests running alongside share the font database: one loading a
/// font between the two layouts can change the face a character falls back to (Hangul's, on
/// Windows), so the pair is laid out again until both draw with the same faces.
fn gaps(d: &Document) -> Vec<f32> {
    let mut off = d.clone();
    let p = para0(&mut off);
    p.props.auto_space_de = Some(false);
    p.props.auto_space_dn = Some(false);
    let faces = |l: &ParaLayout| l.clusters.iter().map(|c| l.styles[c.style as usize].face.id()).collect::<Vec<u32>>();
    for _ in 0..5 {
        let (on, off) = (first_para(&lay(d)), first_para(&lay(&off)));
        if faces(&on) != faces(&off) {
            continue;
        }
        assert_eq!(on.clusters.len(), off.clusters.len());
        return on.clusters.iter().zip(&off.clusters).map(|(a, b)| ((a.adv - b.adv) * 100.0).round() / 100.0).collect();
    }
    panic!("the layouts with and without the automatic space draw with other faces");
}

/// The default font size of `d`'s first paragraph, a quarter of it: the room in front of a Latin
/// run (#2358).
fn quarter(d: &Document) -> f32 {
    let pl = first_para(&lay(d));
    let size = pl.clusters.first().and_then(|c| pl.styles.get(c.style as usize)).unwrap().size;
    (size / 4.0 * 100.0).round() / 100.0
}

/// Half the average character width of the face that draws the cluster starting at byte `at`:
/// the room after a Latin character (#6583).
fn after(d: &Document, at: usize) -> f32 {
    let pl = first_para(&lay(d));
    let c = pl.clusters.iter().find(|c| c.start == at).unwrap();
    let st = pl.styles.get(c.style as usize).unwrap();
    let room = st.size * (st.face.avg_char_width / st.face.upem) as f32 / 2.0;
    (room * 100.0).round() / 100.0
}

#[test]
fn digits_and_latin_beside_asian_text_get_a_quarter_in_front_and_the_latin_average_after() {
    crate::test_fonts();
    let mut d = Document::from_text("法院94年A字");
    let q = quarter(&d);
    // 4 is at byte 7, A at byte 11: the room after each follows the face that draws it.
    let (a4, aa) = (after(&d, 7), after(&d, 11));
    assert!(q > 0.0 && a4 > 0.0 && aa > 0.0);
    assert_ne!(a4, q, "the test face must not average half its em, or this proves nothing");
    // 院|9 and 4|年 (DN), 年|A and A|字 (DE): the room follows the first of the two.
    assert_eq!(gaps(&d), [0.0, q, 0.0, a4, q, aa, 0.0]);
    para0(&mut d).props.auto_space_dn = Some(false);
    assert_eq!(gaps(&d), [0.0, 0.0, 0.0, 0.0, q, aa, 0.0], "numbers off");
    let p = para0(&mut d);
    p.props.auto_space_dn = None;
    p.props.auto_space_de = Some(false);
    assert_eq!(gaps(&d), [0.0, q, 0.0, a4, 0.0, 0.0, 0.0], "Latin off");
}

#[test]
fn the_space_crosses_runs_and_takes_the_size_of_the_character_before_it() {
    crate::test_fonts();
    let mut d = Document::from_text("漢A漢");
    let size = |s: f32| move |c: &mut CharProps| c.size = Some(s);
    let p = para0(&mut d);
    p.format(0, 3, &size(20.0)).unwrap();
    p.format(3, 4, &size(12.0)).unwrap();
    p.format(4, 7, &size(20.0)).unwrap();
    // 漢|A takes a quarter of the 20 pt 漢. A|漢 takes the 12 pt A, not the 20 pt 漢: in Word a
    // 100 pt digit before a 400 pt ideograph moved it 19.85 pt and a 400 pt digit before a
    // 100 pt one moved it 80.65 pt, both the Latin figure at the Latin size (#6583).
    let a = after(&d, 3);
    assert!(a > 0.0 && a < 5.0);
    assert_eq!(gaps(&d), [5.0, a, 0.0]);
}

#[test]
fn the_room_after_a_latin_run_follows_the_latin_face() {
    crate::test_fonts();
    // Two faces whose average character widths differ: the room after the digit differs with
    // them, while the room in front of it stays a quarter of the size.
    let mut rooms = Vec::new();
    for family in ["Source Sans 3", "JetBrains Mono"] {
        let mut d = Document::from_text("漢9漢");
        para0(&mut d).format(3, 4, &|c: &mut CharProps| c.font = Some(family.to_string())).unwrap();
        let (q, a) = (quarter(&d), after(&d, 3));
        assert_eq!(gaps(&d), [q, a, 0.0], "{family}");
        rooms.push(a);
    }
    assert_ne!(rooms[0], rooms[1], "the two faces must differ in average character width");
}

#[test]
fn spaces_punctuation_hangul_and_latin_alone_get_nothing() {
    crate::test_fonts();
    for text in ["例 a", "5 例", "5、", "、8", "abc 123", "漢한", "한A", "漢%", "（A）"] {
        let d = Document::from_text(text);
        assert!(gaps(&d).iter().all(|g| *g == 0.0), "{text}: {:?}", gaps(&d));
    }
}

#[test]
fn a_field_result_is_classed_by_the_text_it_shows() {
    crate::test_fonts();
    let mut d = Document::from_text("漢漢");
    let field =
        InlineObject::Field { instr: "MERGEFIELD n".into(), result: "12".into(), locked: false, dirty: false, code: false, form: String::new() };
    para0(&mut d).insert_object(3, field, &CharProps::default()).unwrap();
    let pl = first_para(&lay(&d));
    let at = pl.clusters.iter().position(|c| c.start == 3).unwrap();
    assert!(pl.shown.iter().any(|s| s.0 == at && s.1 == "12"), "the field shows its result: {:?}", pl.shown);
    let q = quarter(&d);
    assert_eq!(pl.clusters[at].kind, ClKind::Text);
    // 漢|1 and 2|漢: the field's first and last characters, not the object it stands for. The
    // room after the field follows the face it draws its result in.
    assert_eq!(gaps(&d), [q, after(&d, 3), 0.0]);
}

#[test]
fn a_character_grid_the_paragraph_snaps_to_turns_it_off() {
    crate::test_fonts();
    let with = |kind: DocGridType, snap: bool| {
        let mut d = Document::from_text("法院94年");
        d.last_section.doc_grid = Some(DocGrid { kind, line_pitch: 18.0, char_space: 0 });
        para0(&mut d).props.snap_to_grid = Some(snap);
        d
    };
    let d = with(DocGridType::LinesAndChars, true);
    assert!(gaps(&d).iter().all(|g| *g == 0.0), "{:?}", gaps(&d));
    // A line grid alone, or a paragraph that doesn't snap, keeps it.
    for d in [with(DocGridType::Lines, true), with(DocGridType::LinesAndChars, false)] {
        assert_eq!(gaps(&d), [0.0, quarter(&d), 0.0, after(&d, 7), 0.0]);
    }
}

#[test]
fn the_space_takes_no_room_at_the_end_of_a_justified_line() {
    crate::test_fonts();
    let text = "abcdef 1漢漢漢";
    let mut d = Document::from_text(text);
    let pl = first_para(&lay(&d));
    let line = &pl.lines[0];
    let k = pl.clusters.iter().position(|c| c.start == text.find('1').unwrap()).unwrap();
    let gap = gaps(&d)[k];
    assert!(gap > 0.0);
    let plain = pl.clusters[k].adv - gap;
    // The right edge falls a quarter of the way into the space after the 1: the 1 fits, 漢 does
    // not. Counted, the space would overflow by more than the line's one space may shrink.
    let right = line.xs[k - line.c0] + plain + gap / 4.0;
    let p = para0(&mut d);
    p.props.indent_right = Some(line.right - right);
    p.props.align = Some(Align::Justify);
    let pl = first_para(&lay(&d));
    let line = &pl.lines[0];
    assert!(pl.lines.len() > 1);
    assert_eq!(line.c1 - 1, k, "the 1 ends the first line");
    let end = line.xs[k - line.c0] + plain;
    assert!((end - line.right).abs() < 0.01, "the 1 ends at {end}, the line at {}", line.right);
}
