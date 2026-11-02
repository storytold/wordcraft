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
