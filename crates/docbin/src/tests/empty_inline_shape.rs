//! Empty inline shapes ([MS-DOC] §2.9.190 PICF, `mfpf.mm` = MM_SHAPE): an inline picture whose
//! `OfficeArtInlineSpContainer` has a picture-frame shape but no BLIP, no `pib`, no `pibName` and
//! no FBSE. Some writers put one right after a floating text box anchored "as character" (a
//! `SHAPE` field result), and Word lays it out as an invisible inline object of its goal size, so
//! the line reserves the box's height. It must not vanish, and it draws nothing.

use wordcraft_doc::para::Wrap;

use super::drawing::{fopt, fsp, rec, sp};
use super::{StructSpec, grpprl, read, struct_doc, structured_spec};

const PIC_LOCATION: u16 = 0x6A03;
const F_SPEC: u16 = 0x0855;
const MM_SHAPE: u16 = 0x0064;

/// A PICF with `mm`, goal size (twips) and scale (1000 = 100 %), then `rest`.
fn picf(mm: u16, goal: (i16, i16), scale: (u16, u16), rest: &[u8]) -> Vec<u8> {
    let mut picf = vec![0u8; 0x44];
    picf[0..4].copy_from_slice(&((0x44 + rest.len()) as u32).to_le_bytes());
    picf[4..6].copy_from_slice(&0x44u16.to_le_bytes());
    picf[6..8].copy_from_slice(&mm.to_le_bytes());
    picf[28..30].copy_from_slice(&goal.0.to_le_bytes());
    picf[30..32].copy_from_slice(&goal.1.to_le_bytes());
    picf[32..34].copy_from_slice(&scale.0.to_le_bytes());
    picf[34..36].copy_from_slice(&scale.1.to_le_bytes());
    [picf.as_slice(), rest].concat()
}

fn client_anchor() -> Vec<u8> {
    rec(0, 0, 0xF010, &[0, 0, 0, 0x80])
}

/// A picture frame (shape type 75) with only `fLine` off in its FOPT: no pib, no pibName, no FBSE.
fn empty_frame(extra: Vec<Vec<u8>>) -> Vec<u8> {
    let mut kids = vec![fsp(75, 1025, 0xA00), fopt(&[(0x01FF, 0x0008_0000)], &[])];
    kids.extend(extra);
    kids.push(client_anchor());
    sp(&kids)
}

/// "a", the picture, a line break, "After": the picture is at CP 1.
fn doc_with(data: Vec<u8>) -> wordcraft_doc::Document {
    let text = "a\u{1}\u{b}After\r";
    let ccp = text.chars().count() as u32;
    let spec = StructSpec {
        text,
        data_stream: data,
        chpx_runs: vec![(0, grpprl(&[(PIC_LOCATION, &0u32.to_le_bytes()), (F_SPEC, &[1])]))],
        ..structured_spec(text, ccp)
    };
    read(&struct_doc(&spec)).expect("opens")
}

/// The sizes of the first paragraph's inline (not floating) objects.
fn inline_sizes(doc: &wordcraft_doc::Document) -> Vec<(f32, f32)> {
    let p0 = doc.body.first().and_then(|b| b.as_para()).expect("para");
    p0.objects.iter().filter_map(|o| o.frame()).filter(|(_, _, f)| f.wrap == Wrap::Inline).map(|(w, h, _)| (w, h)).collect()
}

fn close(got: &[(f32, f32)], want: (f32, f32)) -> bool {
    got.len() == 1 && (got[0].0 - want.0).abs() <= 0.05 && (got[0].1 - want.1).abs() <= 0.05
}

#[test]
fn an_empty_inline_shape_keeps_its_goal_size() {
    // 9071 x 3040 twips at 100 %: 453.55 x 152.0 pt, as in Word's PDF of the corpus file.
    let doc = doc_with(picf(MM_SHAPE, (9071, 3040), (1000, 1000), &empty_frame(vec![])));
    let got = inline_sizes(&doc);
    assert!(close(&got, (453.55, 152.0)), "{got:?}");
    assert!(doc.media.is_empty(), "no image bytes are invented");
}

#[test]
fn an_empty_inline_shape_takes_its_scale() {
    // 50 % x 200 %: 226.775 x 304.0 pt.
    let doc = doc_with(picf(MM_SHAPE, (9071, 3040), (500, 2000), &empty_frame(vec![])));
    let got = inline_sizes(&doc);
    assert!(close(&got, (226.775, 304.0)), "{got:?}");
}

#[test]
fn an_empty_horizontal_rule_is_still_not_an_object() {
    let tertiary = {
        let mut b = Vec::new();
        for (pid, op) in [(0x0394u16, 1u32), (0x0395, 30), (0x03BF, 0x2800_2800)] {
            b.extend_from_slice(&pid.to_le_bytes());
            b.extend_from_slice(&op.to_le_bytes());
        }
        rec(3, 3, 0xF122, &b)
    };
    let doc = doc_with(picf(MM_SHAPE, (1000, 500), (9072, 60), &empty_frame(vec![tertiary])));
    assert_eq!(inline_sizes(&doc), Vec::<(f32, f32)>::new());
}

#[test]
fn a_truncated_empty_inline_shape_is_skipped_without_panic() {
    let mut data = picf(MM_SHAPE, (9071, 3040), (1000, 1000), &empty_frame(vec![]));
    let lcb = 0x44 + 6;
    data[0..4].copy_from_slice(&(lcb as u32).to_le_bytes());
    let doc = doc_with(data);
    assert!(doc.body.first().and_then(|b| b.as_para()).is_some_and(|p| p.text.contains("After")));
}

// PR #5301 review: an FBSE whose picture can't be read stays a dropped picture, linked
// pictures stay pictures, and the controls the review asked for.

const MM_SHAPEFILE: u16 = 0x0066;

/// A picture frame whose FOPT has `simple` and `complex` properties, then `extra` records.
fn frame_with(simple: &[(u16, u32)], complex: &[(u16, Vec<u8>)], extra: Vec<Vec<u8>>) -> Vec<u8> {
    let mut kids = vec![fsp(75, 1025, 0xA00), fopt(simple, complex)];
    kids.extend(extra);
    kids.push(client_anchor());
    sp(&kids)
}

fn first_para(doc: &wordcraft_doc::Document) -> &wordcraft_doc::para::Paragraph {
    doc.body.first().and_then(|b| b.as_para()).expect("para")
}

fn shapes(doc: &wordcraft_doc::Document) -> usize {
    first_para(doc).objects.iter().filter(|o| matches!(o, wordcraft_doc::para::InlineObject::Shape { .. })).count()
}

fn images(doc: &wordcraft_doc::Document) -> usize {
    first_para(doc).objects.iter().filter(|o| matches!(o, wordcraft_doc::para::InlineObject::Image { .. })).count()
}

fn has_after(doc: &wordcraft_doc::Document) -> bool {
    doc.body.iter().filter_map(|b| b.as_para()).any(|p| p.text.contains("After"))
}

#[test]
fn an_fbse_whose_picture_cannot_be_read_is_still_dropped() {
    // The FBSE is there (a picture was meant) but its PNG BLIP has no bytes: a failed decode,
    // not an absent picture, so no invisible shape takes its place.
    let rest = [empty_frame(vec![]), super::drawing::fbse_png(&[])].concat();
    let doc = doc_with(picf(MM_SHAPE, (1440, 720), (1000, 1000), &rest));
    assert_eq!(shapes(&doc), 0, "{:?}", first_para(&doc).objects);
    assert_eq!(images(&doc), 0, "{:?}", first_para(&doc).objects);
    assert!(doc.media.is_empty());
    assert!(has_after(&doc));
}

#[test]
fn a_shape_naming_a_file_is_a_linked_picture_not_an_empty_shape() {
    let path = r"C:\pics\a.png";
    let frame = frame_with(&[(0x01FF, 0x0008_0000)], &[(0x0105, super::drawing::wz(path))], vec![]);
    let doc = doc_with(picf(MM_SHAPE, (1440, 720), (1000, 1000), &frame));
    assert_eq!((shapes(&doc), images(&doc)), (0, 0), "a linked picture is not an empty shape: {:?}", first_para(&doc).objects);
}

#[test]
fn a_shape_linked_to_a_file_by_its_flags_is_a_linked_picture_not_an_empty_shape() {
    // pibFlags = LinkToFile (0x8), no pibName, no FBSE.
    let frame = frame_with(&[(0x0106, 0x8), (0x01FF, 0x0008_0000)], &[], vec![]);
    let doc = doc_with(picf(MM_SHAPE, (1440, 720), (1000, 1000), &frame));
    assert_eq!((shapes(&doc), images(&doc)), (0, 0), "a linked picture is not an empty shape: {:?}", first_para(&doc).objects);
}

#[test]
fn an_mm_shapefile_picture_is_a_linked_picture_not_an_empty_shape() {
    let mut rest = vec![5u8];
    rest.extend_from_slice(b"a.png");
    rest.extend(empty_frame(vec![]));
    let doc = doc_with(picf(MM_SHAPEFILE, (1440, 720), (1000, 1000), &rest));
    assert_eq!((shapes(&doc), images(&doc)), (0, 0), "a linked picture is not an empty shape: {:?}", first_para(&doc).objects);
}

#[test]
fn the_rule_records_with_the_horizontal_rule_flag_off_keep_an_empty_shape() {
    // The rule test's tertiary FOPT with fUsefHorizRule set and fHorizRule off: an empty shape
    // of goal x scale, 1000/20 x 9.072 = 453.6 by 500/20 x 0.06 = 1.5 pt, drawn as nothing.
    let tertiary = {
        let mut b = Vec::new();
        for (pid, op) in [(0x0394u16, 1u32), (0x0395, 30), (0x03BF, 0x0800_0000)] {
            b.extend_from_slice(&pid.to_le_bytes());
            b.extend_from_slice(&op.to_le_bytes());
        }
        rec(3, 3, 0xF122, &b)
    };
    let doc = doc_with(picf(MM_SHAPE, (1000, 500), (9072, 60), &empty_frame(vec![tertiary])));
    let got = inline_sizes(&doc);
    assert!(close(&got, (453.6, 1.5)), "{got:?}");
    let p = first_para(&doc);
    let Some(wordcraft_doc::para::InlineObject::Shape { fill, stroke, extra, .. }) = p.objects.first() else {
        panic!("an empty shape: {:?}", p.objects);
    };
    assert!(fill.is_none() && stroke.is_none() && extra.hr.is_none());
}

#[test]
fn a_truncated_empty_inline_shape_leaves_no_object() {
    let mut data = picf(MM_SHAPE, (9071, 3040), (1000, 1000), &empty_frame(vec![]));
    data[0..4].copy_from_slice(&((0x44 + 6) as u32).to_le_bytes());
    let doc = doc_with(data);
    assert!(inline_sizes(&doc).is_empty(), "{:?}", first_para(&doc).objects);
    assert!(first_para(&doc).objects.is_empty(), "{:?}", first_para(&doc).objects);
    assert!(has_after(&doc));
}

#[test]
fn an_empty_shape_in_a_shape_field_result_keeps_its_size_and_the_field() {
    // "a", { SHAPE | picture }, line break, "After": the picture is at CP 9.
    let text = "a\u{13} SHAPE \u{14}\u{1}\u{15}\u{b}After\r";
    let ccp = text.chars().count() as u32;
    let spec = StructSpec {
        text,
        data_stream: picf(MM_SHAPE, (9071, 3040), (1000, 1000), &empty_frame(vec![])),
        chpx_runs: vec![(0, grpprl(&[(PIC_LOCATION, &0u32.to_le_bytes()), (F_SPEC, &[1])]))],
        ..structured_spec(text, ccp)
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    let got = inline_sizes(&doc);
    assert!(close(&got, (453.55, 152.0)), "{got:?} {:?}", first_para(&doc).objects);
    let fields: Vec<&str> = first_para(&doc)
        .objects
        .iter()
        .filter_map(|o| if let wordcraft_doc::para::InlineObject::Field { instr, .. } = o { Some(instr.as_str()) } else { None })
        .collect();
    assert_eq!(fields, ["SHAPE"], "{:?}", first_para(&doc).objects);
}
