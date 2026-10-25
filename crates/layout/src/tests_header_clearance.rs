use super::*;
use crate::tests::rect_shape;
use wordcraft_doc::{PartKind, Table};

fn lay(doc: &Document) -> DocLayout {
    layout(doc, &mut LayoutCache::new(), &LayoutOptions::default())
}

fn header_shape(doc: &mut Document, y: f32, h: f32) {
    let mut p = Paragraph::new();
    let float = Float { wrap: Wrap::TopAndBottom, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 72.0, y, dist: 0.0, ..Default::default() };
    p.insert_object(0, rect_shape(468.0, h, float), &Default::default()).unwrap();
    let id = doc.add_part(PartKind::Header, vec![wordcraft_doc::para_block(p)]);
    doc.last_section.headers.default = Some(id);
}

#[test]
fn repeated_rows_clear_header() {
    for headers in [1, 2] {
        let mut doc = Document::new();
        let mut table = Table::new(12, 1, 468.0);
        for (i, row) in table.rows.iter_mut().enumerate() {
            row.props.header = i < headers;
            row.props.height_rule = HeightRule::Exact;
            row.props.height = Some(if i < headers { 20.0 } else { 100.0 });
            row.props.cant_split = true;
        }
        doc.body = vec![Arc::new(Block::Table(table))];
        // The first header row ends before the obstacle. Later header/body rows meet it.
        header_shape(&mut doc, 105.0, 30.0);
        let before = serde_json::to_value(&doc).unwrap();
        let laid = lay(&doc);
        assert!(laid.pages.len() > 1);
        assert_eq!(serde_json::to_value(&doc).unwrap(), before);
        for (pi, page) in laid.pages.iter().enumerate() {
            for item in &page.items {
                if let Placed::Cell { rect, story: StoryRef::Body, .. } = item {
                    assert!(rect.bottom() <= 105.01 || rect.y >= 134.99, "page {pi}: {rect:?} crosses header exclusion");
                }
            }
        }
    }
}

#[test]
fn initial_float_clears_header() {
    let mut doc = Document::new();
    let mut table = Table::new(1, 1, 468.0);
    table.rows[0].props.height = Some(40.0);
    table.rows[0].props.height_rule = HeightRule::Exact;
    table.props.float = Some(TableFloat { h_rel: Anchor::Page, v_rel: Anchor::Page, x: 72.0, y: 72.0, ..Default::default() });
    doc.body = vec![Arc::new(Block::Table(table))];
    header_shape(&mut doc, 72.0, 40.0);
    let before = serde_json::to_value(&doc).unwrap();
    let laid = lay(&doc);
    assert_eq!(serde_json::to_value(&doc).unwrap(), before);
    let cell = laid.pages[0]
        .items
        .iter()
        .find_map(|it| match it {
            Placed::Cell { rect, story: StoryRef::Body, .. } => Some(rect),
            _ => None,
        })
        .unwrap();
    assert!(cell.y >= 111.99, "initial float crosses header exclusion: {cell:?}");
}

enum TotalValue {
    Live,
    Literal,
}

fn total_header(value: TotalValue) -> Document {
    let mut doc = Document::from_text(&vec!["Body words wrapping near the header shape. ".repeat(12); 10].join("\n"));
    doc.styles.default_chr.font = Some("Source Sans 3".into());
    for i in 1..10 {
        doc.para_mut(StoryRef::Body, &Path::top(i)).unwrap().props.page_break_before = Some(true);
    }
    let mut p = Paragraph::new();
    for _ in 0..20 {
        let chr = CharProps { size: Some(24.0), ..Default::default() };
        match value {
            TotalValue::Live => {
                p.insert_object(
                    p.len(),
                    InlineObject::Field {
                        instr: "NUMPAGES".into(),
                        result: String::new(),
                        locked: false,
                        dirty: false,
                        code: false,
                        form: String::new(),
                    },
                    &chr,
                )
                .unwrap();
            }
            TotalValue::Literal => {
                p.insert_text(p.len(), "10", &chr).unwrap();
            }
        }
        p.insert_text(p.len(), " ", &CharProps { size: Some(24.0), ..Default::default() }).unwrap();
    }
    let mut shape = Paragraph::new();
    let float = Float { wrap: Wrap::Square, h_rel: Anchor::Page, v_rel: Anchor::Paragraph, x: 72.0, y: 80.0, dist: 0.0, ..Default::default() };
    shape.insert_object(0, rect_shape(200.0, 80.0, float), &Default::default()).unwrap();
    let id = doc.add_part(PartKind::Header, vec![wordcraft_doc::para_block(p), wordcraft_doc::para_block(shape)]);
    doc.last_section.headers.default = Some(id);
    doc
}

type LineGeometry = (usize, usize, f32);
type BodyGeometry = (usize, u32, f32, f32, Vec<LineGeometry>);

fn body_geometry(laid: &DocLayout) -> Vec<BodyGeometry> {
    laid.pages
        .iter()
        .enumerate()
        .flat_map(|(pi, page)| {
            page.items.iter().filter_map(move |item| match item {
                Placed::Lines { story: StoryRef::Body, path, para, x, y, .. } => {
                    Some((pi, path.0[0], *x, *y, para.lines.iter().map(|line| (line.c0, line.c1, line.left)).collect()))
                }
                _ => None,
            })
        })
        .collect()
}

#[test]
fn header_totals_reflow_body() {
    let doc = total_header(TotalValue::Live);
    let before = serde_json::to_value(&doc).unwrap();
    let actual = lay(&doc);
    let expected = lay(&total_header(TotalValue::Literal));
    assert_eq!(actual.pages.len(), 10);
    assert_eq!(expected.pages.len(), 10);
    assert_eq!(serde_json::to_value(&doc).unwrap(), before);
    assert_eq!(body_geometry(&actual), body_geometry(&expected), "a live total and its final literal value must wrap body text identically");
}

#[test]
fn repeat_page_clears_header() {
    for headers in [1, 2] {
        let mut doc = Document::new();
        let mut table = Table::new(12, 1, 468.0);
        for (i, row) in table.rows.iter_mut().enumerate() {
            row.props.header = i < headers;
            row.props.height_rule = HeightRule::Exact;
            row.props.height = Some(if i < headers { 20.0 } else { 100.0 });
            row.props.cant_split = true;
        }
        doc.body = vec![Arc::new(Block::Table(table))];
        header_shape(&mut doc, 105.0, 30.0);
        let first = doc.add_part(PartKind::Header, vec![wordcraft_doc::para_block(Paragraph::new())]);
        doc.last_section.title_page = true;
        doc.last_section.headers.first = Some(first);
        let before = serde_json::to_value(&doc).unwrap();
        let laid = lay(&doc);
        assert_eq!(serde_json::to_value(&doc).unwrap(), before);
        assert!(laid.pages.len() > 1);
        for (pi, page) in laid.pages.iter().enumerate().skip(1) {
            let mut rows = 0;
            for item in &page.items {
                if let Placed::Cell { rect, story: StoryRef::Body, .. } = item {
                    rows += 1;
                    assert!(rect.bottom() <= 105.01 || rect.y >= 134.99, "headers={headers}, page {pi}: {rect:?}");
                }
            }
            assert!(rows > headers, "each continued page retains repeated headers and a body row");
        }
    }
}

#[test]
fn header_counters_stay_isolated() {
    let mut doc = total_header(TotalValue::Live);
    let num = doc.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    let numbering = wordcraft_doc::props::NumRef { num, level: 0 };
    for block in &mut doc.body {
        let Block::Para(p) = Arc::make_mut(block) else { panic!("fixture paragraph") };
        p.props.numbering = Some(numbering);
    }
    let id = doc.last_section.headers.default.unwrap();
    // Only the even story is numbered, isolating per-page wrap measurement from default-story height measurement.
    let mut blocks = doc.parts.get(&id).unwrap().blocks.clone();
    let Block::Para(p) = Arc::make_mut(&mut blocks[0]) else { panic!("fixture header") };
    p.props.numbering = Some(numbering);
    let even = doc.add_part(PartKind::Header, blocks);
    doc.settings.even_odd_headers = true;
    doc.last_section.headers.even = Some(even);
    let before = serde_json::to_value(&doc).unwrap();
    let laid = lay(&doc);
    assert_eq!(serde_json::to_value(&doc).unwrap(), before);
    let labels: Vec<_> = laid
        .pages
        .iter()
        .flat_map(|page| page.items.iter())
        .filter_map(|item| match item {
            Placed::Lines { story: StoryRef::Body, para, .. } => para.label.as_ref().map(|label| label.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(labels, (1..=10).map(|n| format!("{n}.")).collect::<Vec<_>>());
}

#[test]
fn plain_header_total_stays_local() {
    let mut doc = total_header(TotalValue::Live);
    let id = doc.last_section.headers.default.unwrap();
    doc.parts.get_mut(&id).unwrap().blocks.truncate(1);
    let totals = Totals::first_guess(doc.sections().len());
    let pass = layout_pass(&doc, &mut LayoutCache::new(), &LayoutOptions::default(), &HashMap::new(), &totals, true, (&EarlyFloats::new(), true));
    assert!(matches!(pass, Pass::Done(laid) if laid.pages.len() == 10), "a header without exclusions must not request a body repeat");
}
