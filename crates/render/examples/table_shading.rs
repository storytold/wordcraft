use wordcraft_doc::props::{Border, BorderStyle, Borders, LineSpacing};
use wordcraft_doc::{Block, Document};
use wordcraft_doc::{Paragraph, Rgb, Table, para_block};

const SHADE: Rgb = Rgb(45, 105, 165);
const PAGE_HEIGHT: f32 = 200.0;
const PAGE_MARGIN: f32 = 20.0;
const LINE_HEIGHT: f32 = 20.0;

#[derive(Clone, Copy)]
enum ShadeFlow {
    Whole,
    Partial,
    Widow,
}

#[derive(Clone, Copy)]
enum ShadeKind {
    Paragraph,
    Cell,
    Both,
}

fn shade_doc(flow: ShadeFlow, kind: ShadeKind) -> Document {
    let mut d = Document::new();
    d.last_section.page_h = PAGE_HEIGHT;
    d.last_section.page_w = 400.0;
    d.last_section.margin_top = PAGE_MARGIN;
    d.last_section.margin_bottom = PAGE_MARGIN;
    d.last_section.margin_left = PAGE_MARGIN;
    d.last_section.margin_right = PAGE_MARGIN;
    let para = |text: &str, after, shaded| {
        let mut p =
            Paragraph::with_text(text, wordcraft_doc::CharProps { font: Some("Source Sans 3".into()), size: Some(12.0), ..Default::default() });
        p.props.space_before = Some(0.0);
        p.props.space_after = Some(after);
        p.props.line_spacing = Some(LineSpacing::Exactly(LINE_HEIGHT));
        p.props.widow_control = Some(false);
        p.props.shading = shaded;
        para_block(p)
    };
    let mut t = Table::new(1, 2, 360.0);
    t.props.style = None;
    t.props.cell_margins = Some([4.0, 6.0, 0.0, 6.0]);
    let none = Some(Border { style: BorderStyle::None, ..Default::default() });
    t.props.borders = Some(Borders { top: none, bottom: none, left: none, right: none, between: none, inside_v: none });
    for (index, cell) in t.rows[0].cells.iter_mut().enumerate() {
        cell.props.shading = matches!(kind, ShadeKind::Cell | ShadeKind::Both).then_some(SHADE);
        let shade = (index == 0 && matches!(kind, ShadeKind::Paragraph | ShadeKind::Both)).then_some(SHADE);
        cell.blocks = match flow {
            ShadeFlow::Whole => vec![para("Kept", 0.0, None), para("Moved shade", 130.0, shade)],
            ShadeFlow::Widow => {
                let mut kept = para(&["Kept shade"; 2].join("\n"), 0.0, shade);
                let Block::Para(p) = std::sync::Arc::make_mut(&mut kept) else { return d };
                p.props.widow_control = Some(true);
                vec![para(&["Kept"; 6].join("\n"), 0.0, None), kept]
            }
            ShadeFlow::Partial => vec![para(&vec!["Shade"; 20].join("\n"), 0.0, shade)],
        };
    }
    d.body = vec![std::sync::Arc::new(Block::Table(t))];
    d
}

#[derive(Clone, Copy)]
enum ReviewCase {
    Spacing,
    Partial,
    Widow,
    Cell,
    Equal,
    Closing,
}

impl ReviewCase {
    fn name(self) -> &'static str {
        match self {
            Self::Spacing => "spacing",
            Self::Partial => "partial",
            Self::Widow => "widow",
            Self::Cell => "cell",
            Self::Equal => "equal",
            Self::Closing => "closing",
        }
    }

    fn document(self) -> Document {
        let (flow, kind) = match self {
            Self::Spacing => (ShadeFlow::Whole, ShadeKind::Paragraph),
            Self::Widow => (ShadeFlow::Widow, ShadeKind::Paragraph),
            Self::Cell => (ShadeFlow::Partial, ShadeKind::Cell),
            Self::Equal => (ShadeFlow::Partial, ShadeKind::Both),
            Self::Partial | Self::Closing => (ShadeFlow::Partial, ShadeKind::Paragraph),
        };
        let mut doc = shade_doc(flow, kind);
        if matches!(self, Self::Closing)
            && let Some(block) = doc.body.first_mut()
            && let Block::Table(table) = std::sync::Arc::make_mut(block)
        {
            table.props.cell_margins = Some([4.0, 6.0, 6.0, 6.0]);
            if let Some(borders) = table.props.borders.as_mut() {
                borders.bottom = Some(Border::single(2.0));
            }
        }
        doc
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os().nth(1).map(std::path::PathBuf::from).ok_or("provide an output directory")?;
    std::fs::create_dir_all(&output)?;
    for case in [ReviewCase::Spacing, ReviewCase::Partial, ReviewCase::Widow, ReviewCase::Cell, ReviewCase::Equal, ReviewCase::Closing] {
        let doc = case.document();
        let layout = wordcraft_layout::layout(&doc, &mut wordcraft_layout::LayoutCache::new(), &Default::default());
        for (index, page) in layout.pages.iter().enumerate() {
            let image = wordcraft_render::render_page(&doc, page, 2.0, &Default::default());
            std::fs::write(output.join(format!("{}-{}.png", case.name(), index + 1)), image.to_png())?;
        }
        println!("{}: {} pages", case.name(), layout.pages.len());
    }
    Ok(())
}
