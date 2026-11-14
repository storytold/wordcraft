//! `w:textAlignment` (ECMA-376 §17.3.1.39) in a text box under a document grid, after a save and a
//! reopen: the text beside an inline picture follows the paragraph's vertical alignment. The
//! document is built in code (no personal data) and its picture is a 1×1 PNG.

use wordcraft_doc::para::{Float, ShapeKind};
use wordcraft_doc::props::TextAlignment;
use wordcraft_doc::section::{DocGrid, DocGridType};
use wordcraft_doc::{Block, Document, InlineObject, Paragraph, PartKind, Pos, TextBody, para_block};
use wordcraft_layout::display::{Draw, page_display};
use wordcraft_layout::{LayoutCache, LayoutOptions, Placed, layout};

/// A 1×1 transparent PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
    0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// A text box holding "Alpha" and a 30 × 60 pt picture after it, the paragraph aligned `align`
/// (`None`: unset), in a section whose document grid has 18 pt lines.
fn document(align: Option<TextAlignment>) -> Document {
    let mut d = Document::from_text("Lead");
    d.last_section.doc_grid = Some(DocGrid { kind: DocGridType::Lines, line_pitch: 18.0, char_space: 0 });
    let media = d.add_media(PNG.to_vec(), "png");
    let mut p = Paragraph::with_text("Alpha", Default::default());
    p.props.text_alignment = align;
    let picture = InlineObject::Image { media, w: 30.0, h: 60.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None };
    p.insert_object(5, picture, &Default::default()).unwrap();
    let id = d.add_part(PartKind::TextBox, vec![para_block(p)]);
    d.parts.get_mut(&id).unwrap().body = TextBody::default();
    let shape = InlineObject::Shape {
        kind: ShapeKind::TextBox,
        w: 144.0,
        h: 96.0,
        fill: None,
        stroke: None,
        stroke_width: 0.75,
        float: Float::default(),
        story: Some(id),
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    };
    d.insert_object(&Pos::body(0, 0), shape, &Default::default()).unwrap();
    d
}

/// What a saved and reopened document shows of its text box: the alignment read back, the
/// picture's rectangle, the text's glyph baseline, and the box paragraph's line top and height.
struct Drawn {
    align: Option<TextAlignment>,
    picture: (f32, f32),
    glyph_y: f32,
    line: (f32, f32),
}

fn saved_and_drawn(align: Option<TextAlignment>) -> Drawn {
    let bytes = wordcraft_docx::write(&document(align)).unwrap();
    let reopened = wordcraft_docx::read(&bytes).unwrap();
    let part = reopened.parts.values().find(|p| p.kind == PartKind::TextBox).expect("the text box survives the save");
    let Some(Block::Para(p)) = part.blocks.first().map(|b| &**b) else { panic!("the box's paragraph") };
    let doc_layout = layout(&reopened, &mut LayoutCache::new(), &LayoutOptions::default());
    let (y, para) = doc_layout.pages[0]
        .items
        .iter()
        .find_map(|it| match it {
            Placed::Lines { story: wordcraft_doc::StoryRef::Part(_), para, y, .. } => Some((*y, para.clone())),
            _ => None,
        })
        .expect("the box's line is placed");
    let line = &para.lines[0];
    let draws = page_display(&reopened, &doc_layout.pages[0], &Default::default());
    let glyph_y = draws
        .iter()
        .find_map(|it| match it {
            Draw::Glyphs { glyphs, text, .. } if text.contains("Alpha") => glyphs.first().map(|g| g.2),
            _ => None,
        })
        .expect("the text is drawn");
    let picture = draws
        .iter()
        .find_map(|it| match it {
            Draw::Image { rect, .. } => Some((rect.y, rect.h)),
            _ => None,
        })
        .expect("the picture is drawn");
    Drawn { align: p.props.text_alignment, picture, glyph_y, line: (y + line.top, line.height) }
}

#[test]
fn the_box_line_follows_the_text_alignment_after_a_save() {
    let auto = saved_and_drawn(None);
    assert_eq!(auto.align, None);
    let top = saved_and_drawn(Some(TextAlignment::Top));
    let bottom = saved_and_drawn(Some(TextAlignment::Bottom));
    assert_eq!((top.align, bottom.align), (Some(TextAlignment::Top), Some(TextAlignment::Bottom)));

    // The grid centres the line's text between its pitches, so the line's baseline sits below the
    // picture's top. Top: the picture's top stands on the line's top and the text rises with it.
    let (top_line, _) = top.line;
    assert!((top.picture.0 - top_line).abs() < 0.01, "picture top {} vs line top {top_line}", top.picture.0);
    assert!(top.glyph_y < auto.glyph_y - 0.01, "text {} vs {}", top.glyph_y, auto.glyph_y);

    // Bottom: the picture's bottom edge lands on the line's bottom, and the text drops to it too.
    let (line_top, line_h) = bottom.line;
    assert!((bottom.picture.0 + bottom.picture.1 - (line_top + line_h)).abs() < 0.01, "picture {:?} line {:?}", bottom.picture, bottom.line);
    assert!(bottom.picture.0 > auto.picture.0 + 0.01, "the picture drops under Bottom");
    assert!(bottom.glyph_y > auto.glyph_y + 0.01, "the text drops under Bottom: {} vs {}", bottom.glyph_y, auto.glyph_y);
}
