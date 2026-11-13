//! #1341: paragraphs past the line limit are counted, not silently cut.

use crate::*;

fn lay(doc: &Document) -> DocLayout {
    let mut c = LayoutCache::new();
    layout(doc, &mut c, &LayoutOptions::default())
}

/// #1341: a paragraph past the line limit stops laying out (a guard against hangs), and the
/// layout counts it instead of hiding the cut: nothing after the limit is placed.
#[test]
fn paragraphs_past_the_line_limit_are_counted() {
    let doc = |lines: usize| {
        let text = (1..=lines).map(|i| if i == lines { "TailEnd".to_string() } else { format!("L{i}") }).collect::<Vec<_>>().join("\n");
        let mut d = Document::new();
        d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&text, Default::default()))];
        (d, text)
    };
    // Placed text ends where the last placed line stops.
    let shown_to = |l: &DocLayout| {
        l.pages
            .iter()
            .flat_map(|p| p.items.iter())
            .filter_map(|it| if let Placed::Lines { para, l1, .. } = it { para.lines.get(l1.saturating_sub(1)).map(|ln| ln.stop) } else { None })
            .max()
            .unwrap_or(0)
    };
    // At the limit: whole, nothing counted.
    let (d, text) = doc(3);
    let l = para::with_line_limit(3, || lay(&d));
    assert_eq!(l.truncated_paragraphs, 0);
    assert_eq!(shown_to(&l), text.len());
    // One line more: cut at the limit, counted, and the tail isn't drawn.
    let (d, text) = doc(4);
    let l = para::with_line_limit(3, || lay(&d));
    assert_eq!(l.truncated_paragraphs, 1);
    let tail = text.find("TailEnd").unwrap();
    assert!(shown_to(&l) <= tail, "shown to {} of {}", shown_to(&l), text.len());
    // The production limit leaves ordinary paragraphs alone.
    assert_eq!(lay(&d).truncated_paragraphs, 0);
    // A header paragraph cut at the limit is counted too (#1341 review).
    let mut d = Document::from_text("Body");
    let hp = wordcraft_doc::Paragraph::with_text("L1\nL2\nL3\nTailEnd", Default::default());
    let id = d.add_part(wordcraft_doc::PartKind::Header, vec![wordcraft_doc::para_block(hp)]);
    d.last_section.headers.default = Some(id);
    assert_eq!(para::with_line_limit(3, || lay(&d)).truncated_paragraphs, 1);
}

/// #1341: the cut-paragraph warning is logged once per revision of the cut paragraphs, not on
/// every relayout.
#[test]
fn the_line_limit_warning_is_logged_once_per_revision() {
    let mut d = Document::new();
    d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("L1\nL2\nL3\nTailEnd", Default::default()))];
    let mut c = LayoutCache::new();
    let opts = LayoutOptions::default();
    para::with_line_limit(3, || {
        layout(&d, &mut c, &opts);
        layout(&d, &mut c, &opts);
        assert_eq!(c.line_limit_warnings, 1, "a relayout of the same text doesn't warn again");
        d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().insert_text(0, "X", &Default::default()).unwrap();
        layout(&d, &mut c, &opts);
        assert_eq!(c.line_limit_warnings, 2, "an edit to the cut paragraph warns again");
    });
}
