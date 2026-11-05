//! Design › Watermark on screen.

use super::*;

/// #2954: a text watermark at 80 % transparency is lighter than at 0 %, and invisible at 100 %.
#[test]
fn text_watermark_transparency() {
    let mut d = Document::new();
    let darkest = |d: &Document| {
        let l = layout(d, &mut LayoutCache::new(), &LayoutOptions::default());
        let img = render_page(d, &l.pages[0], 1.0, &RenderOptions::default());
        img.pixels.chunks(4).map(|p| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])).min().unwrap()
    };
    let mut wm = wordcraft_doc::Watermark { text: "WWW".into(), color: wordcraft_doc::props::Rgb::BLACK, diagonal: false, ..Default::default() };
    let mut at = |t: u8| {
        wm.set_transparency(t);
        d.settings.watermark = Some(wm.clone());
        darkest(&d)
    };
    let (opaque, half, light, gone) = (at(0), at(50), at(80), at(100));
    assert!(opaque < 30, "{opaque}");
    assert!(half > opaque + 300 && light > half + 150, "{opaque} {half} {light}");
    assert_eq!(gone, 765);
}
