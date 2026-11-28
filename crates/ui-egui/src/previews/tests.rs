use super::*;

/// #817: equation pictures were black on white; they are now an ink mask (white, transparent paper).
#[test]
fn math_pictures_are_ink_masks() {
    use wordcraft_doc::math::parse_linear;
    let img = math_image(&parse_linear("x/y"), false, 12.0, 40.0, 34.0, 1.0).expect("renders");
    assert!(img.pixels.iter().any(|p| p.a() == 0), "paper is transparent");
    assert!(img.pixels.iter().any(|p| p.a() > 0), "ink is kept");
    assert!(img.pixels.iter().all(|p| *p == egui::Color32::from_white_alpha(p.a())), "ink is white, to be tinted");
}

#[test]
fn ink_mask_keeps_the_ink_and_drops_the_paper() {
    let mut img = egui::ColorImage::new([3, 1], vec![egui::Color32::WHITE, egui::Color32::BLACK, egui::Color32::from_gray(128)]);
    ink_mask(&mut img);
    assert_eq!(img.pixels[0].a(), 0, "paper becomes transparent");
    assert_eq!(img.pixels[1], egui::Color32::from_white_alpha(255), "full ink stays opaque");
    assert_eq!(img.pixels[2].a(), 127, "antialiased ink keeps its coverage");
}

/// #5957: a style drawn in a fallback face before its font was installed is drawn again once a font is
/// added: the style revision follows the font generation.
#[test]
fn the_style_revision_follows_the_font_generation() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), Default::default());
    let before = style_revision(&mut app, Mode::Light);
    assert_eq!(before, style_revision(&mut app, Mode::Light), "the same inputs keep the pictures");
    // Source Sans 3 renamed (same length), so the font is new to the database.
    let mut data = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/SourceSans3-Regular.ttf")).unwrap();
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    for (from, to) in [(utf16("Source Sans 3"), utf16("Revprobe Sans")), (b"Source Sans 3".to_vec(), b"Revprobe Sans".to_vec())] {
        let mut i = 0;
        while let Some(at) = data[i..].windows(from.len()).position(|w| w == from) {
            data[i + at..i + at + to.len()].copy_from_slice(&to);
            i += at + to.len();
        }
    }
    // `add_font` adds faces quietly; `add_document_fonts` is the path that bumps the generation, as a document's
    // embedded fonts do.
    assert!(wordcraft_fonts::FontDb::global().add_document_fonts(vec![(data, "Revprobe Sans".to_string())]) > 0, "the font is added");
    assert_ne!(before, style_revision(&mut app, Mode::Light), "a new font draws the previews again");
}

#[test]
fn font_sample_shows_small_caps_and_the_script_mark() {
    let white = wordcraft_doc::Rgb(255, 255, 255);
    let plain = CharProps { size: Some(20.0), ..Default::default() };
    let draw = |p: &CharProps, mark, dark| font_sample("AaBbYyZz", p, mark, (200.0, 40.0, 1.0), white, dark).map(|i| i.pixels);
    let small = CharProps { small_caps: Some(true), ..plain.clone() };
    assert_ne!(draw(&plain, None, false), draw(&small, None, false), "small caps change the sample");
    let sup = Some(wordcraft_doc::props::VertAlign::Superscript);
    assert_ne!(draw(&plain, None, false), draw(&plain, sup, false), "the mark is drawn");
    assert_ne!(draw(&plain, None, false), draw(&plain, None, true), "Switch Modes transforms the preview");
}
