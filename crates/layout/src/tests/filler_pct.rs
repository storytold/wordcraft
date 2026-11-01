use super::*;
use wordcraft_doc::props::{LineSpacing, ParaProps};
use wordcraft_doc::{PartKind, Table};

#[derive(Clone, Copy, Debug)]
enum MarginStory {
    Header,
    Footer,
}

fn filler_document(kind: MarginStory, start: SectionStart) -> Document {
    let mut doc = Document::new();
    doc.styles = wordcraft_doc::StyleSheet::empty();
    doc.styles.default_chr = CharProps { font: Some("Source Sans 3".into()), size: Some(10.0), ..Default::default() };
    doc.styles.default_para =
        ParaProps { line_spacing: Some(LineSpacing::Exactly(14.0)), space_before: Some(0.0), space_after: Some(0.0), ..Default::default() };
    doc.last_section.margin_top = 10.0;
    doc.last_section.margin_bottom = 10.0;
    doc.last_section.header = 35.0;
    doc.last_section.footer = 35.0;
    let mut table = Table::new(1, 1, 120.0);
    table.props.style = None;
    table.props.width_pct = Some(100.0);
    table.props.cell_margins = Some([0.0; 4]);
    table.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(Paragraph::with_text(&"Width probe ".repeat(12), Default::default()))];
    let id = doc.add_part(
        match kind {
            MarginStory::Header => PartKind::Header,
            MarginStory::Footer => PartKind::Footer,
        },
        vec![Arc::new(Block::Table(table))],
    );
    let mut first = doc.last_section.clone();
    first.page_w = first.margin_left + first.margin_right + 120.0;
    match kind {
        MarginStory::Header => first.headers.default = Some(id),
        MarginStory::Footer => first.footers.default = Some(id),
    }
    let mut p = Paragraph::with_text(if start == SectionStart::OddPage { "Initial\u{000c}More" } else { "Initial" }, Default::default());
    p.section = Some(Box::new(first));
    let mut inherited = Paragraph::with_text("Inherited", Default::default());
    inherited.section = Some(Box::new(doc.last_section.clone()));
    doc.last_section.start = start;
    doc.body = vec![
        wordcraft_doc::para_block(p),
        wordcraft_doc::para_block(inherited),
        wordcraft_doc::para_block(Paragraph::with_text("Final", Default::default())),
    ];
    doc
}

#[test]
fn filler_header_defining_width() {
    filler_defining_width(MarginStory::Header);
}

#[test]
fn filler_footer_defining_width() {
    filler_defining_width(MarginStory::Footer);
}

fn filler_defining_width(kind: MarginStory) {
    for start in [SectionStart::EvenPage, SectionStart::OddPage] {
        for color in [None, Some(wordcraft_doc::Rgb::WHITE)] {
            let mut doc = filler_document(kind, start);
            doc.settings.page_color = color;
            let original = doc.clone();
            let laid = layout(&doc, &mut LayoutCache::new(), &LayoutOptions::default());
            let filler = laid.pages.len() - 2;
            assert_eq!(filler, if start == SectionStart::EvenPage { 2 } else { 3 });
            let normal = laid.pages[filler - 1].body;
            let blank = laid.pages[filler].body;
            assert!((normal.y - blank.y).abs() < 0.01, "{kind:?} {start:?}: normal {normal:?}, filler {blank:?}");
            assert!((normal.bottom() - blank.bottom()).abs() < 0.01, "{kind:?} {start:?}: normal {normal:?}, filler {blank:?}");
            assert_eq!(doc, original);
            let id = doc.para(StoryRef::Body, &Path::top(0)).unwrap().section.as_ref().unwrap();
            let owned = match kind {
                MarginStory::Header => id.headers.default,
                MarginStory::Footer => id.footers.default,
            };
            let inherited = doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap().section.as_mut().unwrap();
            match kind {
                MarginStory::Header => inherited.headers.default = owned,
                MarginStory::Footer => inherited.footers.default = owned,
            }
            let explicit = layout(&doc, &mut LayoutCache::new(), &LayoutOptions::default());
            let own = explicit.pages[filler].body;
            assert!((own.y - blank.y).abs() > 10.0 || (own.bottom() - blank.bottom()).abs() > 10.0, "explicit story must use its wider section");
        }
    }
}
