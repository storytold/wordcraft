use std::sync::Arc;

use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::NumRef;
use wordcraft_doc::{Block, CharProps, Document, ListKind, Paragraph, Table, Watermark, para_block};

use crate::*;

/// Verify preset lines and connectors emit the requested PDF line-cap operator.
#[test]
fn shape_line_caps_reach_pdf_content() {
    use wordcraft_doc::{
        connector::{LineCap as ShapeCap, ShapeExtra},
        para::{Anchor, Float, ShapeKind, Wrap},
    };
    for kind in [ShapeKind::Line, ShapeKind::StraightConnector] {
        for (cap, operator) in [(ShapeCap::Flat, None), (ShapeCap::Round, Some("1 J")), (ShapeCap::Square, Some("2 J"))] {
            let shape = InlineObject::Shape {
                kind,
                w: 40.0,
                h: 0.0,
                fill: None,
                stroke: Some(wordcraft_doc::props::Rgb::BLACK),
                stroke_width: 10.0,
                float: Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 100.0, y: 100.0, ..Default::default() },
                story: None,
                freeform: None,
                effects: Default::default(),
                extra: ShapeExtra { cap, ..Default::default() },
            };
            let mut p = Paragraph::new();
            p.insert_object(0, shape, &Default::default()).unwrap();
            let mut d = Document::new();
            d.body = vec![para_block(p)];
            let bytes = export(&d, &PdfOptions { compress: false, ..Default::default() }).unwrap();
            let raw = String::from_utf8_lossy(&bytes);
            // Content operators can share a line: PDF separates them by whitespace.
            let tokens: Vec<_> = raw.split_ascii_whitespace().collect();
            let has_operator = |op: &str| {
                let expected: Vec<_> = op.split_ascii_whitespace().collect();
                tokens.windows(2).any(|pair| pair == expected.as_slice())
            };
            if let Some(op) = operator {
                assert!(has_operator(op), "{kind:?} {cap:?}: missing PDF cap operator {op}");
            } else {
                assert!(!has_operator("1 J") && !has_operator("2 J"), "flat cap must remain butt");
            }
        }
    }
}

fn png() -> Vec<u8> {
    let img = image::RgbaImage::from_fn(16, 8, |x, y| image::Rgba([(x * 15) as u8, (y * 30) as u8, 200, 255]));
    let mut buf = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png).unwrap();
    buf
}

/// Headings, a bold run, a hyperlink, an internal link, a list, a table and a picture.
fn sample() -> Document {
    let mut d = Document::new();
    d.core.title = "PDF Sample".into();
    d.core.creator = "Ada Writer".into();
    d.core.subject = "Testing".into();
    d.core.keywords = "pdf, export".into();
    d.core.created = "2024-05-06T07:08:09Z".into();
    let mut b = Vec::new();
    let mut h1 = Paragraph::with_text("Introduction", CharProps::default()).styled("Heading1");
    let n = h1.len();
    h1.insert_object(n, InlineObject::BookmarkStart { name: "intro".into() }, &CharProps::default()).unwrap();
    b.push(para_block(h1));
    let mut p = Paragraph::with_text("Hello WordCraft, ", CharProps::default());
    let n = p.len();
    p.insert_text(n, "bold words", &CharProps { bold: Some(true), ..Default::default() }).unwrap();
    let n = p.len();
    p.insert_text(n, " and a website", &CharProps { link: Some("https://example.com/".into()), ..Default::default() }).unwrap();
    let n = p.len();
    p.insert_text(n, " back to top", &CharProps { link: Some("#intro".into()), italic: Some(true), ..Default::default() }).unwrap();
    b.push(para_block(p));
    b.push(para_block(Paragraph::with_text("Details", CharProps::default()).styled("Heading2")));
    let list = d.numbering.add_list(ListKind::Numbered);
    for t in ["alpha item", "beta item"] {
        let mut q = Paragraph::with_text(t, CharProps::default()).styled("ListParagraph");
        q.props.numbering = Some(NumRef { num: list, level: 0 });
        b.push(para_block(q));
    }
    let mut t = Table::new(2, 2, 400.0);
    for (r, c, s) in [(0, 0, "Cell00"), (0, 1, "Cell01"), (1, 0, "Cell10"), (1, 1, "Cell11")] {
        t.rows[r].cells[c].blocks = vec![para_block(Paragraph::with_text(s, CharProps::default()))];
    }
    b.push(Arc::new(Block::Table(t)));
    let key = d.add_media(png(), "png");
    let mut ip = Paragraph::with_text("Figure: ", CharProps::default());
    let n = ip.len();
    ip.insert_object(
        n,
        InlineObject::Image {
            media: key,
            w: 80.0,
            h: 40.0,
            alt: "gradient".into(),
            float: Default::default(),
            crop: [0.1, 0.0, 0.0, 0.0],
            ole: None,
        },
        &CharProps::default(),
    )
    .unwrap();
    b.push(para_block(ip));
    let mut big = Paragraph::with_text("Second page", CharProps::default()).styled("Heading1");
    big.props.page_break_before = Some(true);
    b.push(para_block(big));
    b.push(para_block(Paragraph::with_text(
        "Dotted and wavy",
        CharProps { underline: Some(wordcraft_doc::props::Underline::Wave), ..Default::default() },
    )));
    d.body = b;
    d.settings.watermark = Some(Watermark::default());
    d.settings.page_color = Some(Rgb(0xFF, 0xFE, 0xF0));
    d
}

/// Text of every page through hayro's interpreter (glyph → Unicode via ToUnicode / ActualText).
fn extract_text(bytes: &[u8]) -> Vec<String> {
    use hayro_interpret::font::Glyph;
    use hayro_interpret::{
        BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask,
        interpret_page,
    };
    use hayro_syntax::Pdf;
    struct Ex(String);
    impl Device<'_> for Ex {
        fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
        fn set_blend_mode(&mut self, _: BlendMode) {}
        fn draw_path(&mut self, _: &kurbo::BezPath, _: kurbo::Affine, _: &Paint<'_>, _: &PathDrawMode) {}
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
        fn draw_glyph(&mut self, g: &Glyph<'_>, _: kurbo::Affine, _: kurbo::Affine, _: &Paint<'_>, mode: &GlyphDrawMode) {
            // Synthetic bold fills and then strokes each glyph: one character, two draws.
            if matches!(mode, GlyphDrawMode::Stroke(_)) {
                return;
            }
            match g.as_unicode() {
                Some(hayro_cmap::BfString::Char(c)) => self.0.push(c),
                Some(hayro_cmap::BfString::String(s)) => self.0.push_str(&s),
                None => self.0.push('\u{FFFD}'),
            }
        }
        fn draw_image(&mut self, _: Image<'_, '_>, _: kurbo::Affine) {}
        fn pop_clip_path(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }
    let pdf = Pdf::new(bytes.to_vec()).expect("parse");
    let cache = InterpreterCache::new();
    pdf.pages()
        .iter()
        .map(|page| {
            let mut ctx =
                Context::new(kurbo::Affine::IDENTITY, kurbo::Rect::new(0.0, 0.0, 1.0, 1.0), &cache, pdf.xref(), InterpreterSettings::default());
            let mut ex = Ex(String::new());
            interpret_page(page, &mut ctx, &mut ex);
            ex.0
        })
        .collect()
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn exports_pages_text_and_structure() {
    let d = sample();
    let lay = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    assert!(lay.pages.len() >= 2);
    let bytes = export(&d, &PdfOptions::default()).unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    let pdf = hayro_syntax::Pdf::new(bytes.clone()).expect("parse");
    assert_eq!(pdf.pages().len(), lay.pages.len());
    let text = extract_text(&bytes);
    let all = squash(&text.concat());
    for w in [
        "Introduction",
        "Hello WordCraft,",
        "bold words",
        "website",
        "Details",
        "alpha item",
        "beta item",
        "Cell00",
        "Cell11",
        "Second page",
        "DRAFT",
    ] {
        assert!(all.contains(w), "{w:?} missing from {all:?}");
    }
    assert!(squash(&text[0]).contains("Hello WordCraft, bold words and a website back to top"), "{:?}", text[0]);

    let raw = String::from_utf8_lossy(&export(&d, &PdfOptions { compress: false, ..Default::default() }).unwrap()).into_owned();
    assert!(raw.contains("/FontFile2") || raw.contains("/FontFile3"), "embedded fonts");
    assert!(raw.contains("/ToUnicode"));
    assert!(raw.contains("/URI (https://example.com/)") || raw.contains("/URI(https://example.com/)"), "URL link");
    assert!(raw.contains("/Subtype /Link") || raw.contains("/Subtype/Link"));
    assert!(raw.contains("/Outlines"), "outline");
    assert!(raw.contains("Introduction"));
    assert!(raw.contains("/StructTreeRoot"), "tagged");
    assert!(raw.contains("/Image"), "picture");
    assert!(raw.contains("PDF Sample"), "title");
    assert!(raw.contains("Ada Writer"), "author");
    assert!(raw.contains("D:20240506070809"), "creation date");
}

#[test]
fn page_selection_and_errors() {
    let d = sample();
    let one = export(&d, &PdfOptions { pages: Some(vec![1]), tagged: false, ..Default::default() }).unwrap();
    let pdf = hayro_syntax::Pdf::new(one.clone()).unwrap();
    assert_eq!(pdf.pages().len(), 1);
    assert!(squash(&extract_text(&one)[0]).contains("Second page"));
    assert_eq!(export(&d, &PdfOptions { pages: Some(vec![99]), ..Default::default() }), Err(PdfError::BadPage(100)));
    assert_eq!(export(&d, &PdfOptions { pages: Some(vec![]), ..Default::default() }), Err(PdfError::NoPages));
}

#[test]
fn hostile_documents_export() {
    let mut d = Document::from_text("x");
    d.last_section.page_w = f32::NAN;
    d.last_section.page_h = 1e9;
    d.media.insert("bad.png".into(), Arc::new(b"not an image".to_vec()));
    let mut p = Paragraph::with_text("pic ", CharProps::default());
    p.insert_object(
        4,
        InlineObject::Image {
            media: "bad.png".into(),
            w: f32::INFINITY,
            h: -3.0,
            alt: String::new(),
            float: Default::default(),
            crop: [f32::NAN; 4],
            ole: None,
        },
        &CharProps::default(),
    )
    .unwrap();
    p.insert_object(
        0,
        InlineObject::Image { media: "missing".into(), w: 10.0, h: 10.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None },
        &CharProps::default(),
    )
    .unwrap();
    d.body.push(para_block(p));
    let mut q =
        Paragraph::with_text("Unknown font 😀 עברית", CharProps { font: Some("No Such Font".into()), size: Some(2000.0), ..Default::default() });
    q.props.style = Some("Heading9".into());
    d.body.push(para_block(q));
    d.settings.watermark = Some(Watermark { text: "   ".into(), ..Default::default() });
    let _ = export(&d, &PdfOptions::default());
    d.settings.watermark = Some(Watermark { text: "😀".repeat(500), font: "Nope".into(), diagonal: false, ..Default::default() });
    let _ = export(&d, &PdfOptions { tagged: false, ..Default::default() });
    assert!(export(&Document::new(), &PdfOptions::default()).unwrap().starts_with(b"%PDF"));
}

#[test]
fn tracked_deletions_are_left_out_without_markup() {
    use wordcraft_doc::{Revision, RevisionKind};
    let mut d = Document::new();
    d.revisions.push(Revision { kind: RevisionKind::Delete, author: "Ada".into(), date: String::new(), move_name: None });
    d.revisions.push(Revision { kind: RevisionKind::Insert, author: "Ada".into(), date: String::new(), move_name: None });
    let mut p = Paragraph::with_text("Keep ", CharProps::default());
    let n = p.len();
    p.insert_text(n, "DELETEDTEXT ", &CharProps { del: Some(0), ..Default::default() }).unwrap();
    let n = p.len();
    p.insert_text(n, "INSERTED", &CharProps { ins: Some(1), ..Default::default() }).unwrap();
    d.body = vec![para_block(p)];
    // The default export shows the final text, as Word's "No Markup" does.
    let text = squash(&extract_text(&export(&d, &PdfOptions::default()).unwrap()).concat());
    assert_eq!(text, "Keep INSERTED");
    // With markup the deletion is still there (struck through).
    let text = squash(&extract_text(&export(&d, &PdfOptions { include_markup: true, ..Default::default() }).unwrap()).concat());
    assert_eq!(text, "Keep DELETEDTEXT INSERTED");
}

#[test]
fn deleted_heading_text_stays_out_of_bookmarks_and_tags() {
    use wordcraft_doc::{Revision, RevisionKind};
    let mut d = Document::new();
    d.revisions.push(Revision { kind: RevisionKind::Delete, author: "Ada".into(), date: String::new(), move_name: None });
    let del = CharProps { del: Some(0), ..Default::default() };
    // "Intro SECRET Part" with "SECRET " deleted, then a heading deleted as a whole.
    let mut h = Paragraph::with_text("Intro ", CharProps::default()).styled("Heading1");
    let n = h.len();
    h.insert_text(n, "SECRET ", &del).unwrap();
    let n = h.len();
    h.insert_text(n, "Part", &CharProps::default()).unwrap();
    let gone = Paragraph::with_text("GONE", del.clone()).styled("Heading1");
    d.body = vec![para_block(h), para_block(Paragraph::with_text("Body text", CharProps::default())), para_block(gone)];
    let raw = |opts: PdfOptions| String::from_utf8_lossy(&export(&d, &PdfOptions { compress: false, ..opts }).unwrap()).into_owned();
    // Bookmarks alone (untagged), then bookmarks and heading tags: the final text only.
    for tagged in [false, true] {
        let pdf = raw(PdfOptions { tagged, ..Default::default() });
        assert!(!pdf.contains("SECRET") && !pdf.contains("GONE"), "tagged {tagged}: deleted heading text in the PDF");
        assert!(pdf.contains("/Outlines") && pdf.contains("Intro Part"), "tagged {tagged}: no bookmark for the heading");
    }
    // With markup the deleted text is shown, and so it is in the bookmarks.
    let pdf = raw(PdfOptions { include_markup: true, ..Default::default() });
    assert!(pdf.contains("Intro SECRET Part") && pdf.contains("GONE"));
}

#[test]
fn hidden_heading_text_stays_out_of_bookmarks_and_tags() {
    let mut d = Document::new();
    // Hidden text both directly and through a character style.
    d.styles.upsert(wordcraft_doc::Style {
        id: "Secret".into(),
        name: "Secret".into(),
        kind: wordcraft_doc::StyleKind::Character,
        chr: CharProps { hidden: Some(true), ..Default::default() },
        ..Default::default()
    });
    let hidden = CharProps { hidden: Some(true), ..Default::default() };
    // "Intro SECRET Part" with "SECRET " hidden, a heading hidden as a whole, and one hidden by style.
    let mut h = Paragraph::with_text("Intro ", CharProps::default()).styled("Heading1");
    let n = h.len();
    h.insert_text(n, "SECRET ", &hidden).unwrap();
    let n = h.len();
    h.insert_text(n, "Part", &CharProps::default()).unwrap();
    let gone = Paragraph::with_text("GONE", hidden.clone()).styled("Heading1");
    let styled = Paragraph::with_text("STYLED", CharProps { style: Some("Secret".into()), ..Default::default() }).styled("Heading1");
    d.body = vec![
        para_block(h),
        para_block(Paragraph::with_text("Body text", CharProps::default())),
        para_block(gone),
        para_block(styled),
        para_block(Paragraph::with_text("More text", CharProps::default())),
    ];
    let raw = |opts: PdfOptions| String::from_utf8_lossy(&export(&d, &PdfOptions { compress: false, ..opts }).unwrap()).into_owned();
    // The PDF never prints hidden text, with or without markup, so neither do bookmarks or tags.
    for (tagged, include_markup) in [(false, false), (true, false), (true, true)] {
        let pdf = raw(PdfOptions { tagged, include_markup, ..Default::default() });
        let what = format!("tagged {tagged} markup {include_markup}");
        assert!(!pdf.contains("SECRET") && !pdf.contains("GONE") && !pdf.contains("STYLED"), "{what}: hidden heading text in the PDF");
        assert!(pdf.contains("/Outlines") && pdf.contains("Intro Part"), "{what}: no bookmark for the heading");
    }
}

/// The CPU time `f` takes on this thread, in seconds. On Linux it is the scheduler's count of the
/// time the thread actually ran, so the time it waits while other work on a busy machine has the
/// CPU is left out; elsewhere, wall-clock time.
fn cpu_cost(f: &mut impl FnMut()) -> f64 {
    let ran = || {
        let s = std::fs::read_to_string("/proc/thread-self/schedstat").ok()?;
        s.split_whitespace().next()?.parse::<u64>().ok()
    };
    let (t, before) = (std::time::Instant::now(), ran());
    f();
    match (before, ran()) {
        (Some(a), Some(b)) => b.saturating_sub(a) as f64 * 1e-9,
        _ => t.elapsed().as_secs_f64(),
    }
}

/// How much more `big` costs than `small`, for `big` an input `scale` times the size of `small`'s:
/// (small, big, big / small), ~`scale` when linear and ~`scale`² when quadratic. Each is the
/// cheapest of several runs after a warm-up, the runs alternating so a busy spell slows both sizes
/// alike; `small` is timed `scale` times over per run, so both timings are long against the clock's
/// resolution.
fn cost_ratio(scale: usize, mut small: impl FnMut(), mut big: impl FnMut()) -> (f64, f64, f64) {
    small();
    big();
    let mut small_scaled = || (0..scale).for_each(|_| small());
    let (mut t1, mut tk) = (f64::INFINITY, f64::INFINITY);
    for _ in 0..5 {
        t1 = t1.min(cpu_cost(&mut small_scaled) / scale as f64);
        tk = tk.min(cpu_cost(&mut big));
    }
    (t1, tk, tk / t1.max(1e-9))
}

#[test]
fn long_fragmented_hidden_headings_export_in_linear_time() {
    // Headings of n chars in one-char runs: all hidden with bold alternating, and hidden and visible
    // alternating. Layout and the bookmark/tag titles once scanned every left-out range for every
    // char, quadratic in n. Machine-independent: the cost for 8n against n, linear ~8, quadratic ~64.
    let doc = |n: usize| {
        let heading = |visible_every_other: bool| {
            let mut h = Paragraph::with_text(&"abcdefg ".repeat(n / 8), CharProps::default()).styled("Heading1");
            h.runs = (0..n)
                .map(|i| {
                    let hidden = !(visible_every_other && i % 2 == 1);
                    wordcraft_doc::Run { len: 1, props: CharProps { hidden: Some(hidden), bold: Some(i % 4 < 2), ..Default::default() } }
                })
                .collect();
            para_block(h)
        };
        let mut d = Document::new();
        d.body = vec![heading(false), heading(true), para_block(Paragraph::with_text("After", CharProps::default()))];
        d
    };
    let opts = PdfOptions { compress: false, ..Default::default() };
    let n = 12_500;
    let (small, big) = (doc(n), doc(8 * n));
    let (t1, t8, ratio) = cost_ratio(
        8,
        || {
            export(&small, &opts).unwrap();
        },
        || {
            export(&big, &opts).unwrap();
        },
    );
    eprintln!("t(n) {t1:.3} s, t(8n) {t8:.3} s, ratio {ratio:.1}");
    assert!(ratio < 24.0, "8x the text took {ratio:.1}x the time ({t1:.3} s → {t8:.3} s): not linear");
    let pdf = String::from_utf8_lossy(&export(&big, &opts).unwrap()).into_owned();
    // The all-hidden heading has no bookmark; the other is titled with its visible chars only.
    assert!(!pdf.contains("abcd"), "hidden heading text in a title");
    assert!(pdf.contains("bdf bdf"), "no title for the half-visible heading");
}

#[test]
fn table_style_hidden_heading_text_stays_out_of_bookmarks_and_tags() {
    // Hidden by the table style's header row, so only the layout's resolution (table conditional
    // formatting under the runs' own) says the text is hidden.
    let mut d = Document::new();
    d.styles.upsert(wordcraft_doc::Style {
        id: "SecretHeader".into(),
        name: "Secret Header".into(),
        kind: wordcraft_doc::StyleKind::Table,
        table: Some(wordcraft_doc::styles::TableStyleParts {
            header_chr: CharProps { hidden: Some(true), ..Default::default() },
            ..Default::default()
        }),
        ..Default::default()
    });
    let mut t = Table::new(2, 2, 400.0);
    t.props.style = Some("SecretHeader".into());
    // Header row: a heading hidden as a whole, and "Shown SECRET" where "Shown " is unhidden directly.
    let mut partly = Paragraph::with_text("Shown ", CharProps { hidden: Some(false), ..Default::default() }).styled("Heading1");
    let n = partly.len();
    partly.insert_text(n, "SECRET", &CharProps::default()).unwrap();
    t.rows[0].cells[0].blocks = vec![para_block(Paragraph::with_text("GONE", CharProps::default()).styled("Heading1"))];
    t.rows[0].cells[1].blocks = vec![para_block(partly)];
    t.rows[1].cells[0].blocks = vec![para_block(Paragraph::with_text("Body cell", CharProps::default()))];
    d.body = vec![para_block(Paragraph::with_text("Before", CharProps::default())), Arc::new(Block::Table(t))];
    let raw = |opts: PdfOptions| String::from_utf8_lossy(&export(&d, &PdfOptions { compress: false, ..opts }).unwrap()).into_owned();
    for (tagged, include_markup) in [(false, false), (true, false), (true, true)] {
        let pdf = raw(PdfOptions { tagged, include_markup, ..Default::default() });
        let what = format!("tagged {tagged} markup {include_markup}");
        assert!(!pdf.contains("SECRET") && !pdf.contains("GONE"), "{what}: hidden table heading text in the PDF");
        assert!(pdf.contains("/Outlines") && pdf.contains("Shown"), "{what}: no bookmark for the visible part");
    }
}

#[test]
fn text_mapping_handles_ligatures_and_extras() {
    let d = Document::from_text("office affine");
    let lay = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let bytes = export_layout(&d, &lay, &PdfOptions::default()).unwrap();
    assert!(squash(&extract_text(&bytes)[0]).contains("office affine"));
}

#[test]
fn shapes_have_paths() {
    let r = Rect::new(0.0, 0.0, 10.0, 10.0);
    for k in [
        ShapeKind::Rectangle,
        ShapeKind::RoundedRectangle,
        ShapeKind::Ellipse,
        ShapeKind::FlowchartOr,
        ShapeKind::Star,
        ShapeKind::Heart,
        ShapeKind::Arrow,
        ShapeKind::ArrowLeft,
        ShapeKind::ArrowUp,
        ShapeKind::ArrowDown,
        ShapeKind::Line,
        ShapeKind::Diamond,
        ShapeKind::Triangle,
    ] {
        assert!(shape_path(k, &r, None).is_some(), "{k:?}");
    }
    // A line or connector may be flat (horizontal or vertical); a closed shape may not.
    for k in [ShapeKind::Line, ShapeKind::StraightConnector] {
        assert!(shape_path(k, &Rect::new(0.0, 5.0, 10.0, 0.0), None).is_some(), "{k:?} horizontal");
        assert!(shape_path(k, &Rect::new(5.0, 0.0, 0.0, 10.0), None).is_some(), "{k:?} vertical");
        assert!(shape_path(k, &Rect::new(5.0, 5.0, 0.0, 0.0), None).is_none(), "{k:?} without length");
        // Hostile sizes reach `shape_path` unclamped through the public fields.
        for (w, h) in [(f32::NAN, 0.0), (f32::INFINITY, 0.0), (10.0, f32::NEG_INFINITY), (-10.0, 0.0), (0.0, -10.0), (10.0, -1.0)] {
            assert!(shape_path(k, &Rect { x: 0.0, y: 0.0, w, h }, None).is_none(), "{k:?} {w} x {h}");
        }
    }
    assert!(shape_path(ShapeKind::Rectangle, &Rect::new(0.0, 5.0, 10.0, 0.0), None).is_none());
    assert!(shape_path(ShapeKind::Ellipse, &Rect::new(0.0, 0.0, f32::NAN, 1.0), None).is_none());
    assert_eq!(parse_iso("2024-02-03T04:05:06Z"), Some((2024, 2, 3, 4, 5, 6)));
    assert_eq!(parse_iso("garbage"), None);
    assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
    // Past chrono's ±262143-year range `civil` keeps the proleptic Gregorian date the old
    // hand-rolled converter gave (only the caller's 0..=9999 PDF year clamp applies later), rather
    // than falling back to the epoch. Values pinned from the old formula.
    assert_eq!(civil(10_000_000_000_000), (318857, 5, 20, 17, 46, 40));
    assert_eq!(civil(-10_000_000_000_000), (-314918, 8, 13, 6, 13, 20));
    assert_eq!(civil(253_402_300_800), (10000, 1, 1, 0, 0, 0));
}

#[test]
fn fields_put_their_shown_text_in_the_text_layer() {
    // "Page 1 of 2" in the footer: the page fields' numbers, not the objects' placeholder
    // characters, are what copying, search and screen readers get.
    let mut d = Document::new();
    let mut f = Paragraph::with_text("Page  of ", CharProps::default());
    let field = |instr: &str| InlineObject::Field {
        instr: instr.into(),
        result: String::new(),
        locked: false,
        dirty: false,
        code: false,
        form: String::new(),
    };
    f.insert_object(5, field("PAGE"), &CharProps::default()).unwrap();
    let n = f.len();
    f.insert_object(n, field("NUMPAGES"), &CharProps::default()).unwrap();
    let id = d.add_part(wordcraft_doc::PartKind::Footer, vec![para_block(f)]);
    d.last_section.footers.default = Some(id);
    let mut body = Paragraph::with_text("Body", CharProps::default());
    body.props.page_break_before = None;
    let mut second = Paragraph::with_text("More", CharProps::default());
    second.props.page_break_before = Some(true);
    d.body = vec![para_block(body), para_block(second)];
    let text = extract_text(&export(&d, &PdfOptions::default()).unwrap());
    assert_eq!(text.len(), 2);
    assert!(text.iter().all(|t| !t.contains('\u{FFFC}')), "{text:?}");
    assert!(squash(&text[0]).contains("Page 1 of 2") && squash(&text[1]).contains("Page 2 of 2"), "{text:?}");
}

/// #775: a running header `STYLEREF "Heading 1"` shows each page's chapter in the PDF.
#[test]
fn styleref_headers_show_each_page_s_chapter() {
    let mut d = Document::new();
    let mut h = Paragraph::new();
    let field = InlineObject::Field {
        instr: r#"STYLEREF "Heading 1""#.into(),
        result: "Stale".into(),
        locked: false,
        dirty: false,
        code: false,
        form: String::new(),
    };
    h.insert_object(0, field, &CharProps::default()).unwrap();
    let id = d.add_part(wordcraft_doc::PartKind::Header, vec![para_block(h)]);
    d.last_section.headers.default = Some(id);
    d.body = ["Alpha", "Beta"]
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let mut p = Paragraph::with_text(&format!("Chapter{t}"), CharProps::default()).styled("Heading1");
            p.props.page_break_before = Some(i > 0);
            para_block(p)
        })
        .collect();
    let text = extract_text(&export(&d, &PdfOptions::default()).unwrap());
    assert_eq!(text.len(), 2);
    let pages: Vec<String> = text.iter().map(|t| squash(t)).collect();
    assert_eq!(pages[0].matches("ChapterAlpha").count(), 2, "{pages:?}");
    assert_eq!(pages[1].matches("ChapterBeta").count(), 2, "{pages:?}");
    assert!(!pages.iter().any(|p| p.contains("Stale")), "{pages:?}");
}

/// Bundled Source Sans 3 renamed to `family`, with glyph `gid` turned into a composite glyph whose
/// component list is cut off: fonts still parse and shape, but krilla's subsetter rejects it.
fn unsubsettable_font(family: &str, gid: u16) -> Vec<u8> {
    let mut f = wordcraft_fonts::bundled()[0].to_vec();
    let rename = |f: &mut Vec<u8>, from: &[u8], to: &[u8]| {
        let mut i = 0;
        while i + from.len() <= f.len() {
            if &f[i..i + from.len()] == from {
                f[i..i + from.len()].copy_from_slice(to);
            }
            i += 1;
        }
    };
    let wide = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    assert_eq!(family.len(), "Source Sans 3".len());
    rename(&mut f, &wide("Source Sans 3"), &wide(family));
    rename(&mut f, b"Source Sans 3", family.as_bytes());
    let u16_at = |f: &[u8], o: usize| u16::from_be_bytes([f[o], f[o + 1]]);
    let u32_at = |f: &[u8], o: usize| u32::from_be_bytes([f[o], f[o + 1], f[o + 2], f[o + 3]]);
    let table = |f: &[u8], tag: &[u8]| {
        (0..u16_at(f, 4) as usize).map(|t| 12 + t * 16).find(|r| &f[*r..*r + 4] == tag).map(|r| u32_at(f, r + 8) as usize).unwrap()
    };
    let (head, loca, glyf) = (table(&f, b"head"), table(&f, b"loca"), table(&f, b"glyf"));
    let g = gid as usize;
    let off = if u16_at(&f, head + 50) == 0 {
        // Short offsets (halved): the glyph keeps only its 10-byte header.
        let start = u16_at(&f, loca + g * 2);
        f[loca + g * 2 + 2..loca + g * 2 + 4].copy_from_slice(&(start + 5).to_be_bytes());
        start as usize * 2
    } else {
        let start = u32_at(&f, loca + g * 4);
        f[loca + g * 4 + 4..loca + g * 4 + 8].copy_from_slice(&(start + 10).to_be_bytes());
        start as usize
    };
    // numberOfContours = -1: a composite glyph, with no room left for its components.
    f[glyf + off..glyf + off + 2].copy_from_slice(&(-1i16).to_be_bytes());
    f
}

/// Glyphs drawn as text and paths drawn, over every page.
fn count_glyphs_and_paths(bytes: &[u8]) -> (usize, usize) {
    use hayro_interpret::font::Glyph;
    use hayro_interpret::{
        BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask,
        interpret_page,
    };
    use hayro_syntax::Pdf;
    struct Count(usize, usize);
    impl Device<'_> for Count {
        fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
        fn set_blend_mode(&mut self, _: BlendMode) {}
        fn draw_path(&mut self, _: &kurbo::BezPath, _: kurbo::Affine, _: &Paint<'_>, _: &PathDrawMode) {
            self.1 += 1;
        }
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
        fn draw_glyph(&mut self, _: &Glyph<'_>, _: kurbo::Affine, _: kurbo::Affine, _: &Paint<'_>, _: &GlyphDrawMode) {
            self.0 += 1;
        }
        fn draw_image(&mut self, _: Image<'_, '_>, _: kurbo::Affine) {}
        fn pop_clip_path(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }
    let pdf = Pdf::new(bytes.to_vec()).expect("parse");
    let cache = InterpreterCache::new();
    let mut n = Count(0, 0);
    for page in pdf.pages().iter() {
        let mut ctx = Context::new(kurbo::Affine::IDENTITY, kurbo::Rect::new(0.0, 0.0, 1.0, 1.0), &cache, pdf.xref(), InterpreterSettings::default());
        interpret_page(page, &mut ctx, &mut n);
    }
    (n.0, n.1)
}

/// The filled paths drawn, over every page, in page space.
fn filled_paths(bytes: &[u8]) -> Vec<kurbo::BezPath> {
    use hayro_interpret::font::Glyph;
    use hayro_interpret::{
        BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask,
        interpret_page,
    };
    use hayro_syntax::Pdf;
    struct Fills(Vec<kurbo::BezPath>);
    impl Device<'_> for Fills {
        fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
        fn set_blend_mode(&mut self, _: BlendMode) {}
        fn draw_path(&mut self, path: &kurbo::BezPath, transform: kurbo::Affine, _: &Paint<'_>, mode: &PathDrawMode) {
            if matches!(mode, PathDrawMode::Fill(_)) {
                self.0.push(transform * path.clone());
            }
        }
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
        fn draw_glyph(&mut self, _: &Glyph<'_>, _: kurbo::Affine, _: kurbo::Affine, _: &Paint<'_>, _: &GlyphDrawMode) {}
        fn draw_image(&mut self, _: Image<'_, '_>, _: kurbo::Affine) {}
        fn pop_clip_path(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }
    let pdf = Pdf::new(bytes.to_vec()).expect("parse");
    let cache = InterpreterCache::new();
    let mut fills = Fills(Vec::new());
    for page in pdf.pages().iter() {
        let mut ctx = Context::new(kurbo::Affine::IDENTITY, kurbo::Rect::new(0.0, 0.0, 1.0, 1.0), &cache, pdf.xref(), InterpreterSettings::default());
        interpret_page(page, &mut ctx, &mut fills);
    }
    fills.0
}

/// The straight stroked segments drawn, over every page, in page space: `((x0, y0), (x1, y1), width)`.
fn stroked_segments(bytes: &[u8]) -> Vec<(kurbo::Point, kurbo::Point, f64)> {
    use hayro_interpret::font::Glyph;
    use hayro_interpret::{
        BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask,
        interpret_page,
    };
    use hayro_syntax::Pdf;
    struct Strokes(Vec<(kurbo::Point, kurbo::Point, f64)>);
    impl Device<'_> for Strokes {
        fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
        fn set_blend_mode(&mut self, _: BlendMode) {}
        fn draw_path(&mut self, path: &kurbo::BezPath, transform: kurbo::Affine, _: &Paint<'_>, mode: &PathDrawMode) {
            use kurbo::PathEl;
            let PathDrawMode::Stroke(props) = mode else { return };
            if let [PathEl::MoveTo(a), PathEl::LineTo(b)] = path.elements() {
                let width = f64::from(props.line_width) * transform.determinant().abs().sqrt();
                self.0.push((transform * *a, transform * *b, width));
            }
        }
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
        fn draw_glyph(&mut self, _: &Glyph<'_>, _: kurbo::Affine, _: kurbo::Affine, _: &Paint<'_>, _: &GlyphDrawMode) {}
        fn draw_image(&mut self, _: Image<'_, '_>, _: kurbo::Affine) {}
        fn pop_clip_path(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }
    let pdf = Pdf::new(bytes.to_vec()).expect("parse");
    let cache = InterpreterCache::new();
    let mut strokes = Strokes(Vec::new());
    for page in pdf.pages().iter() {
        let mut ctx = Context::new(kurbo::Affine::IDENTITY, kurbo::Rect::new(0.0, 0.0, 1.0, 1.0), &cache, pdf.xref(), InterpreterSettings::default());
        interpret_page(page, &mut ctx, &mut strokes);
    }
    strokes.0
}

#[test]
fn rounded_rectangle_corners_follow_their_radius() {
    // `roundRect` with its own `adj`: Word's address boxes use small corners (adj 3565: 3.6% of
    // the shorter side), which were drawn at 16%.
    use kurbo::Shape;
    use wordcraft_doc::para::{Anchor, Float, ShapeKind, Wrap};
    let doc = |corner: Option<u32>| {
        let float = Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 72.0, y: 200.0, ..Default::default() };
        let shape = InlineObject::Shape {
            kind: ShapeKind::RoundedRectangle,
            w: 200.0,
            h: 100.0,
            fill: Some(wordcraft_doc::fill::Fill::solid(wordcraft_doc::props::Rgb(0xD9, 0xD9, 0xD9))),
            stroke: None,
            stroke_width: 0.0,
            float,
            story: None,
            freeform: None,
            effects: Default::default(),
            extra: wordcraft_doc::connector::ShapeExtra { corner, ..Default::default() },
        };
        let mut p = Paragraph::with_text("Anchor", CharProps::default());
        p.insert_object(0, shape, &CharProps::default()).unwrap();
        let mut d = Document::new();
        d.body = vec![para_block(p)];
        d
    };
    // Whether the shape covers the point `inset` points in from a corner of its frame.
    let covers = |corner: Option<u32>, inset: f64| {
        let fills = filled_paths(&export(&doc(corner), &PdfOptions { tagged: false, ..Default::default() }).unwrap());
        let shape = fills.iter().find(|p| (p.bounding_box().width() - 200.0).abs() < 0.5).expect("the shape is filled");
        let b = shape.bounding_box();
        shape.contains((b.x0 + inset, b.y0 + inset).into())
    };
    assert!(covers(Some(0), 1.0), "square corner");
    assert!(!covers(Some(3_565), 1.0) && covers(Some(3_565), 4.0), "3.565 pt corner");
    assert!(!covers(None, 4.0), "16 pt corner by default");
}

#[test]
fn unsubsettable_font_is_drawn_as_outlines() {
    let db = wordcraft_fonts::FontDb::global();
    let o = db.face("Source Sans 3", "Regular").glyph_for('o');
    let family = "Broken Sans 3";
    db.add_font(unsubsettable_font(family, u16::try_from(o).unwrap()));
    assert!(db.has_family(family));
    // Leave out the glyph after the broken one: its start moved into the broken glyph's data.
    let text: String = "Broken font, good export".chars().filter(|c| db.face(family, "Regular").glyph_for(*c) != o + 1).collect();
    let doc = |font: &str| {
        let mut d = Document::new();
        d.body = vec![para_block(Paragraph::with_text(&text, CharProps { font: Some(font.into()), ..Default::default() }))];
        d
    };
    let opts = PdfOptions { tagged: false, ..Default::default() };
    // The intact font is embedded as text.
    let (glyphs, _) = count_glyphs_and_paths(&export(&doc("Source Sans 3"), &opts).unwrap());
    assert!(glyphs > 0);
    // The broken one used to fail the whole export ("failed to subset font: malformed font");
    // now its text is drawn as outlines.
    let bytes = export(&doc(family), &opts).unwrap();
    let (glyphs, paths) = count_glyphs_and_paths(&bytes);
    assert_eq!(glyphs, 0, "no text in the font krilla can't subset");
    assert!(paths > 0, "its glyphs are drawn as outlines");
}

#[test]
fn flat_lines_in_a_group_are_drawn() {
    // A horizontal divider inside a group has no height (`a:ext cy="0"`): it used to be left out
    // of the PDF, though the screen drew it. A floating line on its own gets a 1 pt frame.
    use wordcraft_doc::para::{Anchor, Float, GroupChild, ShapeKind, Wrap};
    let line = |w: f32, h: f32| InlineObject::Shape {
        kind: ShapeKind::Line,
        w,
        h,
        fill: None,
        stroke: Some(wordcraft_doc::props::Rgb(192, 0, 0)),
        stroke_width: 1.5,
        float: Float::default(),
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    let doc = |children: Vec<GroupChild>| {
        let float = Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 72.0, y: 200.0, ..Default::default() };
        let group = InlineObject::Group { w: 300.0, h: 100.0, float, ch_w: 300.0, ch_h: 100.0, children, canvas: None };
        let mut p = Paragraph::with_text("Anchor", CharProps::default());
        p.insert_object(0, group, &CharProps::default()).unwrap();
        let mut d = Document::new();
        d.body = vec![para_block(p)];
        d
    };
    let paths = |d: &Document| count_glyphs_and_paths(&export(d, &PdfOptions { tagged: false, ..Default::default() }).unwrap()).1;
    let empty = paths(&doc(Vec::new()));
    let flat = paths(&doc(vec![
        GroupChild { x: 10.0, y: 50.0, obj: line(280.0, 0.0) },
        GroupChild { x: 150.0, y: 0.0, obj: line(0.0, 100.0) },
        GroupChild { x: 20.0, y: 20.0, obj: line(0.0, 0.0) },
    ]));
    assert_eq!(flat, empty + 2, "the horizontal and the vertical line, not the one without length");
}

/// #1740: a triple border is three lines in the PDF (it was drawn as a double one).
#[test]
fn triple_borders_export_three_lines() {
    use wordcraft_doc::props::{Border, BorderStyle, Borders};
    let pdf = |style: BorderStyle| {
        let mut p = Paragraph::with_text("Ruled", CharProps::default());
        p.props.borders = Some(Borders { bottom: Some(Border { style, width: 0.75, color: None, space: 1.0 }), ..Default::default() });
        let mut d = Document::new();
        d.body = vec![para_block(p)];
        export(&d, &PdfOptions { tagged: false, ..Default::default() }).unwrap()
    };
    let paths = |style: BorderStyle| count_glyphs_and_paths(&pdf(style)).1;
    let single = paths(BorderStyle::Single);
    assert_eq!(paths(BorderStyle::Double), single + 1);
    assert_eq!(paths(BorderStyle::Triple), single + 2);

    // The three lines run along the border, side by side: one on the single line's centre and one
    // either side of it, each 3 stroke widths away (1.8 x the line width apart, drawn 0.6 x wide).
    let flat = |style: BorderStyle| -> Vec<(kurbo::Point, kurbo::Point, f64)> {
        stroked_segments(&pdf(style)).into_iter().filter(|(a, b, _)| (a.y - b.y).abs() < 1e-3 && (a.x - b.x).abs() > 1.0).collect()
    };
    let centre = match flat(BorderStyle::Single).as_slice() {
        [(a, _, _)] => a.y,
        other => panic!("one single border line, got {other:?}"),
    };
    let mut triple = flat(BorderStyle::Triple);
    assert_eq!(triple.len(), 3, "three horizontal strokes: {triple:?}");
    triple.sort_by(|l, r| l.0.y.total_cmp(&r.0.y));
    let (x0, x1) = (triple[0].0.x, triple[0].1.x);
    for (a, b, _) in &triple {
        assert!((a.x - x0).abs() < 1e-3 && (b.x - x1).abs() < 1e-3, "the lines span the same run: {triple:?}");
    }
    let width = triple[0].2;
    assert!(width > 0.0 && triple.iter().all(|t| (t.2 - width).abs() < 1e-3), "equally thin lines: {triple:?}");
    let ys: Vec<f64> = triple.iter().map(|t| t.0.y).collect();
    assert!((ys[1] - centre).abs() < 1e-2, "the middle line sits on the border's centre: {ys:?} vs {centre}");
    for gap in [ys[1] - ys[0], ys[2] - ys[1]] {
        assert!((gap - 3.0 * width).abs() < 1e-2, "the lines are 3 stroke widths apart across the border: {ys:?}, width {width}");
    }
}

#[test]
fn field_results_are_extractable_text() {
    // Issue #97 (test from #112 by @LloydNicholson): a REF field's result, not the U+FFFC
    // placeholder, is what the text layer gets.
    let mut d = Document::new();
    let mut p = Paragraph::with_text("See  for details.", CharProps::default());
    p.insert_object(
        4,
        InlineObject::Field {
            instr: " REF _RefTarget \\h ".into(),
            result: "Section 3.01".into(),
            locked: false,
            dirty: false,
            code: false,
            form: String::new(),
        },
        &CharProps::default(),
    )
    .unwrap();
    d.body = vec![para_block(p)];
    let text = extract_text(&export(&d, &PdfOptions::default()).unwrap()).concat();
    assert!(!text.contains('\u{FFFC}'), "{text:?}");
    assert!(squash(&text).contains("See Section 3.01 for details."), "{text:?}");
}

#[test]
fn field_result_ligatures_extract_all_their_letters() {
    // A field result is shaped as one cluster; a ligature in it (Carlito, the Calibri substitute,
    // joins "ti") used to get only its first letter, shifting the rest: "Sectio  3.01". Bundled
    // Source Sans 3 joins "fi" and "ffi", so this doesn't depend on the installed fonts.
    let font = CharProps { font: Some("Source Sans 3".into()), ..Default::default() };
    let result = "the first office";
    let face = wordcraft_fonts::FontDb::global().face("Source Sans 3", "Regular");
    assert!(wordcraft_fonts::shape(&face, result, &[], |c| c).len() < result.chars().count(), "the result has ligatures");
    let mut d = Document::new();
    let mut p = Paragraph::with_text("See  here.", font.clone());
    p.insert_object(
        4,
        InlineObject::Field {
            instr: " REF _RefTarget \\h ".into(),
            result: result.into(),
            locked: false,
            dirty: false,
            code: false,
            form: String::new(),
        },
        &font,
    )
    .unwrap();
    d.body = vec![para_block(p)];
    let text = extract_text(&export(&d, &PdfOptions::default()).unwrap()).concat();
    assert!(squash(&text).contains("See the first office here."), "{text:?}");
}

#[test]
fn synthetic_bold_text_is_extracted_once() {
    // JetBrains Mono ships without a bold face, so its bold is filled and stroked: the text
    // layer still holds each character once.
    let mut d = Document::new();
    let bold = CharProps { font: Some("JetBrains Mono".into()), bold: Some(true), ..Default::default() };
    d.body = vec![para_block(Paragraph::with_text("Mono bold", bold))];
    let text = squash(&extract_text(&export(&d, &PdfOptions::default()).unwrap()).concat());
    assert!(text.contains("Mono bold"), "{text:?}");
}

#[test]
fn turned_cell_text_exports_as_text() {
    // Table Layout › Text Direction (#226): turned cell text is drawn in a turned frame and
    // stays real text in the PDF.
    for dir in [wordcraft_doc::props::TextDirection::Down, wordcraft_doc::props::TextDirection::Up] {
        let mut t = Table::new(1, 2, 300.0);
        t.rows[0].cells[0].blocks = vec![para_block(Paragraph::with_text("Turned", CharProps::default()))];
        t.rows[0].cells[0].props.text_direction = dir;
        let mut d = Document::new();
        d.body = vec![Arc::new(Block::Table(t)), para_block(Paragraph::with_text("Below", CharProps::default()))];
        let text = squash(&extract_text(&export(&d, &PdfOptions::default()).unwrap()).concat());
        assert!(text.contains("Turned") && text.contains("Below"), "{dir:?}: {text:?}");
    }
}

#[test]
fn inline_chart_is_a_figure_with_its_text_inside_it() {
    use wordcraft_doc::graphic::{Graphic, GraphicItem, GraphicKind, TextAlign};
    use wordcraft_doc::props::Rgb;
    let items = vec![GraphicItem::Text {
        rect: [0.0, 0.0, 216.0, 20.0],
        text: "Q1".into(),
        size: 10.0,
        color: Rgb::BLACK,
        bold: false,
        align: TextAlign::Center,
        font: None,
    }];
    let graphic = Graphic { kind: GraphicKind::Chart, items, ..Default::default() };
    let obj = InlineObject::Graphic { w: 216.0, h: 144.0, alt: "Sales by quarter".into(), float: Default::default(), graphic: Arc::new(graphic) };
    let mut d = Document::from_text("Chart");
    d.insert_object(&wordcraft_doc::Pos::body(0, 0), obj, &Default::default()).unwrap();
    let raw = |tagged| String::from_utf8_lossy(&export(&d, &PdfOptions { tagged, compress: false, ..Default::default() }).unwrap()).into_owned();
    assert!(raw(true).contains("/Figure"), "tagged chart is a figure");
    assert!(!raw(false).contains("/Figure"));
}

#[test]
fn chart_without_alt_text_is_a_figure_called_chart() {
    use wordcraft_doc::graphic::{Graphic, GraphicItem, GraphicKind, PathSeg};
    let segs = vec![PathSeg::Move(1.0, 1.0), PathSeg::Line(50.0, 50.0)];
    let items = vec![GraphicItem::Path { segs, fill: None, stroke: Some(wordcraft_doc::props::Rgb::BLACK), stroke_width: 1.0 }];
    let graphic = Graphic { kind: GraphicKind::Chart, items, ..Default::default() };
    let obj = InlineObject::Graphic { w: 216.0, h: 144.0, alt: String::new(), float: Default::default(), graphic: Arc::new(graphic) };
    let mut d = Document::from_text("Chart");
    d.insert_object(&wordcraft_doc::Pos::body(0, 0), obj, &Default::default()).unwrap();
    let raw = String::from_utf8_lossy(&export(&d, &PdfOptions { tagged: true, compress: false, ..Default::default() }).unwrap()).into_owned();
    assert!(raw.contains("/Alt(chart)"), "no chart alt text");
    assert!(!raw.contains("/Alt(picture)"));
}

/// A 25 mm square EMF with a red rectangle over its right half (device units 0.04 per 0.01 mm).
fn red_right_half_emf() -> Vec<u8> {
    fn rec(typ: u32, payload: &[u8]) -> Vec<u8> {
        let mut v = typ.to_le_bytes().to_vec();
        v.extend(((8 + payload.len()) as u32).to_le_bytes());
        v.extend(payload);
        v
    }
    let le32 = |vals: &[i32]| -> Vec<u8> { vals.iter().flat_map(|v| v.to_le_bytes()).collect() };
    let mut h = vec![0u8; 88];
    h[0..4].copy_from_slice(&1u32.to_le_bytes());
    h[4..8].copy_from_slice(&88u32.to_le_bytes());
    h[8..24].copy_from_slice(&le32(&[0, 0, 100, 100]));
    h[24..40].copy_from_slice(&le32(&[0, 0, 2500, 2500]));
    h[40..44].copy_from_slice(&0x464D_4520u32.to_le_bytes());
    h[72..88].copy_from_slice(&le32(&[1000, 1000, 250, 250]));
    [h, rec(39, &le32(&[1, 0, 0x0000_00FF, 0])), rec(37, &le32(&[1])), rec(43, &le32(&[50, 0, 100, 100])), rec(14, &[0; 12])].concat()
}

#[test]
fn metafile_pictures_are_drawn_as_vector_paths() {
    let mut d = Document::new();
    let media = d.add_media(red_right_half_emf(), "emf");
    let mut p = Paragraph::with_text("Figure: ", CharProps::default());
    let n = p.len();
    p.insert_object(
        n,
        InlineObject::Image { media, w: 70.0, h: 70.0, alt: "red".into(), float: Default::default(), crop: [0.0; 4], ole: None },
        &CharProps::default(),
    )
    .unwrap();
    d.body = vec![para_block(p)];
    let bytes = export(&d, &PdfOptions { compress: false, tagged: false, ..Default::default() }).unwrap();
    let raw = String::from_utf8_lossy(&bytes);
    assert!(raw.contains("1 0 0 rg"), "the red rectangle is filled as a path");
    assert!(!raw.contains("0.8156"), "the grey placeholder box is not drawn");
}

/// #1899: a picture cropped to a sliver of its source (3% across here) is drawn at that sliver's
/// size, bitmap or metafile: the source spans 1/0.03 frames, not at most 20 (the old 5% floor).
#[test]
fn a_crop_keeping_a_sliver_draws_the_source_that_much_wider() {
    use hayro_interpret::font::Glyph;
    use hayro_interpret::{
        BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask,
        interpret_page,
    };
    use hayro_syntax::Pdf;
    /// The widest image and the widest fill drawn, in page units.
    #[derive(Default)]
    struct Widest(f64, f64);
    impl Device<'_> for Widest {
        fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
        fn set_blend_mode(&mut self, _: BlendMode) {}
        fn draw_path(&mut self, path: &kurbo::BezPath, transform: kurbo::Affine, _: &Paint<'_>, mode: &PathDrawMode) {
            if matches!(mode, PathDrawMode::Fill(_)) {
                self.1 = self.1.max(kurbo::Shape::bounding_box(&(transform * path.clone())).width());
            }
        }
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
        fn draw_glyph(&mut self, _: &Glyph<'_>, _: kurbo::Affine, _: kurbo::Affine, _: &Paint<'_>, _: &GlyphDrawMode) {}
        fn draw_image(&mut self, _: Image<'_, '_>, transform: kurbo::Affine) {
            self.0 = self.0.max(transform.as_coeffs()[0].abs());
        }
        fn pop_clip_path(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }
    let widest = |bytes: Vec<u8>, ext: &str, crop: [f32; 4]| {
        let mut d = Document::new();
        let media = d.add_media(bytes, ext);
        let mut p = Paragraph::new();
        p.insert_object(
            0,
            InlineObject::Image { media, w: 70.0, h: 70.0, alt: String::new(), float: Default::default(), crop, ole: None },
            &CharProps::default(),
        )
        .unwrap();
        d.body = vec![para_block(p)];
        let pdf = Pdf::new(export(&d, &PdfOptions { compress: false, tagged: false, ..Default::default() }).unwrap()).expect("parse");
        let cache = InterpreterCache::new();
        let mut w = Widest::default();
        for page in pdf.pages().iter() {
            let mut ctx =
                Context::new(kurbo::Affine::IDENTITY, kurbo::Rect::new(0.0, 0.0, 1.0, 1.0), &cache, pdf.xref(), InterpreterSettings::default());
            interpret_page(page, &mut ctx, &mut w);
        }
        w
    };
    let sliver = [0.47, 0.0, 0.5, 0.0];
    // The bitmap: the whole image drawn behind the frame's clip.
    let (whole, cropped) = (widest(png(), "png", [0.0; 4]).0, widest(png(), "png", sliver).0);
    assert!(whole > 0.0 && (cropped / whole - 1.0 / 0.03).abs() < 0.5, "png: {whole} → {cropped}");
    // The metafile: its red right half, a filled path.
    let (whole, cropped) = (widest(red_right_half_emf(), "emf", [0.0; 4]).1, widest(red_right_half_emf(), "emf", sliver).1);
    assert!(whole > 0.0 && (cropped / whole - 1.0 / 0.03).abs() < 0.5, "emf: {whole} → {cropped}");
}

/// A JPEG whose headers say progressive (SOF2, a DC-only first scan) and `w` × `h`, its three
/// components unsubsampled, with no pixel data that size.
fn progressive_jpeg_header(w: u16, h: u16) -> Vec<u8> {
    let mut jpeg = Vec::new();
    image::RgbImage::new(16, 16).write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg).unwrap();
    let sof = jpeg.windows(2).position(|m| m == [0xFF, 0xC0]).unwrap();
    jpeg[sof + 1] = 0xC2;
    jpeg[sof + 5..sof + 7].copy_from_slice(&h.to_be_bytes());
    jpeg[sof + 7..sof + 9].copy_from_slice(&w.to_be_bytes());
    for c in 0..3 {
        jpeg[sof + 11 + 3 * c] = 0x11;
    }
    let sos = jpeg.windows(2).position(|m| m == [0xFF, 0xDA]).unwrap();
    let n = usize::from(jpeg[sos + 4]);
    jpeg[sos + 6 + 2 * n] = 0;
    jpeg
}

/// #3644: a progressive JPEG's coefficients count toward the budget, beside its output.
#[test]
fn progressive_jpeg_coefficients_count_toward_the_image_budget() {
    assert!(wordcraft_geom::decode_fits(7900, 7900, wordcraft_geom::MAX_DECODED_BYTES));
    assert!(load_image(&Arc::new(progressive_jpeg_header(7900, 7900))).is_none());
    let mut baseline = Vec::new();
    image::RgbImage::new(64, 64).write_to(&mut std::io::Cursor::new(&mut baseline), image::ImageFormat::Jpeg).unwrap();
    assert!(load_image(&Arc::new(baseline)).is_some(), "other JPEGs still embed");
}

/// #1502: an inline picture's Figure carries its own alt text, not the "picture" fallback.
#[test]
fn inline_picture_figure_keeps_its_alt_text() {
    let raw = |alt: &str| {
        let mut d = Document::new();
        let media = d.add_media(red_right_half_emf(), "emf");
        let mut p = Paragraph::with_text("Figure: ", CharProps::default());
        let n = p.len();
        let obj = InlineObject::Image { media, w: 70.0, h: 70.0, alt: alt.into(), float: Default::default(), crop: [0.0; 4], ole: None };
        p.insert_object(n, obj, &CharProps::default()).unwrap();
        d.body = vec![para_block(p)];
        String::from_utf8_lossy(&export(&d, &PdfOptions { compress: false, tagged: true, ..Default::default() }).unwrap()).into_owned()
    };
    let with = raw("Blue rectangle used as test image");
    assert!(with.contains("/Alt(Blue rectangle used as test image)"), "inline alt text lost");
    assert!(!with.contains("/Alt(picture)"));
    assert!(raw("").contains("/Alt(picture)"), "no alt text keeps the fallback");
}

/// A tagged PDF of one picture in a paragraph, placed by `float`, with alt text "A red square".
fn tagged_picture_pdf(float: wordcraft_doc::para::Float) -> String {
    let mut d = Document::new();
    let media = d.add_media(red_right_half_emf(), "emf");
    let mut p = Paragraph::with_text("Figure: ", CharProps::default());
    let n = p.len();
    let obj = InlineObject::Image { media, w: 70.0, h: 70.0, alt: "A red square".into(), float, crop: [0.0; 4], ole: None };
    p.insert_object(n, obj, &CharProps::default()).unwrap();
    d.body = vec![para_block(p)];
    String::from_utf8_lossy(&export(&d, &PdfOptions { compress: false, tagged: true, ..Default::default() }).unwrap()).into_owned()
}

/// A picture marked decorative is an artifact in tagged PDF, inline, rotated or floating, so
/// screen readers skip it; the same picture not marked decorative stays a Figure with its alt text.
#[test]
fn decorative_pictures_are_artifacts_not_figures() {
    use wordcraft_doc::para::{AltText, Anchor, Float, Wrap};
    let decorative = AltText { decorative: true, ..Default::default() };
    let floating = Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 72.0, y: 200.0, ..Default::default() };
    let cases = [
        ("inline", Float::default()),
        ("rotated inline", Float { rot: 30.0, ..Default::default() }),
        ("floating", floating.clone()),
        ("rotated floating", Float { rot: 30.0, ..floating }),
    ];
    for (name, float) in cases {
        let plain = tagged_picture_pdf(float.clone());
        assert!(plain.contains("/Alt(A red square)"), "{name}: a described picture is a Figure with its alt text");
        let raw = tagged_picture_pdf(Float { alt: decorative.clone(), ..float });
        assert!(!raw.contains("/Alt("), "{name}: a decorative picture has no Figure and no alt text");
        assert!(raw.contains("1 0 0 rg"), "{name}: the decorative picture is still drawn");
    }
}

/// In a rotated group, each picture is tagged on its own: the decorative one is an artifact, the
/// other stays a Figure with its alt text.
#[test]
fn rotated_group_tags_each_picture_on_its_own() {
    use wordcraft_doc::para::{AltText, Float, GroupChild};
    let mut d = Document::new();
    let media = d.add_media(red_right_half_emf(), "emf");
    let picture =
        |alt: &str, float: Float| InlineObject::Image { media: media.clone(), w: 70.0, h: 70.0, alt: alt.into(), float, crop: [0.0; 4], ole: None };
    let decorative = Float { alt: AltText { decorative: true, ..Default::default() }, ..Default::default() };
    let group = InlineObject::Group {
        w: 160.0,
        h: 70.0,
        float: Float { rot: 30.0, ..Default::default() },
        ch_w: 160.0,
        ch_h: 70.0,
        children: vec![
            GroupChild { x: 0.0, y: 0.0, obj: picture("A decorative border", decorative) },
            GroupChild { x: 90.0, y: 0.0, obj: picture("A red square", Float::default()) },
        ],
        canvas: None,
    };
    let mut p = Paragraph::with_text("Group: ", CharProps::default());
    let n = p.len();
    p.insert_object(n, group, &CharProps::default()).unwrap();
    d.body = vec![para_block(p)];
    let raw = String::from_utf8_lossy(&export(&d, &PdfOptions { compress: false, tagged: true, ..Default::default() }).unwrap()).into_owned();
    assert!(raw.contains("/Alt(A red square)"), "the picture not marked decorative is a Figure with its alt text");
    assert!(!raw.contains("A decorative border"), "the decorative picture has no Figure and no alt text");
    assert_eq!(raw.matches("/Alt(").count(), 1, "one Figure, for the picture not marked decorative");
}

/// An EMF file of the given records, with a 25 mm frame and 1000 device pixels across (0.04 per 0.01 mm).
fn emf_file(records: &[Vec<u8>]) -> Vec<u8> {
    let le32 = |vals: &[i32]| -> Vec<u8> { vals.iter().flat_map(|v| v.to_le_bytes()).collect() };
    let body = records.concat();
    let mut h = vec![0u8; 88];
    h[0..4].copy_from_slice(&1u32.to_le_bytes());
    h[4..8].copy_from_slice(&88u32.to_le_bytes());
    h[8..24].copy_from_slice(&le32(&[0, 0, 100, 100]));
    h[24..40].copy_from_slice(&le32(&[0, 0, 2500, 2500]));
    h[40..44].copy_from_slice(&0x464D_4520u32.to_le_bytes());
    h[72..88].copy_from_slice(&le32(&[1000, 1000, 250, 250]));
    [h, body].concat()
}

/// One EMF record of the given type with little-endian i32 fields.
fn emf_rec(typ: u32, fields: &[i32]) -> Vec<u8> {
    let mut v = typ.to_le_bytes().to_vec();
    let payload: Vec<u8> = fields.iter().flat_map(|f| f.to_le_bytes()).collect();
    v.extend(((8 + payload.len()) as u32).to_le_bytes());
    v.extend(payload);
    v
}

#[test]
fn metafile_strokes_use_round_caps_and_joins() {
    // A 4-unit black pen (handle 1) outlines a rectangle, so the stroke is drawn with round caps and joins.
    let recs =
        [emf_rec(38, &[1, 0, 4, 0, 0]), emf_rec(37, &[1]), emf_rec(37, &[0x8000_0005u32 as i32]), emf_rec(43, &[0, 0, 50, 50]), emf_rec(14, &[0; 3])];
    let mut d = Document::new();
    let media = d.add_media(emf_file(&recs), "emf");
    let mut p = Paragraph::with_text("Figure: ", CharProps::default());
    let n = p.len();
    p.insert_object(
        n,
        InlineObject::Image { media, w: 70.0, h: 70.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None },
        &CharProps::default(),
    )
    .unwrap();
    d.body = vec![para_block(p)];
    let bytes = export(&d, &PdfOptions { compress: false, tagged: false, ..Default::default() }).unwrap();
    let raw = String::from_utf8_lossy(&bytes);
    assert!(raw.contains("1 J"), "round line caps");
    assert!(raw.contains("1 j"), "round line joins");
}

#[test]
fn metafile_fill_items_carry_no_stroke_state() {
    // A stroked outline (NULL brush) followed by a filled rectangle (NULL pen): the fill must not inherit the stroke.
    let recs = [
        emf_rec(38, &[1, 0, 4, 0, 0]),
        emf_rec(37, &[1]),
        emf_rec(37, &[0x8000_0005u32 as i32]),
        emf_rec(43, &[0, 0, 50, 50]),
        emf_rec(39, &[2, 0, 0x0000_00FF, 0]),
        emf_rec(37, &[2]),
        emf_rec(37, &[0x8000_0008u32 as i32]),
        emf_rec(43, &[60, 0, 100, 50]),
        emf_rec(14, &[0; 3]),
    ];
    let mut d = Document::new();
    let media = d.add_media(emf_file(&recs), "emf");
    let mut p = Paragraph::with_text("Figure: ", CharProps::default());
    let n = p.len();
    p.insert_object(
        n,
        InlineObject::Image { media, w: 70.0, h: 70.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None },
        &CharProps::default(),
    )
    .unwrap();
    d.body = vec![para_block(p)];
    let bytes = export(&d, &PdfOptions { compress: false, tagged: false, ..Default::default() }).unwrap();
    let raw = String::from_utf8_lossy(&bytes);
    let fill = raw.find("1 0 0 rg").expect("the red fill is drawn");
    let rest = &raw[fill..];
    let rest = &rest[..rest.find("endstream").unwrap_or(rest.len())];
    assert!(raw[..fill].contains("0 0 0 RG"), "the outline is stroked before the fill");
    assert!(!rest.contains(" RG") && !rest.contains("\nS\n"), "the fill item carries no stroke: {rest:?}");
}

#[test]
fn cosmetic_metafile_pens_are_thin_but_visible() {
    // MOVETO, LINETO with the default cosmetic (width 0) pen: drawn 0.25 pt wide, not 0.1 pt.
    let recs = [emf_rec(27, &[0, 50]), emf_rec(54, &[100, 50]), emf_rec(14, &[0; 3])];
    let mut d = Document::new();
    let media = d.add_media(emf_file(&recs), "emf");
    let mut p = Paragraph::with_text("Figure: ", CharProps::default());
    let n = p.len();
    p.insert_object(
        n,
        InlineObject::Image { media, w: 70.0, h: 70.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None },
        &CharProps::default(),
    )
    .unwrap();
    d.body = vec![para_block(p)];
    let bytes = export(&d, &PdfOptions { compress: false, tagged: false, ..Default::default() }).unwrap();
    let raw = String::from_utf8_lossy(&bytes);
    assert!(raw.contains("0.25 w"), "the hairline is a quarter point wide");
}

/// EMF text records: EXTCREATEFONTINDIRECTW (Arial, 20 units high) in slot 1, SELECTOBJECT,
/// SETTEXTCOLOR (black), then EXTTEXTOUTW `text` at (5, 40) with no dx array.
fn emf_text_records(text: &str) -> Vec<Vec<u8>> {
    let mut font = [82i32.to_le_bytes(), 104u32.to_le_bytes(), 1i32.to_le_bytes(), (-20i32).to_le_bytes()].concat();
    font.extend([0i32, 0, 0, 400, 0, 0].iter().flat_map(|v| v.to_le_bytes()));
    let mut name: Vec<u8> = "Arial".encode_utf16().flat_map(u16::to_le_bytes).collect();
    name.resize(64, 0);
    font.extend(name);
    let units: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let padded = units.len().div_ceil(4) * 4;
    let mut out = [84u32.to_le_bytes(), ((76 + padded) as u32).to_le_bytes()].concat();
    out.extend([0i32, 0, 100, 100, 1, 0, 0, 5, 40, text.encode_utf16().count() as i32, 76, 0, 0, 0, 0, 0, 0].iter().flat_map(|v| v.to_le_bytes()));
    out.extend(units);
    out.resize(76 + padded, 0);
    vec![font, emf_rec(37, &[1]), emf_rec(24, &[0]), out]
}

/// #1764: the text of an EMF picture (EXTTEXTOUTW) is drawn, so it is in the PDF's text layer.
#[test]
fn metafile_text_records_are_drawn_as_text() {
    let mut recs = emf_text_records("Cell42");
    recs.push(emf_rec(14, &[0; 3]));
    let mut d = Document::new();
    let media = d.add_media(emf_file(&recs), "emf");
    let mut p = Paragraph::with_text("Figure: ", CharProps::default());
    let n = p.len();
    p.insert_object(
        n,
        InlineObject::Image { media, w: 70.0, h: 70.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None },
        &CharProps::default(),
    )
    .unwrap();
    d.body = vec![para_block(p)];
    let bytes = export(&d, &PdfOptions { compress: false, tagged: false, ..Default::default() }).unwrap();
    let text = extract_text(&bytes).concat();
    assert!(text.contains("Cell42"), "the metafile's text is drawn: {text:?}");
}

/// #455: a link to a bookmark after a page break inside its paragraph goes to the page the
/// bookmark is on, not the paragraph's first page.
#[test]
fn bookmark_destination_is_the_page_of_its_start() {
    let mut d = Document::new();
    let mut p = Paragraph::with_text("Paragraph start\u{c}Bookmark target", CharProps::default());
    let at = "Paragraph start\u{c}".len();
    p.insert_object(at, InlineObject::BookmarkStart { name: "target".into() }, &CharProps::default()).unwrap();
    p.insert_object(at, InlineObject::BookmarkStart { name: "First".into() }, &CharProps::default()).unwrap();
    p.insert_object(0, InlineObject::BookmarkStart { name: "first".into() }, &CharProps::default()).unwrap();
    d.body = vec![para_block(p)];
    let lay = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    assert_eq!(lay.pages.len(), 2);
    let pages: Vec<usize> = (0..lay.pages.len()).collect();
    let outlined = HashSet::new();
    let ex = Exporter {
        doc: &d,
        lay: &lay,
        opts: &PdfOptions::default(),
        out_index: pages.iter().enumerate().map(|(o, p)| (*p, o)).collect(),
        fonts: HashMap::new(),
        font_error: None,
        outlined: &outlined,
        cmaps: HashMap::new(),
        images: HashMap::new(),
        vectors: HashMap::new(),
        tags: Vec::new(),
        tag_index: HashMap::new(),
        links: Vec::new(),
        bookmarks: std::cell::OnceCell::new(),
    };
    let (page, x, y) = ex.bookmark("target").unwrap();
    assert_eq!(page, 1, "the bookmark is on the second page");
    assert!((x - 72.0).abs() < 1.0 && y < 150.0, "{x} {y}");
    assert_eq!(ex.bookmark("first").map(|b| b.0), Some(0));

    // Case-insensitive lookup matches
    let (page_ci, x_ci, y_ci) = ex.bookmark("TARGET").unwrap();
    assert_eq!(page_ci, 1);
    assert_eq!((x_ci, y_ci), (x, y));
    // An exact name wins; names that differ only in case fall back to the first in the document
    // (a folded index, not a scan of a hash map in arbitrary order for every link).
    assert_eq!(ex.bookmark("First").map(|b| b.0), Some(1));
    assert_eq!(ex.bookmark("FIRST").map(|b| b.0), Some(0));
    assert!(ex.bookmark("absent").is_none());
    let (exact, folded) = ex.bookmarks.get().unwrap();
    assert_eq!((exact.len(), folded.len()), (3, 2));

    // Whitespace in ISO date string is parsed successfully
    assert!(parse_iso("  2024-05-06T07:08:09Z \n").is_some());
}

#[test]
fn ref_fields_export_cached_links() {
    use hayro_syntax::object::{Array, Dict};

    for (instr, outer, code, expected) in [
        (r"REF target \w \h \* MERGEFORMAT", None, false, Some(1)),
        (r#"ref "target"\H"#, None, false, Some(1)),
        (r"REF target", None, false, None),
        (r"REF target \hello", None, false, None),
        (r#"REF target \d "\h""#, None, false, None),
        (r"REF absent \h", None, false, None),
        (r"REF target \h", None, true, None),
        (r"REF target \h", Some("#outer"), false, Some(2)),
    ] {
        let mut d = Document::new();
        let mut p = Paragraph::new();
        p.insert_object(
            0,
            InlineObject::Field { instr: instr.into(), result: "Cached label".into(), locked: true, dirty: false, code, form: String::new() },
            &CharProps { link: outer.map(str::to_owned), ..Default::default() },
        )
        .unwrap();
        d.body = vec![para_block(p)];
        for name in ["target", "outer"] {
            let mut p = Paragraph::with_text("Current target", CharProps::default());
            p.props.page_break_before = Some(true);
            p.insert_object(0, InlineObject::BookmarkStart { name: name.into() }, &CharProps::default()).unwrap();
            d.body.push(para_block(p));
        }
        let before = d.body.clone();
        let bytes = export(&d, &PdfOptions::default()).unwrap();
        assert_eq!(d.body, before, "export must not refresh cached fields");
        if !code {
            assert_eq!(squash(&extract_text(&bytes)[0]), "Cached label");
        }
        let pdf = hayro_syntax::Pdf::new(bytes).unwrap();
        let targets: Vec<_> = pdf.pages()[0]
            .raw()
            .get::<Array>(b"Annots")
            .into_iter()
            .flat_map(|a| a.iter::<Dict>().collect::<Vec<_>>())
            .map(|a| a.get::<Array>(b"Dest").unwrap().iter::<Dict>().next().unwrap().obj_id())
            .collect();
        let expected: Vec<_> = expected.map(|i| pdf.pages()[i].raw().obj_id()).into_iter().collect();
        assert_eq!(targets, expected, "{instr}, enclosing={outer:?}, code={code}");
    }
}

#[test]
fn outline_after_leading_breaks() {
    use hayro_syntax::object::{Array, Dict};

    for (lead, marker) in [
        ("\u{c}", None),
        ("\u{c}\u{c}", None),
        ("\u{c}", Some(InlineObject::BookmarkStart { name: "leading".into() })),
        ("\u{c}\u{c}", Some(InlineObject::CommentStart { id: 0 })),
    ] {
        let mut d = Document::from_text("Earlier page");
        let mut heading = Paragraph::with_text(&format!("{lead}Navigation target"), CharProps::default()).styled("Heading1");
        heading.insert_object(lead.len(), InlineObject::BookmarkStart { name: "target".into() }, &CharProps::default()).unwrap();
        if let Some(marker) = marker {
            heading.insert_object(0, marker, &CharProps::default()).unwrap();
        }
        d.body.push(para_block(heading));
        let bytes = export(&d, &PdfOptions::default()).unwrap();
        let text = extract_text(&bytes);
        let target = text.iter().position(|t| squash(t).contains("Navigation target")).unwrap();
        assert!(target > 0, "the heading follows a page break");
        for pages in [None, Some(vec![target]), Some(vec![target, 0]), Some(vec![0])] {
            let expected = pages.as_ref().map_or(Some(target), |p| p.iter().position(|i| *i == target));
            let bytes = export(&d, &PdfOptions { pages, ..Default::default() }).unwrap();
            let pdf = hayro_syntax::Pdf::new(bytes).unwrap();
            let catalog = pdf.xref().get::<Dict<'_>>(pdf.xref().root_id()).unwrap();
            let outlines = catalog.get::<Dict<'_>>(b"Outlines");
            let Some(expected) = expected else {
                assert!(outlines.is_none(), "a page containing only the break has no heading outline");
                continue;
            };
            let first = outlines.unwrap().get::<Dict<'_>>(b"First").unwrap();
            let dest = first.get::<Array<'_>>(b"Dest").unwrap();
            assert_eq!(dest.iter::<Dict<'_>>().next().unwrap(), *pdf.pages()[expected].raw(), "outline must reach the visible heading");
            assert!(first.get::<Dict<'_>>(b"Next").is_none(), "one outline entry per heading");
        }
    }
}

#[test]
fn outline_keeps_visible_objects() {
    use hayro_syntax::object::{Array, Dict};

    let mut base = Document::from_text("Earlier text");
    let media = base.add_media(png(), "png");
    for object in [
        InlineObject::Field { instr: "QUOTE Visible".into(), result: "Visible".into(), locked: true, dirty: false, code: false, form: String::new() },
        InlineObject::Image { media, w: 16.0, h: 8.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None },
    ] {
        let mut d = base.clone();
        let mut heading = Paragraph::with_text("\u{c}After break", CharProps::default()).styled("Heading1");
        heading.insert_object(0, object, &CharProps::default()).unwrap();
        heading.insert_object(0, InlineObject::BookmarkStart { name: "leading".into() }, &CharProps::default()).unwrap();
        d.body.push(para_block(heading));
        let bytes = export(&d, &PdfOptions::default()).unwrap();
        assert!(squash(&extract_text(&bytes)[1]).contains("After break"));
        let pdf = hayro_syntax::Pdf::new(bytes).unwrap();
        let catalog = pdf.xref().get::<Dict<'_>>(pdf.xref().root_id()).unwrap();
        let outlines = catalog.get::<Dict<'_>>(b"Outlines").unwrap();
        let first = outlines.get::<Dict<'_>>(b"First").unwrap();
        let dest = first.get::<Array<'_>>(b"Dest").unwrap();
        assert_eq!(dest.iter::<Dict<'_>>().next().unwrap(), *pdf.pages()[0].raw(), "visible objects before the break anchor the heading");
    }
}

/// Print Markup (#589): the comments and their replies print beside each page, which is scaled
/// down to keep its paper size; without it they stay out of the PDF.
#[test]
fn comments_print_in_balloons_beside_scaled_pages() {
    let mut d = Document::from_text("Intro line.\nThe quarterly figures look strong.\nClosing line.");
    let part = d.add_part(wordcraft_doc::PartKind::Comment, vec![para_block(Paragraph::with_text("Check the totals", CharProps::default()))]);
    d.comments.insert(0, wordcraft_doc::Comment { author: "Ada Reviewer".into(), part, ..Default::default() });
    let part = d.add_part(wordcraft_doc::PartKind::Comment, vec![para_block(Paragraph::with_text("Totals checked", CharProps::default()))]);
    d.comments.insert(1, wordcraft_doc::Comment { author: "Ben Editor".into(), part, parent: Some(0), ..Default::default() });
    d.insert_object(&wordcraft_doc::Pos::body(1, 13), InlineObject::CommentEnd { id: 0 }, &CharProps::default()).unwrap();
    d.insert_object(&wordcraft_doc::Pos::body(1, 4), InlineObject::CommentStart { id: 0 }, &CharProps::default()).unwrap();
    let size = |bytes: &[u8]| {
        let pdf = hayro_syntax::Pdf::new(bytes.to_vec()).expect("parse");
        pdf.pages().iter().map(|p| p.media_box()).map(|r| (r.width().round(), r.height().round())).collect::<Vec<_>>()
    };
    let plain = export(&d, &PdfOptions::default()).unwrap();
    let text = squash(&extract_text(&plain).concat());
    assert!(text.contains("quarterly figures") && !text.contains("Check the totals") && !text.contains("Ada Reviewer"), "{text}");
    let marked = export(&d, &PdfOptions { include_markup: true, comments: true, ..Default::default() }).unwrap();
    let text = squash(&extract_text(&marked).concat());
    for want in ["quarterly figures", "Ada Reviewer", "Check the totals", "Ben Editor", "Totals checked"] {
        assert!(text.contains(want), "{want} missing: {text}");
    }
    // Same paper: the page and its markup area are scaled to fit it.
    assert_eq!(size(&plain), size(&marked));
    assert_eq!(size(&marked), vec![(612.0, 792.0)]);
    // Tagged or not, with links and an outline, it still writes.
    let mut d2 = sample();
    let part = d2.add_part(wordcraft_doc::PartKind::Comment, vec![para_block(Paragraph::with_text("On the heading", CharProps::default()))]);
    d2.comments.insert(0, wordcraft_doc::Comment { author: "Ada".into(), part, ..Default::default() });
    d2.insert_object(&wordcraft_doc::Pos::body(1, 3), InlineObject::CommentStart { id: 0 }, &CharProps::default()).unwrap();
    for tagged in [false, true] {
        let bytes = export(&d2, &PdfOptions { tagged, comments: true, ..Default::default() }).unwrap();
        assert!(squash(&extract_text(&bytes).concat()).contains("On the heading"));
    }
}

/// A long document whose every paragraph links to a bookmark (a linked table of contents, cross
/// references) exports in time proportional to its length: each link looked its bookmark up by
/// walking the whole document, so NIH's 67-page protocol template took 10.6 s (0.9 s now).
#[test]
fn internal_links_export_in_linear_time() {
    let doc = |n: usize| {
        let mut d = Document::new();
        d.body = (0..n)
            .map(|i| {
                // Eight links a paragraph, like a dense table of contents or cross-references.
                let mut p = wordcraft_doc::Paragraph::new();
                for k in 0..8 {
                    let link = CharProps { link: Some(format!("#mark{}", (i * 7 + k) % n)), ..CharProps::default() };
                    p.insert_text(p.len(), &format!("see {k} "), &link).unwrap();
                }
                p.insert_object(0, InlineObject::BookmarkStart { name: format!("mark{i}") }, &CharProps::default()).unwrap();
                para_block(p)
            })
            .collect();
        d
    };
    let fastest = |d: &Document| {
        (0..3)
            .map(|_| {
                let t = std::time::Instant::now();
                let bytes = export(d, &PdfOptions::default()).unwrap();
                assert!(!bytes.is_empty());
                t.elapsed().as_secs_f64()
            })
            .fold(f64::INFINITY, f64::min)
    };
    let (small, big) = (doc(100), doc(400));
    let _ = export(&small, &PdfOptions::default());
    let ratio = fastest(&big) / fastest(&small).max(1e-9);
    assert!(ratio < 9.0, "4x the paragraphs and links took {ratio:.1}x as long (quadratic would be ~16x)");
}

#[test]
fn all_caps_and_small_caps_text_extracts_as_displayed() {
    // #1473: All Caps / Small Caps text is drawn with capital glyphs, so it extracts in capitals
    // (as Word's PDFs do), and later text without caps keeps its own case: a capital glyph
    // first drawn for a lowercase letter must not make every later capital read as lowercase.
    let mut d = Document::new();
    d.styles.upsert(wordcraft_doc::Style {
        id: "CapsPara".into(),
        name: "Caps Para".into(),
        kind: wordcraft_doc::StyleKind::Paragraph,
        chr: CharProps { caps: Some(true), ..Default::default() },
        ..Default::default()
    });
    let caps = CharProps { caps: Some(true), ..Default::default() };
    let small = CharProps { small_caps: Some(true), ..Default::default() };
    d.body = vec![
        para_block(Paragraph::with_text("Employment Period. Your employment will begin on the date below.", CharProps::default()).styled("CapsPara")),
        para_block(Paragraph::with_text("Direct caps on the run: Mixed Case Words.", caps.clone())),
        para_block(Paragraph::with_text("Plain text, no caps: Employment Period.", CharProps::default())),
        para_block(Paragraph::with_text("Small caps line: Quiet Words.", small)),
        para_block(Paragraph::with_text("Straße", caps)),
        para_block(Paragraph::with_text("Plain again: Straße, Quiet Period.", CharProps::default())),
    ];
    let text = extract_text(&export(&d, &PdfOptions::default()).unwrap()).concat();
    let text = squash(&text);
    for line in [
        "EMPLOYMENT PERIOD. YOUR EMPLOYMENT WILL BEGIN ON THE DATE BELOW.",
        "DIRECT CAPS ON THE RUN: MIXED CASE WORDS.",
        "Plain text, no caps: Employment Period.",
        "SMALL CAPS LINE: QUIET WORDS.",
        "Plain again: Straße, Quiet Period.",
    ] {
        assert!(text.contains(line), "missing {line:?} in {text:?}");
    }
    // An All Caps ß is drawn as SS, as Word does (#1581), and reads as the SS drawn.
    assert!(text.contains("STRASSE"), "{text:?}");
}

#[test]
fn caps_list_labels_extract_as_displayed() {
    // #1473: a lowercase-letter list label in an All Caps paragraph is drawn with capital glyphs;
    // it reads as those capitals, and a later plain capital A still reads as "A".
    let mut d = Document::new();
    d.styles.upsert(wordcraft_doc::Style {
        id: "CapsPara".into(),
        name: "Caps Para".into(),
        kind: wordcraft_doc::StyleKind::Paragraph,
        chr: CharProps { caps: Some(true), ..Default::default() },
        ..Default::default()
    });
    let list = d.numbering.add_list(ListKind::Numbered);
    let mut item = Paragraph::with_text("item", CharProps::default()).styled("CapsPara");
    item.props.numbering = Some(NumRef { num: list, level: 1 });
    d.body = vec![para_block(item), para_block(Paragraph::with_text("Plain Apple.", CharProps::default()))];
    let text = squash(&extract_text(&export(&d, &PdfOptions::default()).unwrap()).concat());
    assert!(text.contains("A."), "the label reads as the capital drawn: {text:?}");
    assert!(!text.contains("a."), "no lowercase label: {text:?}");
    assert!(text.contains("Plain Apple."), "a plain capital A keeps its case: {text:?}");
}

/// The reporter's four paragraphs from #1383 (Hebrew, Arabic and mixed right-to-left paragraphs,
/// and a Latin one), then right-to-left words with a lam-alef ligature and with vowel marks.
fn rtl_sample() -> Document {
    let mut d = Document::from_text("שלום עולם\nمرحبا بالعالم\nייפוי כח מס' 123 – Power of Attorney\nHello world\nالسلام مَرْحَبًا שָׁלוֹם");
    // Right to left: all but "Hello world".
    for i in [0, 1, 2, 4] {
        let at = wordcraft_doc::Pos::body(i, 0);
        d.format_paragraphs(&at, &at, &|p: &mut wordcraft_doc::ParaProps| p.bidi = Some(true)).unwrap();
    }
    for (i, rtl) in [true, true, true, false, true].into_iter().enumerate() {
        let Some(Block::Para(p)) = d.body.get(i).map(|b| &**b) else { panic!("paragraph {i}") };
        assert_eq!(p.props.bidi == Some(true), rtl, "paragraph {i}");
    }
    d
}

/// The `/ActualText` of each marked-content sequence (BMC/BDC) on a page, in content-stream order.
fn actual_texts(page: &hayro_syntax::page::Page<'_>) -> Vec<Option<String>> {
    use hayro_syntax::content::ops::TypedInstruction;
    use hayro_syntax::object::{Object, String as PdfString};
    let mut out = Vec::new();
    let mut ops = page.typed_operations();
    while let Some(op) = ops.next() {
        match op {
            TypedInstruction::BeginMarkedContent(_) => out.push(None),
            TypedInstruction::BeginMarkedContentWithProperties(bdc) => out.push(match bdc.1 {
                Object::Dict(d) => d.get::<PdfString<'_>>("ActualText").map(|s| pdf_text_string(s.as_bytes())),
                _ => None,
            }),
            _ => {}
        }
    }
    out
}

/// A PDF text string: UTF-16BE or UTF-8 after a byte order mark, else (near enough) PDFDocEncoding.
fn pdf_text_string(b: &[u8]) -> String {
    if let Some(utf16) = b.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = utf16.as_chunks::<2>().0.iter().map(|&c| u16::from_be_bytes(c)).collect();
        String::from_utf16_lossy(&units)
    } else if let Some(utf8) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(utf8).into_owned()
    } else {
        b.iter().map(|&c| char::from(c)).collect()
    }
}

/// Each glyph's Unicode text in content-stream order, with its baseline y and its outline's left
/// and right edges (none for a space). Inside a marked-content sequence with `/ActualText` (a
/// cluster whose glyphs' ToUnicode entries can't spell it), that text goes with the first glyph and
/// the rest have none, as PDFium and poppler read it; elsewhere a glyph without a ToUnicode entry
/// has `None` text.
type StreamGlyph = (Option<String>, f64, Option<(f64, f64)>);
fn stream_glyphs(bytes: &[u8]) -> Vec<StreamGlyph> {
    use hayro_interpret::font::Glyph;
    use hayro_interpret::{
        BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask,
        interpret_page,
    };
    use hayro_syntax::Pdf;
    #[derive(Default)]
    struct Units {
        glyphs: Vec<StreamGlyph>,
        /// The page's ActualText per marked-content sequence, and how many have begun.
        actual: Vec<Option<String>>,
        begun: usize,
        /// Open sequences: the ActualText still to hand out (emptied by the first glyph).
        open: Vec<Option<String>>,
    }
    impl Device<'_> for Units {
        fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
        fn set_blend_mode(&mut self, _: BlendMode) {}
        fn draw_path(&mut self, _: &kurbo::BezPath, _: kurbo::Affine, _: &Paint<'_>, _: &PathDrawMode) {}
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
        fn begin_marked_content(&mut self, _: &[u8], _: Option<i32>) {
            self.open.push(self.actual.get(self.begun).cloned().flatten());
            self.begun += 1;
        }
        fn end_marked_content(&mut self) {
            self.open.pop();
        }
        fn draw_glyph(&mut self, g: &Glyph<'_>, t: kurbo::Affine, gt: kurbo::Affine, _: &Paint<'_>, mode: &GlyphDrawMode) {
            if matches!(mode, GlyphDrawMode::Stroke(_)) {
                return;
            }
            let text = if let Some(actual) = self.open.iter_mut().find_map(Option::as_mut) {
                Some(std::mem::take(actual)).filter(|s| !s.is_empty())
            } else {
                match g.as_unicode() {
                    Some(hayro_cmap::BfString::Char(c)) => Some(c.to_string()),
                    Some(hayro_cmap::BfString::String(s)) => Some(s),
                    None => None,
                }
            };
            let edges = match g {
                Glyph::Outline(o) if !o.outline().elements().is_empty() => {
                    use kurbo::Shape;
                    let b = (t * gt * o.outline()).bounding_box();
                    Some((b.x0, b.x1))
                }
                _ => None,
            };
            self.glyphs.push((text, (t * gt * kurbo::Point::ZERO).y, edges));
        }
        fn draw_image(&mut self, _: Image<'_, '_>, _: kurbo::Affine) {}
        fn pop_clip_path(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }
    let pdf = Pdf::new(bytes.to_vec()).expect("parse");
    let cache = InterpreterCache::new();
    let mut out = Units::default();
    for page in pdf.pages().iter() {
        out.actual = actual_texts(page);
        out.begun = 0;
        let mut ctx = Context::new(kurbo::Affine::IDENTITY, kurbo::Rect::new(0.0, 0.0, 1.0, 1.0), &cache, pdf.xref(), InterpreterSettings::default());
        interpret_page(page, &mut ctx, &mut out);
    }
    out.glyphs
}

/// One glyph drawn for two different letters: ToUnicode can map it to only one, so krilla wraps
/// the other in an ActualText span, and `stream_glyphs` reads the span's text (#1773). Whether
/// shaping reuses a glyph like this depends on the font: Noto Naskh Arabic draws a dotted letter
/// as a dotless body that other letters share, so the RTL tests met it only where it's installed.
#[test]
fn stream_glyphs_read_actual_text() {
    let font = Font::new(wordcraft_fonts::bundled()[0].to_vec().into(), 0).unwrap();
    let glyph = |range| KrillaGlyph::new(GlyphId::new(10), 0.5, 0.0, 0.0, 0.0, range, None);
    let mut pdf = krilla::Document::new();
    let mut page = pdf.start_page_with(PageSettings::new(Size::from_wh(200.0, 100.0).unwrap()));
    let mut s = page.surface();
    s.draw_glyphs(Point::from_xy(10.0, 50.0), &[glyph(0..1), glyph(1..2)], font, "AB", 12.0, false);
    s.finish();
    page.finish();
    let bytes = pdf.finish().unwrap();
    let texts: Vec<_> = stream_glyphs(&bytes).into_iter().map(|(t, ..)| t).collect();
    assert_eq!(texts, [Some("A".to_string()), Some("B".to_string())]);
}

/// Words as PDF viewers' text extraction (PDFium in Chrome and Edge, pdf.js, poppler) reads them:
/// glyphs are taken to be in visual order, so a word in a right-to-left script is read with its
/// glyphs reversed. Words end at spaces, at a jump back to the left, and where the
/// baseline moves (more than a raised mark does). ActualText is read as PDFium and poppler read
/// it; pdf.js ignores it.
fn viewer_words(bytes: &[u8]) -> Vec<String> {
    let mut words = Vec::new();
    let mut word: Vec<String> = Vec::new();
    let mut flush = |word: &mut Vec<String>| {
        if word.concat().chars().any(wordcraft_fonts::is_rtl) {
            word.reverse();
        }
        if !word.is_empty() {
            words.push(word.concat());
        }
        word.clear();
    };
    let mut last_y = None;
    let mut last_edges: Option<(f64, f64)> = None;
    for (text, y, edges) in stream_glyphs(bytes) {
        // A new line, or a letter wholly left of the last one (a jump back): a new word. Hebrew
        // and Arabic vowel marks sit anywhere over or under their letter.
        let mark = text.as_deref().is_some_and(|t| t.chars().all(|c| matches!(c, '\u{591}'..='\u{5C7}' | '\u{64B}'..='\u{65F}' | '\u{670}')));
        let edges = edges.filter(|_| !mark);
        let apart = matches!((last_edges, edges), (Some((l0, _)), Some((_, x1))) if x1 < l0);
        if last_y.is_some_and(|l: f64| (l - y).abs() > 6.0) || apart {
            flush(&mut word);
        }
        if !mark {
            last_y = Some(y);
        }
        if edges.is_some() {
            last_edges = edges;
        }
        let Some(text) = text else { continue };
        if text.chars().all(char::is_whitespace) {
            flush(&mut word);
        } else {
            word.push(text);
        }
    }
    flush(&mut word);
    words
}

/// Right-to-left text reads back whole (#1383): Chrome, Edge and other PDF viewers take a run's
/// glyphs to be written left to right as they appear, and reverse right-to-left words to get
/// their reading order. Written in reading order instead, `שלום` came back as `םולש`, and Arabic
/// came apart.
#[test]
fn right_to_left_text_extracts_in_logical_order() {
    let d = rtl_sample();
    let words = viewer_words(&export(&d, &PdfOptions::default()).unwrap());
    for w in ["שלום", "עולם", "مرحبا", "بالعالم", "ייפוי", "כח", "123", "Power", "Attorney", "Hello", "world", "السلام", "مَرْحَبًا", "שָׁלוֹם"]
    {
        assert!(words.iter().any(|x| x == w), "{w:?} not extracted intact from {words:?}");
    }
}

/// Every glyph's outline on the page, as a rounded bounding box in page space (sorted): what the
/// page looks like, whatever order the glyphs were written in.
fn glyph_boxes(bytes: &[u8]) -> Vec<[i64; 4]> {
    use hayro_interpret::font::Glyph;
    use hayro_interpret::{
        BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask,
        interpret_page,
    };
    use hayro_syntax::Pdf;
    use kurbo::Shape;
    struct Boxes(Vec<[i64; 4]>);
    impl Device<'_> for Boxes {
        fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
        fn set_blend_mode(&mut self, _: BlendMode) {}
        fn draw_path(&mut self, _: &kurbo::BezPath, _: kurbo::Affine, _: &Paint<'_>, _: &PathDrawMode) {}
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
        fn draw_glyph(&mut self, g: &Glyph<'_>, t: kurbo::Affine, gt: kurbo::Affine, _: &Paint<'_>, mode: &GlyphDrawMode) {
            if matches!(mode, GlyphDrawMode::Stroke(_)) {
                return;
            }
            // Spaces have no outline: nothing to see.
            if let Glyph::Outline(o) = g
                && !o.outline().elements().is_empty()
            {
                let b = (t * gt * o.outline()).bounding_box();
                let r = |v: f64| (v * 10.0).round() as i64;
                self.0.push([r(b.x0), r(b.y0), r(b.x1), r(b.y1)]);
            }
        }
        fn draw_image(&mut self, _: Image<'_, '_>, _: kurbo::Affine) {}
        fn pop_clip_path(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }
    let pdf = Pdf::new(bytes.to_vec()).expect("parse");
    let cache = InterpreterCache::new();
    let mut out = Boxes(Vec::new());
    for page in pdf.pages().iter() {
        let mut ctx = Context::new(kurbo::Affine::IDENTITY, kurbo::Rect::new(0.0, 0.0, 1.0, 1.0), &cache, pdf.xref(), InterpreterSettings::default());
        interpret_page(page, &mut ctx, &mut out);
    }
    out.0.sort_unstable();
    out.0
}

/// Writing right-to-left runs in reading order doesn't move a glyph: the page draws the same
/// glyph outlines at the same places as layout put them.
#[test]
fn right_to_left_glyphs_stay_where_layout_put_them() {
    use kurbo::Shape;
    let d = rtl_sample();
    let bytes = export(&d, &PdfOptions::default()).unwrap();
    let got = glyph_boxes(&bytes);
    // Layout's glyphs, drawn as their outlines (y down from the page top; PDF is y up).
    let lay = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let page = &lay.pages[0];
    let h = f64::from(page.h);
    let db = wordcraft_fonts::FontDb::global();
    let mut want = Vec::new();
    for item in wordcraft_layout::display::page_display(&d, page, &DisplayOptions::default()) {
        let Draw::Glyphs { face, size, glyphs, .. } = item else { continue };
        let scale = f64::from(size) / face.upem.max(1.0);
        for (g, x, y) in glyphs {
            let mut o = (*db.outline(&face, g)).clone();
            if o.elements().is_empty() {
                continue;
            }
            o.apply_affine(
                kurbo::Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, h])
                    * kurbo::Affine::translate((f64::from(x), f64::from(y)))
                    * kurbo::Affine::scale(scale),
            );
            let b = o.bounding_box();
            let r = |v: f64| (v * 10.0).round() as i64;
            want.push([r(b.x0), r(b.y0), r(b.x1), r(b.y1)]);
        }
    }
    want.sort_unstable();
    assert!(!want.is_empty());
    let close = got.len() == want.len() && got.iter().zip(&want).all(|(a, b)| a.iter().zip(b).all(|(p, q)| (p - q).abs() <= 2));
    assert!(close, "glyphs moved:\n got {got:?}\nwant {want:?}");
}

/// A separately formatted Latin run inside a right-to-left paragraph is written as it was, left
/// to right, and the Hebrew around it still reads back whole.
#[test]
fn latin_run_in_right_to_left_paragraph_reads_back_whole() {
    let mut p = Paragraph::with_text("שלום ", CharProps::default());
    p.props.bidi = Some(true);
    p.insert_text(p.len(), "Latin words", &CharProps { bold: Some(true), ..Default::default() }).unwrap();
    p.insert_text(p.len(), " עולם", &CharProps::default()).unwrap();
    let mut d = Document::new();
    d.body = vec![para_block(p)];
    let words = viewer_words(&export(&d, &PdfOptions::default()).unwrap());
    for w in ["שלום", "Latin", "words", "עולם"] {
        assert!(words.iter().any(|x| x == w), "{w:?} not extracted intact from {words:?}");
    }
}

/// Glyphs already running left to right keep their order: Latin runs are written as before.
#[test]
fn left_to_right_glyphs_keep_their_order() {
    let glyphs = [(1, 0.0, 10.0), (2, 5.0, 10.0), (3, 5.0, 10.0), (4, 9.0, 10.0)];
    assert!(left_to_right(&glyphs));
}

/// A lam-alef some fonts draw as a lam and a zero-width alef at the same x goes in reverse
/// text order, so the run's ranges still decrease and viewers read لا, not ال (#2310).
/// Left-to-right clusters at one x keep their text order.
#[test]
fn clusters_at_the_same_x_go_in_reading_order() {
    let text = "سلا";
    let glyphs = [(3, 20.0, 10.0), (1, 10.0, 10.0), (2, 10.0, 0.0)];
    let ranges = [0..2, 2..4, 4..6];
    assert_eq!(visual_order(text, &glyphs, &ranges, &ranges), [2, 1, 0]);
    let text = "ab";
    let glyphs = [(1, 10.0, 0.0), (2, 10.0, 5.0)];
    let ranges = [0..1, 1..2];
    assert_eq!(visual_order(text, &glyphs, &ranges, &ranges), [0, 1]);
}

/// Right-to-left glyphs (reading order, drawn right to left) are written left to right a
/// cluster at a time. A letter's vowel mark takes its own character and goes before the letter,
/// so a viewer reversing the word reads letter then mark; a lam-alef drawn as two glyphs keeps
/// its glyphs together in shaping order, so no word comes apart.
#[test]
fn right_to_left_clusters_are_written_whole_left_to_right() {
    // Reading order: shin with qamats (mark glyph 2 drawn at x 28, then the letter, glyph 1, at
    // x 30); lam-alef as two glyphs at x 20 and 16; a one-glyph lam-alef at x 8; final mem at 0.
    let text = "שָלالاם";
    let glyphs = [(2, 28.0, 8.0), (1, 30.0, 10.0), (3, 20.0, 10.0), (4, 16.0, 10.0), (5, 8.0, 10.0), (6, 0.0, 10.0)];
    let ranges = [0..4, 0..4, 4..8, 4..8, 8..12, 12..14];
    let rev: std::collections::HashMap<u32, char> = [(1, 'ש'), (2, '\u{5B8}'), (6, 'ם')].into_iter().collect();
    assert!(!left_to_right(&glyphs));
    let parts = split_clusters(text, &glyphs, &ranges, &rev);
    assert_eq!(parts, [2..4, 0..2, 4..8, 4..8, 8..12, 12..14]);
    // Glyph by glyph by x this would be [5, 4, 3, 2, 0, 1]: the lam-alef's glyphs swapped.
    assert_eq!(visual_order(text, &glyphs, &ranges, &parts), [5, 4, 2, 3, 0, 1]);
}

/// A PNG signature, an IHDR declaring `w`×`h` 8-bit RGBA and a short IDAT (all CRCs valid, so
/// the header parses), with no real pixel data.
fn ihdr_only_png(w: u32, h: u32) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        let mut crc = 0xFFFF_FFFFu32;
        for b in &body {
            crc ^= *b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(&body);
        out.extend_from_slice(&(!crc).to_be_bytes());
    }
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = w.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &[0x78, 0x9c, 0, 0]);
    out
}

#[test]
fn extreme_page_index_returns_an_error() {
    let d = Document::from_text("Hostile page selection");
    let lay = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let opts = PdfOptions { pages: Some(vec![usize::MAX]), ..Default::default() };
    assert_eq!(export(&d, &opts), Err(PdfError::BadPage(usize::MAX)));
    assert_eq!(export_layout(&d, &lay, &opts), Err(PdfError::BadPage(usize::MAX)));
}

#[test]
fn metadata_language_overrides_and_fallbacks_are_preserved() {
    let mut d = Document::from_text("Metadata export");
    d.core.created = "2024-05-06T07:08:09Z".into();
    d.core.title = "Original title".into();
    d.core.creator = "Original author".into();
    d.core.description = "Description fallback".into();
    d.core.keywords = "first; second, third;;".into();
    let opts = PdfOptions {
        title: Some("Override title".into()),
        author: Some("Override author".into()),
        tagged: false,
        compress: false,
        ..Default::default()
    };
    for language in [Some("es-CR"), Some("ja"), Some(""), Some("日本語"), None] {
        d.styles.default_chr.lang = language.map(str::to_owned);
        let bytes = export(&d, &opts).unwrap();
        let raw = String::from_utf8_lossy(&bytes);
        let pdf = hayro_syntax::Pdf::new(bytes.clone()).unwrap();
        let catalog = pdf.xref().get::<hayro_syntax::object::Dict<'_>>(pdf.xref().root_id()).unwrap();
        let actual_language = catalog.get::<hayro_syntax::object::String<'_>>(b"Lang");
        for value in ["Override title", "Override author", "Description fallback", "first, second, third", "D:20240506070809"] {
            assert!(raw.contains(value), "missing {value}");
        }
        if let Some(language) = language.filter(|s| !s.is_empty() && s.is_ascii()) {
            assert_eq!(actual_language.unwrap().as_bytes(), language.as_bytes());
            assert!(raw.contains(&format!("<dc:language><rdf:Bag><rdf:li>{language}</rdf:li>")));
        } else {
            assert!(actual_language.is_none());
            assert!(!raw.contains("<dc:language>"));
        }
    }
}

#[test]
fn bundled_unicode_glyphs_are_visible_and_extractable() {
    const TEXT: &str = "café naïve — coöperate Ελληνικά Привет Tiếng Việt";
    let fonts = wordcraft_fonts::FontDb::with_font_dirs(Vec::new());
    let bundled = fonts.face("Source Sans 3", "Regular");
    assert_eq!(bundled.family, "Source Sans 3");
    assert_eq!(bundled.style, "Regular");
    for ch in TEXT.chars() {
        assert!(bundled.covers(ch), "the explicitly bundled face must cover {ch:?}");
    }
    let mut d = Document::new();
    d.core.created = "2024-05-06T07:08:09Z".into();
    d.body = vec![para_block(Paragraph::with_text(TEXT, CharProps { font: Some("Source Sans 3".into()), size: Some(14.0), ..Default::default() }))];
    let lay = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let mut drawn = String::new();
    for page in &lay.pages {
        for draw in page_display(&d, page, &DisplayOptions::default()) {
            if let Draw::Glyphs { face, glyphs, text, .. } = draw {
                assert_eq!(face.data(), bundled.data(), "page glyphs must use the bundled face without system fallback");
                assert!(glyphs.iter().all(|(id, _, _)| *id != 0), "page glyphs must not use .notdef");
                drawn.push_str(&text);
            }
        }
    }
    assert_eq!(squash(&drawn), TEXT);
    let bytes = export_layout(&d, &lay, &PdfOptions::default()).unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("/ToUnicode"));
    assert_eq!(squash(&extract_text(&bytes).concat()), TEXT);
}

/// #2534: text with legacy font effects is drawn from its outlines and keeps its text, once.
#[test]
fn legacy_font_effects_keep_their_text() {
    let mut d = Document::new();
    d.body = [
        CharProps { outline: Some(true), ..Default::default() },
        CharProps { shadow: Some(true), ..Default::default() },
        CharProps { emboss: Some(true), ..Default::default() },
        CharProps { engrave: Some(true), ..Default::default() },
    ]
    .into_iter()
    .map(|c| para_block(Paragraph::with_text("Effect", c)))
    .collect();
    let text = extract_text(&export(&d, &PdfOptions::default()).unwrap()).concat();
    assert_eq!(text.matches("Effect").count(), 4, "{text}");
}

#[test]
fn shape_dash_arrays_export() {
    use wordcraft_doc::{
        connector::ShapeExtra,
        line_dash::LineDash,
        para::{Anchor, Float, ShapeKind, Wrap},
    };
    for kind in [ShapeKind::Rectangle, ShapeKind::Ellipse, ShapeKind::StraightConnector] {
        for dash in [LineDash::Dash, LineDash::Custom(vec![(400_000, 300_000)]), LineDash::Solid] {
            let shape = InlineObject::Shape {
                kind,
                w: 70.0,
                h: 28.0,
                fill: None,
                stroke: Some(wordcraft_doc::props::Rgb(192, 0, 0)),
                stroke_width: 1.0,
                float: Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 100.0, y: 100.0, ..Default::default() },
                story: None,
                freeform: None,
                effects: Default::default(),
                extra: ShapeExtra { dash: dash.clone(), ..Default::default() },
            };
            let mut p = Paragraph::new();
            p.insert_object(0, shape, &Default::default()).unwrap();
            let mut d = Document::new();
            d.body = vec![para_block(p)];
            let bytes = export(&d, &PdfOptions { compress: false, tagged: false, ..Default::default() }).unwrap();
            let raw = String::from_utf8_lossy(&bytes);
            assert_eq!(raw.contains("[4 3]0 d"), !dash.is_solid(), "dash array for {kind:?}");
        }
    }
}

#[test]
fn zero_width_connector_dashes() {
    use wordcraft_doc::{
        connector::ShapeExtra,
        line_dash::LineDash,
        para::{Anchor, Float, ShapeKind, Wrap},
    };
    for kind in [ShapeKind::StraightConnector, ShapeKind::ElbowConnector, ShapeKind::CurvedConnector, ShapeKind::Freeform] {
        let shape = InlineObject::Shape {
            kind,
            w: 70.0,
            h: 28.0,
            fill: None,
            stroke: Some(wordcraft_doc::props::Rgb(192, 0, 0)),
            stroke_width: 0.0,
            float: Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 100.0, y: 100.0, ..Default::default() },
            story: None,
            freeform: (kind == ShapeKind::Freeform).then(|| {
                std::sync::Arc::new(wordcraft_doc::freeform::Freeform {
                    w: 70.0,
                    h: 28.0,
                    paths: vec![wordcraft_doc::freeform::FreePath { pts: vec![[0.0, 0.0], [70.0, 0.0], [70.0, 28.0]], closed: true }],
                    ..Default::default()
                })
            }),
            effects: Default::default(),
            extra: ShapeExtra { dash: LineDash::Dash, ..Default::default() },
        };
        let mut p = Paragraph::new();
        p.insert_object(0, shape, &Default::default()).unwrap();
        let mut d = Document::new();
        d.body = vec![para_block(p)];
        let bytes = export(&d, &PdfOptions { compress: false, tagged: false, ..Default::default() }).unwrap();
        let raw = String::from_utf8_lossy(&bytes);
        assert!(raw.contains("0.75 w"), "effective width for {kind:?}");
        assert!(raw.contains("[3 2.25]0 d"), "stroke-relative dash array for {kind:?}");
    }
}

#[test]
fn image_budget_is_checked_from_the_header() {
    let data = Arc::new(png());
    // The decoded image and its RGBA conversion are charged: two buffers at peak.
    assert!(load_image_within(&data, 1024).is_some());
    assert!(load_image_within(&data, 1023).is_none());
    // 16-bit RGBA decodes to 8 bytes a pixel, alive with its RGBA8 conversion: 12 bytes a pixel.
    let mut wide = Vec::new();
    let img16 = image::ImageBuffer::<image::Rgba<u16>, _>::from_fn(16, 8, |x, y| image::Rgba([x as u16 * 999, y as u16, 9, 65535]));
    image::DynamicImage::ImageRgba16(img16).write_to(&mut std::io::Cursor::new(&mut wide), image::ImageFormat::Png).unwrap();
    let wide = Arc::new(wide);
    assert!(load_image_within(&wide, 1024).is_none(), "two RGBA8 buffers' worth is not enough for RGBA16");
    assert!(load_image_within(&wide, 1536).is_some());
    // A header (valid CRC) claiming 30000x30000 with no pixel data reads fine, then the budget refuses it.
    let hdr = ihdr_only_png(30_000, 30_000);
    let dims = image::ImageReader::new(std::io::Cursor::new(hdr.as_slice())).with_guessed_format().unwrap().into_dimensions().unwrap();
    assert_eq!(dims, (30_000, 30_000));
    assert!(load_image(&Arc::new(hdr)).is_none());
}

/// Scale to paper size (#2108): every page goes on the chosen paper, turned to the page's
/// orientation, with its text, links and Print Markup's balloons scaled onto it.
#[test]
fn pages_print_scaled_to_other_paper() {
    let size = |bytes: &[u8]| {
        let pdf = hayro_syntax::Pdf::new(bytes.to_vec()).expect("parse");
        pdf.pages().iter().map(|p| p.media_box()).map(|r| (r.width().round(), r.height().round())).collect::<Vec<_>>()
    };
    let a4 = Some((595.28, 841.89));
    let d = sample();
    let plain = export(&d, &PdfOptions::default()).unwrap();
    assert!(size(&plain).iter().all(|s| *s == (612.0, 792.0)), "the sample is Letter");
    let on_a4 = export(&d, &PdfOptions { paper: a4, compress: false, ..Default::default() }).unwrap();
    assert!(size(&on_a4).iter().all(|s| *s == (595.0, 842.0)), "{:?}", size(&on_a4));
    assert_eq!(size(&on_a4).len(), size(&plain).len(), "the pagination is the document's");
    assert_eq!(squash(&extract_text(&on_a4).concat()), squash(&extract_text(&plain).concat()));
    // Link areas are scaled with the page: they stay on the narrower paper.
    let raw = String::from_utf8_lossy(&on_a4).into_owned();
    let rects: Vec<Vec<f32>> = raw
        .split("/Rect[")
        .skip(1)
        .filter_map(|r| r.split(']').next())
        .map(|r| r.split_whitespace().filter_map(|n| n.parse().ok()).collect())
        .collect();
    assert!(!rects.is_empty(), "the sample has links");
    for r in &rects {
        assert!(r.len() == 4 && r.iter().all(|v| (0.0..=842.0).contains(v)) && r[0] <= 595.28 && r[2] <= 595.28, "{r:?}");
    }
    // A landscape page gets landscape paper; Legal is taller than Letter.
    let mut wide = Document::from_text("Wide page");
    let s = &mut wide.last_section;
    (s.page_w, s.page_h) = (792.0, 612.0);
    let landscape = export(&wide, &PdfOptions { paper: a4, ..Default::default() }).unwrap();
    assert_eq!(size(&landscape), vec![(842.0, 595.0)]);
    let legal = export(&Document::from_text("Tall"), &PdfOptions { paper: Some((612.0, 1008.0)), ..Default::default() }).unwrap();
    assert_eq!(size(&legal), vec![(612.0, 1008.0)]);
    // Print Markup's page with balloons goes on the same paper.
    let mut c = Document::from_text("Quarterly figures are in.");
    let part = c.add_part(wordcraft_doc::PartKind::Comment, vec![para_block(Paragraph::with_text("Check the totals", CharProps::default()))]);
    c.comments.insert(0, wordcraft_doc::Comment { author: "Ada".into(), part, ..Default::default() });
    c.insert_object(&wordcraft_doc::Pos::body(0, 4), InlineObject::CommentStart { id: 0 }, &CharProps::default()).unwrap();
    let marked = export(&c, &PdfOptions { comments: true, include_markup: true, paper: a4, ..Default::default() }).unwrap();
    assert_eq!(size(&marked), vec![(595.0, 842.0)]);
    assert!(squash(&extract_text(&marked).concat()).contains("Check the totals"));
    // Hostile paper never panics: it prints the page as it is.
    for paper in [(f32::NAN, 1.0), (0.0, 0.0), (-5.0, f32::INFINITY)] {
        let bytes = export(&d, &PdfOptions { paper: Some(paper), ..Default::default() }).unwrap();
        assert_eq!(size(&bytes), size(&plain));
    }
}

/// #1500: gradient shape fills are exported as PDF shadings (axial and radial), and a
/// transparent fill with its opacity.
#[test]
fn shape_gradients_export_as_shadings() {
    use wordcraft_doc::fill::{Fill as ShapeFill, Gradient, GradientFlip, GradientKind, PathShape};
    use wordcraft_doc::wordart::GradStop;
    let stops = vec![GradStop { pos: 0.0, color: Rgb::BLACK, transparency: 0.0 }, GradStop { pos: 100.0, color: Rgb::WHITE, transparency: 0.0 }];
    let grad =
        |kind| ShapeFill::Gradient(Arc::new(Gradient { stops: stops.clone(), kind, tile: None, rot_with_shape: true, flip: GradientFlip::None }));
    let mut d = Document::from_text("");
    let at = wordcraft_doc::Pos { story: wordcraft_doc::StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 0 };
    for fill in [
        grad(GradientKind::Linear { angle: 90.0, scaled: false }),
        grad(GradientKind::Path { shape: PathShape::Circle, focus: [50.0; 4] }),
        ShapeFill::Solid { color: Rgb::WHITE, transparency: 70.0 },
    ] {
        let shape = InlineObject::Shape {
            kind: ShapeKind::Rectangle,
            w: 50.0,
            h: 50.0,
            fill: Some(fill),
            stroke: None,
            stroke_width: 0.0,
            float: Default::default(),
            story: None,
            freeform: None,
            effects: Default::default(),
            extra: Default::default(),
        };
        d.insert_object(&at, shape, &Default::default()).unwrap();
    }
    let raw = String::from_utf8_lossy(&export(&d, &PdfOptions { compress: false, ..Default::default() }).unwrap()).into_owned();
    assert!(raw.contains("/ShadingType 2"), "axial shading");
    assert!(raw.contains("/ShadingType 3"), "radial shading");
    assert!(raw.contains("/ca 0.3"), "30% opaque fill");
}

/// A tagged PDF of a paragraph with one blue rectangle placed by `float`: a text box showing
/// "Inside the box" when `text_box`.
fn tagged_shape_pdf(float: wordcraft_doc::para::Float, text_box: bool) -> String {
    use wordcraft_doc::para::ShapeKind;
    let mut d = Document::from_text("Body");
    let story = text_box
        .then(|| d.add_part(wordcraft_doc::PartKind::TextBox, vec![para_block(Paragraph::with_text("Inside the box", CharProps::default()))]));
    let kind = if text_box { ShapeKind::TextBox } else { ShapeKind::Rectangle };
    let shape = InlineObject::Shape {
        kind,
        w: 120.0,
        h: 60.0,
        fill: Some(wordcraft_doc::fill::Fill::solid(wordcraft_doc::props::Rgb(0, 0, 0xFF))),
        stroke: None,
        stroke_width: 0.0,
        float,
        story,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    d.insert_object(&wordcraft_doc::Pos::body(0, 4), shape, &Default::default()).unwrap();
    String::from_utf8_lossy(&export(&d, &PdfOptions { compress: false, tagged: true, ..Default::default() }).unwrap()).into_owned()
}

/// A shape with a description is a Figure with it in tagged PDF, inline, rotated or floating; one
/// without, or marked decorative, stays an artifact screen readers skip, and so does a text box's
/// frame: its text is read as text, not replaced by its description.
#[test]
fn described_shapes_are_figures_with_their_description() {
    use wordcraft_doc::para::{AltText, Anchor, Float, Wrap};
    let described = AltText { text: "Arrow pointing to the exit".into(), ..Default::default() };
    let floating = Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 72.0, y: 200.0, ..Default::default() };
    let cases = [
        ("inline", Float::default()),
        ("rotated inline", Float { rot: 30.0, ..Default::default() }),
        ("floating", floating.clone()),
        ("rotated floating", Float { rot: 30.0, ..floating }),
    ];
    for (name, float) in cases {
        let raw = tagged_shape_pdf(Float { alt: described.clone(), ..float.clone() }, false);
        assert!(raw.contains("/Alt(Arrow pointing to the exit)"), "{name}: a described shape is a Figure with its description");
        assert_eq!(raw.matches("/Alt(").count(), 1, "{name}: the described shape is one Figure");
        assert!(raw.contains("0 0 1 rg"), "{name}: the shape is drawn");
        assert!(!tagged_shape_pdf(float.clone(), false).contains("/Alt("), "{name}: an undescribed shape stays an artifact");
        let decorative = AltText { decorative: true, ..described.clone() };
        assert!(
            !tagged_shape_pdf(Float { alt: decorative, ..float.clone() }, false).contains("/Alt("),
            "{name}: a decorative shape stays an artifact"
        );
        let text_box = tagged_shape_pdf(Float { alt: described.clone(), ..float }, true);
        assert!(!text_box.contains("/Alt("), "{name}: a text box is not a Figure");
        assert_eq!(text_box.matches("/S/P").count(), 2, "{name}: the text box's text is read as a paragraph");
    }
}

#[test]
fn a_watermark_fills_its_box() {
    // Word stretches the watermark's text over its shape (`fitshape`): one em down the box's
    // height, the advance across its width, the box centred on the margins. WordCraft made the
    // text 70 % of the page diagonal, whatever the box.
    let mut d = Document::new();
    d.body = vec![para_block(Paragraph::with_text("Body", CharProps::default()))];
    d.settings.watermark = Some(Watermark { text: "COPY".into(), diagonal: false, size: Some([300.0, 80.0]), ..Default::default() });
    let frames = glyph_frames(&export(&d, &PdfOptions::default()).unwrap());
    // (Glyph space is a thousandth of the size.)
    let wm: Vec<&(f64, f64, f64)> = frames.iter().filter(|f| (f.2 * 1000.0 - 80.0).abs() < 0.01).collect();
    assert_eq!(wm.len(), 4, "four letters one box high: {frames:?}");
    let s = &d.last_section;
    let centre = f64::from(s.margin_left + (s.page_w - s.margin_left - s.margin_right) / 2.0);
    assert!((wm[0].0 - (centre - 150.0)).abs() < 0.05, "the text starts at the box's left edge: {wm:?}");
    // Stretched across (the box is wider than the text at 80 pt), the same for every letter.
    assert!(wm.iter().all(|f| f.1 > f.2 * 1.01 && (f.1 - wm[0].1).abs() < 1e-6), "{wm:?}");
}

/// Each glyph drawn as text: its origin and its horizontal and vertical scale, in page space.
fn glyph_frames(bytes: &[u8]) -> Vec<(f64, f64, f64)> {
    use hayro_interpret::font::Glyph;
    use hayro_interpret::{
        BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask,
        interpret_page,
    };
    use hayro_syntax::Pdf;
    struct Frames(Vec<(f64, f64, f64)>);
    impl Device<'_> for Frames {
        fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
        fn set_blend_mode(&mut self, _: BlendMode) {}
        fn draw_path(&mut self, _: &kurbo::BezPath, _: kurbo::Affine, _: &Paint<'_>, _: &PathDrawMode) {}
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
        fn draw_glyph(&mut self, _: &Glyph<'_>, transform: kurbo::Affine, glyph: kurbo::Affine, _: &Paint<'_>, _: &GlyphDrawMode) {
            let [a, b, c, d, e, _] = (transform * glyph).as_coeffs();
            self.0.push((e, a.hypot(b), c.hypot(d)));
        }
        fn draw_image(&mut self, _: Image<'_, '_>, _: kurbo::Affine) {}
        fn pop_clip_path(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }
    let pdf = Pdf::new(bytes.to_vec()).expect("parse");
    let cache = InterpreterCache::new();
    let mut f = Frames(Vec::new());
    for page in pdf.pages().iter() {
        let mut ctx = Context::new(kurbo::Affine::IDENTITY, kurbo::Rect::new(0.0, 0.0, 1.0, 1.0), &cache, pdf.xref(), InterpreterSettings::default());
        interpret_page(page, &mut ctx, &mut f);
    }
    f.0
}

#[test]
fn scaled_letters_are_stretched_not_spaced() {
    // `w:w` 200: the letters are twice as wide and stand where the layout put them (their
    // advances doubled), as in Word; at 100 they are drawn as they are.
    let frames = |scale: Option<f32>| {
        let mut d = Document::new();
        d.body = vec![para_block(Paragraph::with_text("HHH", CharProps { scale, ..Default::default() }))];
        glyph_frames(&export(&d, &PdfOptions::default()).unwrap())
    };
    let plain = frames(None);
    let wide = frames(Some(200.0));
    assert_eq!((plain.len(), wide.len()), (3, 3), "{plain:?} {wide:?}");
    for ((x, sx, sy), (wx, wsx, wsy)) in plain.iter().zip(&wide) {
        assert!((sx / sy - 1.0).abs() < 1e-3, "plain letters keep their shape: {sx} {sy}");
        assert!((wsx / wsy - 2.0).abs() < 1e-3, "scaled letters are twice as wide: {wsx} {wsy}");
        assert!((wsy - sy).abs() < 1e-3, "and as high");
        let (x0, w0) = (plain[0].0, wide[0].0);
        assert!(((wx - w0) - 2.0 * (x - x0)).abs() < 0.01, "pen positions follow the doubled advances: {wide:?} {plain:?}");
    }
}

/// The PDF's bytes, uncompressed, as text: content stream operators can be read directly.
fn raw_pdf(d: &Document) -> String {
    String::from_utf8_lossy(&export(d, &PdfOptions { compress: false, tagged: false, ..Default::default() }).unwrap()).into_owned()
}

#[test]
fn borders_are_dashed_with_words_patterns() {
    // Word draws a border's dots one width on and one off, its dashes three on and one off, and
    // dash-dot as three, one, one, one; underlines keep their own patterns.
    use wordcraft_doc::props::{Border, BorderStyle, Borders};
    let bordered = |style: BorderStyle, width: f32| {
        let mut p = Paragraph::with_text("Rule", CharProps::default());
        p.props.borders = Some(Borders { bottom: Some(Border { style, width, ..Default::default() }), ..Default::default() });
        para_block(p)
    };
    let mut d = Document::new();
    d.body = vec![bordered(BorderStyle::Dotted, 1.0), bordered(BorderStyle::Dashed, 2.0), bordered(BorderStyle::DotDash, 1.0)];
    let raw = raw_pdf(&d);
    assert!(raw.contains("[1 1]0 d") || raw.contains("[1 1] 0 d"), "the dotted border");
    assert!(raw.contains("[6 2]0 d") || raw.contains("[6 2] 0 d"), "the dashed border");
    assert!(raw.contains("[3 1 1 1]0 d") || raw.contains("[3 1 1 1] 0 d"), "the dash-dot border");
}

#[test]
fn only_safe_link_targets_become_annotations() {
    use hayro_syntax::object::{Array, Dict, String as PdfString};

    let mut d = Document::new();
    d.body = ["https://example.com/", "mailto:ada@example.com", "javascript:alert(1)", "file:///etc/passwd"]
        .into_iter()
        .map(|url| para_block(Paragraph::with_text("Click here", CharProps { link: Some(url.into()), ..Default::default() })))
        .collect();
    let bytes = export(&d, &PdfOptions::default()).unwrap();
    assert_eq!(squash(&extract_text(&bytes)[0]).matches("Click here").count(), 4, "unlinked text stays");
    let pdf = hayro_syntax::Pdf::new(bytes).unwrap();
    let uris: Vec<String> = pdf.pages()[0]
        .raw()
        .get::<Array>(b"Annots")
        .into_iter()
        .flat_map(|a| a.iter::<Dict>().collect::<Vec<_>>())
        .map(|a| {
            let uri = a.get::<Dict>(b"A").unwrap().get::<PdfString>(b"URI").unwrap();
            String::from_utf8_lossy(uri.as_ref()).into_owned()
        })
        .collect();
    assert_eq!(uris, ["https://example.com/", "mailto:ada@example.com"]);
}

/// A glyph's text runs to the first later glyph with a cluster past its own (the end of the text
/// without one), found in one pass: a long run of glyphs sharing a cluster used to rescan the rest
/// of the run for every glyph.
#[test]
fn cluster_ends_match_the_next_larger_cluster_in_linear_time() {
    let brute = |cl: &[usize], len: usize| -> Vec<usize> {
        (0..cl.len()).map(|i| cl.iter().skip(i + 1).copied().find(|n| *n > cl[i]).unwrap_or(len)).collect::<Vec<_>>()
    };
    for cl in [vec![], vec![0], vec![0, 0, 3, 3, 5], vec![5, 3, 3, 0], vec![2, 0, 4, 1, 1, 6, 3], vec![0, 7, 2, 2, 9, 1]] {
        assert_eq!(cluster_ends(&cl, 10), brute(&cl, 10), "{cl:?}");
    }
    // One base letter and a million combining marks in its cluster, then one more letter.
    // Milliseconds; the rescan (`brute`) took 2.9 s at 50,000 and grows with the square, so
    // about 20 minutes here.
    let n = 1_000_000;
    let mut cl = vec![0; n + 1];
    cl.push(2 * n + 2);
    let start = std::time::Instant::now();
    let ends = cluster_ends(&cl, 2 * n + 4);
    assert!(start.elapsed() < std::time::Duration::from_secs(10), "{:?}", start.elapsed());
    assert!(ends[..=n].iter().all(|e| *e == 2 * n + 2));
    assert_eq!(ends.last(), Some(&(2 * n + 4)));
}

/// A line in a cell cut off at its edge is drawn with that cell's clip, found by path in an index
/// built once per page: every line used to carry a clone of every clipped cell on the page, so a
/// page of exact-height rows cost lines × cells.
#[test]
fn a_line_finds_its_clipped_cell_by_path() {
    let cell = |story, table: Vec<u32>, row, cell, clip| Placed::Cell {
        rect: Rect::default(),
        covered: false,
        table: DocPath(table),
        row,
        cell,
        story,
        clip,
    };
    let mut page = Page { items: vec![cell(StoryRef::Body, vec![0], 0, 1, false), cell(StoryRef::Body, vec![2], 1, 0, true)], ..Default::default() };
    // A nested clipped cell inside the first one; the outer one comes first in the page.
    page.items.push(cell(StoryRef::Body, vec![2, 1, 0, 3], 0, 0, true));
    page.footer.push(cell(StoryRef::Part(7), vec![0], 0, 0, true));
    let clips = clipped_cells(&page);
    let found = |story, path: &[u32]| {
        clip_cell(&clips, story, path).and_then(|c| if let Placed::Cell { table, .. } = c { Some(table.0.clone()) } else { None })
    };
    assert_eq!(found(StoryRef::Body, &[2, 1, 0, 0]), Some(vec![2]));
    assert_eq!(found(StoryRef::Body, &[2, 1, 0, 3, 0, 0, 1]), Some(vec![2]), "the first cell in page order wins, as in layout");
    assert_eq!(found(StoryRef::Body, &[0, 0, 1, 0]), None, "that cell isn't clipped");
    assert_eq!(found(StoryRef::Body, &[2, 0, 0, 0]), None, "another row");
    assert_eq!(found(StoryRef::Part(7), &[0, 0, 0, 2]), Some(vec![0]));
    assert_eq!(found(StoryRef::Body, &[0, 0, 0, 2]), None, "another story");

    // 100 000 clipped cells and as many lines: a lookup each, not a pass over the cells each
    // (10^10 steps, minutes; the lookups take well under a second on a loaded machine).
    let n = 100_000;
    let big = Page { items: (0..n).map(|r| cell(StoryRef::Body, vec![0], r, 0, true)).collect(), ..Default::default() };
    let start = std::time::Instant::now();
    let clips = clipped_cells(&big);
    let hits = (0..n as u32).filter(|r| clip_cell(&clips, StoryRef::Body, &[0, *r, 0, 0]).is_some()).count();
    assert_eq!(hits, n);
    assert!(start.elapsed() < std::time::Duration::from_secs(10), "{:?}", start.elapsed());
}
