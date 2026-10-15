//! "Collapsed by default" headings (#1745): what a collapsed heading hides takes no room and isn't
//! drawn, in Print Layout (so in PDF and print, which lay out the same pages) and the other views;
//! the heading itself shows. Laid out exactly as if the hidden blocks weren't there.
use super::*;
use wordcraft_doc::props::ParaProps;
use wordcraft_doc::{Table, para_block};

fn para(text: &str, props: ParaProps) -> Arc<Block> {
    let mut p = Paragraph::with_text(text, Default::default());
    p.props = props;
    para_block(p)
}

fn heading(text: &str, level: u8, collapsed: bool) -> Arc<Block> {
    para(text, ParaProps { style: Some(format!("Heading{level}")), collapsed: collapsed.then_some(true), ..ParaProps::default() })
}

fn lay(doc: &Document, view: ViewMode) -> DocLayout {
    layout(doc, &mut LayoutCache::new(), &LayoutOptions { view, ..LayoutOptions::default() })
}

/// Each body paragraph laid out (by its text, cells included): its page and top.
fn placed(doc: &Document, l: &DocLayout) -> Vec<(String, usize, f32)> {
    let mut out = Vec::new();
    for (i, pg) in l.pages.iter().enumerate() {
        for it in &pg.items {
            if let Placed::Lines { story: StoryRef::Body, path, y, l0: 0, .. } = it
                && let Some(p) = doc.para(StoryRef::Body, path)
            {
                out.push((p.text.clone(), i, *y));
            }
        }
    }
    out
}

/// Filler, a collapsed Heading 1 over a long kept-together paragraph, a Heading 2 and a table,
/// then the next Heading 1.
fn doc(filler: usize) -> Document {
    let mut d = Document::new();
    d.body = (0..filler).map(|i| para(&format!("Filler {i}"), ParaProps::default())).collect();
    d.body.push(heading("Intro", 1, true));
    d.body.push(para(&"Hidden words ".repeat(120), ParaProps { keep_lines: Some(true), ..ParaProps::default() }));
    d.body.push(heading("Part", 2, false));
    let mut t = Table::new(2, 2, 100.0);
    if let Some(Block::Para(p)) = t.rows.first_mut().and_then(|r| r.cells.first_mut()).and_then(|c| c.blocks.first_mut()).map(Arc::make_mut) {
        *p = Paragraph::with_text("In a hidden cell", Default::default());
    }
    d.body.push(Arc::new(Block::Table(t)));
    d.body.push(heading("Next", 1, false));
    d.body.push(para("End", ParaProps::default()));
    d
}

/// The same document without what "Intro" hides.
fn without_hidden(d: &Document) -> Document {
    let mut v = d.clone();
    let intro = v.body.iter().position(|b| b.as_para().is_some_and(|p| p.text == "Intro")).unwrap();
    v.body.drain(intro + 1..intro + 4);
    v
}

#[test]
fn hidden_blocks_take_no_room_and_are_not_drawn() {
    for view in [ViewMode::Print, ViewMode::Web, ViewMode::Draft, ViewMode::Read] {
        for filler in [0, 30, 38, 40, 42, 44, 46] {
            let d = doc(filler);
            let l = lay(&d, view);
            let got = placed(&d, &l);
            let texts: Vec<&str> = got.iter().map(|(t, ..)| t.as_str()).collect();
            assert!(texts.contains(&"Intro") && texts.contains(&"Next") && texts.contains(&"End"), "{view:?} {filler}: {texts:?}");
            assert!(!texts.iter().any(|t| t.starts_with("Hidden") || *t == "Part" || *t == "In a hidden cell"), "{view:?} {filler}: {texts:?}");
            // Nothing of the hidden table is drawn either (its cell borders, its cell paragraphs).
            assert!(
                !l.index.keys().any(|(_, p)| p.0.first().is_some_and(|b| (filler + 1..filler + 4).contains(&(*b as usize)))),
                "{view:?} {filler}"
            );
            // Pages, positions and keep-with-next are as if the hidden blocks weren't there.
            let v = without_hidden(&d);
            let lv = lay(&v, view);
            assert_eq!(l.pages.len(), lv.pages.len(), "{view:?} {filler}");
            assert_eq!(got, placed(&v, &lv), "{view:?} {filler}");
        }
    }
}

/// Unfolded, the same document shows everything (the flag is what hides it).
#[test]
fn expanded_heading_shows_its_section() {
    let mut d = doc(0);
    for b in &mut d.body {
        if let Block::Para(p) = Arc::make_mut(b) {
            p.props.collapsed = None;
        }
    }
    let got = placed(&d, &lay(&d, ViewMode::Print));
    assert!(got.iter().any(|(t, ..)| t == "Part") && got.iter().any(|(t, ..)| t == "In a hidden cell"));
}

/// Outline view has its own collapsing (view state); the stored flag doesn't hide anything there.
#[test]
fn outline_view_ignores_the_stored_flag() {
    let d = doc(0);
    let got = placed(&d, &lay(&d, ViewMode::Outline));
    assert!(got.iter().any(|(t, ..)| t == "Part"), "{got:?}");
}

/// The fold arrow (#1745) sits in the left margin beside each heading's first line, and says
/// whether the heading is collapsed; body text has none.
#[test]
fn fold_arrows_sit_beside_headings() {
    let d = doc(0);
    let l = lay(&d, ViewMode::Print);
    let arrows = l.fold_arrows(0);
    let blocks: Vec<(usize, bool)> = arrows.iter().map(|a| (a.block, a.collapsed)).collect();
    assert_eq!(blocks, [(0, true), (4, false)]);
    let body = l.pages[0].body;
    for a in &arrows {
        assert!(a.rect.x + a.rect.w <= body.x && a.rect.x >= body.x - 20.0, "{a:?} {body:?}");
        let c = (a.rect.x + a.rect.w / 2.0, a.rect.y + a.rect.h / 2.0);
        assert_eq!(l.fold_arrow_at(0, c.0, c.1), Some(a.block));
    }
    assert_eq!(l.fold_arrow_at(0, body.x + 50.0, arrows[0].rect.y + 2.0), None);
    assert!(l.fold_arrows(99).is_empty());
}

/// Notes referenced only from hidden text are left out alike: a footnote's note isn't placed
/// (its reference isn't), and neither is an endnote's at the end of the document.
#[test]
fn notes_referenced_from_hidden_text_are_left_out_alike() {
    use wordcraft_doc::para::{InlineObject, NoteKind};
    use wordcraft_doc::{PartKind, Pos};
    let mut d = doc(0);
    let note = |d: &mut Document, kind: PartKind, text: &str| d.add_part(kind, vec![para(text, ParaProps::default())]);
    let foot = note(&mut d, PartKind::Footnote, "Hidden footnote");
    let end = note(&mut d, PartKind::Endnote, "Hidden endnote");
    let shown_end = note(&mut d, PartKind::Endnote, "Shown endnote");
    for (block, kind, id) in [(1, NoteKind::Footnote, foot), (1, NoteKind::Endnote, end), (5, NoteKind::Endnote, shown_end)] {
        d.insert_object(&Pos::body(block, 0), InlineObject::NoteRef { kind, id, custom: String::new() }, &Default::default()).unwrap();
    }
    let l = lay(&d, ViewMode::Print);
    let parts: Vec<u32> = l
        .pages
        .iter()
        .flat_map(|p| p.items.iter())
        .filter_map(|it| if let Placed::Lines { story: StoryRef::Part(id), .. } = it { Some(*id) } else { None })
        .collect();
    assert!(parts.contains(&shown_end), "{parts:?}");
    assert!(!parts.contains(&foot) && !parts.contains(&end), "{parts:?}");
}

/// A right-to-left heading's arrow sits on its start side, the right, and says so.
#[test]
fn a_right_to_left_headings_arrow_is_on_the_right() {
    let mut d = doc(0);
    if let Some(Block::Para(p)) = d.body.first_mut().map(Arc::make_mut) {
        p.props.bidi = Some(true);
    }
    let l = lay(&d, ViewMode::Print);
    let arrows = l.fold_arrows(0);
    let body = l.pages[0].body;
    let rtl = arrows.iter().find(|a| a.block == 0).unwrap();
    assert!(rtl.rtl && rtl.rect.x >= body.x + body.w, "{rtl:?} {body:?}");
    let ltr = arrows.iter().find(|a| a.block == 4).unwrap();
    assert!(!ltr.rtl && ltr.rect.x + ltr.rect.w <= body.x, "{ltr:?}");
    // The line beside it: hovering there shows the arrow, the rest of the page doesn't.
    assert!(ltr.line.0 >= body.x - 0.5 && ltr.line.1 <= body.x + body.w + 0.5, "{ltr:?}");
}
