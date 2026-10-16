//! Colour glyphs (#1269): emoji from colour fonts (bitmap strikes, COLR) are drawn in colour.

use super::*;
use wordcraft_layout::{LayoutCache, LayoutOptions, layout};

/// Colour emoji families the system fallback may pick.
const COLOR_EMOJI: &[&str] = &["Noto Color Emoji", "Apple Color Emoji", "Segoe UI Emoji"];

/// A strongly coloured (non-grey) pixel.
fn colourful(p: [u8; 4]) -> bool {
    let (hi, lo) = (p[0].max(p[1]).max(p[2]), p[0].min(p[1]).min(p[2]));
    hi - lo > 60
}

/// System-dependent: skips (passes) when no colour emoji font is installed. Each emoji glyph's box
/// on the rendered page holds coloured pixels.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn emoji_are_drawn_in_colour() {
    let Some((img, boxes)) = emoji_page(&RenderOptions::default()) else { return };
    for (x0, y0, x1, y1) in boxes {
        let n = (x0..x1).flat_map(|x| (y0..y1).map(move |y| (x, y))).filter(|(x, y)| colourful(img.pixel(*x, *y))).count();
        assert!(n > 20, "only {n} coloured pixels in emoji box {x0},{y0}..{x1},{y1}");
    }
}

/// A dark page (View › Switch Modes) doesn't invert the emoji: they keep the font's colours, the
/// same as pictures. The pixels well inside each emoji (coloured with coloured neighbours, so the
/// paper doesn't show through) match the light page's. Skips like [`emoji_are_drawn_in_colour`].
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn emoji_keep_their_colours_on_a_dark_page() {
    let Some((light, boxes)) = emoji_page(&RenderOptions::default()) else { return };
    let Some((dark, _)) = emoji_page(&RenderOptions { dark: true, ..Default::default() }) else { return };
    for (x0, y0, x1, y1) in boxes {
        let inside = |x: u32, y: u32| (x - 1..=x + 1).all(|i| (y - 1..=y + 1).all(|j| colourful(light.pixel(i, j))));
        let mut n = 0;
        for (x, y) in (x0 + 1..x1.saturating_sub(1)).flat_map(|x| (y0 + 1..y1.saturating_sub(1)).map(move |y| (x, y))) {
            if !inside(x, y) {
                continue;
            }
            let (l, d) = (light.pixel(x, y), dark.pixel(x, y));
            assert!(l.iter().zip(d).all(|(a, b)| a.abs_diff(b) <= 2), "emoji pixel {x},{y} changed on a dark page: {l:?} → {d:?}");
            n += 1;
        }
        assert!(n > 10, "only {n} pixels well inside emoji box {x0},{y0}..{x1},{y1}");
    }
}

/// Two emoji rendered at 2x with `opts`, and each emoji glyph's box in
/// pixels; None (the test skips) when no colour emoji font is installed.
#[cfg(not(target_arch = "wasm32"))]
fn emoji_page(opts: &RenderOptions) -> Option<(Rendered, Vec<(u32, u32, u32, u32)>)> {
    let Some(face) = FontDb::global().fallback_for('🧠', u32::MAX).filter(|f| COLOR_EMOJI.contains(&f.family.as_str())) else {
        eprintln!("skipped: no colour emoji font installed");
        return None;
    };
    // Request the colour face so Windows cannot choose a monochrome symbol fallback for ✅.
    let mut d = Document::new();
    d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(
        "🧠✅",
        wordcraft_doc::CharProps { font: Some(face.family.clone()), ..Default::default() },
    ))];
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let page = &l.pages[0];
    let scale = 2.0;
    let img = render_page(&d, page, scale, opts);
    let mut boxes = vec![];
    for it in page_display(&d, page, &DisplayOptions::default()) {
        if let Draw::Glyphs { face, size, glyphs, .. } = it
            && COLOR_EMOJI.contains(&face.family.as_str())
        {
            for (_, x, y) in glyphs {
                boxes.push((((x * scale) as u32), ((y - size) * scale) as u32, ((x + size) * scale) as u32, (y * scale) as u32));
            }
        }
    }
    assert_eq!(boxes.len(), 2, "both emoji use the colour emoji font");
    Some((img, boxes))
}
