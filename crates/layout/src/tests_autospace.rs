//! Automatic space between Asian text and Latin text or digits (`w:autoSpaceDE`, `w:autoSpaceDN`,
//! #2358). Word's PDF of the issue's file shows about a quarter of the font size between 院 and 94
//! at 16 pt. Assertions compare a layout with the flags on to the same layout with them off, never
//! font metrics (fonts differ between machines; CJK glyphs may be missing).

use super::*;
use crate::para::ClKind;
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
/// rounded to a hundredth.
fn gaps(d: &Document) -> Vec<f32> {
    let mut off = d.clone();
    let p = para0(&mut off);
    p.props.auto_space_de = Some(false);
    p.props.auto_space_dn = Some(false);
    let (on, off) = (first_para(&lay(d)), first_para(&lay(&off)));
    assert_eq!(on.clusters.len(), off.clusters.len());
    on.clusters.iter().zip(&off.clusters).map(|(a, b)| ((a.adv - b.adv) * 100.0).round() / 100.0).collect()
}

/// The default font size of `d`'s first paragraph, a quarter of it.
fn quarter(d: &Document) -> f32 {
    let pl = first_para(&lay(d));
    let size = pl.clusters.first().and_then(|c| pl.styles.get(c.style as usize)).unwrap().size;
    (size / 4.0 * 100.0).round() / 100.0
}

#[test]
fn digits_and_latin_beside_asian_text_get_a_quarter_of_the_font_size() {
    let mut d = Document::from_text("法院94年A字");
    let q = quarter(&d);
    assert!(q > 0.0);
    // 院|9 and 4|年 (DN), 年|A and A|字 (DE): the space follows the first of the two.
    assert_eq!(gaps(&d), [0.0, q, 0.0, q, q, q, 0.0]);
    para0(&mut d).props.auto_space_dn = Some(false);
    assert_eq!(gaps(&d), [0.0, 0.0, 0.0, 0.0, q, q, 0.0], "numbers off");
    let p = para0(&mut d);
    p.props.auto_space_dn = None;
    p.props.auto_space_de = Some(false);
    assert_eq!(gaps(&d), [0.0, q, 0.0, q, 0.0, 0.0, 0.0], "Latin off");
}

#[test]
fn the_space_crosses_runs_and_takes_the_asian_size() {
    let mut d = Document::from_text("漢A漢");
    let size = |s: f32| move |c: &mut CharProps| c.size = Some(s);
    let p = para0(&mut d);
    p.format(0, 3, &size(20.0)).unwrap();
    p.format(3, 4, &size(10.0)).unwrap();
    p.format(4, 7, &size(20.0)).unwrap();
    assert_eq!(gaps(&d), [5.0, 5.0, 0.0]);
}

#[test]
fn spaces_punctuation_hangul_and_latin_alone_get_nothing() {
    for text in ["例 a", "5 例", "5、", "、8", "abc 123", "漢한", "한A", "漢%", "（A）"] {
        let d = Document::from_text(text);
        assert!(gaps(&d).iter().all(|g| *g == 0.0), "{text}: {:?}", gaps(&d));
    }
}

#[test]
fn a_field_result_is_classed_by_the_text_it_shows() {
    let mut d = Document::from_text("漢漢");
    let field =
        InlineObject::Field { instr: "MERGEFIELD n".into(), result: "12".into(), locked: false, dirty: false, code: false, form: String::new() };
    para0(&mut d).insert_object(3, field, &CharProps::default()).unwrap();
    let pl = first_para(&lay(&d));
    let at = pl.clusters.iter().position(|c| c.start == 3).unwrap();
    assert!(pl.shown.iter().any(|s| s.0 == at && s.1 == "12"), "the field shows its result: {:?}", pl.shown);
    let q = quarter(&d);
    assert_eq!(pl.clusters[at].kind, ClKind::Text);
    // 漢|1 and 2|漢: the field's first and last characters, not the object it stands for.
    assert_eq!(gaps(&d), [q, q, 0.0]);
}

#[test]
fn a_character_grid_the_paragraph_snaps_to_turns_it_off() {
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
        let q = quarter(&d);
        assert_eq!(gaps(&d), [0.0, q, 0.0, q, 0.0]);
    }
}

#[test]
fn the_space_takes_no_room_at_the_end_of_a_justified_line() {
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
