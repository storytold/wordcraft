use super::*;
use crate::{LayoutCache, LayoutOptions, Placed, layout};
use wordcraft_doc::props::CharProps;

fn paragraph(text: &str, objects: Vec<InlineObject>) -> Paragraph {
    let mut p = Paragraph::with_text(text, CharProps::default());
    p.text = text.to_string();
    p.runs = vec![wordcraft_doc::Run { len: text.len(), props: CharProps::default() }];
    p.objects = objects;
    p
}

fn backgrounds(doc: &Document, p: &Paragraph, opts: &DisplayOptions) -> Vec<Draw> {
    let laid = layout(doc, &mut LayoutCache::new(), &LayoutOptions::default());
    let authors = Authors::new(doc);
    let mut out = Vec::new();
    for page in &laid.pages {
        for item in &page.items {
            let Placed::Lines { story, path, para: pl, l0, l1, x, y, .. } = item else { continue };
            for li in *l0..*l1 {
                let line = &pl.lines[li];
                let top = y + line.top - pl.lines[*l0].top;
                let ctx = LineContext {
                    doc,
                    story: *story,
                    path,
                    pl,
                    para: Some(p),
                    line,
                    li,
                    x: *x,
                    top,
                    base: top + line.baseline - line.top,
                    bottom: top + line.height,
                    opts,
                    authors: &authors,
                    alpha: 0.6,
                    down: false,
                    auto: None,
                };
                comment_backgrounds(&ctx, &mut out);
            }
        }
    }
    out
}

/// Preserve the existing first-open, orphan-end and unfinished-range behavior, including
/// mismatched IDs and missing/excess object storage, rather than imposing a pairing policy.
#[test]
fn comment_background_walk_preserves_marker_and_switch_behavior() {
    let start = |id| InlineObject::CommentStart { id };
    let end = |id| InlineObject::CommentEnd { id };
    let text = "é\u{fffc}one\u{fffc}two\u{fffc}three\u{fffc}four\u{fffc}tail";
    let p = paragraph(text, vec![end(9), start(1), start(2), end(7), start(3)]);
    let mut doc = Document::from_text(text);
    *doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap() = p.clone();
    let opts = DisplayOptions::default();
    let actual = backgrounds(&doc, &p, &opts);
    let without_inner = paragraph(text, vec![end(9), start(1), InlineObject::BookmarkStart { name: "unrelated".into() }, end(7), start(3)]);
    assert_eq!(format!("{actual:?}"), format!("{:?}", backgrounds(&doc, &without_inner, &opts)));
    assert_eq!(actual.len(), 3, "orphan end, first open and trailing open each paint");
    assert!(actual.iter().all(|d| matches!(d, Draw::Fill { color: Rgb(0xEF, 0xE3, 0xF7), alpha: 0.6, text_bg: false, rect } if rect.w > 0.0)));
    let mut off = opts.clone();
    off.markup = false;
    assert!(backgrounds(&doc, &p, &off).is_empty());
    off.markup = true;
    off.revisions.comments = false;
    assert!(backgrounds(&doc, &p, &off).is_empty());
    let mut extra = p.clone();
    extra.objects.push(end(3));
    assert_eq!(format!("{actual:?}"), format!("{:?}", backgrounds(&doc, &extra, &opts)));
    let mut missing = p.clone();
    missing.objects.truncate(4);
    assert_eq!(backgrounds(&doc, &missing, &opts).len(), 2);
}

/// Rendering comment backgrounds must not recount the paragraph prefix for every unrelated
/// object. Time the actual paint function separately from shaping and page layout.
#[test]
fn comment_background_walk_scales_with_unrelated_markers() {
    fn dense(n: usize) -> Paragraph {
        paragraph(&"\u{fffc}".repeat(n), vec![InlineObject::BookmarkEnd { name: "witness".into() }; n])
    }
    let doc = Document::from_text("Witness");
    let laid = layout(&doc, &mut LayoutCache::new(), &LayoutOptions::default());
    let Placed::Lines { story, path, para: pl, x, y, .. } = &laid.pages[0].items[0] else { panic!("text line") };
    let authors = Authors::new(&doc);
    let opts = DisplayOptions::default();
    let line = &pl.lines[0];
    let paint = |p: &Paragraph| {
        let ctx = LineContext {
            doc: &doc,
            story: *story,
            path,
            pl,
            para: Some(p),
            line,
            li: 0,
            x: *x,
            top: *y,
            base: *y + line.baseline - line.top,
            bottom: *y + line.height,
            opts: &opts,
            authors: &authors,
            alpha: 1.0,
            down: false,
            auto: None,
        };
        let mut out = Vec::new();
        comment_backgrounds(&ctx, &mut out);
        assert!(std::hint::black_box(out).is_empty());
    };
    let (small, big) = (dense(2_000), dense(16_000));
    let cost = |f: &mut dyn FnMut()| {
        let cpu = || std::fs::read_to_string("/proc/thread-self/schedstat").ok()?.split_whitespace().next()?.parse::<u64>().ok();
        let before = cpu();
        let start = std::time::Instant::now();
        // A short /proc sample can expose only part of a scheduler tick. Batch enough
        // work to measure the traversal instead of dividing by an undercounted tiny sample.
        let mut samples = 0u32;
        loop {
            f();
            samples += 1;
            if start.elapsed() >= std::time::Duration::from_millis(100) {
                break;
            }
        }
        let elapsed = match (before, cpu()) {
            (Some(a), Some(b)) if b > a => (b - a) as f64 * 1e-9,
            _ => start.elapsed().as_secs_f64(),
        };
        elapsed / f64::from(samples)
    };
    paint(&small);
    paint(&big);
    let (mut a, mut b) = (f64::INFINITY, f64::INFINITY);
    for _ in 0..5 {
        a = a.min(cost(&mut || (0..8).for_each(|_| paint(&small))) / 8.0);
        b = b.min(cost(&mut || paint(&big)));
    }
    let ratio = b / a.max(1e-9);
    eprintln!("comment background CPU {a:.9}s -> {b:.9}s, {ratio:.2}x for 8x markers");
    assert!(ratio < 24.0, "comment background discovery rescans prefixes: {ratio:.2}x");
}
