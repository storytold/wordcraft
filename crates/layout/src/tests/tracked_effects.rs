use super::*;
use wordcraft_doc::wordart::{TextEffects, TextFill};

/// A tracked insertion whose run has both a legacy outline and a WordArt fill: with markup shown
/// (revision colours) it keeps the legacy outline and drops only the WordArt fill (#2534).
#[test]
fn tracked_text_in_revision_colours_keeps_its_legacy_effects() {
    let mut d = Document::from_text("Outlined");
    d.revisions.push(wordcraft_doc::Revision {
        kind: wordcraft_doc::RevisionKind::Insert,
        author: "Ana".into(),
        date: String::new(),
        move_name: None,
    });
    let wordart = TextFill::Solid { color: wordcraft_doc::props::Rgb::WHITE, transparency: 0.0 };
    d.format_range(&Pos::body(0, 0), &Pos::body(0, 8), &|c| {
        c.outline = Some(true);
        c.text_effects = Some(Box::new(TextEffects { fill: Some(wordart.clone()), ..Default::default() }));
        c.ins = Some(0);
    })
    .unwrap();
    let l = lay(&d);
    let art = |markup: bool| {
        flat(display::page_display(&d, &l.pages[0], &display::DisplayOptions { markup, ..Default::default() }))
            .into_iter()
            .find_map(|i| if let display::Draw::Art { fx, .. } = i { Some(fx) } else { None })
    };
    let shown = art(true).expect("the tracked run keeps its legacy outline");
    assert_eq!(shown.fill, Some(TextFill::None), "hollow letters from the legacy outline, not the WordArt fill");
    assert!(shown.outline.is_some());
    // Without markup the run isn't in revision colours, and WordArt's fill applies.
    assert_eq!(art(false).and_then(|fx| fx.fill), Some(wordart));
}
