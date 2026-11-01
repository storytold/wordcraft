//! Decoded-media cache: hits need the same allocation, and the byte budget bounds what is kept.

use std::collections::HashMap;
use std::sync::Arc;

use wordcraft_doc::Document;
use wordcraft_doc::props::Rgb;
use wordcraft_layout::display::{Draw, Stroke};

use super::{CACHE_BYTES, MediaCache, RenderOptions, render_items, wave_path};

#[test]
fn media_cache_only_hits_the_same_allocation() {
    let mut c: MediaCache<u32> = MediaCache { map: HashMap::new(), bytes: 0 };
    let a = Arc::new(vec![1u8, 2, 3]);
    let b = Arc::new(vec![1u8, 2, 3]);
    c.insert("pic", &a, 7, 3);
    assert_eq!(c.get("pic", &a), Some(7));
    assert_eq!(c.get("pic", &b), None, "equal bytes in another allocation are another picture");
    drop(a);
    assert_eq!(c.get("pic", &Arc::new(vec![1u8, 2, 3])), None, "a freed source's address isn't reused");
}

#[test]
fn media_cache_is_bounded_by_bytes() {
    let mut c: MediaCache<u32> = MediaCache { map: HashMap::new(), bytes: 0 };
    let src = Arc::new(vec![0u8]);
    let big = CACHE_BYTES / 2 + 1;
    c.insert("a", &src, 1, big);
    c.insert("b", &src, 2, big);
    assert_eq!(c.get("a", &src), None, "the first entry is evicted to make room");
    assert_eq!(c.get("b", &src), Some(2));
    assert!(c.bytes <= CACHE_BYTES);
    c.insert("huge", &src, 3, CACHE_BYTES + 1);
    assert_eq!(c.get("huge", &src), None, "a value over the whole budget is not kept");
}

#[test]
fn media_cache_counts_what_it_keeps_and_lets_sources_go() {
    let mut c: MediaCache<Option<u32>> = MediaCache { map: HashMap::new(), bytes: 0 };
    let src = Arc::new(vec![0u8; 1000]);
    c.insert("broken", &src, None, 0);
    c.insert("pic", &src, Some(1), 24);
    assert_eq!(c.bytes, 24, "an entry counts what it decoded to");
    assert_eq!(Arc::strong_count(&src), 1, "and doesn't keep its source alive");
    // A source over the whole budget is cached by what it decoded to (calloc'd, so cheap), so a
    // broken one isn't decoded again on every render either.
    let huge = Arc::new(vec![0u8; CACHE_BYTES + 1]);
    c.insert("huge", &huge, None, 0);
    assert_eq!(c.get("huge", &huge), Some(None));
    assert_eq!(CACHE_BYTES * 2, 256 * 1024 * 1024, "the image and vector caches share about 256 MB");
}

/// A wave stroke starting around 1e20 never advanced (`x + step == x`), pushing curves until memory ran out.
#[test]
fn wave_stroke_at_a_huge_coordinate_ends() {
    assert!(wave_path(1e20, 0.0, 2e20, 0.0, 1.0).elements().len() < 3);
    assert!(wave_path(0.0, 0.0, 1e20, 0.0, 1.0).elements().len() < 20_100);
    assert!(wave_path(0.0, 0.0, 100.0, 0.0, 1.0).elements().len() > 10);
    assert!(wave_path(0.0, 0.0, 0.0, 100.0, 1.0).elements().len() > 10, "vertical wave strokes produce elements");
}

/// #1740: a triple stroke is three lines across its direction, horizontal or vertical.
#[test]
fn triple_strokes_draw_three_lines() {
    let line = |x0: f32, y0: f32, x1: f32, y1: f32, stroke: Stroke| Draw::Line {
        x0,
        y0,
        x1,
        y1,
        width: 1.0,
        color: Rgb::BLACK,
        stroke,
        alpha: 1.0,
        text_bg: None,
    };
    // Dark runs met walking across the line at its middle.
    let runs = |it: Draw, vertical: bool| {
        let r = render_items(&Document::new(), &[it], wordcraft_geom::Rect::new(0.0, 0.0, 20.0, 20.0), 8.0, &RenderOptions::default());
        let mut n = 0;
        let mut inside = false;
        for i in 0..r.width.min(r.height) {
            let px = if vertical { r.pixel(i, 80) } else { r.pixel(80, i) };
            let dark = px[3] > 128;
            n += usize::from(dark && !inside);
            inside = dark;
        }
        n
    };
    assert_eq!(runs(line(2.0, 10.0, 18.0, 10.0, Stroke::Triple), false), 3, "horizontal triple");
    assert_eq!(runs(line(10.0, 2.0, 10.0, 18.0, Stroke::Triple), true), 3, "vertical triple");
    assert_eq!(runs(line(2.0, 10.0, 18.0, 10.0, Stroke::Double), false), 2, "horizontal double");
}
