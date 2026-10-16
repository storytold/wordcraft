//! SVG pictures (#1910) draw at device resolution: sharp at any zoom.

use super::*;
use wordcraft_layout::{LayoutCache, LayoutOptions, layout};

/// A 10 × 10 px SVG: red left half, white right half.
const HALVES: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 10 10"><rect width="5" height="10" fill="#ff0000"/><rect x="5" width="5" height="10" fill="#ffffff"/></svg>"##;

fn doc_with(svg: &[u8]) -> (Document, wordcraft_layout::DocLayout, wordcraft_geom::Rect) {
    let mut d = Document::new();
    let media = d.add_media(svg.to_vec(), "svg");
    let mut p = wordcraft_doc::Paragraph::new();
    let obj = wordcraft_doc::para::InlineObject::Image {
        media,
        w: 70.0,
        h: 70.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        ole: None,
    };
    p.insert_object(0, obj, &Default::default()).unwrap();
    d.body = vec![wordcraft_doc::para_block(p)];
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let rect = page_display(&d, &l.pages[0], &RenderOptions::default().display)
        .iter()
        .find_map(|i| match i {
            Draw::Image { rect, .. } => Some(*rect),
            _ => None,
        })
        .expect("a picture on the page");
    (d, l, rect)
}

#[test]
fn svg_picture_is_drawn_not_the_grey_box() {
    let (d, l, rect) = doc_with(HALVES);
    let img = render_page(&d, &l.pages[0], 1.0, &RenderOptions::default());
    let y = (rect.y + rect.h / 2.0) as u32;
    let left = img.pixel((rect.x + rect.w * 0.25) as u32, y);
    let right = img.pixel((rect.x + rect.w * 0.75) as u32, y);
    assert!(left[0] > 200 && left[1] < 60 && left[2] < 60, "left half should be red: {left:?}");
    assert!(right[..3].iter().all(|c| *c > 240), "right half should be white, not the grey box: {right:?}");
}

#[test]
fn svg_picture_is_sharp_when_zoomed() {
    // 10 px drawn 70 pt wide, at 4×: 280 device pixels. Scaling a 10 px raster up would blur the
    // edge over many pixels; drawn at device resolution it stays within a pixel or two.
    let (d, l, rect) = doc_with(HALVES);
    let scale = 4.0;
    let img = render_page(&d, &l.pages[0], scale, &RenderOptions::default());
    let edge = ((rect.x + rect.w / 2.0) * scale).round() as u32;
    let y = ((rect.y + rect.h / 2.0) * scale) as u32;
    let near_left = img.pixel(edge - 2, y);
    let near_right = img.pixel(edge + 2, y);
    assert!(near_left[0] > 240 && near_left[1] < 20, "2 px left of the edge is red: {near_left:?}");
    assert!(near_right[..3].iter().all(|c| *c > 235), "2 px right of the edge is white: {near_right:?}");
}

#[test]
fn unreadable_svg_keeps_the_grey_box() {
    let (d, l, rect) = doc_with(b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\"");
    let img = render_page(&d, &l.pages[0], 1.0, &RenderOptions::default());
    assert_eq!(img.pixel((rect.x + rect.w / 2.0) as u32, (rect.y + rect.h / 2.0) as u32), [0xD0, 0xD0, 0xD0, 255]);
}
