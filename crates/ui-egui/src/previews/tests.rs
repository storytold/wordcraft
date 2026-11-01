use super::*;

/// #848: each trip through the Font box loaded every family it showed, for good. Entries are
/// now drawn with a face read for the preview alone: installed families stay unloaded.
#[test]
fn font_menu_entries_leave_installed_families_unloaded() {
    let db = wordcraft_fonts::FontDb::global();
    let ink = |img: &egui::ColorImage| img.pixels.iter().filter(|p| p.r() < 128).count();
    assert!(ink(&font_entry_image("Source Sans 3", 1.0)) > 20, "the bundled face draws");
    assert!(ink(&font_entry_image("No Such Font 848", 1.0)) > 20, "an unknown font shows in the default face");
    // Tests running alongside share this database and may load a fallback (a math font for
    // an equation, a font for a script) at any time: leave the math fonts out, and allow a
    // stray load or two. Before #848 every family previewed was loaded.
    let shared = |f: &str| wordcraft_fonts::math::MATH_SUBSTITUTES.iter().any(|n| f.eq_ignore_ascii_case(n));
    let unloaded: Vec<String> = db.families().into_iter().filter(|f| !db.is_loaded(f) && !shared(f)).take(25).collect();
    let mut loaded = Vec::new();
    for f in &unloaded {
        let img = font_entry_image(f, 1.0);
        assert_eq!(img.size, [244, 22]);
        if db.is_loaded(f) {
            loaded.push(f.clone());
        }
    }
    assert!(loaded.len() <= 2, "previewing loaded {loaded:?}");
}

/// A window whose Styles gallery lists `style` first: runs the gallery for a few frames and
/// returns the painted shapes of the last one and every texture picture uploaded on the way.
fn run_gallery(mut doc: Document, style: wordcraft_doc::styles::Style) -> (Vec<egui::epaint::ClippedShape>, Vec<egui::ColorImage>) {
    // Only the style under test is in the gallery.
    for other in &mut doc.styles.styles {
        other.quick = false;
    }
    doc.styles.upsert(style);
    let mut app = WordApp::new(wordcraft_engine::Session::new(doc), Default::default());
    app.ui.language = "en".into();
    let ctx = egui::Context::default();
    let mut shapes = Vec::new();
    let mut images = Vec::new();
    for frame in 0..6 {
        let input = egui::RawInput {
            time: Some(f64::from(frame) * 0.1),
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 200.0))),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            ui.add_space(20.0);
            style_gallery(&mut app, ui, &json!({}));
        });
        for (_, deltas) in out.textures_delta.set.drain() {
            for delta in deltas {
                if let (None, egui::ImageData::Color(img)) = (delta.pos, &delta.image) {
                    images.push(egui::ColorImage::clone(img));
                }
            }
        }
        out.textures_delta.clear();
        shapes = out.shapes;
    }
    (shapes, images)
}

fn gallery_style(name: &str) -> wordcraft_doc::styles::Style {
    wordcraft_doc::styles::Style { id: "Backfill".into(), name: name.into(), quick: true, priority: Some(0), ..Default::default() }
}

/// Regression test for #2: a style gallery label longer than its tile was painted at full width
/// and ran into the neighbouring tiles; it is now cut with an ellipsis to fit the tile.
#[test]
fn a_long_style_name_is_truncated_to_its_tile() {
    let name = "An extraordinarily long custom style name that cannot fit a tile";
    let (shapes, _) = run_gallery(Document::new(), gallery_style(name));
    let label = shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.job.text == name => Some(t),
            _ => None,
        })
        .expect("the long style's label is painted");
    // A tile is 74 points wide.
    assert!(label.galley.size().x <= 74.0, "the label is {} points wide, wider than its 74-point tile", label.galley.size().x);
    assert!(label.galley.elided, "the label is cut short with an ellipsis");
}

/// Regression test for #2: a style whose hanging indent is larger than its left indent previewed
/// with its first line starting off the tile's left edge; the first line now starts at the edge.
#[test]
fn a_hanging_indent_style_previews_from_the_tile_edge() {
    let mut style = gallery_style("Hanging");
    style.para.indent_left = Some(0.0);
    style.para.indent_first = Some(-36.0);
    let (_, images) = run_gallery(Document::new(), style);
    // The style is the only one in the gallery: its preview is the picture of a tile's size (the other upload is egui's font texture).
    let preview = images.iter().find(|i| (200..300).contains(&i.size[0]) && (20..60).contains(&i.size[1])).expect("the preview is rendered");
    let [w, h] = preview.size;
    let inked: Vec<bool> = (0..w).map(|x| (0..h).any(|y| preview.pixels.get(y * w + x).is_some_and(|p| p.a() > 0 && p.r() < 160))).collect();
    let first = inked.iter().position(|&i| i).expect("the preview has text");
    let last = inked.iter().rposition(|&i| i).unwrap_or(first);
    // "AaBbCcDd" begins at the left margin (1 point) and is whole: it is about 60 points wide.
    assert!(first < 6, "the first line starts off the tile's left edge (ink from column {first})");
    assert!(last - first > 40, "the first line is cut off by the tile's left edge (ink {first}..{last})");
}

/// #663: gallery previews are dimmed in the dark theme, unless they show Switch Modes' dark
/// page; the light theme keeps a white page.
#[test]
fn previews_are_dimmed_in_the_dark_theme() {
    let (light, dark) = (crate::theme::Tokens::light(), crate::theme::Tokens::dark());
    assert_eq!(super::preview_tint(&light, false), egui::Color32::WHITE);
    assert_eq!(super::preview_tint(&dark, true), egui::Color32::WHITE, "the dark page is already dark");
    let dim = super::preview_tint(&dark, false);
    assert!(dim.r() < 0xE0 && dim.r() > 0xA0, "{dim:?}");
}

/// Font-menu previews defer over budget, render on a later frame, and reuse cached entries
/// without consuming budget or deadlocking.
#[test]
fn font_menu_previews_render_defer_and_reuse_without_deadlocking() {
    // A nested Context lock used to hang the first uncached menu entry. Bound the wait so
    // a regression fails the test instead of hanging the entire test suite.
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let ctx = egui::Context::default();
        let preview = Previews::default().font_preview_fn();
        let budget_id = egui::Id::unique("font_preview_budget");
        let sans_id = egui::Id::unique("font:Source Sans 3");
        let inter_id = egui::Id::unique("font:Inter");
        ctx.data_mut(|d| d.insert_temp(budget_id, (1.0_f64, 11_u32)));

        for time in [1.0, 2.0, 3.0] {
            let input = egui::RawInput { time: Some(time), ..Default::default() };
            ctx.run_ui(input, |ui| {
                preview(ui, "Source Sans 3");
                preview(ui, "Inter");
            })
            .drop_without_applying_deltas();
            ctx.data(|d| {
                assert!(d.get_temp::<TextureHandle>(sans_id).is_some());
                if time == 1.0 {
                    assert!(d.get_temp::<TextureHandle>(inter_id).is_none(), "over-budget entry should defer");
                    assert_eq!(d.get_temp::<(f64, u32)>(budget_id), Some((1.0, 13)));
                } else {
                    assert!(d.get_temp::<TextureHandle>(inter_id).is_some(), "deferred entry should render next frame");
                    assert_eq!(d.get_temp::<(f64, u32)>(budget_id), Some((2.0, 1)), "cached entries should not consume budget");
                }
            });
        }
        tx.send(()).unwrap();
    });
    rx.recv_timeout(std::time::Duration::from_secs(30)).expect("font menu preview rendering hung or failed");
    worker.join().unwrap();
}

/// #817: equation pictures were black on white; they are now an ink mask (white, transparent paper).
#[test]
fn math_pictures_are_ink_masks() {
    use wordcraft_doc::math::parse_linear;
    let img = math_image(&parse_linear("x/y"), false, 12.0, 40.0, 34.0, 1.0).expect("renders");
    assert!(img.pixels.iter().any(|p| p.a() == 0), "paper is transparent");
    assert!(img.pixels.iter().any(|p| p.a() > 0), "ink is kept");
    assert!(img.pixels.iter().all(|p| *p == egui::Color32::from_white_alpha(p.a())), "ink is white, to be tinted");
}

/// Every picture the Equation tab and Insert › Equation draw: (key, nodes, display, pt, tile size).
fn equation_pictures() -> Vec<(String, wordcraft_doc::math::Arg, bool, f32, egui::Vec2)> {
    use wordcraft_doc::math::{MNode, MRun, parse_linear};
    use wordcraft_engine::math_gallery::{BUILT_INS, STRUCTURES, SYMBOL_SETS};
    let mut out = Vec::new();
    for g in STRUCTURES {
        let pic = crate::equation_tab::button_picture(g.id);
        out.push((format!("btn:{pic}"), parse_linear(pic), false, 12.0, vec2(40.0, 32.0)));
        for (_, templates) in g.sections {
            for t in templates.iter() {
                out.push((format!("tpl:{}", t.id), t.nodes(), true, 14.0, vec2(56.0, 52.0)));
                out.push((format!("tpl:{}", t.id), t.nodes(), true, 13.0, vec2(154.0, 56.0)));
            }
        }
    }
    for (id, _, lin) in BUILT_INS {
        out.push((format!("builtin:{id}"), parse_linear(lin), true, 11.0, vec2(346.0, 60.0)));
    }
    for (_, chars) in SYMBOL_SETS {
        for c in chars.chars() {
            let nodes = vec![MNode::Run(MRun { text: c.to_string(), lit: true, ..Default::default() })];
            out.push((format!("sym:{c}"), nodes, false, 11.0, vec2(22.0, 22.0)));
        }
    }
    out
}

/// #1002: drawn the way the Equation tab draws them (the picture tinted with the theme's text
/// colour on an input-coloured tile), every gallery tile, button and symbol is the theme's
/// greys only: its background is the tile colour and its ink no darker or lighter than the
/// text, in light and dark mode, with no red or teal anywhere.
#[test]
fn equation_pictures_follow_the_theme() {
    let mix = |fg: u8, bg: u8, a: u8| ((u32::from(fg) * u32::from(a) + u32::from(bg) * (255 - u32::from(a))) / 255) as u8;
    for (key, nodes, display, pt, size) in equation_pictures() {
        let img = math_image(&nodes, display, pt, size.x, size.y, 1.0).unwrap_or_else(|| panic!("{key} renders"));
        assert!(img.pixels.iter().all(|p| *p == egui::Color32::from_white_alpha(p.a())), "{key}: an ink mask");
        // Ink may reach the corner (a matrix bracket with Windows fonts): look for paper anywhere.
        assert!(img.pixels.iter().any(|p| p.a() == 0), "{key}: transparent paper");
        for t in [Tokens::light(), Tokens::dark()] {
            let (bg, fg) = (t.input, t.text);
            let (lo, hi) = (bg.r().min(fg.r()), bg.r().max(fg.r()));
            for p in &img.pixels {
                let a = p.a();
                let c = [mix(fg.r(), bg.r(), a), mix(fg.g(), bg.g(), a), mix(fg.b(), bg.b(), a)];
                assert!(c[0] == c[1] && c[1] == c[2], "{key}: a grey, not {c:?}");
                assert!((lo..=hi).contains(&c[0]), "{key}: between the tile and the text colour, not {c:?}");
            }
        }
    }
}

/// #1002: the pictures share one texture, written in place: browsing every gallery creates no
/// texture per tile and frees none (hundreds of small textures, freed in bulk, showed other
/// textures' memory on DirectX 12).
#[test]
fn equation_pictures_share_one_texture() {
    let ctx = egui::Context::default();
    let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
    let pictures = equation_pictures();
    // Symbols repeat across sets (∀, ≠…): each is drawn once.
    let unique = pictures.iter().map(|(key, _, _, pt, size)| format!("{key}:{pt}:{size:?}")).collect::<std::collections::HashSet<_>>().len();
    let mut drawn: HashMap<String, (egui::TextureId, Rect)> = HashMap::new();
    let mut freed = 0;
    for _ in 0..200 {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let ctx = ui.ctx().clone();
            for (key, nodes, display, pt, size) in &pictures {
                let k = format!("{key}:{pt}:{size:?}");
                if let std::collections::hash_map::Entry::Vacant(e) = drawn.entry(k)
                    && let Some(t) = math_texture(&mut app, &ctx, key, nodes, *display, *pt, *size)
                {
                    e.insert(t);
                }
            }
        });
        freed += out.textures_delta.free.len();
        out.textures_delta.clear();
        if drawn.len() == unique {
            break;
        }
    }
    assert_eq!(drawn.len(), unique, "every picture is drawn");
    let ids: std::collections::HashSet<_> = drawn.values().map(|(id, _)| *id).collect();
    assert_eq!(ids.len(), 1, "one texture for every picture");
    assert_eq!(freed, 0, "no texture freed while browsing");
    // No two pictures overlap, and each lies inside the texture.
    let rects: Vec<Rect> = drawn.values().map(|(_, uv)| *uv).collect();
    for (i, a) in rects.iter().enumerate() {
        assert!(a.min.x >= 0.0 && a.min.y >= 0.0 && a.max.x <= 1.0 && a.max.y <= 1.0, "{a:?} inside");
        for b in rects.iter().skip(i + 1) {
            assert!(!a.intersects(*b) || a.intersect(*b).area() <= 0.0, "{a:?} overlaps {b:?}");
        }
    }
}

/// #1002: pictures are packed row by row, and pictures that can never fit are refused.
#[test]
fn math_atlas_packs_rows() {
    let mut a = AtlasPage::default();
    assert_eq!(a.place(100, 40), Some([0, 0]));
    assert_eq!(a.place(100, 20), Some([100 + MATH_ATLAS_GAP, 0]));
    assert_eq!(a.place(MATH_ATLAS, 10), Some([0, 40 + MATH_ATLAS_GAP]), "a new row under the tallest");
    assert_eq!(a.place(MATH_ATLAS + 1, 10), None);
    assert_eq!(a.place(10, 0), None);
    assert_eq!(a.place(10, MATH_ATLAS), None, "full");
    assert_eq!(a.place(10, 10), Some([0, 40 + MATH_ATLAS_GAP + 10 + MATH_ATLAS_GAP]), "still room after a refusal");
}

/// #1002: a page is never wiped in a frame that handed out one of its pictures (they are
/// already on screen); once all pages are full, the least recently drawn page starts over.
#[test]
fn math_atlas_never_wipes_a_page_drawn_from_this_frame() {
    let ctx = egui::Context::default();
    let img = || egui::ColorImage::filled([MATH_ATLAS, MATH_ATLAS], egui::Color32::TRANSPARENT);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        let mut a = MathAtlas::default();
        let first: Vec<_> = (0..MATH_ATLAS_PAGES).map(|i| a.add(ui.ctx(), 1, &format!("p{i}"), img())).collect();
        assert!(first.iter().all(Option::is_some), "a page each");
        let ids: std::collections::HashSet<_> = first.iter().flatten().map(|(id, _)| *id).collect();
        assert_eq!(ids.len(), MATH_ATLAS_PAGES);
        assert!(a.add(ui.ctx(), 1, "more", img()).is_none(), "every page was drawn from this frame");
        assert_eq!(a.wipes, 0);
        assert!(a.uv.len() == MATH_ATLAS_PAGES, "nothing forgotten");
        // Next frame: page 0 is drawn from again, so the one drawn longest ago is page 1.
        assert_eq!(a.get(2, "p0"), first.first().copied().flatten());
        let (id, _) = a.add(ui.ctx(), 2, "more", img()).expect("a page starts over");
        assert_eq!(a.wipes, 1);
        assert_eq!(Some(id), first.get(1).copied().flatten().map(|(id, _)| id), "the least recently drawn page");
        assert!(a.uv.contains_key("p0") && !a.uv.contains_key("p1"));
        // Pages 0 and 1 are now in use this frame; 2 and 3 are free to start over, then none.
        assert!(a.add(ui.ctx(), 2, "x", img()).is_some());
        assert!(a.add(ui.ctx(), 2, "y", img()).is_some());
        assert!(a.add(ui.ctx(), 2, "z", img()).is_none());
        assert_eq!(a.wipes, 3);
        assert!(a.get(2, "p0").is_some() && a.get(2, "more").is_some() && a.get(2, "x").is_some() && a.get(2, "y").is_some());
    });
    out.textures_delta.clear();
}

/// #1002: at a display scale of 3, the 22 built-in equations need more than one page; the
/// built-in menu opened again draws them all from the atlas, with nothing wiped or moved
/// (a single page used to start over every frame: flicker and constant re-rendering).
#[test]
fn builtin_menu_at_scale_3_fits_without_wiping() {
    use wordcraft_doc::math::parse_linear;
    use wordcraft_engine::math_gallery::BUILT_INS;
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(3.0);
    let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
    let builtins: Vec<_> = BUILT_INS.iter().map(|(id, _, lin)| (format!("builtin:{id}"), parse_linear(lin))).collect();
    let draw = |app: &mut WordApp| {
        let mut got = Vec::new();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(ui.ctx().pixels_per_point(), 3.0);
            let ctx = ui.ctx().clone();
            got = builtins.iter().map(|(key, nodes)| math_texture(app, &ctx, key, nodes, true, 11.0, vec2(346.0, 60.0))).collect();
        });
        out.textures_delta.clear();
        got
    };
    // The first opening renders a few a frame.
    let mut first = draw(&mut app);
    for _ in 0..20 {
        if first.iter().all(Option::is_some) {
            break;
        }
        first = draw(&mut app);
    }
    assert!(first.iter().all(Option::is_some), "every built-in drawn");
    let wipes = app.previews.math.wipes;
    let again = draw(&mut app);
    assert_eq!(again, first, "the same texture and place for each");
    assert_eq!(app.previews.math.wipes, wipes, "nothing started over");
    assert_eq!(wipes, 0);
}

#[test]
fn ink_mask_keeps_the_ink_and_drops_the_paper() {
    let mut img = egui::ColorImage::new([3, 1], vec![egui::Color32::WHITE, egui::Color32::BLACK, egui::Color32::from_gray(128)]);
    ink_mask(&mut img);
    assert_eq!(img.pixels[0].a(), 0, "paper becomes transparent");
    assert_eq!(img.pixels[1], egui::Color32::from_white_alpha(255), "full ink stays opaque");
    assert_eq!(img.pixels[2].a(), 127, "antialiased ink keeps its coverage");
}

/// #535: font menu entries drew on a white strip in dark mode. They are now tinted with the
/// theme's text colour and draw no background of their own.
#[test]
fn font_menu_entries_take_the_theme_text_colour() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx, crate::theme::Appearance::System);
        ctx.set_theme(theme);
        let preview = Previews::default().font_preview_fn();
        let mut out = None;
        for time in [1.0, 2.0] {
            let input = egui::RawInput { time: Some(time), ..Default::default() };
            let mut o = ctx.run_ui(input, |ui| {
                preview(ui, "Source Sans 3");
            });
            o.textures_delta.clear();
            out = Some(o);
        }
        let want = if theme == egui::Theme::Dark { Tokens::dark().text } else { Tokens::light().text };
        let shapes = out.map(|o| o.shapes).unwrap_or_default();
        let mut images = 0;
        for s in &shapes {
            match &s.shape {
                egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default() => {
                    images += 1;
                    assert!(m.vertices.iter().all(|v| v.color == want), "{theme:?}: the entry is not tinted with the text colour");
                }
                egui::Shape::Rect(r) => panic!("{theme:?}: an idle entry paints no background, got {:?}", r.fill),
                _ => {}
            }
        }
        assert_eq!(images, 1, "{theme:?}: the entry draws its rendered name");
    }
}
