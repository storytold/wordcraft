//! Drawing shapes and text boxes read from `\shp` (#6212): the shape's own description,
//! `\shpinst`, is read, and its `\shprslt` copy (for readers without shapes) is not.

use super::*;
use wordcraft_doc::fill::Fill;
use wordcraft_doc::para::{Anchor, FloatAlign, ShapeKind, Wrap};

/// The document's shapes, in order: kind, size, fill, outline, placement and text.
fn shapes(d: &Document) -> Vec<(ShapeKind, f32, f32, Option<Fill>, Option<Rgb>, wordcraft_doc::para::Float, String)> {
    d.body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| match o {
            InlineObject::Shape { kind, w, h, fill, stroke, float, story, .. } => {
                let text = story
                    .and_then(|id| d.parts.get(&id))
                    .map(|p| p.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect::<Vec<_>>().join("\n"))
                    .unwrap_or_default();
                Some((*kind, *w, *h, fill.clone(), *stroke, float.clone(), text))
            }
            _ => None,
        })
        .collect()
}

fn images(d: &Document) -> usize {
    d.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).filter(|o| matches!(o, InlineObject::Image { .. })).count()
}

/// A text box as Word writes one: shape type 202 with its text in `\shptxt`, and an old-style
/// drawing object holding the same text in `\shprslt`.
const TEXT_BOX: &str = r"{\rtf1\ansi Before\par
{\shp{\*\shpinst\shpleft0\shptop0\shpright5000\shpbottom1400\shpbxcolumn\shpbxignore\shpbypara\shpbyignore\shpwr1\shpwrk0\shpfblwtxt0\shpz2
{\sp{\sn shapeType}{\sv 202}}{\sp{\sn fillColor}{\sv 16445672}}
{\shptxt \pard\plain Boxed note\par}}
{\shprslt{\*\do\dobxcolumn\dobypara\dptxbx{\dptxbxtext \pard\plain Boxed note\par}\dpx0\dpy0\dpxsize5000\dpysize1400}}}
After\par}";

#[test]
fn a_text_box_is_read_with_its_text_fill_and_wrapping() {
    let d = import(TEXT_BOX.as_bytes()).unwrap();
    let s = shapes(&d);
    assert_eq!(s.len(), 1, "{s:?}");
    let (kind, w, h, fill, stroke, float, text) = &s[0];
    assert_eq!(*kind, ShapeKind::TextBox);
    // Twips to points.
    assert_eq!((*w, *h), (250.0, 70.0));
    // `fillColor` is 0x00BBGGRR.
    assert_eq!(*fill, Some(Fill::solid(Rgb(232, 240, 250))));
    // A line unless `fLine` turns it off: Word's black hairline.
    assert_eq!(*stroke, Some(Rgb::BLACK));
    assert_eq!(float.wrap, Wrap::TopAndBottom);
    assert_eq!((float.h_rel, float.v_rel), (Anchor::Column, Anchor::Paragraph));
    assert_eq!((float.x, float.y), (0.0, 0.0));
    assert_eq!(text, "Boxed note");
    // Its text is in the box, once: not also in the body.
    assert!(!d.plain_text(wordcraft_doc::StoryRef::Body).contains("Boxed"), "{}", d.plain_text(wordcraft_doc::StoryRef::Body));
}

#[test]
fn an_arrow_in_a_shape_field_is_read_in_line() {
    // Word writes a shape in line with text as a SHAPE field whose result is the shape, then a
    // picture of it for readers without shapes.
    let rtf = r"{\rtf1\ansi {\field\fldedit\fldlock{\*\fldinst SHAPE  \\* MERGEFORMAT }{\fldrslt
{\shp{\*\shpinst\shpleft0\shptop0\shpright2400\shpbottom800\shpbxcolumn\shpbxignore\shpbypara\shpbyignore\shpwr3\shpwrk0\shpfblwtxt0
{\sp{\sn shapeType}{\sv 13}}{\sp{\sn fillColor}{\sv 42495}}{\sp{\sn posrelh}{\sv 3}}{\sp{\sn posrelv}{\sv 3}}{\sp{\sn fPseudoInline}{\sv 1}}
{\sp{\sn fLine}{\sv 0}}}
{\shprslt{\*\do\dobxcolumn\dobypara\dppolygon\dppolycount3\dpptx0\dppty0\dpptx2400\dppty400\dpptx0\dppty800\dpx0\dpy0\dpxsize2400\dpysize800}}}}}
caption\par}";
    let d = import(rtf.as_bytes()).unwrap();
    let s = shapes(&d);
    assert_eq!(s.len(), 1, "{s:?}");
    let (kind, w, h, fill, stroke, float, _) = &s[0];
    assert_eq!(*kind, ShapeKind::Arrow);
    assert_eq!((*w, *h), (120.0, 40.0));
    assert_eq!(*fill, Some(Fill::solid(Rgb(255, 165, 0))));
    assert_eq!(*stroke, None);
    assert_eq!(float.wrap, Wrap::Inline);
    assert!(d.plain_text(wordcraft_doc::StoryRef::Body).contains("caption"));
}

/// A placeable WMF: the 9-word header (type 1, version 0x300, 12 words, max record 3), then EOF.
fn wmf_hex() -> String {
    [1u16, 9, 0x300, 12, 0, 0, 3, 0, 0, 3, 0, 0].iter().flat_map(|w| w.to_le_bytes()).map(|b| format!("{b:02x}")).collect()
}

#[test]
fn the_picture_after_a_shape_read_is_not_a_second_copy() {
    // After the shape, Word writes a picture of it marked `\defshp` for readers without shapes.
    let pict = format!(r"{{\pict{{\*\picprop\defshp{{\sp{{\sn shapeType}}{{\sv 75}}}}}}\picwgoal2400\pichgoal800\wmetafile8 {}}}", wmf_hex());
    let shape =
        |t: u32| format!(r"{{\shp{{\*\shpinst\shpright2400\shpbottom800{{\sp{{\sn shapeType}}{{\sv {t}}}}}{{\sp{{\sn fPseudoInline}}{{\sv 1}}}}}}}}");
    let field = |t: u32| format!(r"{{\rtf1\ansi {{\field{{\*\fldinst SHAPE }}{{\fldrslt {}{pict}}}}} caption\par}}", shape(t));
    let d = import(field(13).as_bytes()).unwrap();
    assert_eq!((shapes(&d).len(), images(&d)), (1, 0));
    // A shape not read here (a line) keeps its picture.
    let d = import(field(20).as_bytes()).unwrap();
    assert_eq!((shapes(&d).len(), images(&d)), (0, 1));
    // A `\defshp` picture with no shape read before it is kept.
    let d = import(format!(r"{{\rtf1\ansi {pict} caption\par}}").as_bytes()).unwrap();
    assert_eq!(images(&d), 1);
    // Only the picture right after a shape read is its copy: a line read earlier in the same
    // group doesn't take the picture of a later shape not read here.
    let d = import(format!(r"{{\rtf1\ansi {}{}{pict} caption\par}}", shape(13), shape(20)).as_bytes()).unwrap();
    assert_eq!((shapes(&d).len(), images(&d)), (1, 1));
    let d = import(format!(r"{{\rtf1\ansi {}{pict}{pict} caption\par}}", shape(13)).as_bytes()).unwrap();
    assert_eq!((shapes(&d).len(), images(&d)), (1, 1));
}

#[test]
fn a_shape_behind_text_is_placed_on_the_page() {
    let rtf = r"{\rtf1\ansi {\shp{\*\shpinst\shpleft1440\shptop2880\shpright4320\shpbottom4320\shpbxpage\shpbypage\shpwr3\shpfblwtxt1
{\sp{\sn shapeType}{\sv 3}}{\sp{\sn posh}{\sv 2}}{\sp{\sn fFilled}{\sv 0}}{\sp{\sn lineColor}{\sv 255}}{\sp{\sn lineWidth}{\sv 25400}}
{\sp{\sn fFlipH}{\sv 1}}}}Text\par}";
    let d = import(rtf.as_bytes()).unwrap();
    let s = shapes(&d);
    assert_eq!(s.len(), 1, "{s:?}");
    let (kind, w, h, fill, stroke, float, _) = &s[0];
    assert_eq!(*kind, ShapeKind::Ellipse);
    assert_eq!((*w, *h), (144.0, 72.0));
    assert_eq!(*fill, None);
    assert_eq!(*stroke, Some(Rgb(255, 0, 0)));
    assert_eq!(float.wrap, Wrap::BehindText);
    assert_eq!((float.h_rel, float.v_rel), (Anchor::Page, Anchor::Page));
    assert_eq!((float.x, float.y), (72.0, 144.0));
    assert_eq!(float.h_align, Some(FloatAlign::Center));
    assert!(float.flip_h);
    let width = d.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).find_map(|o| match o {
        InlineObject::Shape { stroke_width, .. } => Some(*stroke_width),
        _ => None,
    });
    assert_eq!(width, Some(2.0));
}

#[test]
fn a_picture_shape_keeps_one_picture() {
    // Shape type 75 (a picture frame) stays on its `\shprslt` copy: its `pib` picture isn't
    // read a second time.
    let png = "89504e470d0a1a0a0000000d4948445200000001000000010806000000";
    let rtf = format!(
        r"{{\rtf1\ansi {{\shp{{\*\shpinst\shpleft0\shptop0\shpright1440\shpbottom1440\shpbxcolumn\shpbypara\shpwr2
{{\sp{{\sn shapeType}}{{\sv 75}}}}{{\sp{{\sn pib}}{{\sv {{\pict\pngblip\picwgoal1440\pichgoal1440 {png}}}}}}}}}}}
{{\shprslt\par\pard{{\pict\pngblip\picwgoal1440\pichgoal1440 {png}}}\par}}}}Text\par}}"
    );
    let d = import(rtf.as_bytes()).unwrap();
    assert!(shapes(&d).is_empty());
    assert_eq!(images(&d), 1);
}

#[test]
fn shapes_inside_a_group_are_left_to_the_groups_copy() {
    // A group's members are placed in the group's own coordinates: they stay on the group's
    // `\shprslt` copy rather than being read at the wrong place.
    let rtf = r"{\rtf1\ansi {\shpgrp{\*\shpinst\shpleft0\shptop0\shpright2000\shpbottom2000\shpbxcolumn\shpbypara\shpwr3}
{\shp{\*\shpinst\shpleft0\shptop0\shpright1000\shpbottom1000{\sp{\sn shapeType}{\sv 1}}}}
{\shprslt kept}}Text\par}";
    let d = import(rtf.as_bytes()).unwrap();
    assert!(shapes(&d).is_empty());
    assert!(d.plain_text(wordcraft_doc::StoryRef::Body).contains("kept"));
}

#[test]
fn truncated_and_hostile_shapes_do_not_crash() {
    for rtf in [
        r"{\rtf1 Before\par{\shp{\*\shpinst{\shptxt Inside",
        r"{\rtf1 Before\par{\shp{\*\shpinst\shpleft-2147483648\shpright2147483647{\sp{\sn shapeType}{\sv 202}}{\shptxt A\par}}}}",
        r"{\rtf1 Before\par{\shp{\*\shpinst{\sp{\sn shapeType}{\sv 99999999999999999999}}{\sp{\sn fillColor}{\sv -5}}}}}",
        r"{\rtf1 Before\par{\shp{\*\shpinst{\sp{\sn shapeType}{\sv 202}}{\shptxt {\shp{\*\shpinst{\sp{\sn shapeType}{\sv 202}}{\shptxt deeper\par}}}\par}}}}",
        r"{\rtf1 Before\par{\footnote{\shp{\*\shpinst{\sp{\sn shapeType}{\sv 202}}{\shptxt in a note\par}}}}}",
    ] {
        let d = import(rtf.as_bytes()).unwrap();
        assert!(d.plain_text(wordcraft_doc::StoryRef::Body).contains("Before"), "{rtf}");
    }
    // A shape too large for a page is held to Word's largest.
    let d = import(r"{\rtf1 {\shp{\*\shpinst\shpleft-2147483648\shpright2147483647\shpbottom100{\sp{\sn shapeType}{\sv 1}}}}}".as_bytes()).unwrap();
    assert!(shapes(&d).iter().all(|s| s.1 <= 1584.0 && s.2 >= 1.0));
}

#[test]
fn the_flow_reader_is_unchanged() {
    // The public flow reader, `parse`, asks for no shapes: the box's text is left out as before.
    let f = parse(TEXT_BOX.as_bytes()).unwrap();
    let all: String = f.blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p.text()) } else { None }).collect();
    assert!(!all.contains("Boxed"), "{all}");
    assert!(f.blocks.iter().all(|b| match b {
        FBlock::Para(p) => p.inlines.iter().all(|i| !matches!(i, Inline::Shape { .. })),
        FBlock::Table(_) => true,
    }));
}
