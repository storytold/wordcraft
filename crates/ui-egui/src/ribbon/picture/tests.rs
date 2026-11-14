use super::*;
use image::{Rgba, RgbaImage};
use wordcraft_doc::para::{InlineObject, OBJ};
use wordcraft_doc::{Document, Pos};

fn png(color: [u8; 4]) -> Vec<u8> {
    let mut bytes = Vec::new();
    RgbaImage::from_pixel(8, 8, Rgba(color)).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    bytes
}

fn app_with_picture(bytes: Vec<u8>) -> WordApp {
    let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
    app.session.doc.media.insert("m".into(), Arc::new(bytes));
    let picture = InlineObject::Image {
        media: "m".into(),
        w: 72.0,
        h: 72.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    };
    app.session.doc.insert_object(&Pos::body(0, 0), picture, &Default::default()).unwrap();
    app.run("select.range", json!({"anchor": Pos::body(0, 0), "focus": Pos::body(0, OBJ.len_utf8())})).unwrap();
    app
}

/// Draw the real gallery and read the texture it put on the tile.
fn gallery_texture(app: &mut WordApp, ctx: &egui::Context, items: &[Item]) -> egui::TextureId {
    let output = ctx.run_ui(egui::RawInput::default(), |ui| gallery(ui, app, "test", items, 1));
    let texture = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Mesh(mesh) => Some(mesh.texture_id),
        _ => None,
    });
    output.drop_without_applying_deltas();
    texture.expect("the gallery paints its picture texture")
}

#[test]
fn compression_preserves_the_resolution_of_the_visible_crop() {
    assert_eq!(compression_pixels(72.0, 72.0, [0.0; 4], 330.0), 330);
    assert_eq!(compression_pixels(72.0, 72.0, [0.45; 4], 330.0), 3300, "one inch showing a tenth of the source needs 3300 source pixels");
    assert_eq!(compression_pixels(72.0, 144.0, [0.45, 0.0, 0.45, 0.0], 330.0), 3300, "the source's longer axis can differ from the display's");
    assert_eq!(compression_pixels(144.0, 72.0, [0.0, 0.45, 0.0, 0.45], 220.0), 2200);
    assert_eq!(compression_pixels(1.0, 1.0, [0.0; 4], 96.0), 32);
    assert_eq!(compression_pixels(4000.0, 4000.0, [0.95; 4], 330.0), 20000);
}

#[test]
fn gallery_sources_and_tiles_refresh_when_equal_length_media_reuses_a_key() {
    let (mut first, mut replacement) = (png([200, 50, 10, 255]), png([10, 100, 220, 255]));
    let len = first.len().max(replacement.len());
    first.resize(len, 0);
    replacement.resize(len, 0);
    let mut app = app_with_picture(first);
    let ctx = egui::Context::default();
    let items = [Item::new("None", "picture.color", json!({"recolor": "none"}))];
    let old_source = tile_source(&app, &ctx, "m").unwrap();
    let old_texture = gallery_texture(&mut app, &ctx, &items);
    assert_eq!(gallery_texture(&mut app, &ctx, &items), old_texture, "an unchanged picture reuses its tiles");

    app.session.doc.media.insert("m".into(), Arc::new(replacement));
    let new_source = tile_source(&app, &ctx, "m").unwrap();
    assert_eq!(new_source.get_pixel(0, 0).0, [10, 100, 220, 255]);
    assert_ne!(new_source, old_source);
    assert_ne!(gallery_texture(&mut app, &ctx, &items), old_texture);
}

#[test]
fn gallery_tiles_refresh_when_the_theme_changes_item_parameters() {
    let mut app = app_with_picture(png([100, 100, 100, 255]));
    let ctx = egui::Context::default();
    let mut items = [Item::new("Accent 1, Dark", "picture.color", json!({"recolor": "dark", "color": "0000FF"}))];
    let blue = gallery_texture(&mut app, &ctx, &items);
    items[0].params["color"] = json!("00FF00");
    let green = gallery_texture(&mut app, &ctx, &items);
    assert_ne!(green, blue, "changed theme parameters must regenerate the tile");
    assert_eq!(gallery_texture(&mut app, &ctx, &items), green);
}
