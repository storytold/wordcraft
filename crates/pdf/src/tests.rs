use std::sync::Arc;

use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::NumRef;
use wordcraft_doc::{Block, CharProps, Document, ListKind, Paragraph, Table, Watermark, para_block};

use crate::*;

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

fn extract_text(bytes: &[u8]) -> Vec<String> {
    extract_page_text(bytes).expect("parse")
}

/// The wasm build has no PDF interpreter and passes `None` here; that must never read as a
/// searchable PDF (a bare `%PDF` signature is not text).
#[test]
fn a_build_without_an_extractor_never_claims_a_pdf_is_searchable() {
    assert_eq!(searchability_verdict(None), TextSearchability::Unavailable);
    assert_eq!(searchability_verdict(None).is_searchable(), None);
    assert_eq!(searchability_verdict(Some(false)), TextSearchability::NotSearchable);
    assert_eq!(searchability_verdict(Some(false)).is_searchable(), Some(false));
    assert_eq!(searchability_verdict(Some(true)), TextSearchability::Searchable);
    assert_eq!(searchability_verdict(Some(true)).is_searchable(), Some(true));
}

/// A non-PDF byte string is confidently not searchable (this is what the wasm signature check
/// used to get wrong); a real exported PDF with text is searchable.
#[test]
fn text_searchability_inspects_real_bytes() {
    assert_eq!(text_searchability(b"%PDF-not-really"), TextSearchability::NotSearchable);
    assert_eq!(text_searchability(b"hello"), TextSearchability::NotSearchable);
    let bytes = export(&sample(), &PdfOptions::default()).unwrap();
    assert_eq!(text_searchability(&bytes), TextSearchability::Searchable);
}

/// A scanned filing with a searchable page number is still an unverified picture: the page number
/// extracts, but text on a page a picture dominates never certifies the picture's content. The page
/// is reported as uncertain (a warning naming the page), never certified as not searchable.
#[test]
fn a_scanned_page_with_a_searchable_page_number_is_uncertain() {
    let mut d = Document::new();
    // The "scan": a full-page picture with no text of its own.
    let media = d.add_media(png(), "png");
    let mut p = Paragraph::with_text("", CharProps::default());
    p.insert_object(
        0,
        InlineObject::Image { media, w: 300.0, h: 400.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None },
        &CharProps::default(),
    )
    .unwrap();
    d.body = vec![para_block(p)];
    // A page number in the footer: real, extractable text.
    let mut f = Paragraph::with_text("", CharProps::default());
    f.insert_object(
        0,
        InlineObject::Field { instr: "PAGE".into(), result: String::new(), locked: false, dirty: false, code: false, form: String::new() },
        &CharProps::default(),
    )
    .unwrap();
    let id = d.add_part(wordcraft_doc::PartKind::Footer, vec![para_block(f)]);
    d.last_section.footers.default = Some(id);

    let bytes = export(&d, &PdfOptions::default()).unwrap();
    assert!(squash(&extract_text(&bytes).concat()).contains('1'), "the page number is extractable");
    assert_eq!(text_searchability(&bytes), TextSearchability::Uncertain);
    assert_eq!(crate::extract::extract_verdict_detail(&bytes).unverifiable_pages, vec![1]);
}

/// A scan with no extractable text at all is an unverified picture, not a document confidently
/// without a text layer: the verdict preserves the page detail so it can be warned about.
#[test]
fn an_image_only_scan_is_uncertain_not_not_searchable() {
    let mut d = Document::new();
    let media = d.add_media(png(), "png");
    let mut scan = Paragraph::with_text("", CharProps::default());
    scan.insert_object(
        0,
        InlineObject::Image { media, w: 300.0, h: 400.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None },
        &CharProps::default(),
    )
    .unwrap();
    d.body = vec![para_block(scan)];
    let bytes = export(&d, &PdfOptions::default()).unwrap();
    assert_eq!(text_searchability(&bytes), TextSearchability::Uncertain);
    let detail = crate::extract::extract_verdict_detail(&bytes);
    assert_eq!(detail.verdict, crate::extract::Verdict::Uncertain);
    assert_eq!(detail.unverifiable_pages, vec![1]);
}

/// Extractable numeric body text is a text layer: a short document that is all digits is
/// searchable, just as alphabetic text is. (A number is only furniture when it is artifact-marked,
/// or when it sits on a page a picture dominates; untagged body digits are the document's text.)
#[test]
fn numeric_body_text_is_a_text_layer() {
    let mut d = Document::new();
    d.body = vec![para_block(Paragraph::with_text("123", CharProps::default()))];
    let bytes = export(&d, &PdfOptions { tagged: false, ..Default::default() }).unwrap();
    assert!(squash(&extract_text(&bytes).concat()).contains("123"));
    assert_eq!(text_searchability(&bytes), TextSearchability::Searchable);
    // A paragraph of alphabetic text certifies it too.
    d.body = vec![para_block(Paragraph::with_text("The quick brown fox jumps over the lazy dog.", CharProps::default()))];
    let bytes = export(&d, &PdfOptions { tagged: false, ..Default::default() }).unwrap();
    assert_eq!(text_searchability(&bytes), TextSearchability::Searchable);
}

/// A searchable cover followed by scanned (image-only) body pages is **not** text-searchable: one
/// page carrying text does not certify a whole PDF that has pages of its own with no text.
#[test]
fn a_searchable_cover_with_scanned_body_pages_is_not_searchable() {
    let mut d = Document::new();
    let media = d.add_media(png(), "png");
    let mut scan = Paragraph::with_text("", CharProps::default());
    scan.insert_object(
        0,
        InlineObject::Image { media, w: 300.0, h: 400.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None },
        &CharProps::default(),
    )
    .unwrap();
    scan.props.page_break_before = Some(true);
    d.body = vec![para_block(Paragraph::with_text("Cover page", CharProps::default())), para_block(scan)];
    let bytes = export(&d, &PdfOptions::default()).unwrap();
    let pages = extract_text(&bytes);
    assert_eq!(pages.len(), 2);
    assert!(squash(&pages[0]).contains("Cover page"));
    assert_eq!(text_searchability(&bytes), TextSearchability::Uncertain, "a scanned body page is unverifiable");
    let detail = crate::extract::extract_verdict_detail(&bytes);
    assert_eq!(detail.unverifiable_pages, vec![2]);
}

/// A scan with a caption beside it cannot be certified searchable: the caption extracts, but the
/// scanned writing it labels does not.
#[test]
fn a_scan_with_a_caption_is_uncertain() {
    let mut d = Document::new();
    let media = d.add_media(png(), "png");
    let mut scan = Paragraph::with_text("", CharProps::default());
    scan.insert_object(
        0,
        InlineObject::Image { media, w: 300.0, h: 400.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None },
        &CharProps::default(),
    )
    .unwrap();
    // The caption lands on the same page as the scan it labels.
    d.body = vec![para_block(scan), para_block(Paragraph::with_text("Exhibit A", CharProps::default()))];
    let bytes = export(&d, &PdfOptions::default()).unwrap();
    assert_eq!(extract_text(&bytes).len(), 1, "the scan and its caption share a page");
    assert!(squash(&extract_text(&bytes).concat()).contains("Exhibit A"), "the caption is extractable");
    assert_eq!(text_searchability(&bytes), TextSearchability::Uncertain, "a caption beside a scan must not certify the scan as searchable");
    assert_eq!(crate::extract::extract_verdict_detail(&bytes).unverifiable_pages, vec![1]);
}

/// Any caption beside a dominant scan must not pass as searchable: accompanying text never
/// certifies picture content.
#[test]
fn a_dominant_scan_with_a_long_caption_is_uncertain() {
    let mut d = Document::new();
    let media = d.add_media(png(), "png");
    let mut scan = Paragraph::with_text("", CharProps::default());
    scan.insert_object(
        0,
        InlineObject::Image { media, w: 300.0, h: 400.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None },
        &CharProps::default(),
    )
    .unwrap();
    let caption = "one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty twentyone twentytwo twentythree twentyfour twentyfive";
    d.body = vec![para_block(scan), para_block(Paragraph::with_text(caption, CharProps::default()))];
    let bytes = export(&d, &PdfOptions::default()).unwrap();
    assert_eq!(text_searchability(&bytes), TextSearchability::Uncertain);
    let detail = crate::extract::extract_verdict_detail(&bytes);
    assert_eq!(detail.verdict, crate::extract::Verdict::Uncertain);
    assert_eq!(detail.unverifiable_pages, vec![1]);
}

/// A page dominated by a picture with text that is neither clearly its content nor clearly a
/// caption cannot be verified: the PDF is reported as uncertain, not as searchable.
#[test]
fn a_dominant_picture_with_some_text_is_uncertain() {
    let mut d = Document::new();
    let media = d.add_media(png(), "png");
    let mut scan = Paragraph::with_text("", CharProps::default());
    scan.insert_object(
        0,
        InlineObject::Image { media, w: 300.0, h: 400.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None },
        &CharProps::default(),
    )
    .unwrap();
    // Ten words: more than a caption, less than a page's own text.
    d.body = vec![para_block(scan), para_block(Paragraph::with_text("one two three four five six seven eight nine ten", CharProps::default()))];
    let bytes = export(&d, &PdfOptions::default()).unwrap();
    assert_eq!(text_searchability(&bytes), TextSearchability::Uncertain);
    assert_eq!(text_searchability(&bytes).is_searchable(), None);
}

/// A small figure beside a caption is not a scan: the picture does not dominate the page, so the
/// caption is the page's own content and the PDF stays searchable.
#[test]
fn a_small_figure_with_a_caption_stays_searchable() {
    let mut d = Document::new();
    let media = d.add_media(png(), "png");
    let mut figure = Paragraph::with_text("Figure 1: ", CharProps::default());
    let n = figure.len();
    figure
        .insert_object(
            n,
            InlineObject::Image { media, w: 80.0, h: 40.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None },
            &CharProps::default(),
        )
        .unwrap();
    let n = figure.len();
    figure.insert_text(n, "gradient", &CharProps::default()).unwrap();
    d.body = vec![para_block(figure)];
    let bytes = export(&d, &PdfOptions::default()).unwrap();
    assert_eq!(text_searchability(&bytes), TextSearchability::Searchable);
}

/// There is no minimum document length: a fully extractable one-line document (far under any
/// earlier character threshold) is searchable.
#[test]
fn a_short_text_document_is_searchable() {
    let d = Document::from_text("Agreed.");
    let bytes = export(&d, &PdfOptions::default()).unwrap();
    assert_eq!(text_searchability(&bytes), TextSearchability::Searchable);
}

/// Blank and furniture-only pages do not defeat coverage: a text page with an inserted blank page
/// after it is still text-searchable.
#[test]
fn a_blank_page_does_not_defeat_searchability() {
    let mut d = Document::new();
    let mut blank = Paragraph::with_text("", CharProps::default());
    blank.props.page_break_before = Some(true);
    d.body = vec![para_block(Paragraph::with_text("The only real text.", CharProps::default())), para_block(blank)];
    let bytes = export(&d, &PdfOptions::default()).unwrap();
    assert_eq!(extract_text(&bytes).len(), 2, "the blank page is its own page");
    assert_eq!(text_searchability(&bytes), TextSearchability::Searchable);
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
    let wide = |s: &str| s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect::<Vec<u8>>();
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
            fill: Some(wordcraft_doc::props::Rgb(0xD9, 0xD9, 0xD9)),
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

/// #455: a link to a bookmark after a page break inside its paragraph goes to the page the
/// bookmark is on, not the paragraph's first page.
#[test]
fn bookmark_destination_is_the_page_of_its_start() {
    let mut d = Document::new();
    let mut p = Paragraph::with_text("Paragraph start\u{c}Bookmark target", CharProps::default());
    let at = "Paragraph start\u{c}".len();
    p.insert_object(at, InlineObject::BookmarkStart { name: "target".into() }, &CharProps::default()).unwrap();
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

/// Each glyph's Unicode text in content-stream order, with its baseline y and its outline's left
/// and right edges (none for a space); `None` text for a glyph without a ToUnicode entry of its
/// own (a cluster's extra glyph, covered by ActualText).
type StreamGlyph = (Option<String>, f64, Option<(f64, f64)>);
fn stream_glyphs(bytes: &[u8]) -> Vec<StreamGlyph> {
    use hayro_interpret::font::Glyph;
    use hayro_interpret::{
        BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask,
        interpret_page,
    };
    use hayro_syntax::Pdf;
    struct Units(Vec<StreamGlyph>);
    impl Device<'_> for Units {
        fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
        fn set_blend_mode(&mut self, _: BlendMode) {}
        fn draw_path(&mut self, _: &kurbo::BezPath, _: kurbo::Affine, _: &Paint<'_>, _: &PathDrawMode) {}
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
        fn draw_glyph(&mut self, g: &Glyph<'_>, t: kurbo::Affine, gt: kurbo::Affine, _: &Paint<'_>, mode: &GlyphDrawMode) {
            if matches!(mode, GlyphDrawMode::Stroke(_)) {
                return;
            }
            let text = match g.as_unicode() {
                Some(hayro_cmap::BfString::Char(c)) => Some(c.to_string()),
                Some(hayro_cmap::BfString::String(s)) => Some(s),
                None => None,
            };
            let edges = match g {
                Glyph::Outline(o) if !o.outline().elements().is_empty() => {
                    use kurbo::Shape;
                    let b = (t * gt * o.outline()).bounding_box();
                    Some((b.x0, b.x1))
                }
                _ => None,
            };
            self.0.push((text, (t * gt * kurbo::Point::ZERO).y, edges));
        }
        fn draw_image(&mut self, _: Image<'_, '_>, _: kurbo::Affine) {}
        fn pop_clip_path(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }
    let pdf = Pdf::new(bytes.to_vec()).expect("parse");
    let cache = InterpreterCache::new();
    let mut out = Units(Vec::new());
    for page in pdf.pages().iter() {
        let mut ctx = Context::new(kurbo::Affine::IDENTITY, kurbo::Rect::new(0.0, 0.0, 1.0, 1.0), &cache, pdf.xref(), InterpreterSettings::default());
        interpret_page(page, &mut ctx, &mut out);
    }
    out.0
}

/// Words as PDF viewers' text extraction (PDFium in Chrome and Edge, pdf.js, poppler) reads them:
/// glyphs are taken to be in visual order, so a word in a right-to-left script is read with its
/// glyphs reversed. Words end at spaces, at a jump back to the left, and where the
/// baseline moves (more than a raised mark does).
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
