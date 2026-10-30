use super::*;
use crate::table_resize::tests::{H, cell, screen, steps, table_window};

/// #3931: while Insert › Table's grid asks for a preview the page is laid out with that
/// table; the document, its revision and undo history stay as they were, and without a
/// request the page is the document's own again.
#[test]
fn the_page_shows_the_table_the_grid_previews() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Lanterns")), crate::Services::default());
    let cells = |l: &DocLayout| l.index.keys().filter(|(st, p)| *st == StoryRef::Body && p.0.len() == 4).count();
    let (doc, rev) = (app.session.doc.clone(), app.session.rev());
    app.canvas.table_preview = Some((2, 3));
    assert_eq!(cells(&shown_layout(&mut app)), 6);
    assert_eq!(app.canvas.table_preview, None, "a request lasts one frame");
    assert_eq!((&app.session.doc, app.session.rev(), app.session.can_undo()), (&doc, rev, false));
    app.canvas.table_preview = Some((2, 3));
    let again = shown_layout(&mut app);
    app.canvas.table_preview = Some((2, 3));
    assert!(std::sync::Arc::ptr_eq(&again, &shown_layout(&mut app)), "a still pointer reuses the preview");
    assert_eq!(cells(&shown_layout(&mut app)), 0, "no request: the document's own page");
}

/// #3931 review: the preview's pages are drawn from the copy they were laid out from. The
/// table moves "Kites" down a block, so the document's own block there is not its paragraph.
#[test]
fn the_previewed_pages_are_drawn_from_the_previewed_document() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Lanterns\nKites")), crate::Services::default());
    app.session.run("caret.end", &json!({})).unwrap();
    app.canvas.table_preview = Some((2, 2));
    let layout = shown_layout(&mut app);
    let text = |doc: &wordcraft_doc::Document, story, path| doc.para(story, path).map(|p| p.text.clone());
    let mut kites = 0;
    for it in layout.pages.iter().flat_map(|p| &p.items) {
        let Placed::Lines { story, path, .. } = it else { continue };
        let shown = text(page_doc(&app), *story, path);
        assert!(shown.is_some(), "{path:?} names a paragraph of the drawn document");
        kites += usize::from(shown.as_deref() == Some("Kites"));
    }
    assert_eq!(kites, 1, "Kites is drawn as itself");
    assert_eq!(shown_layout(&mut app).index, app.session.layout().index);
    assert!(std::ptr::eq(page_doc(&app), &app.session.doc), "no preview: the document's own pages");
}

/// #3931 review: with a selection the table replaces it in the copy, so the page draws the
/// copy's caret, in the table, and no selection: the document's selection names text that has
/// moved in the preview's layout.
#[test]
fn the_preview_draws_the_copy_caret_and_selection() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Lanterns\nKites")), crate::Services::default());
    app.session.sel = wordcraft_engine::Selection { anchor: Pos::body(0, 3), focus: Pos::body(1, 2) };
    app.canvas.table_preview = Some((2, 2));
    let layout = shown_layout(&mut app);
    let sel = preview_sel(&app).expect("the preview's selection").clone();
    assert!(sel.is_collapsed(), "the table took the selection's place");
    assert_ne!(sel, app.session.sel);
    let c = layout.caret_on(&sel.focus, 0).expect("the caret is on the preview's page");
    assert!(layout.pages[c.page].items.iter().any(|it| matches!(it, Placed::Lines { path, .. } if path.0.len() == 4 && *path == sel.focus.path)));
    assert!(sel.focus.path.cell().is_some(), "the caret is in the table");
    assert_eq!(shown_layout(&mut app).index, app.session.layout().index);
    assert!(preview_sel(&app).is_none(), "no preview: the document's own selection");
}

/// #3931 review: the preview is laid out again when the caret moves or the view changes with
/// the grid still hovered, so the page shows the table where a click would put it, at the
/// view's width.
#[test]
fn the_preview_follows_the_caret_and_the_view() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Lanterns\nKites")), crate::Services::default());
    let tables =
        |l: &DocLayout| l.index.keys().filter(|(st, p)| *st == StoryRef::Body && p.0.len() == 4).map(|(_, p)| p.0[0]).collect::<HashSet<u32>>();
    app.session.run("caret.docEnd", &json!({})).unwrap();
    app.canvas.table_preview = Some((2, 2));
    let at_end = tables(&shown_layout(&mut app));
    app.session.run("caret.docStart", &json!({})).unwrap();
    app.canvas.table_preview = Some((2, 2));
    let at_start = tables(&shown_layout(&mut app));
    assert_ne!(at_start, at_end, "the caret moved: so did the table");
    let (_, _, fresh) = app.session.preview_layout("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
    assert_eq!(at_start, tables(&fresh));
    app.session.view.mode = wordcraft_layout::ViewMode::Web;
    app.session.view.web_width = 300.0;
    app.canvas.table_preview = Some((2, 2));
    let web = shown_layout(&mut app);
    assert_eq!(web.pages[0].w, app.session.layout().pages[0].w, "laid out at the Web view's width");
}

/// A window-following layout size (#1876, Read Mode): applied at once on entry or when forced,
/// held while the window is dragged, applied once the size has been still for the settle time.
#[test]
fn settle_waits_for_a_drag_to_stop() {
    let mut s = Settle::default();
    assert!(s.entering());
    assert!(s.apply(vec2(1400.0, 700.0), 1.0, true, false), "entering applies at once");
    assert!(!s.apply(vec2(1400.0, 700.0), 1.01, false, false), "nothing changed");
    assert!(!s.apply(vec2(1300.0, 700.0), 1.02, true, false), "a drag frame waits");
    assert!(!s.apply(vec2(1200.0, 700.0), 1.10, true, false), "still dragging");
    assert!(!s.apply(vec2(1200.0, 700.0), 1.20, true, false), "not settled yet");
    assert!(s.apply(vec2(1200.0, 700.0), 1.11 + RESIZE_SETTLE, true, false), "settled");
    assert!(s.apply(vec2(1100.0, 700.0), 2.0, true, true), "forced applies mid-drag");
    assert!(s.apply(vec2(1100.0, 700.0), 5.0, true, false), "a zoom with a still window applies now");
    s.leave();
    assert!(s.entering());
}

/// #2545: notes belong to the body editing context, not the faded header/footer context.
#[test]
fn editing_notes_keeps_the_body_and_active_note_opaque() {
    use wordcraft_layout::display::{Draw, page_display};
    for command in ["references.footnote", "references.endnote"] {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Body marker.")), Default::default());
        app.run(command, json!({"text": "Note marker."})).unwrap();
        let note = app.session.sel.focus.story;
        assert!(matches!(part_kind(&app, note), Some(PartKind::Footnote | PartKind::Endnote)));
        app.run("insert.header", json!({"text": "Header marker."})).unwrap();
        let header = app.session.sel.focus.story;
        for story in [note, StoryRef::Body, header] {
            app.session.sel = wordcraft_engine::Selection::caret(app.session.doc.end_of(story));
            let layout = app.session.layout();
            for dark in [false, true] {
                app.session.view.dark_mode = dark;
                let opts = page_render_options(&app, dims_body(&app, &layout));
                let draws = page_display(&app.session.doc, &layout.pages[0], &opts.display);
                for marker in ["Body", "Note", "Header"] {
                    let alphas: Vec<f32> = draws
                        .iter()
                        .filter_map(|d| match d {
                            Draw::Glyphs { text, alpha, .. } if text.contains(marker) => Some(*alpha),
                            _ => None,
                        })
                        .collect();
                    assert!(!alphas.is_empty(), "{command}: {marker} is drawn");
                    let active = if marker == "Header" { story == header } else { story != header };
                    assert!(
                        alphas.iter().all(|a| if active { *a == 1.0 } else { *a > 0.0 && *a < 1.0 }),
                        "{command}, {story:?}, dark={dark}, {marker}: {alphas:?}"
                    );
                }
            }
        }
    }
}

/// What `page_image` replaced: the raster un-premultiplied, for egui to premultiply again.
fn by_straight_alpha(img: &wordcraft_render::Rendered) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.to_straight())
}

/// #2177: only a link to a program-made hidden bookmark hides its name in the hover tip.
#[test]
fn hover_tip_hides_generated_bookmark_names_only() {
    assert!(is_generated_bookmark_link("#_Toc123456"));
    assert!(is_generated_bookmark_link("#_Ref42"));
    assert!(!is_generated_bookmark_link("#Chapter1"));
    assert!(!is_generated_bookmark_link("https://example.com/_x"));
    assert!(!is_generated_bookmark_link(""));
}

#[test]
fn page_image_keeps_every_pixel_the_straight_alpha_round_trip_gave() {
    // Every premultiplied pixel there is: a channel from 0 up to its alpha, for each alpha.
    let mut pixels = Vec::new();
    for a in 0..=255u8 {
        for c in 0..=a {
            pixels.extend([c, a, a - c, a]);
        }
    }
    let every = wordcraft_render::Rendered { width: (pixels.len() / 4) as u32, height: 1, pixels };
    assert!(page_image(&every).pixels == by_straight_alpha(&every).pixels, "a premultiplied pixel changed");
    // And real pages as the canvas draws them: text, a table, a picture.
    let mut s = wordcraft_engine::Session::new(wordcraft_engine::sample::sample_document());
    let layout = s.layout();
    assert!(!layout.pages.is_empty());
    for page in &layout.pages {
        let img = wordcraft_render::render_page(&s.doc, page, 2.0, &screen_render_options());
        let ours = page_image(&img);
        assert_eq!(ours.size, [img.width as usize, img.height as usize]);
        assert!(ours.pixels == by_straight_alpha(&img).pixels, "a page pixel changed");
    }
}

#[test]
fn wheel_scrolls_while_drag_selecting() {
    assert!(wheel_hovered(true, false, false));
    assert!(wheel_hovered(true, true, true), "wheel keeps scrolling during a drag-select");
    assert!(!wheel_hovered(true, true, false), "another widget's drag still owns the wheel");
    assert!(!wheel_hovered(false, true, true));
}

/// #1772: the page-turn bar at the bottom right covered the last lines of the column under it.
#[test]
fn reading_columns_end_above_the_page_turn_bar() {
    let scale = PX_PER_PT;
    for (w, h) in [(1280.0, 740.0), (900.0, 700.0), (2000.0, 1200.0)] {
        for requested in [None, Some(180.0), Some(350.0), Some(612.0)] {
            let (_, ph) = reading_size(vec2(w, h), scale, requested);
            let bottom = GAP + ph * scale;
            assert!(bottom <= h - READ_NAV_BAR + 0.5, "{w}×{h} {requested:?}: column ends at {bottom}, bar starts at {}", h - READ_NAV_BAR);
        }
    }
}

/// #1772: a chosen column width (or a resized window) left the last visible column cut off at
/// the right edge. The reading viewport shows only whole columns, centred.
#[test]
fn reading_viewport_shows_whole_columns() {
    let scale = PX_PER_PT;
    for (w, h) in [(1280.0, 740.0), (900.0, 700.0), (1440.0, 820.0), (500.0, 600.0)] {
        let full = Rect::from_min_size(pos2(10.0, 20.0), vec2(w, h));
        for requested in [None, Some(180.0), Some(240.0), Some(350.0), Some(612.0)] {
            for zoom in [0.75, 1.0, 1.5] {
                let (cw, _) = reading_size(full.size(), scale * zoom, requested);
                let column = cw * scale * zoom;
                let area = reading_viewport(full, column, scale * zoom);
                assert!(full.contains_rect(area), "inside the canvas");
                assert_eq!(area.height(), full.height());
                assert!((area.center().x - full.center().x).abs() < 0.5, "centred");
                // Columns at rest start at GAP + k·step from the viewport's left edge. Past
                // the whole ones, only a column short by under half a point each may show.
                let step = column + GAP;
                let whole = ((area.width() - GAP) / step + 1e-3).floor().max(1.0);
                let used = whole * step + GAP;
                let short = used + step - area.width();
                assert!(
                    area.width() <= used + 0.5 || whole == 1.0 || short <= (whole + 1.0) * 0.5 * scale * zoom + 0.5,
                    "{w}×{h} {requested:?} {zoom}: {} wide shows a cut column (columns use {used})",
                    area.width()
                );
            }
        }
    }
    // With no width chosen the columns are sized to fill the canvas exactly, so none may be
    // dropped: the stored width (pt) and back to px must still fit every one of them.
    for zoom in [0.75, 1.0, 1.25, 1.5] {
        let scale = PX_PER_PT * zoom;
        for w in (300..=4000).step_by(7) {
            let full = Rect::from_min_size(pos2(0.0, 0.0), vec2(w as f32, 700.0));
            let count = (full.width() / 295.0).round().clamp(1.0, 5.0);
            let px = (full.width() - (count + 1.0) * GAP) / count;
            if px < 192.0 {
                continue; // the minimum width overflows; whole columns then shrink the view
            }
            let (cw, _) = reading_size(full.size(), scale, None);
            let column = cw * scale;
            let area = reading_viewport(full, column, scale);
            let needed = count * column + (count + 1.0) * GAP;
            assert!(area.width() >= needed - 0.5, "{w} at {zoom}: {} wide drops a column of {count} ({needed})", area.width());
        }
    }
    // Junk sizes never panic and fall back to the whole canvas.
    let full = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    assert_eq!(reading_viewport(full, f32::NAN, 1.0), full);
    assert_eq!(reading_viewport(full, 0.0, f32::NAN), full);
}

#[test]
fn reading_viewport_keeps_columns_when_the_window_shrinks_a_little() {
    // The column width is only re-laid out when it changes by more than half a point, so a
    // window a few pixels narrower keeps slightly too wide columns: none may drop out.
    for (zoom, shrink) in [(1.0, 1.0), (1.0, 3.0), (1.5, 1.0), (1.5, 3.0), (0.75, 1.0)] {
        let scale = PX_PER_PT * zoom;
        for w in (300..=4000).step_by(7) {
            let before = vec2(w as f32, 700.0);
            let count = (before.x / 295.0).round().clamp(1.0, 5.0);
            if (before.x - (count + 1.0) * GAP) / count < 192.0 {
                continue;
            }
            let after = before - vec2(shrink, 0.0);
            if (after.x / 295.0).round() != count {
                continue; // a different column count reflows anyway
            }
            let (kept, _) = reading_size(before, scale, None);
            let (fresh, _) = reading_size(after, scale, None);
            if (kept - fresh).abs() > 0.5 {
                continue; // re-laid out at once
            }
            let full = Rect::from_min_size(pos2(0.0, 0.0), after);
            let area = reading_viewport(full, kept * scale, scale);
            assert!(
                area.width() >= full.width() - 0.5,
                "{w}-{shrink} at {zoom}: kept {kept}pt columns, view narrowed to {} of {}",
                area.width(),
                full.width()
            );
        }
    }
}

#[test]
fn reading_animation_eases_then_lands_exactly_on_column_start() {
    let goal = 330.0;
    let one = reading_animation_step(0.0, goal, 1.0 / 60.0);
    assert!(one > 0.0 && one < goal);
    let mut pos = one;
    for _ in 0..60 {
        let next = reading_animation_step(pos, goal, 1.0 / 60.0);
        assert!((pos..=goal).contains(&next));
        pos = next;
    }
    assert_eq!(pos, goal);
    assert!(reading_animation_step(500.0, goal, 0.1).is_finite());
}

/// A large point-unit wheel tick turns exactly one reading column with no spill; line ticks turn
/// one column either way, small touchpad deltas accumulate, and columns align against the pane.
#[test]
fn reading_wheel_snaps_one_column_per_tick_and_respects_touchpads() {
    let ev = |unit, y| egui::Event::MouseWheel {
        unit,
        delta: vec2(0.0, y),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
        source: egui::MouseWheelSource::Unknown,
    };
    let mut partial = 0.0;
    for _ in 0..6 {
        assert_eq!(read_wheel_turns(&[ev(egui::MouseWheelUnit::Point, -65.0)], true, &mut partial), 1);
        assert_eq!(partial, 0.0, "a large wheel tick cannot spill into the next column");
    }
    assert_eq!(read_wheel_turns(&[ev(egui::MouseWheelUnit::Line, -1.0)], true, &mut partial), 1);
    assert_eq!(read_wheel_turns(&[ev(egui::MouseWheelUnit::Line, 1.0)], true, &mut partial), -1);
    assert_eq!(read_wheel_turns(&[ev(egui::MouseWheelUnit::Point, -20.0)], true, &mut partial), 0);
    assert_eq!(read_wheel_turns(&[ev(egui::MouseWheelUnit::Point, -20.0)], true, &mut partial), 0);
    assert_eq!(read_wheel_turns(&[ev(egui::MouseWheelUnit::Point, -20.0)], true, &mut partial), 1);
    assert_eq!(read_wheel_turns(&[ev(egui::MouseWheelUnit::Line, -1.0)], false, &mut partial), 0);
    let (rects, size) = place_read_pages(&[(240.0, 500.0); 10], 1.0, vec2(1100.0, 600.0));
    let last_left = rects.last().unwrap().left();
    assert!((size.x - 1100.0 - (last_left - GAP)).abs() < 1.0, "last column aligns against the pane");
    assert!(reading_size(vec2(1440.0, 750.0), PX_PER_PT, Some(612.0)).0 <= 612.0);
}

/// #299: the caret is black on light backgrounds and white on dark ones, whatever the UI
/// theme: on white paper, a dark page colour, a dark page (Switch Modes) and a highlight.
#[test]
fn caret_contrasts_with_what_lies_under_it() {
    use wordcraft_doc::props::Rgb;
    assert_eq!(caret_contrast(Rgb::WHITE), Color32::BLACK);
    assert_eq!(caret_contrast(Rgb::BLACK), Color32::WHITE);
    assert_eq!(caret_contrast(Rgb(0xFF, 0xFF, 0x00)), Color32::BLACK, "yellow highlight");
    assert_eq!(caret_contrast(Rgb(0x00, 0x00, 0x80)), Color32::WHITE, "dark blue");
    assert_eq!(caret_contrast(Rgb(0x33, 0x33, 0x33)), Color32::WHITE, "dark paper");
    assert_eq!(caret_contrast(Rgb(0xC0, 0xC0, 0xC0)), Color32::BLACK, "light grey");

    let doc = wordcraft_doc::Document::from_text("Hello WordCraft");
    let mut a = WordApp::new(wordcraft_engine::Session::new(doc), Default::default());
    let at = |a: &mut WordApp, off: usize| {
        let layout = a.session.layout();
        let c = layout.caret_on(&Pos::body(0, off), 0).unwrap();
        let view = view_key(a, false, None);
        caret_color_at(a, &layout, c.page, c.x, c.top + c.height / 2.0, view, false)
    };
    assert_eq!(at(&mut a, 2), Color32::BLACK, "white paper");
    a.run("design.pageColor", json!({"color": "#102040"})).unwrap();
    assert_eq!(at(&mut a, 2), Color32::WHITE, "dark page colour");
    a.run("design.pageColor", json!({})).unwrap();
    assert_eq!(at(&mut a, 2), Color32::BLACK, "page colour removed");
    a.session.view.dark_mode = true;
    assert_eq!(at(&mut a, 2), Color32::WHITE, "dark page");
    a.session.view.dark_mode = false;
    a.run("select.text", json!({"text": "Hello"})).unwrap();
    a.run("format.highlight", json!({"color": "black"})).unwrap();
    assert_eq!(at(&mut a, 2), Color32::WHITE, "inside a black highlight");
    assert_eq!(at(&mut a, 9), Color32::BLACK, "past it");
    assert_eq!(at(&mut a, 2), Color32::WHITE, "again, from the cache");
}

/// Read Mode › View › Page Color: the reading columns are drawn on white, sepia or a dark
/// page, in place of the document's page color; other views keep the page color.
#[test]
fn read_mode_page_color_sets_the_reading_paper() {
    use wordcraft_doc::props::Rgb;
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Hello WordCraft")), Default::default());
    a.run("design.pageColor", json!({"color": "#102040"})).unwrap();
    let paper = |a: &WordApp| page_render_options(a, false).paper_of(&a.session.doc);
    assert_eq!(paper(&a), Rgb(0x10, 0x20, 0x40), "Print Layout shows the page color");
    let print_key = view_key(&a, false, None);
    a.run("view.readMode", json!({})).unwrap();
    assert_eq!(paper(&a), Rgb::WHITE, "None: white, the page color ignored");
    let none_key = view_key(&a, false, None);
    a.run("view.readPageColor", json!({"value": "sepia"})).unwrap();
    assert_eq!(paper(&a), wordcraft_render::SEPIA_PAPER);
    assert!(!page_render_options(&a, false).dark, "Sepia keeps the text's colors");
    let sepia_key = view_key(&a, false, None);
    a.run("view.readPageColor", json!({"value": "inverse"})).unwrap();
    assert!(page_render_options(&a, false).dark);
    let d = wordcraft_render::DARK_PAPER;
    assert_eq!(paper(&a), Rgb(d, d, d));
    let inverse_key = view_key(&a, false, None);
    let keys = [print_key, none_key, sepia_key, inverse_key];
    for (i, k) in keys.iter().enumerate() {
        assert!(!keys[..i].contains(k), "each paper has its own view key");
    }
    a.run("view.printLayout", json!({})).unwrap();
    assert_eq!(paper(&a), Rgb(0x10, 0x20, 0x40), "leaving Read Mode brings the page color back");
    assert!(!page_render_options(&a, false).dark);
    // With Switch Modes on, Read Mode's paper still follows the Page color: None is white and Sepia is sepia.
    a.session.view.dark_mode = true;
    a.run("view.readMode", json!({})).unwrap();
    assert!(!page_render_options(&a, false).dark);
    assert_eq!(paper(&a), Rgb::WHITE);
    a.run("view.readPageColor", json!({"value": "sepia"})).unwrap();
    assert_eq!(paper(&a), wordcraft_render::SEPIA_PAPER);
    a.run("view.printLayout", json!({})).unwrap();
    assert!(page_render_options(&a, false).dark, "Print Layout keeps Switch Modes");
}

/// #299: the caret's colour is worked out only when the caret is drawn: with text selected
/// there is no caret, so the page's draw list isn't built for it.
#[test]
fn caret_colour_is_not_worked_out_while_the_caret_is_hidden() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Hello WordCraft")), Default::default());
    a.run("select.text", json!({"text": "Hello"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(4);
    assert!(h.state().canvas.caret_bg.is_none(), "no caret, no colour lookup");
}

/// #167: dragging a ruler's margin edge sets that margin, snapped, never negative, and
/// always leaving an inch of text.
#[test]
fn margin_drags_are_clamped() {
    let s = wordcraft_doc::section::SectionProps {
        page_w: 612.0,
        page_h: 792.0,
        margin_left: 72.0,
        margin_right: 72.0,
        margin_top: 72.0,
        margin_bottom: 72.0,
        gutter: 0.0,
        ..Default::default()
    };
    assert_eq!(dragged_margin(MarginEdge::Left, 100.0, &s), 99.0, "snapped to 1/16 inch");
    assert_eq!(dragged_margin(MarginEdge::Right, 612.0 - 36.0, &s), 36.0);
    assert_eq!(dragged_margin(MarginEdge::Left, -50.0, &s), 0.0, "never negative");
    assert_eq!(dragged_margin(MarginEdge::Left, 600.0, &s), 612.0 - 72.0 - 72.0, "an inch of text is left");
    assert_eq!(dragged_margin(MarginEdge::Right, 0.0, &s), 612.0 - 72.0 - 72.0);
    assert_eq!(dragged_margin(MarginEdge::Top, 9.0, &s), 9.0);
    assert_eq!(dragged_margin(MarginEdge::Bottom, 100.0, &s), 792.0 - 72.0 - 72.0);
    let g = wordcraft_doc::section::SectionProps { gutter: 36.0, ..s.clone() };
    assert_eq!(dragged_margin(MarginEdge::Left, 144.0, &g), 108.0, "the gutter isn't margin");
    assert_eq!(dragged_margin(MarginEdge::Left, f32::NAN, &s), 72.0, "junk keeps the margin");

    // Through the command: the result is applied and is one undo step.
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    let sect = wordcraft_engine::cmd::page::sect(&app.session);
    let v = dragged_margin(MarginEdge::Left, sect.page_w, &sect);
    app.run("layout.margins", json!({"left": v})).unwrap();
    let after = wordcraft_engine::cmd::page::sect(&app.session);
    assert!(after.page_w - after.margin_left - after.margin_right - after.gutter >= MIN_TEXT - 0.01);
    assert!(app.session.undo());
    assert_eq!(wordcraft_engine::cmd::page::sect(&app.session).margin_left, sect.margin_left);
}

/// #2914: on an even page of a mirrored layout the page is the odd page reflected, so dragging
/// its visible left edge edits the stored right margin and the visible right edge the left one.
#[test]
fn ruler_margin_drags_on_even_mirrored_pages_move_the_visible_edge() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("one")), Default::default());
    app.session.doc.settings.mirror_margins = true;
    app.run("layout.margins", json!({"left": 72.0, "right": 36.0})).unwrap();
    app.run("caret.docEnd", json!({})).unwrap();
    app.run("insert.pageBreak", json!({})).unwrap();
    let sect = wordcraft_engine::cmd::page::sect(&app.session);
    let body = |app: &mut WordApp, page: usize| {
        let l = app.session.layout();
        let b = l.pages.get(page).expect("the page exists").body;
        (b.x, b.right(), l.pages[page].number)
    };
    let (x, right, number) = body(&mut app, 1);
    assert_eq!(number, 2, "the second page is even");
    assert!((x - 36.0).abs() < 0.01 && (right - (sect.page_w - 72.0)).abs() < 0.01, "mirrored: outside margin on the left ({x}, {right})");
    // Odd pages and unmirrored layouts keep the plain mapping.
    assert_eq!(margin_drag(MarginEdge::Left, 90.0, &sect, false), (MarginEdge::Left, 90.0));
    assert_eq!(margin_drag(MarginEdge::Right, sect.page_w - 90.0, &sect, false), (MarginEdge::Right, 90.0));

    // The visible left edge of the even page, dragged to 90 pt, becomes the right margin.
    let (edge, v) = margin_drag(MarginEdge::Left, 90.0, &sect, true);
    assert_eq!(edge, MarginEdge::Right);
    app.run("layout.margins", json!({edge.param(): v})).unwrap();
    let after = wordcraft_engine::cmd::page::sect(&app.session);
    assert_eq!((after.margin_left, after.margin_right), (72.0, 90.0), "the left margin was not touched");
    let (x, right, _) = body(&mut app, 1);
    assert!((x - 90.0).abs() < 0.01 && (right - (sect.page_w - 72.0)).abs() < 0.01, "the visible left edge moved to the pointer ({x}, {right})");
    let (x, right, _) = body(&mut app, 0);
    assert!((x - 72.0).abs() < 0.01 && (right - (sect.page_w - 90.0)).abs() < 0.01, "the odd page follows ({x}, {right})");

    // The visible right edge, dragged to 500 pt, becomes the left margin (snapped to 1/16 inch).
    let sect = after;
    let (edge, v) = margin_drag(MarginEdge::Right, 500.0, &sect, true);
    assert_eq!(edge, MarginEdge::Left);
    app.run("layout.margins", json!({edge.param(): v})).unwrap();
    let (_, right, _) = body(&mut app, 1);
    assert!((right - 500.0).abs() <= 2.25, "the visible right edge is at the pointer ({right})");
    // Top and bottom are not mirrored.
    assert_eq!(margin_drag(MarginEdge::Top, 50.0, &sect, true), (MarginEdge::Top, dragged_margin(MarginEdge::Top, 50.0, &sect)));
}

/// Dragging a margin edge on the vertical ruler leaves the view where it is. Each step of the
/// drag runs `layout.margins`, which brought the caret back into view: with the caret off
/// screen (the bottom margin is far from the text at the top) the page slid under the pointer
/// mid-drag, and the margin ran far past it (a drag of about 1 cm set it about 6 cm larger).
#[test]
fn dragging_the_bottom_margin_on_the_ruler_keeps_the_view_and_follows_the_pointer() {
    let text = vec!["A line of text."; 20].join("\n");
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text(&text)), Default::default());
    a.run("view.ruler", json!({"value": true})).unwrap();
    a.run("caret.docStart", json!({})).unwrap();
    let mut h: H = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(0.25).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(6);
    let body = h.state_mut().session.layout().pages[0].body;
    let area = h.state().canvas.canvas_rect.expect("the canvas is shown");
    // Scroll the first page's bottom margin to the middle of the window: the caret, at the
    // top of the page, is off screen.
    let bottom = screen(&mut h, 0.0, body.bottom()).y;
    h.state_mut().canvas.scroll_request = Some(vec2(0.0, bottom - area.center().y));
    h.run_steps(4);
    assert!(screen(&mut h, 0.0, body.y).y < area.min.y, "the caret's line is above the view");
    let offset = h.state().canvas.scroll_offset;
    let sect = wordcraft_engine::cmd::page::sect(&h.state().session);
    let depth = h.state().session.undo_depth();

    // The bottom margin's edge on the vertical ruler, dragged up 36 points in a few moves.
    let x = area.min.x - RULER / 2.0;
    let start = pos2(x, screen(&mut h, 0.0, body.bottom()).y);
    let end = pos2(x, screen(&mut h, 0.0, body.bottom() - 36.0).y);
    h.event(egui::Event::PointerMoved(start));
    h.run_steps(2);
    h.event(egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    for k in 1..=5 {
        h.event(egui::Event::PointerMoved(start + (end - start) * (k as f32 / 5.0)));
        h.step();
        assert_eq!(h.state().canvas.scroll_offset, offset, "the view moved during the drag (step {k})");
    }
    h.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(3);
    assert_eq!(h.state().canvas.scroll_offset, offset, "the view moved after the drag");
    let after = wordcraft_engine::cmd::page::sect(&h.state().session);
    assert!((after.margin_bottom - (sect.margin_bottom + 36.0)).abs() <= 4.5, "{} → {}", sect.margin_bottom, after.margin_bottom);
    assert_eq!(h.state().session.undo_depth(), depth + 1, "one undo step for the drag");
}

/// Show revisions in balloons: a document whose only revision is a deletion gets the markup
/// area for its balloon; the other modes have nothing to show there, and neither does the view
/// while the Comments pane is open.
#[test]
fn a_deletion_alone_opens_the_markup_area_when_revisions_go_in_balloons() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("document.setText", json!({"text": "keep gone"})).unwrap();
    a.run("review.trackChanges", json!({"value": true})).unwrap();
    a.run("select.range", json!({"anchor": {"block": 0, "off": 5}, "focus": {"block": 0, "off": 9}})).unwrap();
    a.run("text.delete", json!({})).unwrap();
    assert_eq!(markup_width(&a), 0.0, "comments and formatting only: nothing to show");
    a.run("review.trackingOptions", json!({"balloons": "revisions"})).unwrap();
    assert!(markup_width(&a) > 0.0, "the deletion's balloon needs the markup area");
    a.run("view.commentsPane", json!({"value": true})).unwrap();
    assert_eq!(markup_width(&a), 0.0);
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    a.run("review.trackingOptions", json!({"insertionsDeletions": false})).unwrap();
    assert_eq!(markup_width(&a), 0.0, "deletions not shown at all");
}

/// #549: presses count as double/triple as they go down; slow or far presses start again.
#[test]
fn presses_count_up_to_three() {
    let at = pos2(100.0, 100.0);
    assert_eq!(press_count(None, 5.0, at, 0.3, 6.0), 1);
    assert_eq!(press_count(Some((5.0, at, 1)), 5.1, at, 0.3, 6.0), 2);
    assert_eq!(press_count(Some((5.1, at, 2)), 5.2, at + vec2(3.0, 0.0), 0.3, 6.0), 3);
    assert_eq!(press_count(Some((5.2, at, 3)), 5.3, at, 0.3, 6.0), 3, "stays a triple");
    assert_eq!(press_count(Some((5.0, at, 1)), 5.5, at, 0.3, 6.0), 1, "too slow");
    assert_eq!(press_count(Some((5.0, at, 1)), 5.1, at + vec2(20.0, 0.0), 0.3, 6.0), 1, "too far");
    assert_eq!(press_count(Some((5.0, at, 1)), 4.0, at, 0.3, 6.0), 1, "the clock went back");
}

/// #549: double-click a word and drag: the selection grows by whole words; triple-click and
/// drag: by whole paragraphs (Word's behaviour).
#[test]
fn multi_click_drag_extends_by_units() {
    use egui::{Event, PointerButton};
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Alpha beta gamma delta\rSecond para\rThird para"})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    let screen = |h: &mut egui_kittest::Harness<'_, WordApp>, pos: Pos| {
        let c = h.state_mut().session.layout().caret(&pos).expect("caret");
        page_to_screen(h.state_mut(), c.page, c.x + 1.0, c.top + c.height / 2.0).expect("on screen")
    };
    let button = |at: Pos2, pressed: bool| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    let multi_drag = |h: &mut egui_kittest::Harness<'_, WordApp>, from: Pos2, to: Pos2, clicks: usize| {
        h.event(Event::PointerMoved(from));
        for i in 0..clicks {
            h.event(button(from, true));
            h.step();
            if i + 1 < clicks {
                h.event(button(from, false));
                h.step();
            }
        }
        for k in 1..=4 {
            h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
            h.step();
        }
        h.event(button(to, false));
        for _ in 0..3 {
            h.step();
        }
    };
    // Double-click inside "beta", drag into "gamma": "beta gamma " is selected.
    let (from, to) = (screen(&mut h, Pos::body(0, 8)), screen(&mut h, Pos::body(0, 13)));
    multi_drag(&mut h, from, to, 2);
    let (s, e) = h.state().session.sel.ordered();
    assert_eq!((s, e), (Pos::body(0, 6), Pos::body(0, 17)), "whole words");
    // Let the double-click time pass, then triple-click the second paragraph and drag into the third.
    for _ in 0..40 {
        h.step();
    }
    let (from, to) = (screen(&mut h, Pos::body(1, 3)), screen(&mut h, Pos::body(2, 2)));
    multi_drag(&mut h, from, to, 3);
    let sel = h.state().session.sel.clone();
    assert_eq!((sel.anchor, sel.focus), (Pos::body(1, 0), Pos::body(2, 10)), "whole paragraphs");
    // A plain double-click (no drag) still selects just the word.
    h.state_mut().run("select.collapse", json!({})).unwrap();
    for _ in 0..40 {
        h.step();
    }
    let at = screen(&mut h, Pos::body(0, 1));
    multi_drag(&mut h, at, at, 2);
    let (s, e) = h.state().session.sel.ordered();
    assert_eq!((s, e), (Pos::body(0, 0), Pos::body(0, 6)));
}

/// #2551: dragging the vertical scrollbar shows the page at the top of the window beside the
/// pointer ("Page 7 of 12"), on one line as the number grows (#3341), and the tip goes when the
/// button is let go.
#[test]
fn dragging_the_scrollbar_names_the_page_at_the_top() {
    use egui::accesskit::Role;
    use egui::{Event, PointerButton};
    use egui_kittest::kittest::Queryable;
    for hide_white_space in [false, true] {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        for _ in 0..11 {
            a.run("text.insert", json!({"text": "Page text"})).unwrap();
            a.run("layout.break", json!({"kind": "page"})).unwrap();
        }
        a.run("view.commentsPane", json!({"value": false})).unwrap();
        a.run("view.hideWhiteSpace", json!({"value": hide_white_space})).unwrap();
        let (rev, sel) = (a.session.rev(), a.session.sel.clone());
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        assert_eq!(h.state_mut().session.layout().pages.len(), 12);
        let bar = h
            .query_all_by_role(Role::ScrollBar)
            .map(|n| n.rect())
            .filter(|r| r.height() > r.width())
            .max_by(|a, b| a.height().total_cmp(&b.height()))
            .expect("the canvas's vertical scrollbar");
        let button =
            |at: Pos2, pressed: bool| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        let from = pos2(bar.center().x, bar.top() + 8.0);
        h.event(Event::PointerMoved(from));
        h.step();
        assert_eq!(h.state().canvas.scrollbar_page, None, "no tip before a drag");
        h.event(button(from, true));
        h.step();
        let to = pos2(from.x, bar.top() + bar.height() * 0.65);
        for k in 1..=6 {
            h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 6.0)));
            h.step();
        }
        let page = h.state().canvas.scrollbar_page.expect("a tip while the scrollbar is dragged");
        assert!((4..9).contains(&page), "a page well down the document: {page}");
        assert!(h.query_by_label(&format!("Page {} of 12", page + 1)).is_some(), "the tip names page {}", page + 1);
        let end = pos2(from.x, bar.bottom() - 4.0);
        for k in 1..=6 {
            h.event(Event::PointerMoved(to + (end - to) * (k as f32 / 6.0)));
            h.step();
        }
        let last = h.state().canvas.scrollbar_page.expect("still dragging");
        assert!(last >= 9, "a two-digit page: {last}");
        let label = format!("Page {} of 12", last + 1);
        // The status bar (at the left) says the same page (#3880); the tip is the one beside the pointer.
        let tip = h.query_all_by_label(&label).map(|n| n.rect()).find(|r| r.center().x > 700.0).expect("the tip names the last page");
        let body = egui::TextStyle::Body.resolve(&h.ctx.global_style());
        assert!(tip.height() < 1.5 * h.ctx.fonts_mut(|f| f.row_height(&body)), "one line as the number grows: {tip:?}");
        h.event(button(end, false));
        h.step();
        h.step();
        assert_eq!(h.state().canvas.scrollbar_page, None, "the tip goes with the button");
        assert!(h.query_all_by_label(&label).all(|n| n.rect().center().x < 700.0), "and its text (the status bar still names the page)");
        assert_eq!((h.state().session.rev(), &h.state().session.sel), (rev, &sel), "scrolling changes neither the document nor the selection");
    }
}

/// #3880: the status bar names the page being looked at. With the caret scrolled out of view, that
/// is the page at the top of the window; while the caret is on screen, its page.
#[test]
fn the_status_bar_names_the_page_in_view_not_only_the_caret_page() {
    use egui_kittest::kittest::Queryable;
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    for _ in 0..11 {
        a.run("text.insert", json!({"text": "Page text"})).unwrap();
        a.run("layout.break", json!({"kind": "page"})).unwrap();
    }
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    a.run("caret.docStart", json!({})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    assert_eq!(h.state_mut().session.layout().pages.len(), 12);
    assert!(h.query_by_label("Page 1 of 12").is_some(), "the caret is on page 1 and in view");
    // Scroll to page 9 without touching the caret.
    let (rev, sel) = (h.state().session.rev(), h.state().session.sel.clone());
    let target = h.state().canvas.page_rects.get(8).map(|r| r.min.y - h.state().canvas.page_rects[0].min.y).expect("page 9");
    let at = h.state().canvas.scroll_offset.y;
    h.state_mut().canvas.scroll_request = Some(vec2(0.0, at + target));
    for _ in 0..4 {
        h.step();
    }
    assert!(h.query_by_label("Page 9 of 12").is_some(), "scrolled to page 9, the status bar says page 9");
    assert!(h.query_by_label("Page 1 of 12").is_none());
    assert_eq!((h.state().session.rev(), &h.state().session.sel), (rev, &sel), "scrolling changes neither the document nor the selection");
}

/// `viewed_page`: the caret's page while any of its line is on screen, else the window's top page.
#[test]
fn the_page_in_view_follows_the_caret_while_it_is_on_screen() {
    let rects: Vec<Rect> = (0..4).map(|i| Rect::from_min_size(pos2(0.0, i as f32 * 110.0), vec2(80.0, 100.0))).collect();
    let whole = vec![(0.0, 100.0); 4];
    let caret = |page: usize, y: f32| Some((page, rects[page].min.y + y, rects[page].min.y + y + 10.0));
    // Window 0..150 shows page 0 and the top of page 1.
    assert_eq!(viewed_page(caret(1, 5.0), &rects, &whole, 0.0, 150.0), 1, "the caret is on screen, on page 1");
    assert_eq!(viewed_page(caret(0, 5.0), &rects, &whole, 0.0, 150.0), 0);
    // Scrolled to page 3: the caret on page 0 is off screen.
    assert_eq!(viewed_page(caret(0, 5.0), &rects, &whole, 330.0, 100.0), 3);
    assert_eq!(viewed_page(None, &rects, &whole, 220.0, 100.0), 2, "no caret: the top page");
    assert_eq!(viewed_page(caret(2, 90.0), &rects, &whole, 330.0, 100.0), 3, "a caret just above the window is not in view");
}

/// `top_page`: the page at a window's top, the one below in a gap, the last one past the end.
#[test]
fn the_page_at_the_top_of_the_window() {
    let rects: Vec<Rect> = (0..3).map(|i| Rect::from_min_size(pos2(0.0, i as f32 * 110.0), vec2(80.0, 100.0))).collect();
    let whole: Vec<(f32, f32)> = rects.iter().map(|r| (0.0, r.height())).collect();
    assert_eq!(top_page(&rects, &whole, 0.0), 0);
    assert_eq!(top_page(&rects, &whole, 99.0), 0);
    assert_eq!(top_page(&rects, &whole, 105.0), 1, "in the gap: the page below");
    assert_eq!(top_page(&rects, &whole, 250.0), 2);
    assert_eq!(top_page(&rects, &whole, 1e6), 2, "past the end: the last page");
    assert_eq!(top_page(&[], &[], 0.0), 0);

    // Hide White Space: the rects overlap in the hidden margins. Just past the seam the next
    // page's text shows, so that page is at the top, though the previous rect reaches further.
    let sizes = [(612.0, 792.0); 3];
    let (full, _) = place_pages(&sizes, 0.0, 1.0, 1, vec2(800.0, 600.0));
    let (moved, bands, _) = collapse_white_space(&full, &[(72.0, 720.0); 3], 1.0, 600.0);
    let seam = moved[0].min.y + bands[0].1;
    assert!(moved[0].max.y > seam + SEAM + 2.0, "the rects overlap past the seam");
    assert_eq!(top_page(&moved, &bands, seam - 2.0), 0);
    assert_eq!(top_page(&moved, &bands, seam + SEAM + 2.0), 1, "across the seam: the next page");
}

/// #1014: Ctrl+drag on a shape leaves it and drops a copy; a plain drag moves it.
#[test]
fn ctrl_drag_copies_an_object() {
    use egui::{Event, Modifiers, PointerButton};
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Some text"})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
    a.run("arrange.bounds", json!({"width": 100, "height": 60, "page": 0, "x": 200, "y": 300})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    let objects = |h: &mut egui_kittest::Harness<'_, WordApp>| {
        let doc = &h.state().session.doc;
        doc.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).map_or(0, |p| p.objects.len())
    };
    let xs = |h: &mut egui_kittest::Harness<'_, WordApp>| {
        let mut v: Vec<i32> = Vec::new();
        for x in (0..600).step_by(50) {
            if h.state_mut().session.layout().find_object(0, |o| (o.rect.x - x as f32).abs() < 0.5).is_some() {
                v.push(x);
            }
        }
        v
    };
    let drag = |h: &mut egui_kittest::Harness<'_, WordApp>, mods: Modifiers| {
        let from = page_to_screen(h.state_mut(), 0, 250.0, 330.0).expect("on screen");
        let to = page_to_screen(h.state_mut(), 0, 400.0, 330.0).expect("on screen");
        h.event(Event::ModifiersChanged(mods));
        h.event(Event::PointerMoved(from));
        h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: mods });
        h.step();
        for k in 1..=4 {
            h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
            h.step();
        }
        h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: mods });
        for _ in 0..3 {
            h.step();
        }
        h.event(Event::ModifiersChanged(Modifiers::NONE));
        h.step();
    };
    assert_eq!((objects(&mut h), xs(&mut h)), (1, vec![200]));
    drag(&mut h, Modifiers::COMMAND);
    assert_eq!((objects(&mut h), xs(&mut h)), (2, vec![200, 350]), "the original stays, a copy lands 150pt right");
    // Without Ctrl the object under the pointer (the original) is moved, not copied again:
    // it lands on the copy.
    drag(&mut h, Modifiers::NONE);
    assert_eq!((objects(&mut h), xs(&mut h)), (2, vec![350]));
}

/// #856: the left margin beside body text is Word's selection bar. A click selects the line,
/// a double-click the paragraph, a triple-click (or Ctrl/⌘+click) the document; dragging
/// selects whole lines and Shift+click extends by lines.
#[test]
fn selection_bar_clicks_select_lines() {
    use egui::{Event, Modifiers, PointerButton};
    let long = "The quick brown fox jumps over the lazy dog. ".repeat(8);
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": format!("{}\rSecond para\rThird para", long.trim_end())})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    // A point in the left margin beside the line `li` lines below the one holding `pos`.
    let margin = |h: &mut egui_kittest::Harness<'_, WordApp>, pos: Pos, li: f32| {
        let c = h.state_mut().session.layout().caret(&pos).expect("caret");
        page_to_screen(h.state_mut(), c.page, 30.0, c.top + c.height * (li + 0.5)).expect("on screen")
    };
    let line_start = |h: &mut egui_kittest::Harness<'_, WordApp>, at: Pos2| {
        let (rects, scale) = (h.state().canvas.page_rects.clone(), h.state().canvas.scale);
        let layout = h.state_mut().session.layout();
        let (pg, x, y) = page_at(&rects, &layout, scale, at).expect("page");
        layout.selection_bar_at(pg, x, y).expect("in the selection bar")
    };
    let span = |h: &mut egui_kittest::Harness<'_, WordApp>, pos: &Pos| h.state_mut().session.layout().line_span(pos, 0).expect("line");
    let button = |at: Pos2, pressed: bool| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    let clicks_drag = |h: &mut egui_kittest::Harness<'_, WordApp>, from: Pos2, to: Pos2, clicks: usize| {
        h.event(Event::PointerMoved(from));
        for i in 0..clicks {
            h.event(button(from, true));
            h.step();
            if i + 1 < clicks {
                h.event(button(from, false));
                h.step();
            }
        }
        for k in 1..=4 {
            h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
            h.step();
        }
        h.event(button(to, false));
        for _ in 0..3 {
            h.step();
        }
    };
    let pause = |h: &mut egui_kittest::Harness<'_, WordApp>| {
        for _ in 0..40 {
            h.step();
        }
    };
    // Click beside the second line: that whole line is selected, not a caret at its start.
    let at = margin(&mut h, Pos::body(0, 0), 1.0);
    let l2 = line_start(&mut h, at);
    assert!(l2.off > 0 && l2.path == wordcraft_doc::Path::top(0));
    clicks_drag(&mut h, at, at, 1);
    let (_, l2_end, _) = span(&mut h, &l2);
    assert_eq!(h.state().session.sel.ordered(), (l2.clone(), l2_end.clone()), "the line");
    // Double-click: the paragraph, mark included.
    pause(&mut h);
    clicks_drag(&mut h, at, at, 2);
    assert_eq!(h.state().session.sel.ordered(), (Pos::body(0, 0), Pos::body(1, 0)), "the paragraph");
    // Triple-click: the whole document.
    pause(&mut h);
    clicks_drag(&mut h, at, at, 3);
    assert_eq!(h.state().session.sel.ordered(), (Pos::body(0, 0), Pos::body(2, 10)), "everything");
    // Drag from beside the first line down past the third: three whole lines.
    pause(&mut h);
    let (from, to) = (margin(&mut h, Pos::body(0, 0), 0.0), margin(&mut h, Pos::body(0, 0), 2.0));
    let l3 = line_start(&mut h, to);
    clicks_drag(&mut h, from, to, 1);
    let (_, l3_end, _) = span(&mut h, &l3);
    let sel = h.state().session.sel.clone();
    assert_eq!((sel.anchor, sel.focus), (Pos::body(0, 0), l3_end), "three lines");
    // Shift+click beside "Second para" extends by lines, to that paragraph's mark.
    pause(&mut h);
    let second = margin(&mut h, Pos::body(1, 0), 0.0);
    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    h.step();
    clicks_drag(&mut h, second, second, 1);
    h.event(Event::ModifiersChanged(Modifiers::default()));
    h.step();
    let sel = h.state().session.sel.clone();
    assert_eq!((sel.anchor, sel.focus), (Pos::body(0, 0), Pos::body(2, 0)), "extended by lines");
    // Hovering the bar shows the arrow, not the I-beam.
    h.event(Event::PointerMoved(at));
    h.step();
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Default);
}

/// A right-to-left paragraph's selection bar is the right margin; the left margin beside it
/// is nothing special (a click there is the caret's).
#[test]
fn selection_bar_of_a_right_to_left_paragraph_is_on_the_right() {
    use egui::{Event, PointerButton};
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "First para\r\u{5e9}\u{5dc}\u{5d5}\u{5dd} \u{5e9}\u{5dc}\u{5d5}\u{5dd}\rThird para"})).unwrap();
    a.run("caret.set", json!({"pos": serde_json::to_value(Pos::body(1, 0)).unwrap()})).unwrap();
    a.run("para.rtl", json!({})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    let c = h.state_mut().session.layout().caret(&Pos::body(1, 0)).expect("caret");
    let (body_x, page_w) = {
        let l = h.state_mut().session.layout();
        (l.pages[c.page].body.right(), l.pages[c.page].w)
    };
    let y = c.top + c.height / 2.0;
    let right = page_to_screen(h.state_mut(), c.page, (body_x + page_w) / 2.0, y).expect("on screen");
    let left = page_to_screen(h.state_mut(), c.page, 30.0, y).expect("on screen");
    let click = |h: &mut egui_kittest::Harness<'_, WordApp>, at: Pos2| {
        h.event(Event::PointerMoved(at));
        h.step();
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.step();
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
        for _ in 0..3 {
            h.step();
        }
    };
    h.event(Event::PointerMoved(right));
    h.step();
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Default, "the bar's arrow on the right");
    click(&mut h, right);
    let (s, e) = h.state().session.sel.ordered();
    assert_eq!(s, Pos::body(1, 0), "the line is selected");
    assert!(e > s && (e.path == wordcraft_doc::Path::top(1) || e == Pos::body(2, 0)), "{s:?}..{e:?}");
    // Beside it on the left: no bar (not the bar's arrow, and a caret, not a line selection).
    // The cursor there is the text cursor, or none at all on some platforms' test windows.
    h.event(Event::PointerMoved(left));
    h.step();
    assert_ne!(h.output().platform_output.cursor_icon, egui::CursorIcon::Default);
    click(&mut h, left);
    assert!(h.state().session.sel.is_collapsed(), "a click in the left margin beside it is a caret, not a line");
}

/// #3453: while the Format Painter is armed the pointer over the text says so, and it goes
/// back once the painter is off. The selection bar keeps its arrow while it is armed.
#[test]
fn an_armed_format_painter_changes_the_pointer() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Paint this formatting"})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    let c = h.state_mut().session.layout().caret(&Pos::body(0, 6)).expect("caret");
    let over_text = page_to_screen(h.state_mut(), c.page, c.x, c.top + c.height / 2.0).expect("on screen");
    let over_bar = page_to_screen(h.state_mut(), c.page, 30.0, c.top + c.height / 2.0).expect("on screen");
    let pointer_after = |h: &mut egui_kittest::Harness<'_, WordApp>, cmd: Option<&str>| {
        if let Some(cmd) = cmd {
            h.state_mut().run(cmd, json!({})).unwrap();
        }
        h.event(egui::Event::PointerMoved(over_text));
        h.step();
        h.output().platform_output.cursor_icon
    };
    // Where the I-beam is drawn (Windows, #1746) the system pointer over the text is hidden.
    let ibeam = if h.state().canvas.drawn_ibeam { egui::CursorIcon::None } else { egui::CursorIcon::Text };
    assert_eq!(pointer_after(&mut h, None), ibeam, "no painter yet");
    assert_eq!(pointer_after(&mut h, Some("edit.formatPainter")), egui::CursorIcon::Copy, "armed");
    assert_eq!(pointer_after(&mut h, Some("edit.formatPainter")), ibeam, "turned off");
    h.state_mut().run("edit.formatPainter", json!({})).unwrap();
    h.event(egui::Event::PointerMoved(over_bar));
    h.step();
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Default, "the selection bar keeps its arrow");
}

/// #1085: a selected balloon's buttons work (pressing them is not a click on the page that
/// leaves the balloon): a new comment with no text resolves and deletes from its balloon.
#[test]
fn an_empty_comment_balloon_deletes() {
    for body in ["", "Some text here"] {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        if !body.is_empty() {
            a.run("text.insert", json!({"text": body})).unwrap();
            a.run("select.text", json!({"text": "text"})).unwrap();
        }
        let id = a.run("review.newComment", json!({})).unwrap()["id"].as_u64().unwrap() as u32;
        a.run("view.commentsPane", json!({"value": false})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        let click = |h: &mut egui_kittest::Harness<'_, WordApp>, at: egui::Pos2| {
            h.event(egui::Event::PointerMoved(at));
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
            h.step();
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
            for _ in 0..3 {
                h.step();
            }
        };
        let card = h.state().canvas.balloon_rects.iter().find(|(i, _)| *i == id).map(|(_, r)| *r).expect("the comment has a balloon");
        click(&mut h, card.left_top() + vec2(30.0, 8.0));
        assert_eq!(h.state().canvas.balloon, Some(id), "{body:?}: the click selected the balloon");
        use egui_kittest::kittest::Queryable;
        let resolve = h.get_by_label("Resolve").rect().center();
        click(&mut h, resolve);
        assert!(h.state().session.doc.comments[&id].resolved, "{body:?}: Resolve worked");
        assert_eq!(h.state().canvas.balloon, Some(id), "{body:?}: the balloon stays selected");
        let del = h.get_by_label("Delete").rect().center();
        click(&mut h, del);
        assert!(h.state().session.doc.comments.is_empty(), "{body:?}: the empty comment was deleted");
    }
}

/// A typed character is laid out and painted in the frame that delivers the key, not the next
/// one: keys are handled before the canvas takes the layout it paints.
#[test]
fn a_keystroke_is_painted_in_its_own_frame() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Some text"})).unwrap();
    a.canvas.want_focus = true;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    assert!(h.state().canvas.focused, "the canvas has the keyboard focus");
    h.event(egui::Event::Text("x".into()));
    h.step();
    assert_eq!(h.state().session.doc.plain_text(StoryRef::Body), "Some textx");
    let latest = h.state_mut().session.layout();
    let painted = h.state().canvas.page_keys.layout.clone();
    assert!(painted.is_some_and(|l| std::sync::Arc::ptr_eq(&l, &latest)), "the frame painted the layout with the typed character");
}

/// #167: a comment balloon edits in place like Word's: click it, type, leave it, and the
/// comment has the new text as one undo step.
/// Lays `text` out as a read-only balloon does, with a document face that covers Arabic, and
/// returns each glyph's character and left edge.
fn balloon_glyphs(text: &str, width: f32) -> Option<Vec<(char, f32)>> {
    balloon_glyphs_at_scale(text, width, 1.0)
}

/// Lays `text` out for a balloon at `ppp` with the document fallback face and returns each glyph's
/// character and physical left edge, asserting that wrapping stays within `width`.
fn balloon_glyphs_at_scale(text: &str, width: f32, ppp: f32) -> Option<Vec<(char, f32)>> {
    let face = wordcraft_fonts::FontDb::global().fallback_for('ش', u32::MAX)?;
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(ppp);
    let mut fonts = crate::theme::font_definitions(false, false);
    crate::theme::add_fallback_faces(&mut fonts, &[face]);
    ctx.set_fonts(fonts);
    ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
    let mut out = Vec::new();
    ctx.run_ui(egui::RawInput::default(), |ui| {
        let g = balloon_text(ui.painter(), text.to_string(), egui::FontId::proportional(14.0), Color32::BLACK, width);
        let shape = g.text_shape(Pos2::ZERO, Color32::BLACK);
        let atlas = ui.fonts(|fonts| fonts.image());
        for r in &shape.galley.rows {
            out.extend(r.row.glyphs.iter().map(|gl| (gl.chr, shape.pos.x + r.pos.x + gl.pos.x)));
            for glyph in &r.row.glyphs {
                // epaint rounds glyph origins individually while row advance stays fractional.
                // The two edge roundings can differ by one physical pixel.
                assert!(
                    shape.pos.x + r.pos.x + glyph.max_x() <= width + 1.0 / ppp,
                    "wrapped glyph {:?}: origin {}, max {}, row width {}, limit {} (text {:?})",
                    glyph.chr,
                    shape.pos.x + r.pos.x,
                    glyph.max_x(),
                    r.size.x,
                    width,
                    text
                );
                if !matches!(glyph.chr, '(' | ')') {
                    continue;
                }
                // Glyph.chr remains logical in 0.37. Check the actual mirrored bitmap,
                // not a manually reversed display string, to verify bracket orientation.
                let [x0, y0] = glyph.uv_rect.min.map(usize::from);
                let [x1, y1] = glyph.uv_rect.max.map(usize::from);
                let y = usize::midpoint(y0, y1);
                let (mut ink, mut moment) = (0.0, 0.0);
                for x in x0..x1 {
                    let alpha = f32::from(atlas[(x, y)].a());
                    ink += alpha;
                    moment += (x - x0) as f32 * alpha;
                }
                assert!(ink > 0.0 && x1 > x0, "bracket must have a visible bitmap");
                let drawn = if moment / ink < (x1 - x0 - 1) as f32 / 2.0 { '(' } else { ')' };
                let expected = if glyph.is_rtl() { if glyph.chr == '(' { ')' } else { '(' } } else { glyph.chr };
                assert_eq!(drawn, expected, "logical bracket {} at bidi level {}", glyph.chr, glyph.bidi_level);
            }
        }
    })
    .drop_without_applying_deltas();
    Some(out)
}

/// Where egui draws a right-to-left balloon's words, measured: the first word on the right.
#[test]
fn a_right_to_left_balloon_reads_from_the_right() {
    let x = |g: &[(char, f32)], c: char| g.iter().find(|(ch, _)| *ch == c).map(|(_, x)| *x).unwrap();
    let Some(g) = balloon_glyphs("شركة اي", 400.0) else { return };
    assert!(x(&g, 'ش') > x(&g, 'ي'), "the first word is right of the second: {g:?}");
    assert!(x(&g, 'ش') > 300.0, "the line is set against the right edge: {g:?}");
    // A Latin word inside keeps its letters left to right; its brackets face it.
    let g = balloon_glyphs("الربط (APIs) هنا", 400.0).unwrap();
    assert!(x(&g, 'A') < x(&g, 's'));
    assert!(x(&g, 'ط') > x(&g, 'A') && x(&g, 'A') > x(&g, 'ه'), "{g:?}");
    // The logical opening bracket is on the right and is drawn as a closing bracket.
    // balloon_glyphs also checks the atlas bitmap, so this verifies actual `(APIs)` output.
    assert!(x(&g, ')') < x(&g, 'A') && x(&g, '(') > x(&g, 's'), "brackets surround the word: {g:?}");
    // A paragraph that starts in Latin runs left to right.
    let g = balloon_glyphs("@Ahmed برجاء مراجعة", 400.0).unwrap();
    assert!(x(&g, '@') < x(&g, 'ب') && x(&g, '@') < 50.0, "{g:?}");
    assert!(x(&g, 'ب') > x(&g, 'م'), "the Arabic words still read from the right: {g:?}");
    // Wrapped rows stay inside the width.
    let long = "بيان الأنظمة المطلوب ربطها ومتطلبات التكامل وواجهات الربط البرمجي وآلية تبادل البيانات";
    for ppp in [1.0, 1.25, 1.5, 2.0] {
        let g = balloon_glyphs_at_scale(long, 200.0, ppp).unwrap();
        assert!(!g.is_empty()); // Check real glyph right edges at each physical scale.
    }
    // Alignment follows each paragraph, including a neutral paragraph's inherited direction.
    let g = balloon_glyphs("شركة\n123\nLatin", 400.0).unwrap();
    assert!(x(&g, 'ش') > 300.0 && x(&g, '1') > 300.0 && x(&g, 'L') < 1.0, "{g:?}");
}

struct BalloonFontFixture {
    family: &'static str,
    probe: &'static str,
    text: &'static str,
}

/// Explicit embedded bytes only: no OS font discovery or implicit default-font fallback.
/// Optional-input builds always exercise Latin; qualification must exercise both RTL faces.
fn balloon_font_fixtures() -> (egui::FontDefinitions, Vec<BalloonFontFixture>) {
    let mut fonts = egui::FontDefinitions::empty();
    fonts.font_data.insert(
        "Balloon Latin".into(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!("../../../../assets/fonts/Inter-Regular.ttf"))),
    );
    let mut names = vec!["Balloon Latin".to_string()];
    let mut fixtures = vec![BalloonFontFixture { family: "Balloon Latin", probe: "Reading", text: "Reading long_unbreakable_text\n123\nLatin\n" }];
    for (family, script, probe, text) in [
        ("Noto Sans Arabic", "Arab", "شركة", "شركة اي\n123\nالربط (APIs) هنا\nLatin\n"),
        ("Noto Sans Hebrew", "Hebr", "שלום", "שלום עולם\n123\nשלום (APIs) עולם\nLatin\n"),
    ] {
        if let Some(face) = wordcraft_fonts::CRAFT_FONTS.iter().find(|face| face.family == family && face.style == "Regular") {
            assert!(face.scripts.contains(&script));
            assert!(!face.bytes.is_empty());
            fonts.font_data.insert(family.into(), std::sync::Arc::new(egui::FontData::from_static(face.bytes)));
            names.push(family.to_string());
            fixtures.push(BalloonFontFixture { family, probe, text });
        }
    }
    if option_env!("CRAFT_FONTS_REQUIRED").is_some() || std::env::var_os("CRAFT_FONTS_REQUIRED").is_some() {
        assert_eq!(fixtures.len(), 3, "qualification requires both exact embedded Arabic and Hebrew faces");
    }
    for name in &names {
        fonts.families.insert(egui::FontFamily::Name(name.clone().into()), vec![name.clone()]);
    }
    fonts.families.insert(egui::FontFamily::Proportional, names.clone());
    fonts.families.insert(egui::FontFamily::Monospace, names);
    (fonts, fixtures)
}

/// Independent pre-normalization reading-space oracle: position the original shaped rows
/// using their paragraph direction, then recompute complete bounds. Returns the reference
/// galley and whether this fixture actually exercised RTL shaping.
fn balloon_reading_reference(fixture: &BalloonFontFixture, original: &egui::Galley, width: f32, ppp: f32) -> (std::sync::Arc<egui::Galley>, bool) {
    let mut reference = (*original).clone();
    reference.rect = Rect::from_min_size(Pos2::ZERO, vec2(width, original.rect.height()));
    reference.mesh_bounds = Rect::NOTHING;
    for row in &mut reference.rows {
        let factor = if row.rtl_paragraph { 1.0 } else { 0.0 };
        row.pos.x = ((f64::from(width) - f64::from(row.size.x)) * factor * f64::from(ppp)).floor() as f32 / ppp;
        reference.rect |= row.rect();
        reference.mesh_bounds |= row.visuals.mesh_bounds.translate(row.pos.to_vec2());
    }
    let mut rtl = false;
    if fixture.family != "Balloon Latin" {
        assert!(original.rows.iter().any(|row| row.rtl_paragraph), "actual RTL shaping is required");
        if width == 0.0 {
            assert!(
                reference.rows.iter().any(|row| row.rtl_paragraph && row.pos.x < 0.0 && row.size.x > width),
                "actual negative reading-space overflow is required"
            );
        }
        if width == 200.0 {
            let row = &reference.rows[0];
            let (first, second) = if fixture.family == "Noto Sans Arabic" { ('ش', 'ي') } else { ('ש', 'ע') };
            let x = |c| row.glyphs.iter().find(|glyph| glyph.chr == c).unwrap().pos.x;
            assert!(x(first) > x(second), "the first RTL word is physically right of the next");
            assert!(reference.rows.iter().find(|row| row.text() == "123").unwrap().rtl_paragraph, "neutral paragraph inherits RTL");
            assert!(!reference.rows.iter().find(|row| row.text() == "Latin").unwrap().rtl_paragraph);
        }
        rtl = true;
    }
    (std::sync::Arc::new(reference), rtl)
}

/// Every aligned row keeps its logical position, shared glyph meshes and finite ink bounds.
fn assert_balloon_aligned_rows(galley: &egui::Galley, paint_offset: egui::Vec2, original: &egui::Galley, reference: &egui::Galley, case: &str) {
    let ppp = original.pixels_per_point;
    let (mut empty_mesh_rows, mut drawn_mesh_rows) = (0, 0);
    for (row_index, ((row, expected), old)) in galley.rows.iter().zip(&reference.rows).zip(&original.rows).enumerate() {
        assert!(std::sync::Arc::ptr_eq(&row.row, &old.row), "glyph order/meshes remain shared: {case}, row={row_index}");
        assert_eq!(row.pos.y, old.pos.y, "row Y: {case}, row={row_index}");
        assert!((row.pos + paint_offset - expected.pos).length() * ppp < 1e-4, "absolute row position: {case}, row={row_index}");
        assert!(
            galley.rect.contains_rect(row.rect()),
            "logical row containment: {case}, row={row_index} text={:?} logical={:?} galley={:?}",
            row.text(),
            row.rect(),
            galley.rect
        );
        if row.visuals.mesh.vertices.is_empty() {
            // Empty/whitespace rows still have logical height, but no
            // finite ink rectangle: translating NOTHING yields NaNs.
            empty_mesh_rows += 1;
            assert!(row.visuals.mesh.indices.is_empty(), "empty mesh indices: {case}, row={row_index} text={:?}", row.text());
            assert_eq!(row.visuals.mesh_bounds, Rect::NOTHING, "empty ink sentinel: {case}, row={row_index} text={:?}", row.text());
        } else {
            drawn_mesh_rows += 1;
            let ink = row.visuals.mesh_bounds.translate(row.pos.to_vec2());
            assert!(ink.is_finite(), "finite row ink: {case}, row={row_index} text={:?} ink={ink:?}", row.text());
            assert!(galley.mesh_bounds.is_finite(), "finite galley ink: {case}, row={row_index} bounds={:?}", galley.mesh_bounds);
            assert!(
                galley.mesh_bounds.contains_rect(ink),
                "finite ink containment: {case}, row={row_index} text={:?} ink={ink:?} galley={:?}",
                row.text(),
                galley.mesh_bounds
            );
            for (vertex_index, vertex) in row.visuals.mesh.vertices.iter().enumerate() {
                let pos = row.pos + vertex.pos.to_vec2();
                assert!(
                    galley.mesh_bounds.contains(pos),
                    "ink vertex containment: {case}, row={row_index} vertex={vertex_index} text={:?} pos={pos:?} galley={:?}",
                    row.text(),
                    galley.mesh_bounds
                );
            }
        }
    }
    assert!(
        empty_mesh_rows > 0 && drawn_mesh_rows > 0,
        "fixture must observe empty and nonempty meshes: {case}, empty={empty_mesh_rows} drawn={drawn_mesh_rows}"
    );
}

/// Run `balloon_text` for one width/origin and prove its galley and painted ink match the
/// independent reference, with the caller's clip retained.
fn assert_balloon_paint_case(
    ctx: &egui::Context,
    fixture: &BalloonFontFixture,
    font: &egui::FontId,
    original: &egui::Galley,
    reference: &std::sync::Arc<egui::Galley>,
    width: f32,
    origin: Pos2,
) {
    let ppp = original.pixels_per_point;
    let rounding = ctx.tessellation_options(|options| options.round_text_to_pixels);
    let case = format!("family={} ppp={ppp} width={width} rounding={rounding} origin={origin:?}", fixture.family);
    let mut clip = Rect::NOTHING;
    let output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let aligned = balloon_text(ui.painter(), fixture.text.into(), (*font).clone(), Color32::BLACK, width);
        let craft_ui::text::AlignedGalley::Aligned { galley, paint_offset, reading_rect } = &aligned else { panic!("valid width must align") };
        assert_eq!(aligned.height(), original.rect.height());
        assert_eq!(*reading_rect, Rect::from_min_size(Pos2::ZERO, vec2(width, original.rect.height())));
        assert_eq!(galley.job.halign, egui::Align::Min);
        assert_eq!(galley.rect.left_top(), Pos2::ZERO);
        assert_eq!(galley.text(), fixture.text);
        assert_eq!(galley.rows.len(), original.rows.len());
        assert_eq!(galley.num_vertices, original.num_vertices);
        assert_eq!(galley.num_indices, original.num_indices);
        assert_balloon_aligned_rows(galley, *paint_offset, original, reference, &case);
        clip = ui.painter().clip_rect();
        aligned.paint(ui.painter(), origin, Color32::BLACK);
    });
    let shapes: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(shape) => Some((clipped.clip_rect, shape)),
            _ => None,
        })
        .collect();
    assert_eq!(shapes.len(), 1, "the actual helper paint emits one shape");
    assert_eq!(shapes[0].0, clip, "painting retains the caller's clip");
    let options = ctx.tessellation_options(|options| *options);
    let atlas_size = ctx.fonts(|fonts| fonts.image().size);
    let tessellate = |shape: &egui::epaint::TextShape| {
        let mut tessellator = egui::epaint::Tessellator::new(ppp, options, atlas_size, Vec::new());
        let mut mesh = egui::epaint::Mesh::default();
        tessellator.tessellate_text(shape, &mut mesh);
        mesh
    };
    let actual = tessellate(shapes[0].1);
    let expected = tessellate(&egui::epaint::TextShape::new(origin, std::sync::Arc::clone(reference), Color32::BLACK));
    assert!(!expected.vertices.is_empty(), "real font ink must be present");
    assert_eq!(actual.indices, expected.indices);
    assert_eq!(actual.vertices.len(), expected.vertices.len());
    for (actual, expected) in actual.vertices.iter().zip(&expected.vertices) {
        assert_eq!(actual.uv, expected.uv);
        assert_eq!(actual.color, expected.color);
        assert!((actual.pos - expected.pos).length() * ppp < 1e-4, "absolute physical ink must remain unchanged");
    }
    drop(shapes);
    output.drop_without_applying_deltas();
}

/// At four scales and three widths, balloon wrapping keeps the original font rows and glyph
/// meshes, preserves logical and physical ink, and keeps RTL reading origins, requiring a real
/// covering face for every fixture.
#[test]
fn balloon_wrapper_keeps_real_font_rows_ink_and_reading_origins() {
    let (fonts, fixtures) = balloon_font_fixtures();
    let mut rtl_cases = 0;
    for ppp in [1.0, 1.25, 1.5, 2.0] {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(ppp);
        ctx.set_fonts(fonts.clone());
        ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        for fixture in &fixtures {
            let probe_font = egui::FontId::new(14.0, egui::FontFamily::Name(fixture.family.into()));
            assert!(ctx.fonts_mut(|fonts| fonts.has_glyphs(&probe_font, fixture.probe)), "the selected face must actually cover {}", fixture.family);
            for width in [0.0, 23.3, 200.0] {
                let font = egui::FontId::proportional(14.0);
                assert!(
                    ctx.fonts_mut(|fonts| fixture.text.chars().filter(|c| !c.is_control()).all(|c| fonts.has_glyph(&font, c))),
                    "no missing/tofu fixture characters"
                );
                let mut job = egui::text::LayoutJob::simple(fixture.text.into(), font.clone(), Color32::BLACK, width);
                job.direction = egui::epaint::text::TextDirection::Auto;
                job.halign = egui::Align::Min;
                job.halign_min_is_start = false;
                let original = ctx.fonts_mut(|fonts| fonts.layout_job(job));
                let (reference, rtl) = balloon_reading_reference(fixture, &original, width, ppp);
                if rtl {
                    rtl_cases += 1;
                }
                for rounding in [false, true] {
                    ctx.tessellation_options_mut(|options| options.round_text_to_pixels = rounding);
                    for origin in [pos2(37.25, 19.375), pos2(0.5 / ppp - 1e-5, 0.5 / ppp)] {
                        assert_balloon_paint_case(&ctx, fixture, &font, &original, &reference, width, origin);
                    }
                }
            }
        }
    }
    assert_eq!(rtl_cases, (fixtures.len() - 1) * 12);
    eprintln!("balloon fixture coverage: {} exact embedded faces, {rtl_cases} RTL geometry cases", fixtures.len());
}

fn assert_balloon_timestamp(when: &str) {
    assert_eq!(when.len(), 16, "WordCraft comments carry a local YYYY-MM-DD HH:MM timestamp: {when:?}");
    let b = when.as_bytes();
    assert_eq!((b.get(4), b.get(7), b.get(10), b.get(13)), (Some(&b'-'), Some(&b'-'), Some(&b' '), Some(&b':')), "timestamp separators: {when:?}");
    assert!(b.iter().enumerate().all(|(i, c)| matches!(i, 4 | 7 | 10 | 13) || c.is_ascii_digit()), "timestamp fields are digits: {when:?}");
}

/// One expected read-only card line: find its text shape, prove its origin, glyph coverage and
/// paragraph reading edge, and return the next line's expected top.
fn assert_balloon_line(
    ctx: &egui::Context,
    output: &egui::FullOutput,
    card: Rect,
    family: &str,
    expected_lines: &[(&str, f32)],
    line_index: usize,
    mut y: f32,
) -> (f32, (String, f32, f32, Pos2, f32)) {
    let (text, gap) = expected_lines[line_index];
    y += gap;
    let shapes: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(shape) if shape.galley.text() == text => Some((clipped.clip_rect, shape)),
            _ => None,
        })
        .collect();
    let occurrence = expected_lines[..line_index].iter().filter(|entry| entry.0 == text).count();
    let expected_occurrences = expected_lines.iter().filter(|entry| entry.0 == text).count();
    assert_eq!(shapes.len(), expected_occurrences, "the actual read-only line must be painted once per occurrence: text={text:?} line={line_index}");
    let (clip, shape) = shapes[occurrence];
    assert_eq!(shape.galley.pixels_per_point, 1.5, "the actual card galley uses the fixture's physical scale: {text}");
    // Recreate the original shaping job independently of the wrapper.
    // Complete normalized bounds may exceed its preserved reading height.
    let original = ctx.fonts_mut(|fonts| fonts.layout_job((*shape.galley.job).clone()));
    assert_eq!(original.rows.len(), shape.galley.rows.len(), "the wrapper preserves original row count: {text}");
    for (row_index, (row, original_row)) in shape.galley.rows.iter().zip(&original.rows).enumerate() {
        assert_eq!(row.pos.y, original_row.pos.y, "the wrapper preserves original row Y: text={text:?} row={row_index}");
    }
    let reading_height = original.rect.height();
    let geometry = (text.to_string(), reading_height, shape.galley.rect.height(), shape.pos, y);
    assert!(card.contains_rect(clip) && clip.is_positive(), "bodies, timestamps and replies keep the actual card clip");
    assert!(
        (shape.pos.y - y).abs() * 1.5 <= 0.501,
        "same reading-origin Y and vertical gaps: text={text:?} line={line_index} expected={y} actual={} gap={gap}",
        shape.pos.y
    );
    assert!(shape.galley.rows.iter().any(|row| !row.visuals.mesh.vertices.is_empty()), "real glyph ink is painted");
    assert!(
        ctx.fonts_mut(|fonts| text.chars().filter(|c| !c.is_control()).all(|c| fonts.has_glyph(&egui::FontId::proportional(14.0), c))),
        "actual app fonts cover text without tofu"
    );
    let width = card.width() - 16.0;
    let origin_x = ((card.min.x + 10.0) * 1.5).round() / 1.5;
    for (row_index, row) in shape.galley.rows.iter().enumerate() {
        let expected_x = if row.rtl_paragraph { ((f64::from(width) - f64::from(row.size.x)) * 1.5).floor() as f32 / 1.5 } else { 0.0 };
        assert!(
            (shape.pos.x + row.pos.x - origin_x - expected_x).abs() * 1.5 < 1e-3,
            "same absolute paragraph reading edge: family={} text={text:?} row={row_index} ppp={} card={card:?} shape_pos={:?} row_pos={:?} origin_x={origin_x} expected_x={expected_x}",
            family,
            shape.galley.pixels_per_point,
            shape.pos,
            row.pos
        );
        assert!(shape.galley.rect.contains_rect(row.rect()));
    }
    (y + reading_height, geometry)
}

/// The read-only card paints exactly the author, local timestamp, body, reply author,
/// timestamp and reply lines, each on the real font's card geometry.
fn assert_balloon_card(app: &WordApp, ctx: &egui::Context, output: &egui::FullOutput, id: u32, reply_id: u32, fixture: &BalloonFontFixture) -> bool {
    let Some((_, card)) = app.canvas.balloon_rects.iter().find(|(i, _)| *i == id) else {
        return false;
    };
    assert_eq!(ctx.native_pixels_per_point(), Some(1.5), "the real app receives the fixture's monitor scale");
    assert_eq!(ctx.pixels_per_point(), 1.5, "Interface Auto retains the fixture's physical scale");
    assert_eq!(app.canvas.balloon_rects.len(), 1, "the reply stays in its parent's actual hit card");
    // Current main paints each comment's author, its local timestamp, then its body;
    // a reply repeats that under a 9 pt rule. Model that sequence from the
    // independently shaped originals, not from production's painted Y.
    let comment_when = crate::local_time::comment_time(&app.session.doc.comments[&id]);
    let reply_when = crate::local_time::comment_time(&app.session.doc.comments[&reply_id]);
    for when in [&comment_when, &reply_when] {
        assert_balloon_timestamp(when);
    }
    let body = fixture.text.trim();
    let reply = format!("{} (reply)", fixture.probe);
    let expected_lines = [
        ("Balloon author", 0.0),
        (comment_when.as_str(), 1.0),
        (body, 4.0),
        ("Reply author", 9.0),
        (reply_when.as_str(), 1.0),
        (reply.as_str(), 4.0),
    ];
    let mut y = card.min.y + 6.0;
    let mut line_geometry = Vec::new();
    for line_index in 0..expected_lines.len() {
        let (next_y, geometry) = assert_balloon_line(ctx, output, *card, fixture.family, &expected_lines, line_index, y);
        y = next_y;
        line_geometry.push(geometry);
    }
    assert!(
        (y + 6.0 - card.bottom()).abs() < 1e-3,
        "card height still sums original author, timestamp, body and reply heights and gaps: family={} expected_bottom={} card={card:?} delta={} lines={line_geometry:?}",
        fixture.family,
        y + 6.0,
        y + 6.0 - card.bottom()
    );
    true
}

/// Read-only comment and reply cards use the exact embedded faces and keep author, timestamp, body
/// and reply line geometry, including RTL cases, without editing the document.
#[test]
fn read_only_balloon_bodies_and_replies_keep_real_font_card_geometry() {
    let (_, fixtures) = balloon_font_fixtures();
    let db = wordcraft_fonts::FontDb::with_font_dirs(Vec::new());
    let mut rtl_cases = 0;
    for fixture in &fixtures {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        if fixture.family != "Balloon Latin" {
            let face = db.face(fixture.family, "Regular");
            assert_eq!(face.family, fixture.family, "do not accept a substitute fixture face");
            let embedded = wordcraft_fonts::CRAFT_FONTS.iter().find(|font| font.family == fixture.family && font.style == "Regular").unwrap();
            assert_eq!(face.data(), embedded.bytes);
            assert!(fixture.probe.chars().all(|c| face.covers(c)));
            app.canvas.ime_faces.push(face);
            rtl_cases += 1;
        }
        app.session.author = "Balloon author".into();
        app.run("text.insert", json!({"text": "Some text here"})).unwrap();
        app.run("select.text", json!({"text": "text"})).unwrap();
        let body = fixture.text.trim();
        let id = app.run("review.newComment", json!({"text": body})).unwrap()["id"].as_u64().unwrap() as u32;
        app.session.author = "Reply author".into();
        let reply = format!("{} (reply)", fixture.probe);
        let reply_id = app.run("review.reply", json!({"id": id, "text": reply})).unwrap()["id"].as_u64().unwrap() as u32;
        app.run("view.commentsPane", json!({"value": false})).unwrap();
        app.canvas.balloon = None;
        let rev = app.session.rev();
        let ctx = egui::Context::default();
        ctx.tessellation_options_mut(|options| options.round_text_to_pixels = true);
        let mut checked = false;
        for frame in 0..6 {
            let mut input = egui::RawInput {
                time: Some(f64::from(frame) / 60.0),
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))),
                ..Default::default()
            };
            // Interface Auto follows the monitor scale; setting only the context's
            // UI zoom would be reset by the real app before these settling frames.
            input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().native_pixels_per_point = Some(1.5);
            let output = ctx.run_ui(input, |ui| {
                app.logic(ui.ctx());
                app.ui(ui);
            });
            if frame >= 3 {
                checked |= assert_balloon_card(&app, &ctx, &output, id, reply_id, fixture);
            }
            output.drop_without_applying_deltas();
        }
        assert!(checked, "actual author, timestamp, body and reply shapes must be observed");
        assert_eq!(app.session.rev(), rev, "painting never edits the document");
        assert_eq!(app.session.doc.plain_text(StoryRef::Part(app.session.doc.comments[&id].part)), body);
        assert_eq!(app.session.doc.plain_text(StoryRef::Part(app.session.doc.comments[&reply_id].part)), reply);
    }
    assert_eq!(rtl_cases, fixtures.len() - 1);
    eprintln!("actual balloon card coverage: {} faces, {rtl_cases} RTL author/timestamp/body/reply cases", fixtures.len());
}

/// Painted balloon text is clipped inside its comment card.
#[test]
fn read_only_balloon_text_is_clipped_to_its_card() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("text.insert", json!({"text": "Some text here"})).unwrap();
    app.run("select.text", json!({"text": "text"})).unwrap();
    let body = "Balloon ink stays within its own card";
    let id = app.run("review.newComment", json!({"text": body})).unwrap()["id"].as_u64().unwrap() as u32;
    app.run("view.commentsPane", json!({"value": false})).unwrap();
    app.canvas.balloon = None;
    let ctx = egui::Context::default();
    let mut checked = false;
    for frame in 0..6 {
        let input = egui::RawInput {
            time: Some(f64::from(frame) / 60.0),
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1440.0, 900.0))),
            ..Default::default()
        };
        let out = ctx.run_ui(input, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        if let Some((_, card)) = app.canvas.balloon_rects.iter().find(|(i, _)| *i == id) {
            for clipped in &out.shapes {
                if let egui::Shape::Text(text) = &clipped.shape
                    && text.galley.text() == body
                {
                    assert!(card.contains_rect(clipped.clip_rect), "comment ink must not cross onto the page");
                    assert!(clipped.clip_rect.is_positive());
                    checked = true;
                }
            }
        }
        out.drop_without_applying_deltas();
    }
    assert!(checked, "a read-only comment body must actually be painted");
}

#[test]
fn a_balloon_edits_its_comment() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Some text here"})).unwrap();
    a.run("select.text", json!({"text": "text"})).unwrap();
    let id = a.run("review.newComment", json!({"text": "Original"})).unwrap()["id"].as_u64().unwrap() as u32;
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    let card = h.state().canvas.balloon_rects.iter().find(|(i, _)| *i == id).map(|(_, r)| *r).expect("the comment has a balloon");
    let at = card.left_top() + vec2(30.0, 8.0);
    h.event(egui::Event::PointerMoved(at));
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    for _ in 0..3 {
        h.step();
    }
    assert_eq!(h.state().canvas.balloon, Some(id), "the click selected the balloon");
    let depth = h.state().session.undo_depth();
    for piece in ["Edited", " twice"] {
        h.event(egui::Event::Text(piece.into()));
        h.step();
    }
    assert_eq!(h.state().session.undo_depth(), depth, "nothing is written while typing");
    leave_field(&mut h);
    let part = h.state().session.doc.comments[&id].part;
    let text = h.state().session.doc.plain_text(StoryRef::Part(part));
    assert!(text.contains("Edited twice") && text.contains("Original"), "{text:?}");
    assert_eq!(h.state().session.undo_depth(), depth + 1, "one undo step for the edit");
    assert!(h.state_mut().session.undo());
    assert_eq!(h.state().session.doc.plain_text(StoryRef::Part(part)), "Original");
}

/// #960: double-clicking the top margin of a document without a header creates one and
/// edits it (the bottom margin: a footer), as in Word; typing goes into it, and
/// double-clicking the body leaves it. One left empty is taken away again.
#[test]
fn double_click_margin_creates_header_and_footer() {
    use egui::{Event, PointerButton};
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Body text"})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 2200.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    let double_click = |h: &mut egui_kittest::Harness<'_, WordApp>, at: Pos2| {
        h.event(Event::PointerMoved(at));
        for _ in 0..2 {
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
            h.step();
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
            h.step();
        }
        // Let the double-click time pass before the next one.
        for _ in 0..40 {
            h.step();
        }
    };
    let kind = |h: &egui_kittest::Harness<'_, WordApp>| part_kind(h.state(), h.state().session.sel.focus.story);
    let page = h.state_mut().session.layout().pages[0].clone();
    assert!(page.header_story.is_none() && page.footer_story.is_none());
    let (x, top, bottom) = (page.body.x + 20.0, page.body.y / 2.0, (page.body.bottom() + page.h) / 2.0);
    let body = page_to_screen(h.state_mut(), 0, x, page.body.y + 6.0).expect("body on screen");
    // Top margin: a new header, the caret in it, typing goes there.
    let at = page_to_screen(h.state_mut(), 0, x, top).expect("top margin on screen");
    double_click(&mut h, at);
    assert_eq!(kind(&h), Some(PartKind::Header), "editing a new header");
    let header = h.state().session.sel.focus.story;
    h.event(Event::Text("Running head".into()));
    for _ in 0..3 {
        h.step();
    }
    assert_eq!(h.state().session.doc.plain_text(header), "Running head");
    assert_eq!(h.state().session.doc.plain_text(StoryRef::Body), "Body text");
    // Double-click the body: back in it, the header kept.
    double_click(&mut h, body);
    assert_eq!(h.state().session.sel.focus.story, StoryRef::Body);
    assert_eq!(h.state_mut().session.layout().pages[0].header_story.map(StoryRef::Part), Some(header));
    // Bottom margin: a new footer; left empty, leaving takes it away again.
    let at = page_to_screen(h.state_mut(), 0, x, bottom).expect("bottom margin on screen");
    double_click(&mut h, at);
    assert_eq!(kind(&h), Some(PartKind::Footer), "editing a new footer");
    double_click(&mut h, body);
    assert_eq!(h.state().session.sel.focus.story, StoryRef::Body);
    assert!(h.state().session.doc.last_section.footers.default.is_none(), "the empty footer is gone");
    // And a written footer stays.
    double_click(&mut h, at);
    let footer = h.state().session.sel.focus.story;
    assert_eq!(kind(&h), Some(PartKind::Footer));
    h.event(Event::Text("Page foot".into()));
    h.step();
    double_click(&mut h, body);
    assert_eq!(h.state().session.sel.focus.story, StoryRef::Body);
    assert_eq!(h.state().session.doc.plain_text(footer), "Page foot");
}

/// #2826: editing the default header, then turning on Different First Page: a double-click in
/// page 1's top margin goes into page 1's own (new) header, and typing lands there.
#[test]
fn double_click_reaches_the_first_page_header_from_another_header() {
    use egui::{Event, PointerButton};
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Body text"})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    a.run("insert.header", json!({"text": "Head"})).unwrap();
    let head = a.session.sel.focus.story;
    a.run("layout.differentFirstPage", json!({"value": true})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 2200.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    let page = h.state_mut().session.layout().pages[0].clone();
    assert!(page.header_story.is_none());
    let at = page_to_screen(h.state_mut(), 0, page.body.x + 20.0, page.body.y / 2.0).expect("top margin on screen");
    h.event(Event::PointerMoved(at));
    for _ in 0..2 {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.step();
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.step();
    }
    let first = h.state().session.sel.focus.story;
    assert_eq!(part_kind(h.state(), first), Some(PartKind::Header));
    assert_ne!(first, head, "page 1's own header, not the default one");
    h.event(Event::Text("Cover".into()));
    for _ in 0..3 {
        h.step();
    }
    assert_eq!(h.state().session.doc.plain_text(first), "Cover");
    assert_eq!(h.state_mut().session.layout().pages[0].header_story.map(StoryRef::Part), Some(first));
}

/// #1893: a single click on a shape only selects it; a double-click puts the caret in its
/// text (a shape with none gets one) and typing lands there. A line has no text to enter.
#[test]
fn double_click_a_shape_types_in_it() {
    use egui::{Event, PointerButton};
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    a.run("insert.shape", json!({"kind": "rectangle", "width": 120, "height": 80})).unwrap();
    let pos = wordcraft_engine::cmd::objects::selected(&a.session).unwrap().0;
    a.session.sel = wordcraft_engine::Selection::caret(Pos::body(0, 0));
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    let rect = h.state_mut().session.layout().object(&pos, 0).expect("the shape is laid out").rect;
    let at = crate::frame::to_screen(h.state().canvas.page_rects[0], h.state().canvas.scale, rect).center();
    let click = |h: &mut egui_kittest::Harness<'_, WordApp>, n: usize| {
        h.event(Event::PointerMoved(at));
        for _ in 0..n {
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
            h.step();
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
            h.step();
        }
    };
    click(&mut h, 1);
    let picked = wordcraft_engine::cmd::objects::object_selection(&h.state().session).map(|(p, _)| p);
    assert_eq!(picked, Some(pos.clone()), "a click selects the shape");
    assert_eq!(h.state().session.sel.focus.story, StoryRef::Body, "and doesn't enter its text");
    for _ in 0..40 {
        h.step();
    }
    click(&mut h, 2);
    assert!(in_text_box(h.state()), "the caret is in the shape's text");
    h.event(Event::Text("Hello".into()));
    for _ in 0..3 {
        h.step();
    }
    let story = h.state().session.sel.focus.story;
    assert_eq!(h.state().session.doc.plain_text(story), "Hello");
}

/// #1893: a text box so small that all of it counts as its frame still gives a double-click
/// to its text (select the word) when it has text, instead of moving the caret to the end.
#[test]
fn double_click_in_a_tiny_text_box_selects_its_word() {
    use egui::{Event, PointerButton};
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    a.run("view.zoom", json!({"value": 50})).unwrap();
    a.run("insert.textBox", json!({"text": "Hi", "width": 18, "height": 18})).unwrap();
    let pos = Pos::body(0, 0);
    a.session.sel = wordcraft_engine::Selection::caret(Pos::body(0, 0));
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    let rect = h.state_mut().session.layout().object(&pos, 0).expect("the box is laid out").rect;
    let at = crate::frame::to_screen(h.state().canvas.page_rects[0], h.state().canvas.scale, rect).center();
    h.event(Event::PointerMoved(at));
    for _ in 0..2 {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.step();
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.step();
    }
    let sel = &h.state().session.sel;
    assert_ne!(sel.anchor, sel.focus, "the double-click selected a word instead of parking the caret at the end");
}

/// #1026: while editing the header, double-clicking the bottom margin switches to the
/// footer (and back), as in Word, instead of selecting a word in the header.
#[test]
fn double_click_switches_between_header_and_footer() {
    use egui::{Event, PointerButton};
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Body text"})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 2200.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    let double_click = |h: &mut egui_kittest::Harness<'_, WordApp>, at: Pos2| {
        h.event(Event::PointerMoved(at));
        for _ in 0..2 {
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
            h.step();
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
            h.step();
        }
        for _ in 0..40 {
            h.step();
        }
    };
    let kind = |h: &egui_kittest::Harness<'_, WordApp>| part_kind(h.state(), h.state().session.sel.focus.story);
    let page = h.state_mut().session.layout().pages[0].clone();
    let (x, top, bottom) = (page.body.x + 20.0, page.body.y / 2.0, (page.body.bottom() + page.h) / 2.0);
    let head_at = page_to_screen(h.state_mut(), 0, x, top).expect("top margin on screen");
    let foot_at = page_to_screen(h.state_mut(), 0, x, bottom).expect("bottom margin on screen");
    double_click(&mut h, head_at);
    assert_eq!(kind(&h), Some(PartKind::Header), "editing a new header");
    let header = h.state().session.sel.focus.story;
    h.event(Event::Text("Head".into()));
    for _ in 0..3 {
        h.step();
    }
    // Header -> footer.
    double_click(&mut h, foot_at);
    assert_eq!(kind(&h), Some(PartKind::Footer), "header -> footer band double-click switches");
    assert!(h.state().session.sel.is_collapsed(), "no word selected");
    assert_eq!(h.state().session.doc.plain_text(header), "Head");
    h.event(Event::Text("Foot".into()));
    for _ in 0..3 {
        h.step();
    }
    let footer = h.state().session.sel.focus.story;
    // Footer -> header.
    double_click(&mut h, head_at);
    assert_eq!(h.state().session.sel.focus.story, header, "footer -> header band double-click switches");
    assert!(h.state().session.sel.is_collapsed(), "no word selected");
    assert_eq!(h.state().session.doc.plain_text(footer), "Foot");
    assert_eq!(h.state().session.doc.plain_text(header), "Head");
}

/// #513: right-clicking a picture selects it and offers Wrap Text, as in Word; choosing
/// Square there wraps text around it.
/// #1348: an app with one comment ("Original") on some text, its balloon on screen.
fn comment_harness() -> (egui_kittest::Harness<'static, WordApp>, u32) {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Some text here"})).unwrap();
    a.run("select.text", json!({"text": "text"})).unwrap();
    let id = a.run("review.newComment", json!({"text": "Original"})).unwrap()["id"].as_u64().unwrap() as u32;
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    (h, id)
}

/// Click comment `id`'s balloon (from no selected balloon), so its text has the keyboard.
fn click_balloon(h: &mut egui_kittest::Harness<'static, WordApp>, id: u32) {
    h.state_mut().canvas.balloon = None;
    for _ in 0..2 {
        h.step();
    }
    let card = h.state().canvas.balloon_rects.iter().find(|(i, _)| *i == id).map(|(_, r)| *r).expect("the comment has a balloon");
    let at = card.left_top() + vec2(30.0, 8.0);
    h.event(egui::Event::PointerMoved(at));
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    for _ in 0..3 {
        h.step();
    }
    assert_eq!(h.state().canvas.balloon, Some(id), "the click selected the balloon");
}

fn type_in(h: &mut egui_kittest::Harness<'static, WordApp>, text: &str) {
    h.event(egui::Event::Text(text.into()));
    h.step();
}

/// Leave the comment field by clicking on the page (which also leaves the balloon).
fn leave_field(h: &mut egui_kittest::Harness<'static, WordApp>) {
    let at = h.state().canvas.page_rects[0].center();
    h.event(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        h.step();
    }
    for _ in 0..3 {
        h.step();
    }
}

fn press_escape(h: &mut egui_kittest::Harness<'static, WordApp>) {
    h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
    for _ in 0..3 {
        h.step();
    }
}

fn comment_text(app: &WordApp, id: u32) -> String {
    let part = app.session.doc.comments[&id].part;
    app.session.doc.plain_text(StoryRef::Part(part))
}

/// #1348: an edit the document refuses (here read-only protection) keeps what was typed and
/// says why; once the edit is allowed, leaving the field again writes it.
#[test]
fn a_refused_comment_edit_keeps_the_draft() {
    let (mut h, id) = comment_harness();
    click_balloon(&mut h, id);
    type_in(&mut h, "Edited");
    h.state_mut().session.doc.settings.protection = Some("readOnly".into());
    leave_field(&mut h);
    assert_eq!(comment_text(h.state(), id), "Original", "the protected document refused the edit");
    let status = h.state().status_msg.as_ref().map(|(s, _)| s.clone()).unwrap_or_default();
    assert!(status.contains("protected"), "the refusal is shown: {status:?}");
    h.state_mut().session.doc.settings.protection = None;
    click_balloon(&mut h, id);
    leave_field(&mut h);
    let text = comment_text(h.state(), id);
    assert!(text.contains("Edited"), "the typed text survived the refusal: {text:?}");
}

/// #3268: clicking a balloon's Reply button, typing a reply and clicking away posts it as a
/// reply comment under the original, not a draft that silently disappears.
#[test]
fn replying_to_a_comment_from_its_balloon_posts_the_reply() {
    use egui_kittest::kittest::Queryable;
    let (mut h, id) = comment_harness();
    click_balloon(&mut h, id);
    h.get_by_label("Reply").click();
    for _ in 0..3 {
        h.step();
    }
    type_in(&mut h, "A reply");
    leave_field(&mut h);
    let replies: Vec<u32> = h.state().session.doc.comments.iter().filter(|(_, c)| c.parent == Some(id)).map(|(k, _)| *k).collect();
    assert_eq!(replies.len(), 1, "{:?}", h.state().session.doc.comments);
    assert_eq!(comment_text(h.state(), replies[0]), "A reply");
}

/// #1348: a draft typed in one document never lands in another document's comment that happens
/// to have the same id.
#[test]
fn a_comment_draft_stays_with_its_document() {
    let (mut h, id) = comment_harness();
    click_balloon(&mut h, id);
    type_in(&mut h, " stale");
    // An agent replaces the document while the balloon is still being typed in.
    h.state_mut().session.set_document(wordcraft_doc::Document::new());
    let a = h.state_mut();
    a.run("text.insert", json!({"text": "Other words here"})).unwrap();
    a.run("select.text", json!({"text": "words"})).unwrap();
    let fresh = a.run("review.newComment", json!({"text": "Fresh"})).unwrap()["id"].as_u64().unwrap() as u32;
    assert_eq!(fresh, id, "the new document reuses the comment id");
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    for _ in 0..3 {
        h.step();
    }
    click_balloon(&mut h, fresh);
    leave_field(&mut h);
    assert_eq!(comment_text(h.state(), fresh), "Fresh", "the old document's draft didn't leak in");
}

/// #1348: saving while a comment is still being typed in saves what was typed.
#[test]
fn saving_writes_a_pending_comment_draft_first() {
    let (mut h, id) = comment_harness();
    click_balloon(&mut h, id);
    type_in(&mut h, "Typed ");
    let path = std::env::temp_dir().join(format!("wordcraft-1348-save-{}.docx", std::process::id()));
    h.state_mut().run("file.save", json!({"path": path.to_string_lossy()})).unwrap();
    let text = comment_text(h.state(), id);
    assert!(text.contains("Typed"), "the draft was written before saving: {text:?}");
    assert!(!h.state().session.dirty, "and saved");
    let saved = wordcraft_engine::io::open_path(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    let part = saved.comments.values().next().expect("the comment was saved").part;
    assert!(saved.plain_text(StoryRef::Part(part)).contains("Typed"), "the file has the typed text");
}

/// #1348: closing a document whose only change is a comment still being typed asks to save.
#[test]
fn closing_with_a_pending_comment_draft_asks_to_save() {
    let (mut h, id) = comment_harness();
    h.state_mut().session.dirty = false;
    click_balloon(&mut h, id);
    type_in(&mut h, "Typed ");
    assert!(!h.state_mut().close_requested(), "the typed comment is an unsaved change");
    assert!(matches!(h.state().dialog, Some(crate::dialogs::Dialog::SaveChanges { .. })));
}

/// #1348: an agent can't close the document while comment text is still being typed unless
/// it passes `force`, as with unsaved changes. New and Open open beside it, losing nothing.
#[test]
fn an_agent_cannot_close_the_document_over_a_pending_comment_draft() {
    let (mut h, id) = comment_harness();
    h.state_mut().session.dirty = false;
    click_balloon(&mut h, id);
    type_in(&mut h, "Typed ");
    let open = json!({"path": "/nonexistent/other.docx"});
    assert!(crate::control::close_refused(h.state(), "file.open", &open).is_none());
    assert!(crate::control::close_refused(h.state(), "file.new", &json!({})).is_none());
    assert!(crate::control::close_refused(h.state(), "file.close", &json!({})).is_some());
    assert!(crate::control::close_refused(h.state(), "file.close", &json!({"force": true})).is_none());
    assert!(crate::control::close_refused(h.state(), "text.insert", &json!({"text": "x"})).is_none());
}

/// #1348: a comment edit typed, then refused (the document became read-only).
fn refused_draft() -> (egui_kittest::Harness<'static, WordApp>, u32) {
    let (mut h, id) = comment_harness();
    h.state_mut().session.dirty = false;
    click_balloon(&mut h, id);
    type_in(&mut h, "Refused ");
    h.state_mut().session.doc.settings.protection = Some("readOnly".into());
    (h, id)
}

/// #1348: a draft the document refuses doesn't keep the window open: closing asks to save,
/// and Don't Save drops the draft and closes.
#[test]
fn a_refused_comment_draft_can_be_dropped_on_close() {
    let (mut h, _) = refused_draft();
    assert!(!h.state_mut().close_requested(), "the refused draft is an unsaved change");
    assert!(matches!(h.state().dialog, Some(crate::dialogs::Dialog::SaveChanges { .. })), "the save prompt asks about it");
    h.state_mut().run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
    assert!(h.state_mut().close_requested(), "Don't Save closes");
}

/// #1348: New with a refused draft opens beside it, asking nothing; the draft waits with its
/// document and is there when that document is shown again.
#[test]
fn a_refused_comment_draft_stays_with_its_document_on_new() {
    let (mut h, _) = refused_draft();
    let document = h.state().session.document_id();
    h.state_mut().run("file.new", json!({})).unwrap();
    assert!(h.state().dialog.is_none(), "nothing is asked");
    assert_ne!(h.state().session.document_id(), document, "New made a document");
    assert!(!crate::panes::has_comment_drafts(h.state()), "the draft isn't the new document's");
    h.state_mut().run("view.switchWindows", json!({"id": document})).unwrap();
    assert!(crate::panes::has_comment_drafts(h.state()), "the draft is back with its document");
}

/// #1348: Save with a refused draft saves the rest of the document, keeps the draft and says why.
#[test]
fn saving_with_a_refused_comment_draft_saves_the_document() {
    let (mut h, id) = refused_draft();
    h.state_mut().session.dirty = true;
    let path = std::env::temp_dir().join(format!("wordcraft-1348-refused-{}.docx", std::process::id()));
    let r = h.state_mut().run("file.save", json!({"path": path.to_string_lossy()}));
    let saved = wordcraft_engine::io::open_path(&path);
    let _ = std::fs::remove_file(&path);
    assert!(r.is_ok(), "{r:?}");
    assert!(saved.is_ok(), "the document was saved");
    assert!(!h.state().session.dirty);
    assert!(crate::panes::comment_draft(h.state(), id).is_some_and(|d| d.contains("Refused")), "the draft stays");
    let status = h.state().status_msg.as_ref().map(|(s, _)| s.clone()).unwrap_or_default();
    assert!(status.contains("protected"), "the refusal is shown: {status:?}");
}

/// #1348: in a read-only document the comment fields can't be typed in.
#[test]
fn comment_fields_are_read_only_where_comments_cant_change() {
    let (mut h, id) = comment_harness();
    h.state_mut().session.doc.settings.protection = Some("readOnly".into());
    h.step();
    click_balloon(&mut h, id);
    type_in(&mut h, "Typed ");
    assert_eq!(crate::panes::comment_draft(h.state(), id), None, "nothing was typed");
    assert_eq!(comment_text(h.state(), id), "Original");
}

/// #1348: Escape in a comment field throws the draft away and shows the comment's text again.
#[test]
fn escape_reverts_a_comment_draft() {
    let (mut h, id) = comment_harness();
    click_balloon(&mut h, id);
    type_in(&mut h, "Discarded ");
    press_escape(&mut h);
    assert_eq!(crate::panes::comment_draft(h.state(), id), None, "the draft is gone");
    assert_eq!(comment_text(h.state(), id), "Original", "nothing was written");
    leave_field(&mut h);
    assert_eq!(comment_text(h.state(), id), "Original", "and nothing is written later");
}

/// #513: right-clicking a picture selects it and offers Wrap Text, as in Word; choosing
/// Square there wraps text around it.
#[test]
fn right_click_on_a_picture_offers_wrap_text() {
    use egui_kittest::kittest::Queryable;
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Text before the picture "})).unwrap();
    let mut png = Vec::new();
    let img = image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 30, 30, 255]));
    image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    a.run("insert.picture", json!({"data": wordcraft_engine::cmd::insert::base64_encode(&png), "width": 160})).unwrap();
    let pos = wordcraft_engine::cmd::objects::selected(&a.session).unwrap().0;
    a.session.sel = wordcraft_engine::Selection::caret(Pos::body(0, 0));
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    let rect = h.state_mut().session.layout().object(&pos, 0).expect("the picture is laid out").rect;
    let at = crate::frame::to_screen(h.state().canvas.page_rects[0], h.state().canvas.scale, rect).center();
    h.event(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
        h.step();
    }
    h.step();
    let picked = wordcraft_engine::cmd::objects::object_selection(&h.state().session).map(|(p, _)| p);
    assert_eq!(picked, Some(pos.clone()), "the right-click selected the picture");
    h.get_by_label_contains("Wrap Text").hover();
    for _ in 0..3 {
        h.step();
    }
    assert!(h.query_by_label_contains("Fix Position on Page").is_some(), "the anchoring choices are offered");
    h.get_by_label_contains("Square").click();
    for _ in 0..3 {
        h.step();
    }
    let o = wordcraft_engine::cmd::objects::selected(&h.state().session).unwrap().1;
    assert_eq!(o.frame().unwrap().2.wrap, wordcraft_doc::para::Wrap::Square);
}

/// #1500: right-clicking a selected shape offers Format Shape…, which opens the Format Shape
/// pane and leaves that shape selected.
#[test]
fn right_click_on_a_shape_opens_the_format_shape_pane() {
    use egui_kittest::kittest::Queryable;
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Text before the shape "})).unwrap();
    a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
    let pos = wordcraft_engine::cmd::objects::selected(&a.session).unwrap().0;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    assert!(crate::widgets::enabled(h.state(), "shape.fill"), "Shape Fill applies to the selected shape");
    assert!(!h.state().session.view.format_shape_pane, "the pane starts closed");
    let rect = h.state_mut().session.layout().object(&pos, 0).expect("the shape is laid out").rect;
    let at = crate::frame::to_screen(h.state().canvas.page_rects[0], h.state().canvas.scale, rect).center();
    // A menu item's label ends with its (empty) shortcut column; the ribbon's own Format
    // Shape… launcher has none, so this names the context menu's entry only.
    let entry = "Format Shape… ";
    assert!(h.query_by_label(entry).is_none(), "no context menu yet");
    h.event(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
        h.step();
    }
    h.step();
    h.get_by_label(entry).click();
    for _ in 0..3 {
        h.step();
    }
    assert!(h.state().session.view.format_shape_pane, "Format Shape… opened the pane");
    let picked = wordcraft_engine::cmd::objects::object_selection(&h.state().session).map(|(p, _)| p);
    assert_eq!(picked, Some(pos), "the same shape stays selected");
}

/// #1197: the context menu on a table of contents entry offers Update Field, as in Word, and
/// it brings the table up to date.
#[test]
fn context_menu_on_a_toc_entry_updates_the_table() {
    use egui_kittest::kittest::Queryable;
    use wordcraft_doc::{Block, Paragraph, Path, props::CharProps};
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::report()), Default::default());
    a.run("caret.docStart", json!({})).unwrap();
    a.run("references.toc", json!({})).unwrap();
    let end = a.session.doc.body.len();
    let heading = Paragraph::with_text("Fresh heading", CharProps::default()).styled("Heading1");
    a.session.doc.insert_block(StoryRef::Body, &Path::top(end), Block::Para(heading)).unwrap();
    a.session.relayout();
    let entry = a.session.doc.body.iter().position(|b| b.as_para().is_some_and(|p| p.props.style.as_deref() == Some("TOC1"))).expect("a TOC entry");
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    let c = h.state_mut().session.layout().caret(&Pos::body(entry, 2)).expect("the entry is laid out");
    let at = page_to_screen(h.state_mut(), c.page, c.x + 1.0, c.top + c.height / 2.0).expect("on screen");
    h.event(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
        h.step();
    }
    h.step();
    assert!(h.query_all_by_label_contains("Paste").next().is_some(), "the context menu is open");
    h.get_by_label_contains("Update Field").click();
    for _ in 0..3 {
        h.step();
    }
    let text = h.state().session.doc.plain_text(StoryRef::Body);
    assert_eq!(text.matches("Fresh heading").count(), 2, "the new heading is listed in the table: {text}");
}

/// #2179: the context menu on a table of contents entry also offers Remove Table of Contents.
#[test]
fn context_menu_on_a_toc_entry_removes_the_table() {
    use egui_kittest::kittest::Queryable;
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::report()), Default::default());
    a.run("caret.docStart", json!({})).unwrap();
    a.run("references.toc", json!({})).unwrap();
    let is_entry = |b: &wordcraft_doc::Block| b.as_para().is_some_and(|p| p.props.style.as_deref() == Some("TOC1"));
    let entry = a.session.doc.body.iter().position(|b| is_entry(b)).expect("a TOC entry");
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    let c = h.state_mut().session.layout().caret(&Pos::body(entry, 2)).expect("the entry is laid out");
    let at = page_to_screen(h.state_mut(), c.page, c.x + 1.0, c.top + c.height / 2.0).expect("on screen");
    h.event(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
        h.step();
    }
    h.step();
    h.get_by_label_contains("Remove Table of Contents").click();
    for _ in 0..3 {
        h.step();
    }
    assert!(!h.state().session.doc.body.iter().any(|b| is_entry(b)), "the table is gone");
}

/// #2182: after Update Field has refreshed a table of contents once, the context menu on it
/// still offers Update Field, so a later heading change can be brought in again.
#[test]
fn context_menu_on_a_toc_still_offers_update_field_after_an_update() {
    use egui_kittest::kittest::Queryable;
    use wordcraft_doc::{Block, Paragraph, Path, props::CharProps};
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::report()), Default::default());
    a.run("caret.docStart", json!({})).unwrap();
    a.run("references.toc", json!({})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for round in 0..2 {
        let name = format!("Fresh heading {round}");
        let end = h.state().session.doc.body.len();
        let heading = Paragraph::with_text(&name, CharProps::default()).styled("Heading1");
        h.state_mut().session.doc.insert_block(StoryRef::Body, &Path::top(end), Block::Para(heading)).unwrap();
        h.state_mut().session.relayout();
        for _ in 0..6 {
            h.step();
        }
        let entry = h
            .state()
            .session
            .doc
            .body
            .iter()
            .position(|b| b.as_para().is_some_and(|p| p.props.style.as_deref() == Some("TOC1")))
            .expect("a TOC entry");
        let c = h.state_mut().session.layout().caret(&Pos::body(entry, 2)).expect("the entry is laid out");
        let at = page_to_screen(h.state_mut(), c.page, c.x + 1.0, c.top + c.height / 2.0).expect("on screen");
        h.event(egui::Event::PointerMoved(at));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
            h.step();
        }
        h.step();
        assert!(h.query_all_by_label_contains("Paste").next().is_some(), "round {round}: the context menu is open");
        assert!(h.query_by_label_contains("Update Field").is_some(), "round {round}: Update Field is offered");
        h.get_by_label_contains("Update Field").click();
        for _ in 0..3 {
            h.step();
        }
        let text = h.state().session.doc.plain_text(StoryRef::Body);
        assert_eq!(text.matches(&name).count(), 2, "round {round}: the new heading is listed: {text}");
    }
}

/// #2583: the context menu offers the ribbon's Paste menu, item for item, and each pastes its
/// own way: Paste keeps the copied bold and colour (Word's Keep Source Formatting), Merge
/// Formatting keeps only the bold, Keep Text Only neither; Paste Special… opens its dialog.
#[test]
fn context_menu_offers_the_ribbons_paste_items() {
    use egui_kittest::kittest::Queryable;
    use wordcraft_doc::props::{Rgb, TextColor};
    let red = Some(TextColor::Rgb(Rgb(0xC0, 0, 0)));
    let items = [
        ("Paste", "edit.paste"),
        ("Keep Text Only", "edit.pasteText"),
        ("Merge Formatting", "edit.pasteMerge"),
        ("Paste Special…", "edit.pasteSpecial"),
    ];
    for (choice, pasted) in
        [("Paste", Some((true, red))), ("Merge Formatting", Some((true, None))), ("Keep Text Only", Some((false, None))), ("Paste Special…", None)]
    {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Bold\nTarget")), Default::default());
        a.session
            .doc
            .format_range(&Pos::body(0, 0), &Pos::body(0, 4), &|c| {
                c.bold = Some(true);
                c.color = red;
            })
            .unwrap();
        a.session.relayout();
        a.run("select.text", json!({"text": "Bold"})).unwrap();
        a.run("edit.copy", json!({})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        let c = h.state_mut().session.layout().caret(&Pos::body(1, 6)).expect("laid out");
        let at = page_to_screen(h.state_mut(), c.page, c.x, c.top + c.height / 2.0).expect("on screen");
        h.event(egui::Event::PointerMoved(at));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
            h.step();
        }
        h.step();
        // Each item reads as its name and its shortcut (none for Merge Formatting).
        let labels: Vec<String> = items.iter().map(|(name, id)| format!("{name} {}", crate::widgets::shortcut_text(h.state(), id))).collect();
        for l in &labels {
            assert!(h.query_by_label(l).is_some(), "{choice}: {l:?} is offered");
        }
        let k = items.iter().position(|(name, _)| *name == choice).unwrap();
        h.get_by_label(&labels[k]).click();
        for _ in 0..3 {
            h.step();
        }
        match pasted {
            Some((bold, colour)) => {
                let doc = &h.state().session.doc;
                assert_eq!(doc.plain_text(StoryRef::Body), "Bold\nTargetBold", "{choice}: pasted at the end of the second paragraph");
                let p = doc.para_at(&Pos::body(1, 0)).expect("the target paragraph");
                assert_eq!(p.props_of_char(7).bold == Some(true), bold, "{choice}: bold");
                assert_eq!(p.props_of_char(7).color, colour, "{choice}: colour");
            }
            None => {
                assert!(matches!(h.state().dialog, Some(crate::dialogs::Dialog::PasteSpecial { .. })), "{choice}: the dialog opens");
            }
        }
    }
}

/// Word's frame around the equation being edited: as wide as the equation plus the options
/// strip at its right, as tall as its line, with the grip over its top-left corner. The
/// strip and grip keep their screen size at every zoom.
#[test]
fn the_equation_frame_holds_the_equation_and_its_strip() {
    let math = wordcraft_layout::math::MathLayout { width: 40.0, ascent: 10.0, descent: 5.0, ..Default::default() };
    let bx = wordcraft_layout::hit::EquationBox { page: 0, x: 100.0, base: 90.0, top: 76.0, bottom: 98.0, math: std::sync::Arc::new(math) };
    let page = Rect::from_min_size(pos2(10.0, 20.0), vec2(600.0, 800.0));
    for scale in [1.0, 2.5] {
        let f = equation_frame(page, &bx, scale);
        let (left, right) = (10.0 + 100.0 * scale, 10.0 + 140.0 * scale);
        assert!(f.outline.min.x < left && f.strip.min.x > right && f.strip.max.x == f.outline.max.x, "{f:?}");
        assert!(f.outline.min.y < 20.0 + 76.0 * scale && f.outline.max.y > 20.0 + 98.0 * scale, "{f:?}");
        assert_eq!((f.strip.min.y, f.strip.max.y), (f.outline.min.y, f.outline.max.y));
        assert!(f.grip.contains(f.outline.left_top() + vec2(1.0, 1.0)), "{f:?}");
        assert_eq!(f.strip.width(), 10.0);
    }
}

/// #996: on macOS a Control-click is the right-click (the trackpad's and the one-button
/// mouse's way to it), so it opens the same menu and leaves the caret where the menu needs it.
#[cfg(target_os = "macos")]
#[test]
fn control_click_on_macos_opens_the_context_menu() {
    use egui_kittest::kittest::Queryable;
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "The quick brwn fox"})).unwrap();
    a.session.sel = wordcraft_engine::Selection::caret(Pos::body(0, 12));
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    let (p, ht) = caret_screen(h.state_mut()).unwrap();
    let at = p + egui::vec2(0.0, ht / 2.0);
    h.state_mut().session.sel = wordcraft_engine::Selection::caret(Pos::body(0, 0));
    h.step();
    let ctrl = egui::Modifiers { ctrl: true, ..Default::default() };
    // The events winit delivers for a Control-click, through the app's input hook as eframe runs it.
    let send = |h: &mut egui_kittest::Harness<'_, WordApp>, events: Vec<egui::Event>| {
        let mut raw = egui::RawInput { events, ..Default::default() };
        let ctx = h.ctx.clone();
        h.state_mut().raw_input_hook(&ctx, &mut raw);
        for e in raw.events {
            h.event(e);
        }
        h.step();
    };
    send(&mut h, vec![egui::Event::ModifiersChanged(ctrl), egui::Event::PointerMoved(at)]);
    for pressed in [true, false] {
        send(&mut h, vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: ctrl }]);
    }
    h.event(egui::Event::ModifiersChanged(Default::default()));
    h.step();
    assert!(h.state().canvas.context_menu_open, "the Control-click opened the menu");
    h.get_by_label("brown").click();
    for _ in 0..3 {
        h.step();
    }
    assert_eq!(h.state().session.doc.plain_text(StoryRef::Body), "The quick brown fox");
}

/// #3340: review.spelling (the Review tab's Spelling & Grammar button and the Home tab's
/// Editor button both run it) only moved the selection to the next issue — with no way to see
/// or apply a suggestion from there, just the generic selection toolbar, it looked broken.
/// Running it now also opens the same suggestions menu Shift+F10 and a Control-click already
/// open for the word at the caret.
#[test]
fn spelling_and_grammar_opens_suggestions_for_the_issue_it_selects() {
    use egui_kittest::kittest::Queryable;
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "The quick brwn fox"})).unwrap();
    a.session.sel = wordcraft_engine::Selection::caret(Pos::body(0, 0));
    a.run("review.spelling", json!({})).unwrap();
    assert!(a.canvas.context_menu_from_keys, "queues the open the context menu already uses for Shift+F10");
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    assert!(h.state().canvas.context_menu_open, "Spelling & Grammar opens the suggestions menu");
    h.get_by_label("brown").click();
    for _ in 0..3 {
        h.step();
    }
    assert_eq!(h.state().session.doc.plain_text(StoryRef::Body), "The quick brown fox");
}

#[test]
fn floating_picture_can_be_dragged_in_front_of_or_behind_text() {
    for wrap in ["inFrontOfText", "behindText"] {
        drag_floating_picture(wrap, false, true);
    }
}

#[test]
fn floating_header_picture_can_be_dragged_in_front_of_or_behind_text() {
    for wrap in ["inFrontOfText", "behindText"] {
        drag_floating_picture(wrap, true, true);
    }
}

/// A Behind Text picture pressed over the text it sits behind, with no object selected: the
/// press becomes a drag (moving it), while a plain click still edits that text.
#[test]
fn behind_picture_dragged_over_text_without_being_selected_first() {
    drag_floating_picture("behindText", false, false);
    drag_floating_picture("behindText", true, false);
}

fn drag_floating_picture(wrap: &str, header: bool, preselect: bool) {
    use egui::{Event, PointerButton};
    {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        if header {
            a.run("insert.editHeader", json!({})).unwrap();
        }
        a.run("text.insert", json!({"text": "The picture overlaps this line of text."})).unwrap();
        let mut png = Vec::new();
        let img = image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 30, 30, 255]));
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        a.run("insert.picture", json!({"data": wordcraft_engine::cmd::insert::base64_encode(&png), "width": 160})).unwrap();
        let pos = wordcraft_engine::cmd::objects::selected(&a.session).unwrap().0;
        a.run("arrange.bounds", json!({"x": 120, "y": 80})).unwrap();
        a.run("arrange.wrap", json!({"wrap": wrap})).unwrap();
        let before = a.session.layout().object(&pos, 0).unwrap().rect;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        let page = h.state().canvas.page_rects[0];
        let scale = h.state().canvas.scale;
        let frame = crate::frame::to_screen(page, scale, before);
        // The top edge crosses the document's first text line, so Behind Text must still be
        // draggable where its pixels overlap body text. Stay away from resize handles.
        let from = pos2(frame.left() + frame.width() * 0.25, frame.top() + 5.0 * scale);
        let to = from + vec2(50.0, 40.0);
        let button = |at, pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        if wrap == "behindText" {
            // A click without drag threshold still reaches the text above the picture.
            h.event(Event::PointerMoved(from));
            h.event(button(from, true));
            h.step();
            h.event(button(from, false));
            for _ in 0..3 {
                h.step();
            }
            assert!(wordcraft_engine::cmd::objects::object_selection(&h.state().session).is_none(), "a click still selects text");
            if preselect {
                let end = Pos { off: pos.off + wordcraft_doc::para::OBJ.len_utf8(), ..pos.clone() };
                h.state_mut().run("select.range", json!({"anchor": pos, "focus": end})).unwrap();
            }
            for _ in 0..40 {
                h.step();
            }
        }
        h.event(Event::PointerMoved(from));
        h.event(button(from, true));
        h.step();
        assert!(h.state().canvas.obj_drag.is_some(), "{wrap}: pressing the image starts a frame drag");
        for k in 1..=4 {
            h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
            h.step();
        }
        h.event(button(to, false));
        for _ in 0..3 {
            h.step();
        }
        let picked = wordcraft_engine::cmd::objects::object_selection(&h.state().session).map(|(p, _)| p);
        let moved = picked.and_then(|p| h.state_mut().session.layout().object(&p, 0)).map(|o| o.rect);
        assert!(moved.is_some_and(|after| (after.x - before.x).abs() > 1.0 || (after.y - before.y).abs() > 1.0), "{wrap}: {before:?} -> {moved:?}");
    }
}

#[test]
fn only_web_and_email_links_are_followed() {
    for ok in ["https://example.com/a?b#c", "http://example.com", "HTTPS://EXAMPLE.COM", "mailto:someone@example.com"] {
        assert!(is_followable_link(ok), "{ok}");
    }
    for bad in [
        "file:///etc/passwd",
        "file://C:/Windows/System32/calc.exe",
        "C:\\Windows\\System32\\calc.exe",
        "/usr/bin/xterm",
        "javascript:alert(1)",
        "ms-msdt:/id",
        "smb://host/share",
        " https://example.com",
        "\thttps://example.com",
        "https\u{0}://example.com",
        "",
        "example.com",
    ] {
        assert!(!is_followable_link(bad), "{bad:?}");
    }
}

/// Ctrl+click on a link to a bookmark puts the caret at the bookmark's start, as Word does;
/// only Go To selects the bookmark's text (#1236).
#[test]
fn a_link_to_a_bookmark_puts_the_caret_at_its_start() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Alpha beta gamma\nSee here")), Default::default());
    a.run("select.text", json!({"text": "beta"})).unwrap();
    a.run("insert.bookmark", json!({"name": "Spot"})).unwrap();
    a.run("select.text", json!({"text": "here"})).unwrap();
    a.run("insert.link", json!({"url": "#Spot"})).unwrap();
    a.run("caret.docEnd", json!({})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    steps(&mut h, 6);
    // Between the link's "h" and "e".
    let c = h.state_mut().session.layout().caret(&Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(1), off: 5 }).unwrap();
    let at = screen(&mut h, c.x, c.top + c.height / 2.0);
    h.event(egui::Event::PointerMoved(at));
    h.step();
    let press = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::COMMAND };
    h.event_modifiers(press, egui::Modifiers::COMMAND);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    steps(&mut h, 3);
    let s = &h.state().session;
    let (start, _) = wordcraft_engine::cmd::insert::bookmark_range(&s.doc, "Spot").unwrap();
    assert!(s.sel.is_collapsed(), "a caret, not a selection: {:?}", s.sel);
    assert_eq!(s.sel.focus, start, "at the bookmark's start");
}

/// #2945: only a `REF` or `PAGEREF` with `\h` and a bookmark makes a link.
#[test]
fn only_hyperlinked_reference_fields_make_links() {
    let field = |instr: &str| wordcraft_doc::InlineObject::Field {
        instr: instr.into(),
        result: "x".into(),
        locked: false,
        dirty: false,
        code: false,
        form: String::new(),
    };
    assert_eq!(field_link(&field(r"REF _Ref000000001 \h")).as_deref(), Some("#_Ref000000001"));
    assert_eq!(field_link(&field(r"PAGEREF Spot \h")).as_deref(), Some("#Spot"));
    assert_eq!(field_link(&field(r"ref Spot \* MERGEFORMAT \H")).as_deref(), Some("#Spot"));
    for plain in [r"REF Spot", r"REF \h", "REF", "", r"PAGE \h", r"TOC \h", r"SEQ Figure \h"] {
        assert_eq!(field_link(&field(plain)), None, "{plain:?}");
    }
}

/// #2945: a cross-reference (`REF … \h`) is a link to its target: Ctrl+click on it goes there,
/// and its hover tip names no bookmark.
#[test]
fn ctrl_click_on_a_cross_reference_goes_to_its_target() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Rock\nSee ")), Default::default());
    a.run("select.text", json!({"text": "Rock"})).unwrap();
    a.run("references.caption", json!({"label": "Figure", "text": "A rock"})).unwrap();
    a.run("caret.docEnd", json!({})).unwrap();
    a.run("insert.crossReference", json!({"to": "figure", "target": "Figure 1", "show": "number"})).unwrap();
    let caption = a.session.doc.body.iter().position(|b| b.as_para().is_some_and(|p| p.props.style.as_deref() == Some("Caption"))).unwrap();
    let last = a.session.doc.body.len() - 1;
    assert_ne!(caption, last);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    steps(&mut h, 6);
    // The field sits after "See ": click between its two edges.
    let obj = wordcraft_doc::para::OBJ.len_utf8();
    let (before, after) = {
        let l = h.state_mut().session.layout();
        let pos = |off| Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(last), off };
        (l.caret(&pos(4)).unwrap(), l.caret(&pos(4 + obj)).unwrap())
    };
    let at = screen(&mut h, (before.x + after.x) / 2.0, before.top + before.height / 2.0);
    h.event(egui::Event::PointerMoved(at));
    h.step();
    let press = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::COMMAND };
    h.event_modifiers(press, egui::Modifiers::COMMAND);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    steps(&mut h, 3);
    let s = &h.state().session;
    assert_eq!(s.sel.focus.path.last(), caption, "the caret went to the caption: {:?}", s.sel);
}

/// #3368: right-clicking a link dismisses the hover tooltip. A tooltip draws above the
/// context menu and covered its first items (Cut, Copy, Paste, Font…), so the menu looked
/// truncated; the tooltip must stand down while the menu owns the pointer.
#[test]
fn right_clicking_a_link_hides_the_hover_tooltip() {
    use egui_kittest::kittest::Queryable;
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("See here")), Default::default());
    a.run("select.text", json!({"text": "here"})).unwrap();
    a.run("insert.link", json!({"url": "https://example.com/very/long/tracking/url?with=parameters&and=still-more"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    steps(&mut h, 6);
    // Between the link's "h" and "e".
    let c = h.state_mut().session.layout().caret(&Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 5 }).unwrap();
    let at = screen(&mut h, c.x, c.top + c.height / 2.0);
    h.event(egui::Event::PointerMoved(at));
    steps(&mut h, 2);
    let tips = |h: &egui_kittest::Harness<'_, WordApp>| h.query_all_by_label_contains("Click to follow link").count();
    assert!(tips(&h) > 0, "hovering the link shows its target");
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
        h.step();
    }
    steps(&mut h, 2);
    assert!(h.state().canvas.context_menu_open, "the context menu is open");
    assert_eq!(tips(&h), 0, "the link tooltip is gone, so it can't cover the menu");
}

/// #688: the drag auto-scroll waits in the middle, ramps in from both edges and never leaves
/// the scrollable range.
#[test]
fn edge_auto_scroll_ramps_from_the_margins_and_clamps() {
    let dt = 1.0 / 60.0;
    let (lo, hi) = (100.0, 500.0);
    // Nothing while the pointer is inside, including just within the margin.
    for y in [(lo + hi) / 2.0, lo + EDGE_MARGIN, hi - EDGE_MARGIN] {
        assert_eq!(edge_scroll(y, lo, hi, dt), 0.0, "y = {y}");
    }
    // Direction: above the top edge scrolls back (positive), below the bottom scrolls on
    // (negative), in the wheel's convention.
    assert!(edge_scroll(lo + 1.0, lo, hi, dt) > 0.0);
    assert!(edge_scroll(hi - 1.0, lo, hi, dt) < 0.0);
    // The ramp grows monotonically from the margin to the edge and caps past it.
    let above: Vec<f32> = (0..=8).map(|k| edge_scroll(lo + EDGE_MARGIN - k as f32 * EDGE_MARGIN / 8.0, lo, hi, dt)).collect();
    assert_eq!(above[0], 0.0);
    assert!(above.windows(2).all(|w| w[0] < w[1] && w[1] > 0.0), "{above:?}");
    assert_eq!(above[8], EDGE_SPEED_MAX * dt, "the hard edge scrolls at full speed");
    assert_eq!(edge_scroll(lo - 3.0 * EDGE_MARGIN, lo, hi, dt), above[8], "past the edge the speed caps");
    let below: Vec<f32> = (0..=4).map(|k| edge_scroll(hi - EDGE_MARGIN + k as f32 * EDGE_MARGIN / 4.0, lo, hi, dt)).collect();
    assert_eq!(below[0], 0.0);
    assert!(below.windows(2).all(|w| w[0] > w[1] && w[1] < 0.0), "{below:?}");
    assert_eq!(below[4], -EDGE_SPEED_MAX * dt);
    assert_eq!(edge_scroll(hi + 3.0 * EDGE_MARGIN, lo, hi, dt), below[4], "past the edge the speed caps");
    // A stalled frame doesn't jump the page.
    assert!(edge_scroll(hi - 1.0, lo, hi, 100.0) <= EDGE_SPEED_MAX * EDGE_MAX_DT + 1e-3);
    // Junk in, nothing out; neither an empty (or inverted) band.
    for (p, lo, hi, dt) in
        [(f32::NAN, lo, hi, dt), (200.0, f32::NAN, hi, dt), (200.0, lo, f32::INFINITY, dt), (200.0, lo, hi, f32::NAN), (200.0, 500.0, 100.0, dt)]
    {
        assert_eq!(edge_scroll(p, lo, hi, dt), 0.0);
    }

    // Applying it clamps to the scrollable range; a document that fits never scrolls.
    let max = vec2(0.0, 400.0);
    assert_eq!(clamped_scroll(egui::Vec2::ZERO, vec2(0.0, 50.0), max).y, 0.0, "never past the top");
    assert_eq!(clamped_scroll(vec2(0.0, 400.0), vec2(0.0, -50.0), max).y, 400.0, "never past the bottom");
    assert_eq!(clamped_scroll(vec2(0.0, 300.0), vec2(0.0, 50.0), max).y, 250.0);
    assert_eq!(clamped_scroll(egui::Vec2::ZERO, vec2(0.0, 50.0), egui::Vec2::ZERO).y, 0.0, "nothing to scroll");
}

/// #1065: a middle click starts scrolling, the pointer below the click scrolls down on its
/// own frame after frame, and a second middle click stops it; a primary click stops it without
/// moving the caret. Held down and dragged, letting go stops it.
#[test]
fn middle_click_scrolls_toward_the_pointer_until_clicked_again() {
    let text = vec!["A line of text in a long document."; 400].join("\n");
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text(&text)), Default::default());
    a.canvas.auto_scroll.supported = true;
    let mut h: H = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(6);
    let anchor = h.state().canvas.viewport.expect("the canvas is shown").center();
    let button = |h: &mut H, at: Pos2, button, pressed| {
        h.event(egui::Event::PointerButton { pos: at, button, pressed, modifiers: Default::default() });
        h.step();
    };
    let offset = |h: &H| h.state().canvas.scroll_offset.y;
    let below = anchor + vec2(0.0, 150.0);

    h.event(egui::Event::PointerMoved(anchor));
    button(&mut h, anchor, egui::PointerButton::Middle, true);
    button(&mut h, anchor, egui::PointerButton::Middle, false);
    h.run_steps(3);
    assert!(h.state().canvas.auto_scroll.active());
    assert_eq!(offset(&h), 0.0, "the pointer at the click doesn't scroll");
    h.event(egui::Event::PointerMoved(below));
    h.run_steps(10);
    let scrolled = offset(&h);
    assert!(scrolled > 50.0, "the pointer below the click scrolls down: {scrolled}");
    button(&mut h, below, egui::PointerButton::Middle, true);
    button(&mut h, below, egui::PointerButton::Middle, false);
    let stopped = offset(&h);
    h.run_steps(10);
    assert!(!h.state().canvas.auto_scroll.active());
    assert_eq!(offset(&h), stopped, "a second click stops it");

    // A primary click stops it too, and isn't a click on the page.
    let caret = h.state().session.sel.clone();
    button(&mut h, anchor, egui::PointerButton::Middle, true);
    button(&mut h, anchor, egui::PointerButton::Middle, false);
    assert!(h.state().canvas.auto_scroll.active());
    button(&mut h, anchor, egui::PointerButton::Primary, true);
    button(&mut h, anchor, egui::PointerButton::Primary, false);
    h.run_steps(2);
    assert!(!h.state().canvas.auto_scroll.active());
    assert_eq!(h.state().session.sel, caret, "the stopping click leaves the caret");

    // Held down and dragged: scrolls while held, stops on release.
    h.event(egui::Event::PointerMoved(anchor));
    button(&mut h, anchor, egui::PointerButton::Middle, true);
    h.event(egui::Event::PointerMoved(below));
    h.run_steps(5);
    let held = offset(&h);
    assert!(held > stopped, "scrolls while the wheel is held");
    button(&mut h, below, egui::PointerButton::Middle, false);
    let released = offset(&h);
    h.run_steps(10);
    assert!(!h.state().canvas.auto_scroll.active());
    assert_eq!(offset(&h), released, "letting go after a drag stops it");
}

/// A right-click at `at`, the button pressed and released a frame apart.
fn right_click(h: &mut H, at: Pos2) {
    h.event(egui::Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
        h.step();
    }
    steps(h, 2);
}

/// Where the open context menu's entry holding `label` is (an entry's label also has its
/// shortcut or submenu arrow, and the ribbon above the page may show the same name).
fn menu_entry(h: &H, label: &str) -> Option<Pos2> {
    use egui_kittest::kittest::Queryable;
    let top = h.state().canvas.page_rects.first().map_or(0.0, |r| r.min.y);
    h.query_all_by_label_contains(label).map(|n| n.rect()).find(|r| r.min.y >= top).map(|r| r.center())
}

/// Picks `path` in the open context menu with the mouse: over each submenu on the way, then a
/// click on the last entry.
fn pick(h: &mut H, path: &[&str]) {
    let Some((last, submenus)) = path.split_last() else { return };
    for label in submenus {
        let at = menu_entry(h, label).expect(label);
        h.event(egui::Event::PointerMoved(at));
        steps(h, 2);
    }
    let at = menu_entry(h, last).expect(last);
    h.event(egui::Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        h.step();
    }
    steps(h, 2);
}

/// Whether the open context menu has an entry holding each of `labels` (`true`) or none
/// (`false`).
fn assert_menu(h: &H, labels: &[&str], there: bool) {
    for label in labels {
        assert_eq!(menu_entry(h, label).is_some(), there, "{label} in the menu: {there}");
    }
}

/// Right-clicking a table cell puts the caret there, and the table's items follow what is
/// selected, as in Word: with the caret in a cell, Split Cells; with the whole table,
/// Distribute, AutoFit and Insert Caption… (never Select or Cell Alignment). Delete is there
/// for both.
#[test]
fn the_context_menu_in_a_table_follows_the_selection() {
    let mut h = table_window();
    let b = cell(&mut h, 1, 2);
    let at = screen(&mut h, b.x + b.w / 2.0, b.y + b.h / 2.0);
    right_click(&mut h, at);
    assert_eq!(h.state().session.sel.focus.path.cell().map(|(_, r, c)| (r, c)), Some((1, 2)), "the caret moved to the cell");
    assert_menu(&h, &["Insert", "Delete", "Split Cells", "Text Direction", "Table Properties…"], true);
    assert_menu(&h, &["Merge Cells", "Distribute Rows", "AutoFit", "Select", "Cell Alignment"], false);
    let depth = h.state().session.undo_depth();
    pick(&mut h, &["Insert", "Insert Rows Below"]);
    assert_eq!(h.state().session.doc.body.iter().find_map(|b| b.as_table()).map(|t| t.rows.len()), Some(3));
    assert_eq!(h.state().session.undo_depth(), depth + 1);
    h.state_mut().run("table.selectTable", json!({})).unwrap();
    right_click(&mut h, at);
    assert!(h.state().session.cell_range().is_some(), "a right-click on the selected table keeps it");
    assert_menu(&h, &["Delete", "Distribute Rows", "Distribute Columns", "AutoFit", "Insert Caption…", "Table Properties…"], true);
    assert_menu(&h, &["Split Cells", "Merge Cells"], false);
    pick(&mut h, &["Delete", "Delete Table"]);
    assert!(h.state().session.doc.body.iter().all(|b| b.as_table().is_none()));
}

/// #1230: right-clicking a picture offers Insert Caption…, as in Word. For a picture in a
/// table cell the dialog proposes a Figure caption (not Table), and the caption goes under
/// the picture in that cell.
#[test]
fn right_click_on_a_picture_offers_insert_caption() {
    let mut h = table_window();
    h.state_mut().run("caret.end", json!({})).unwrap();
    let mut png = Vec::new();
    let img = image::RgbaImage::from_pixel(40, 20, image::Rgba([30, 90, 200, 255]));
    image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    h.state_mut().run("insert.picture", json!({"data": wordcraft_engine::cmd::insert::base64_encode(&png), "width": 60})).unwrap();
    let pos = wordcraft_engine::cmd::objects::selected(&h.state().session).unwrap().0;
    h.state_mut().session.sel = wordcraft_engine::Selection::caret(Pos::body(1, 0));
    steps(&mut h, 4);
    let rect = h.state_mut().session.layout().object(&pos, 0).expect("the picture is laid out").rect;
    let at = crate::frame::to_screen(h.state().canvas.page_rects[0], h.state().canvas.scale, rect).center();
    right_click(&mut h, at);
    assert!(wordcraft_engine::cmd::objects::object_selection(&h.state().session).is_some(), "the right-click selected the picture");
    assert_menu(&h, &["Insert Caption…", "Wrap Text"], true);
    pick(&mut h, &["Insert Caption…"]);
    let form = match &h.state().dialog {
        Some(crate::dialogs::Dialog::Refs(d)) => match &**d {
            crate::dialogs_refs::RefsDialog::Caption(f) => f.clone(),
            d => panic!("not the caption dialog: {d:?}"),
        },
        d => panic!("no caption dialog: {d:?}"),
    };
    assert_eq!(form.label, "Figure", "a picture in a table gets a figure caption");
    h.state_mut().dialog = None;
    h.state_mut().run("references.caption", form.params()).unwrap();
    let (table, r, c) = pos.path.cell().unwrap();
    let cell: Vec<String> = h.state().session.doc.table(StoryRef::Body, &table).unwrap().rows[r].cells[c]
        .blocks
        .iter()
        .filter_map(|b| b.as_para())
        .map(|p| p.plain_text())
        .collect();
    assert_eq!(cell.last().map(String::as_str), Some("Figure 1"), "{cell:?}");
}

/// With just the caret in a cell, right-click › Delete › Delete Table removes the table, in
/// one undo step (#1198: the menu offered only Delete Cells there).
#[test]
fn a_right_click_in_a_cell_deletes_the_table() {
    let mut h = table_window();
    let b = cell(&mut h, 0, 1);
    let at = screen(&mut h, b.x + b.w / 2.0, b.y + b.h / 2.0);
    right_click(&mut h, at);
    assert!(h.state().session.cell_range().is_none(), "just the caret");
    let depth = h.state().session.undo_depth();
    pick(&mut h, &["Delete", "Delete Table"]);
    assert!(h.state().session.doc.body.iter().all(|b| b.as_table().is_none()));
    assert_eq!(h.state().session.undo_depth(), depth + 1);
}

/// With a column's cells selected, a right-click on one of them keeps them: the menu is a
/// column's (Merge Cells, Distribute) and its Text Direction turns just the
/// column, in one undo step; one on a cell beside them drops them, though that cell lies
/// between the selection's ends in the text, and the menu is a cell's again.
#[test]
fn a_right_click_keeps_selected_cells_only_on_them() {
    use wordcraft_doc::props::TextDirection;
    let mut h = table_window();
    h.state_mut().run("table.selectColumn", json!({})).unwrap();
    assert_eq!(h.state().session.cell_range().map(|r| (r.rows, r.cols)), Some(((0, 1), (0, 0))));
    let on = cell(&mut h, 1, 0);
    let at = screen(&mut h, on.x + on.w / 2.0, on.y + on.h / 2.0);
    right_click(&mut h, at);
    assert_eq!(h.state().session.cell_range().map(|r| r.cols), Some((0, 0)), "still the column");
    assert_menu(&h, &["Merge Cells", "Distribute Rows", "Distribute Columns", "Text Direction"], true);
    assert_menu(&h, &["Split Cells", "AutoFit"], false);
    let depth = h.state().session.undo_depth();
    pick(&mut h, &["Text Direction"]);
    assert_eq!(h.state().session.undo_depth(), depth + 1);
    let t = h.state().session.doc.body.iter().find_map(|b| b.as_table()).unwrap();
    let dir = |r: usize, c: usize| t.rows[r].cells[c].props.text_direction;
    assert_eq!(
        [dir(0, 0), dir(1, 0), dir(0, 1), dir(1, 2)],
        [TextDirection::Down, TextDirection::Down, TextDirection::Horizontal, TextDirection::Horizontal]
    );
    let beside = cell(&mut h, 0, 2);
    let at = screen(&mut h, beside.x + beside.w / 2.0, beside.y + beside.h / 2.0);
    right_click(&mut h, at);
    assert!(h.state().session.cell_range().is_none(), "the column is no longer selected");
    assert_eq!(h.state().session.sel.focus.path.cell().map(|(_, r, c)| (r, c)), Some((0, 2)));
    assert_menu(&h, &["Split Cells"], true);
    assert_menu(&h, &["Merge Cells"], false);
}

/// Issue #123: zoomed out, pages sit side by side in rows when the width allows; at 100% a
/// letter page in an ordinary window stays one per row.
#[test]
fn pages_flow_into_rows_when_they_fit() {
    let letter = 612.0 * PX_PER_PT;
    assert_eq!(columns_for(1200.0, letter, 10, 1, true), 1, "100%: one page per row");
    assert_eq!(columns_for(1200.0, letter * 0.6, 10, 1, true), 2, "60%: two per row");
    assert_eq!(columns_for(1200.0, letter * 0.25, 10, 1, true), 5, "25%: five per row");
    assert_eq!(columns_for(1200.0, letter * 0.25, 3, 1, true), 3, "never more columns than pages");
    assert_eq!(columns_for(1200.0, letter * 0.25, 10, 1, false), 1, "fit modes that show one page");
    assert_eq!(columns_for(300.0, letter, 10, 2, true), 2, "Multiple Pages keeps two");
    // Hostile numbers never panic and give at least one column.
    for (w, slot, n) in [(f32::NAN, letter, 5), (1200.0, f32::NAN, 5), (f32::INFINITY, 1.0, 5), (1200.0, 0.0, 5), (-5.0, -1.0, 5), (1e30, 1e-30, 0)] {
        let c = columns_for(w, slot, n, 1, true);
        assert!((1..=n.max(1)).contains(&c), "{w} {slot} {n}: {c}");
    }

    // Mixed sizes: a landscape page among portrait ones, three per row.
    let sizes = [(612.0, 792.0), (792.0, 612.0), (612.0, 792.0), (612.0, 792.0)];
    let scale = 0.25;
    let (rects, size) = place_pages(&sizes, 0.0, scale, 3, vec2(1000.0, 600.0));
    assert_eq!(rects.len(), 4);
    assert_eq!(rects[0].top(), rects[1].top());
    assert_eq!(rects[1].top(), rects[2].top());
    assert!(rects[0].right() < rects[1].left() && rects[1].right() < rects[2].left(), "left to right: {rects:?}");
    assert!(rects[3].top() > rects[0].bottom(), "next row below the tallest page");
    assert_eq!(rects[3].left(), rects[0].left(), "rows share columns");
    assert!((rects[1].width() - 792.0 * scale).abs() < 1e-3);
    assert!(size.x >= 1000.0 && size.y >= rects[3].bottom());
    // Each page's centre maps back to that page.
    for (i, r) in rects.iter().enumerate() {
        assert_eq!(nearest_page(&rects, r.center()), Some(i));
    }
    // Hostile page sizes and scales stay finite.
    let bad = [(f32::NAN, f32::INFINITY), (-1.0, 0.0), (1e30, 1e30)];
    for scale in [f32::NAN, 0.0, -1.0, 1e9] {
        let (rects, size) = place_pages(&bad, f32::NAN, scale, 0, vec2(f32::NAN, -1.0));
        assert_eq!(rects.len(), 3);
        assert!(size.x.is_finite() && size.y.is_finite());
        assert!(rects.iter().all(|r| r.min.x.is_finite() && r.max.y.is_finite()), "{rects:?}");
    }
}

#[test]
fn long_web_page_uses_only_visible_native_resolution_tiles() {
    // A 17,843 pt Web Layout page at 100% on a 2x display would need ~47k raster
    // rows. One 8192px texture would upscale the page over five times, blurring text.
    let screen_scale = PX_PER_PT;
    let raster_scale = screen_scale * 2.0;
    let page = Rect::from_min_size(pos2(100.0, -10_000.0), vec2(800.0 * screen_scale, 17_843.0 * screen_scale));
    let viewport = Rect::from_min_size(pos2(0.0, 100.0), vec2(1440.0, 700.0));
    assert!(17_843.0 * raster_scale > 8192.0);
    assert!(800.0 * raster_scale < 8192.0);
    let tiles = visible_tiles(page, viewport, screen_scale, raster_scale, TILE_HEIGHT_PX);
    assert!(!tiles.is_empty());
    assert!(tiles.len() <= 3, "only the viewport should be rasterized: {tiles:?}");
    let total_px = (17_843.0 * raster_scale).ceil() as usize;
    for tile in tiles {
        let start = tile * TILE_HEIGHT_PX;
        assert!(start < total_px);
        assert!(total_px.saturating_sub(start).min(TILE_HEIGHT_PX) <= TILE_HEIGHT_PX);
    }
    assert_eq!(visible_tiles(page, Rect::from_min_size(pos2(1500.0, 0.0), vec2(300.0, 500.0)), screen_scale, raster_scale, TILE_HEIGHT_PX), 0..0);
    assert_eq!(visible_tiles(page, viewport, 0.0, raster_scale, TILE_HEIGHT_PX), 0..0);
    assert_eq!(visible_tiles(page, viewport, screen_scale, f32::NAN, TILE_HEIGHT_PX), 0..0);
    assert_eq!(visible_tiles(page, viewport, screen_scale, raster_scale, 0), 0..0);
    assert!(visible_tiles(page, viewport, screen_scale, raster_scale, 512).len() <= 5);
}

#[test]
fn texel_aligned_rect_snaps_the_origin_and_sizes_from_texture_pixels() {
    let layout = Rect::from_min_size(pos2(10.25, 20.75), vec2(80.0, 120.0));
    let visual = texel_aligned_rect(layout, vec2(161.0, 241.0), 2.0);

    assert_eq!(visual.min, pos2(10.5, 21.0));
    assert_eq!(visual.size(), vec2(80.5, 120.5));
}

/// 0.5 cm in points.
const HALF_CM: f32 = 72.0 * 0.5 / 2.54;

#[test]
fn table_gridlines_include_header_and_footer_tables() {
    let cell = |y: f32, covered: bool, story: StoryRef| Placed::Cell {
        rect: wordcraft_geom::Rect::new(0.0, y, 10.0, 10.0),
        covered,
        table: Default::default(),
        row: 0,
        cell: 0,
        story,
        clip: false,
    };
    let page = Page {
        items: vec![cell(100.0, false, StoryRef::Body), cell(110.0, true, StoryRef::Body)],
        header: vec![cell(10.0, false, StoryRef::Part(1)), cell(20.0, true, StoryRef::Part(1))],
        footer: vec![cell(200.0, false, StoryRef::Part(2))],
        ..Default::default()
    };
    let ys: Vec<f32> = gridline_cells(&page, true, None).map(|r| r.y).collect();
    assert_eq!(ys, [100.0, 10.0, 200.0]);
    let ys: Vec<f32> = gridline_cells(&page, true, Some(StoryRef::Part(2))).map(|r| r.y).collect();
    assert_eq!(ys, [100.0, 10.0, 200.0], "explicit View Gridlines still shows the other stories");
    let ys = |editing| gridline_cells(&page, false, editing).map(|r| r.y).collect::<Vec<_>>();
    assert_eq!(ys(Some(StoryRef::Part(1))), [10.0]);
    assert_eq!(ys(Some(StoryRef::Part(2))), [200.0]);
    assert!(ys(None).is_empty(), "closing header/footer editing removes the automatic guides");
}

#[test]
fn drawing_grid_starts_at_the_margins_and_stays_inside_them() {
    // Letter page with 1" margins: the text area is 468 x 648 pt from (72, 72).
    let body = wordcraft_geom::Rect::new(72.0, 72.0, 468.0, 648.0);
    let (xs, ys) = drawing_grid(body, (HALF_CM, HALF_CM));
    assert_eq!(xs.first().copied(), Some(72.0));
    assert_eq!(ys.first().copied(), Some(72.0));
    assert!(xs.iter().all(|x| (72.0..=540.0).contains(x)), "{xs:?}");
    assert!(ys.iter().all(|y| (72.0..=720.0).contains(y)), "{ys:?}");
    // 0.5 cm apart, the same both ways: 468 pt holds 33 steps, 648 pt holds 45.
    assert_eq!(xs.len(), 34);
    assert_eq!(ys.len(), 46);
    for w in xs.windows(2).chain(ys.windows(2)) {
        assert!((w[1] - w[0] - HALF_CM).abs() < 0.01, "{w:?}");
    }
    // The document's own spacing, different across and down: 1/8 inch across, 1/4 inch down.
    let (xs, ys) = drawing_grid(body, (9.0, 18.0));
    assert_eq!((xs.len(), ys.len()), (53, 37));
    // A grid line that lands on the margin is kept.
    let (xs, _) = drawing_grid(wordcraft_geom::Rect::new(0.0, 0.0, 100.0, 10.0), (25.0, 25.0));
    assert_eq!(xs, vec![0.0, 25.0, 50.0, 75.0, 100.0]);
}

#[test]
fn drawing_grid_survives_hostile_sizes() {
    let r = wordcraft_geom::Rect::new;
    for (body, step) in [
        (r(72.0, 72.0, 468.0, 648.0), 0.0),
        (r(72.0, 72.0, 468.0, 648.0), -5.0),
        (r(72.0, 72.0, 468.0, 648.0), f32::NAN),
        (wordcraft_geom::Rect { x: f32::NAN, y: 0.0, w: f32::INFINITY, h: -10.0 }, HALF_CM),
    ] {
        let (xs, ys) = drawing_grid(body, (step, step));
        assert!(xs.is_empty() && ys.is_empty(), "{body:?} {step}");
    }
    let (xs, ys) = drawing_grid(r(0.0, 0.0, 1.0e9, 1.0e9), (HALF_CM, HALF_CM));
    assert_eq!((xs.len(), ys.len()), (DRAWING_GRID_MAX_LINES, DRAWING_GRID_MAX_LINES));
    // Zoomed far out, lines thin out instead of filling the page.
    let grid = wordcraft_doc::DEFAULT_GRID;
    assert_eq!(drawing_grid_step(grid, PX_PER_PT), grid);
    assert_eq!(drawing_grid_step(HALF_CM, PX_PER_PT), HALF_CM);
    assert!(drawing_grid_step(grid, 0.1) * 0.1 >= DRAWING_GRID_MIN_PX);
    assert_eq!(drawing_grid_step(grid, 0.0), grid);
    assert_eq!(drawing_grid_step(grid, f32::NAN), grid);
    // A missing or broken spacing falls back to the spec's 1/8 inch.
    for bad in [0.0, -3.0, f32::NAN, f32::INFINITY] {
        assert_eq!(drawing_grid_step(bad, PX_PER_PT), grid);
    }
}

#[test]
fn texel_aligned_rect_keeps_integer_pixel_pages_unchanged() {
    let layout = Rect::from_min_size(pos2(10.0, 20.0), vec2(80.0, 120.0));
    let visual = texel_aligned_rect(layout, vec2(160.0, 240.0), 2.0);

    assert_eq!(visual, layout);
}

/// #692: a right-to-left paragraph mirrors the horizontal ruler's scale: the inch numbers
/// count from the right margin, and reading a pointer position back inverts the mapping
/// exactly, so the indent drags keep editing the same indents.
#[test]
fn the_ruler_scale_mirrors_for_right_to_left() {
    let (x0, scale, page_w, unit) = (100.0, 1.25, 612.0, 72.0);
    // 0 sits on the page's right edge and the inches rise leftward.
    assert_eq!(ruler_x(0.0, x0, scale, page_w, true), x0 + page_w * scale);
    assert_eq!(ruler_x(page_w, x0, scale, page_w, true), x0);
    // The inches count from the start margin, which is the right one: on an asymmetric page
    // (body from 72 to 468) 1 sits an inch left of the right body edge, mirroring left-to-right.
    let (body_x, body_right) = (72.0, 468.0);
    assert_eq!(ruler_x(body_x + unit, x0, scale, page_w, false), x0 + (body_x + unit) * scale);
    assert_eq!(ruler_x(page_w - body_right + unit, x0, scale, page_w, true), x0 + (body_right - unit) * scale);
    // Reading the pointer back inverts placing exactly, both ways: drags see the same indents.
    for rtl in [false, true] {
        for d in [0.0, 9.0, 72.0, 306.0, 612.0] {
            let x = ruler_x(d, x0, scale, page_w, rtl);
            assert!((ruler_d(x, x0, scale, page_w, rtl) - d).abs() < 1e-3, "rtl={rtl}: {x} reads back near {d}");
        }
    }

    // The marks: inches labelled from the start margin (the page edge an inch before it gets
    // a whole-inch tick, not a number, #1878; the margin itself has no mark), half inches the
    // long ticks, ending on the far page edge.
    let marks = ruler_marks(72.0, body_right - body_x, page_w, (unit, 8, 1), scale);
    let at = |d: f32| marks.iter().find(|(td, _)| (*td - d).abs() < 1e-4).map(|(_, m)| *m);
    assert_eq!(marks.first().map(|(d, _)| *d), Some(0.0));
    assert_eq!(marks.last().map(|(d, _)| *d), Some(page_w));
    assert_eq!(at(0.0), Some(RulerMark::Tick(UNIT_TICK)));
    assert_eq!(at(72.0), None);
    assert_eq!(at(72.0 + unit), Some(RulerMark::Label(1)));
    assert_eq!(at(72.0 + 2.0 * unit), Some(RulerMark::Label(2)));
    assert_eq!(at(72.0 + unit / 2.0), Some(RulerMark::Tick(5.0)));
    assert_eq!(at(72.0 + unit / 8.0), Some(RulerMark::Tick(1.5)));
    // Zoomed out, sub-half-inch ticks drop out rather than smudge together.
    let coarse = ruler_marks(72.0, body_right - body_x, page_w, (unit, 8, 1), 0.3);
    assert!(!coarse.iter().any(|(_, m)| matches!(m, RulerMark::Tick(1.5))));
    assert!(coarse.iter().any(|(_, m)| matches!(m, RulerMark::Tick(3.0))));
    // Laid on screen, the same ticks run the other way when mirrored.
    let xs = |rtl| marks.iter().map(|(d, _)| ruler_x(*d, x0, scale, page_w, rtl)).collect::<Vec<_>>();
    assert!(xs(false).windows(2).all(|w| w[0] < w[1]));
    assert!(xs(true).windows(2).all(|w| w[0] > w[1]));
}

/// #692: Home › Paragraph › Right-to-Left Text Direction flips the ruler on screen: the
/// inch numbers mirror about the page's centre and rise right-to-left.
#[test]
fn setting_a_paragraph_rtl_mirrors_the_ruler_numbers() {
    let ctx = egui::Context::default();
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Hello"})).unwrap();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0));
    let mut t = 0.0;
    // The horizontal ruler's inch labels: (text, centre x).
    let mut labels = |a: &mut WordApp| {
        t += 1.0 / 60.0;
        let input = egui::RawInput { time: Some(t), screen_rect: Some(screen), ..Default::default() };
        let out = ctx.run_ui(input, |ui| {
            a.logic(ui.ctx());
            a.ui(ui);
        });
        // The first frames only install fonts; the canvas appears after them.
        let found = a.canvas.canvas_rect.as_ref().map_or_else(Vec::new, |area| {
            // Ruler labels sit in the strip above the canvas, right of the vertical ruler.
            out.shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) if text.pos.y >= area.min.y - RULER && text.pos.y < area.min.y && text.pos.x >= area.min.x => {
                        Some((text.galley.text().to_owned(), text.pos.x + text.galley.size().x / 2.0))
                    }
                    _ => None,
                })
                .collect::<Vec<(String, f32)>>()
        });
        out.drop_without_applying_deltas();
        found
    };
    let inch =
        |labels: &[(String, f32)], n: &str| labels.iter().find(|(s, _)| s == n).map(|(_, x)| *x).unwrap_or_else(|| panic!("the ruler shows {n}"));
    // The first frames install fonts and lay the page out.
    for _ in 0..4 {
        labels(&mut a);
    }
    let centre = a.canvas.page_rects[a.session.page_hint].center().x;
    let before = labels(&mut a);
    let (before1, before2) = (inch(&before, "1"), inch(&before, "2"));
    assert!(before1 < centre && before1 < before2, "left to right: the numbers rise away from the left margin");

    a.run("para.rtl", json!({})).unwrap();
    let after = labels(&mut a);
    let (after1, after2) = (inch(&after, "1"), inch(&after, "2"));
    assert!(after1 > centre && after1 > after2, "right to left: the numbers rise away from the right margin");
    assert!((after1 - (2.0 * centre - before1)).abs() < 1.0, "1 mirrors about the page centre: {before1} → {after1}");
    assert!((after2 - (2.0 * centre - before2)).abs() < 1.0, "2 mirrors about the page centre: {before2} → {after2}");
}

/// #1878: a ruler's numbers and ticks thin out as the page shrinks on screen, so they never
/// pile up: the fewest units between numbers (1, 2, 4, 8…) that keep them 30 px apart, and
/// the fewest subdivisions between ticks that keep those 4 px apart.
#[test]
fn ruler_density_thins_out_as_the_page_shrinks() {
    let d = |px, div| {
        let d = ruler_density(px, div);
        (d.label_every, d.tick_every)
    };
    // Inches (8 subdivisions) at 100%, 50%, 25% and 10%: 96, 48, 24 and 9.6 px per inch.
    assert_eq!(d(96.0, 8), (1, 1));
    assert_eq!(d(48.0, 8), (1, 1));
    assert_eq!(d(24.0, 8), (2, 2));
    assert_eq!(d(9.6, 8), (4, 4));
    // Centimetres (4 subdivisions) at the same zooms: 37.8, 18.9, 9.45 and 3.78 px per cm.
    assert_eq!(d(37.8, 4), (1, 1));
    assert_eq!(d(18.9, 4), (2, 1));
    assert_eq!(d(9.45, 4), (4, 2));
    assert_eq!(d(3.78, 4), (8, 8));
    // Hostile sizes leave the ruler blank rather than loop or panic.
    assert_eq!(d(0.0, 8), (RULER_SPARSEST, RULER_SPARSEST));
    for px in [-1.0, f32::NAN, f32::INFINITY] {
        let (l, t) = d(px, 0);
        assert!(l >= 1 && t >= 1, "{px}");
    }
    // Every numbered position is on the ticks' grid, whatever the zoom.
    for div in [4, 8] {
        for z in 1..=200 {
            let px = z as f32 * 0.5;
            let d = ruler_density(px, div);
            assert_eq!(div * d.label_every % d.tick_every, 0, "{px} px per unit, {div} subdivisions");
        }
    }
}

/// #1878: a ruler numbers the text area only (the margins get ticks), its numbers are never
/// closer than 30 px however far out the page is zoomed, and whole units without a number
/// keep a long tick.
#[test]
fn ruler_marks_number_the_text_area_and_thin_out_zoomed_out() {
    let (unit, page_w, text_w) = (72.0, 612.0, 468.0);
    let at = |marks: &[(f32, RulerMark)], d: f32| marks.iter().find(|(td, _)| (*td - d).abs() < 1e-3).map(|(_, m)| *m);
    let labels = |marks: &[(f32, RulerMark)]| {
        marks
            .iter()
            .filter_map(|(d, m)| match m {
                RulerMark::Label(n) => Some((*d, *n)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    // 90 px per inch: a number at every inch of the text area, 1 to 6; the margins get the
    // whole-inch tick instead (the page edge an inch before the margin, 7 past it).
    let marks = ruler_marks(72.0, text_w, page_w, (unit, 8, 1), 1.25);
    assert_eq!(labels(&marks), (1..=6).map(|n| (72.0 + n as f32 * unit, n)).collect::<Vec<_>>());
    assert_eq!(at(&marks, 0.0), Some(RulerMark::Tick(UNIT_TICK)));
    assert_eq!(at(&marks, 72.0 + 7.0 * unit), Some(RulerMark::Tick(UNIT_TICK)));
    // 10.8 px per inch: a number every 4 inches, ticks at whole and half inches only.
    let tiny = ruler_marks(72.0, text_w, page_w, (unit, 8, 1), 0.15);
    assert_eq!(labels(&tiny), vec![(72.0 + 4.0 * unit, 4)]);
    assert!(tiny.iter().all(|(d, _)| ((d - 72.0) / (unit / 2.0)).fract().abs() < 1e-3), "{tiny:?}");
    assert!(tiny.len() >= 8, "{tiny:?}");
    // At every zoom and in every unit the numbers are at least 30 px apart, positive, and
    // only across the text area.
    for z in 1..=100 {
        let scale = z as f32 * 0.05;
        for (unit, div, step) in [(72.0, 8, 1), (wordcraft_geom::PT_PER_CM, 4, 1), (wordcraft_geom::PT_PER_CM, 4, 10), (72.0, 8, 72)] {
            let marks = ruler_marks(72.0, text_w, page_w, (unit, div, step), scale);
            let found = labels(&marks);
            let xs: Vec<f32> = found.iter().map(|(d, _)| d * scale).collect();
            assert!(xs.windows(2).all(|w| w[1] - w[0] >= RULER_LABEL_PX - 1e-3), "scale {scale}, unit {unit}: {xs:?}");
            assert!(found.iter().all(|(d, n)| *d > 72.0 && *d <= 72.0 + text_w + 1e-3 && *n > 0), "scale {scale}, unit {unit}: {found:?}");
        }
    }
    // Hostile sizes: nothing, or a bounded list, and no panic.
    assert!(ruler_marks(f32::NAN, text_w, page_w, (unit, 8, 1), 1.0).is_empty());
    assert!(ruler_marks(72.0, text_w, f32::INFINITY, (unit, 8, 1), 1.0).is_empty());
    assert!(ruler_marks(72.0, text_w, page_w, (0.0, 8, 1), 1.0).is_empty());
    assert!(ruler_marks(1e9, text_w, 1e9, (unit, 8, 1), 1.0).len() <= RULER_MAX_MARKS);
    assert!(ruler_marks(72.0, text_w, 1e9, (unit, 8, 1), 1.0).len() <= RULER_MAX_MARKS);
}

/// The rulers as one frame of `a` drew them: each bar (the grey strip over the page) and the
/// numbers along it, as (text, centre x) across and (text, centre y) down.
#[derive(Default)]
struct DrawnRulers {
    h_bar: Option<Rect>,
    v_bar: Option<Rect>,
    h_labels: Vec<(String, f32)>,
    v_labels: Vec<(String, f32)>,
}

fn drawn_rulers(ctx: &egui::Context, a: &mut WordApp, t: &mut f64) -> DrawnRulers {
    *t += 1.0 / 60.0;
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0));
    let input = egui::RawInput { time: Some(*t), screen_rect: Some(screen), ..Default::default() };
    let out = ctx.run_ui(input, |ui| {
        a.logic(ui.ctx());
        a.ui(ui);
    });
    let grey = Tokens::get(ctx).ruler_margin;
    let mut d = DrawnRulers::default();
    if let Some(area) = a.canvas.canvas_rect {
        let h = Rect::from_min_max(pos2(area.min.x, area.min.y - RULER), pos2(area.max.x, area.min.y));
        let v = Rect::from_min_max(pos2(area.min.x - RULER, area.min.y), pos2(area.min.x, area.max.y));
        for c in &out.shapes {
            match &c.shape {
                // The bars: the widest grey rect across the top strip, the tallest down the
                // left one (a table's markers are grey too, but small).
                egui::Shape::Rect(rs) if rs.fill == grey => {
                    let r = rs.rect;
                    if r.min.y >= h.min.y && r.max.y <= h.max.y && d.h_bar.is_none_or(|b| b.width() < r.width()) {
                        d.h_bar = Some(r);
                    }
                    if r.min.x >= v.min.x && r.max.x <= v.max.x && d.v_bar.is_none_or(|b| b.height() < r.height()) {
                        d.v_bar = Some(r);
                    }
                }
                egui::Shape::Text(text) => {
                    let centre = text.pos + text.galley.size() / 2.0;
                    if text.pos.y >= h.min.y && text.pos.y < h.max.y && text.pos.x >= h.min.x {
                        d.h_labels.push((text.galley.text().to_owned(), centre.x));
                    } else if text.pos.x >= v.min.x && text.pos.x < v.max.x && text.pos.y >= v.min.y && text.pos.y < v.max.y {
                        d.v_labels.push((text.galley.text().to_owned(), centre.y));
                    }
                }
                _ => {}
            }
        }
    }
    out.drop_without_applying_deltas();
    d
}

/// #1878: in Multiple Pages both rulers belong to the caret's page: the horizontal one spans
/// that page's rect on screen and the vertical one its height, and they follow the caret to
/// another page of the row and to the next row.
#[test]
fn the_rulers_span_the_caret_page_in_multiple_pages() {
    let ctx = egui::Context::default();
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "One"})).unwrap();
    for text in ["Two", "Three", "Four"] {
        a.run("insert.pageBreak", json!({})).unwrap();
        a.run("text.insert", json!({"text": text})).unwrap();
    }
    a.run("edit.goto", json!({"page": 1})).unwrap();
    a.run("view.multiplePages", json!({})).unwrap();
    let mut t = 0.0;
    let frames = |a: &mut WordApp, t: &mut f64| {
        for _ in 0..4 {
            drawn_rulers(&ctx, a, t);
        }
        drawn_rulers(&ctx, a, t)
    };
    let d = frames(&mut a, &mut t);
    let rects = a.canvas.page_rects.clone();
    assert!(rects.len() == 4 && a.canvas.cols >= 2, "pages side by side: {} per row of {}", a.canvas.cols, rects.len());
    let same_x = |bar: Option<Rect>, pr: Rect| bar.is_some_and(|b| (b.min.x - pr.min.x).abs() < 0.5 && (b.max.x - pr.max.x).abs() < 0.5);
    let same_y = |bar: Option<Rect>, pr: Rect| bar.is_some_and(|b| (b.min.y - pr.min.y).abs() < 0.5 && (b.max.y - pr.max.y).abs() < 0.5);
    assert!(same_x(d.h_bar, rects[0]), "the horizontal ruler spans page 1: {:?} vs {:?}", d.h_bar, rects[0]);
    assert!(same_y(d.v_bar, rects[0]), "the vertical ruler spans page 1: {:?} vs {:?}", d.v_bar, rects[0]);
    assert!(!same_x(d.h_bar, rects[1]), "pages 1 and 2 sit side by side");
    // The caret moves to the second page of the row: the horizontal ruler follows it there.
    a.run("edit.goto", json!({"page": 2})).unwrap();
    let d = frames(&mut a, &mut t);
    let rects = a.canvas.page_rects.clone();
    assert!(same_x(d.h_bar, rects[1]), "the horizontal ruler follows the caret to page 2: {:?} vs {:?}", d.h_bar, rects[1]);
    assert!(same_y(d.v_bar, rects[1]));
    // And down to the next row: the vertical ruler moves to that page's height.
    let below = (0..rects.len()).find(|i| rects[*i].min.y > rects[0].min.y + 1.0).expect("a second row of pages");
    a.run("edit.goto", json!({"page": below + 1})).unwrap();
    let d = frames(&mut a, &mut t);
    let rects = a.canvas.page_rects.clone();
    assert!(same_y(d.v_bar, rects[below]), "the vertical ruler spans page {}: {:?} vs {:?}", below + 1, d.v_bar, rects[below]);
    assert!(same_x(d.h_bar, rects[below]));
    assert!(!same_y(d.v_bar, rects[0]), "page 1 is in the row above");
}

/// #1878: the rulers number the text area only, a number gives way to an indent marker on
/// it, and zoomed far out the numbers thin out instead of piling up.
#[test]
fn ruler_numbers_skip_the_margins_and_the_markers_and_thin_out() {
    let ctx = egui::Context::default();
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Hello"})).unwrap();
    a.run("view.zoom", json!({"value": 100})).unwrap();
    let mut t = 0.0;
    let frames = |a: &mut WordApp, t: &mut f64| {
        for _ in 0..4 {
            drawn_rulers(&ctx, a, t);
        }
        drawn_rulers(&ctx, a, t)
    };
    let names = |labels: &[(String, f32)]| labels.iter().map(|(s, _)| s.clone()).collect::<Vec<String>>();
    // Inches at 100% on a Letter page with inch margins: 1 to 6 across the text, 1 to 9 down
    // it; the page edges an inch outside the margins get no "1", "7" or "10".
    let d = frames(&mut a, &mut t);
    assert_eq!(names(&d.h_labels), ["1", "2", "3", "4", "5", "6"]);
    let down = names(&d.v_labels);
    assert!(down.len() >= 6 && down[..6] == ["1", "2", "3", "4", "5", "6"], "down the page to the bottom of the view: {down:?}");
    let pr = a.canvas.page_rects[0];
    let scale = pr.width() / 612.0;
    assert!((d.v_labels[0].1 - (pr.min.y + 144.0 * scale)).abs() < 1.0, "the first number is an inch below the top margin");
    // A left indent of an inch puts the indent markers on the "1", which gives way.
    a.run("para.indents", json!({"left": 72.0})).unwrap();
    let d = frames(&mut a, &mut t);
    assert_eq!(names(&d.h_labels), ["2", "3", "4", "5", "6"]);
    a.run("para.indents", json!({"left": 0.0})).unwrap();
    let d = frames(&mut a, &mut t);
    assert_eq!(names(&d.h_labels), ["1", "2", "3", "4", "5", "6"]);
    // At 10% an inch is 9.6 px: a number every 4 inches, the text keeps its size.
    a.run("view.zoom", json!({"value": 10})).unwrap();
    let d = frames(&mut a, &mut t);
    assert_eq!(names(&d.h_labels), ["4"]);
    assert_eq!(names(&d.v_labels), ["4", "8"]);
    let (y4, y8) = (d.v_labels[0].1, d.v_labels[1].1);
    assert!(y8 - y4 >= RULER_LABEL_PX, "the numbers stay apart: {y4} and {y8}");
}

/// #1878: a number also gives way to a tab stop's glyph (set, or being dragged to it) and to
/// a table's column and row markers sitting on it.
#[test]
fn ruler_numbers_give_way_to_tab_stops_and_table_markers() {
    use crate::ruler_tabs::TabDrag;
    use wordcraft_doc::props::{TabAlign, TabLeader};
    let ctx = egui::Context::default();
    let mut t = 0.0;
    let frames = |a: &mut WordApp, t: &mut f64| {
        for _ in 0..4 {
            drawn_rulers(&ctx, a, t);
        }
        drawn_rulers(&ctx, a, t)
    };
    let names = |labels: &[(String, f32)]| labels.iter().map(|(s, _)| s.clone()).collect::<Vec<String>>();
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Hello"})).unwrap();
    a.run("view.zoom", json!({"value": 100})).unwrap();
    assert_eq!(names(&frames(&mut a, &mut t).h_labels), ["1", "2", "3", "4", "5", "6"]);
    // A tab stop an inch in sits on the "1".
    a.run("para.tabs", json!({"tabs": [{"pos": 72.0}]})).unwrap();
    assert_eq!(names(&frames(&mut a, &mut t).h_labels), ["2", "3", "4", "5", "6"]);
    // Dragged to two inches it counts where it lands: the "1" is back and the "2" gives way.
    let para = a.session.sel.focus.clone();
    let drag = |at: f32, off: bool| Some(TabDrag { from: Some(72.0), para: para.clone(), align: TabAlign::Left, leader: TabLeader::None, at, off });
    a.canvas.tab_drag = drag(144.0, false);
    assert_eq!(names(&drawn_rulers(&ctx, &mut a, &mut t).h_labels), ["1", "3", "4", "5", "6"]);
    // Off the ruler it counts nowhere.
    a.canvas.tab_drag = drag(144.0, true);
    assert_eq!(names(&drawn_rulers(&ctx, &mut a, &mut t).h_labels), ["1", "2", "3", "4", "5", "6"]);
    a.run("para.tabs", json!({"clearAll": true})).unwrap();
    assert_eq!(names(&frames(&mut a, &mut t).h_labels), ["1", "2", "3", "4", "5", "6"]);
    // A three-column table with the caret in the first cell. Narrowing that column puts the
    // table's right border six inches from the margin: its marker takes the "6", while the
    // caret cell's own edge (and so its indent marker) ends well short of any whole inch.
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("insert.table", json!({"rows": 1, "cols": 3})).unwrap();
    a.run("text.insert", json!({"text": "A"})).unwrap();
    a.run("view.zoom", json!({"value": 100})).unwrap();
    let cell = |a: &mut WordApp, c: usize| {
        a.session.layout().pages[0]
            .items
            .iter()
            .find_map(|it| match it {
                wordcraft_layout::Placed::Cell { rect, row: 0, cell, .. } if *cell == c => Some(*rect),
                _ => None,
            })
            .unwrap()
    };
    let body = a.session.layout().pages[0].body;
    let (first, last) = (cell(&mut a, 0), cell(&mut a, 2));
    let width = body.x + 432.0 - first.x - (last.right() - first.right());
    a.run("table.columnWidth", json!({"width": width})).unwrap();
    let d = frames(&mut a, &mut t);
    let (first, last) = (cell(&mut a, 0), cell(&mut a, 2));
    assert!((last.right() - (body.x + 432.0)).abs() < 0.5, "the table ends six inches in: {last:?}");
    let off_inch = (first.right() - body.x).rem_euclid(72.0);
    assert!((12.0..=60.0).contains(&off_inch), "the caret cell's edge is off the whole inches: {first:?}");
    assert_eq!(names(&d.h_labels), ["1", "2", "3", "4", "5"]);
    let down = names(&d.v_labels);
    assert!(down.len() >= 3 && down[..3] == ["1", "2", "3"], "{down:?}");
    // The row's bottom an inch below the top margin takes the "1" down the page.
    let height = body.y + 72.0 - first.y;
    a.run("table.rowHeight", json!({"height": height, "rule": "exact"})).unwrap();
    let d = frames(&mut a, &mut t);
    assert!((cell(&mut a, 0).bottom() - (body.y + 72.0)).abs() < 0.5);
    let down = names(&d.v_labels);
    assert!(down.len() >= 3 && down[..3] == ["2", "3", "4"], "{down:?}");
    assert_eq!(names(&d.h_labels), ["1", "2", "3", "4", "5"]);
}

/// Ruler tab gestures are safe: a click on an indent marker places nothing, only the
/// primary button places or moves stops, Escape cancels a drag, and a drag whose paragraph
/// the caret leaves is abandoned instead of applied there.
#[test]
fn ruler_tab_gestures_are_primary_only_and_cancellable() {
    use egui::{Event, Key, PointerButton};
    use wordcraft_doc::props::TabAlign;
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Name\tScore\rSecond"})).unwrap();
    a.run("view.ruler", json!({"value": true})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    let area = h.state().canvas.canvas_rect.expect("the canvas is laid out");
    let ruler_x = |h: &mut egui_kittest::Harness<'_, WordApp>, d: f32| {
        let body_x = h.state_mut().session.layout().pages[0].body.x;
        page_to_screen(h.state_mut(), 0, body_x + d, 0.0).expect("on screen").x
    };
    let on_ruler = |h: &mut egui_kittest::Harness<'_, WordApp>, d: f32| pos2(ruler_x(h, d), area.min.y - 7.0);
    let button = |at: Pos2, b: PointerButton, pressed: bool| Event::PointerButton { pos: at, button: b, pressed, modifiers: Default::default() };
    let drag_events = |h: &mut egui_kittest::Harness<'_, WordApp>, from: Pos2, to: Pos2, b: PointerButton| {
        h.event(Event::PointerMoved(from));
        h.event(button(from, b, true));
        h.step();
        for k in 1..=4 {
            h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
            h.step();
        }
        h.event(button(to, b, false));
        for _ in 0..3 {
            h.step();
        }
    };
    let stops_of = |h: &egui_kittest::Harness<'_, WordApp>, block: usize| {
        let s = &h.state().session;
        let para = s.doc.para_at(&Pos::body(block, 0)).expect("a paragraph");
        s.doc.styles.resolve_para(&para.props).tabs.iter().map(|t| (t.pos, t.align)).collect::<Vec<_>>()
    };

    // A plain click on the left-indent marker (at the paragraph's start edge) places nothing.
    let marker = pos2(ruler_x(&mut h, 0.0), area.min.y - 6.0);
    h.event(Event::PointerMoved(marker));
    h.event(button(marker, PointerButton::Primary, true));
    h.step();
    h.event(button(marker, PointerButton::Primary, false));
    for _ in 0..3 {
        h.step();
    }
    assert!(stops_of(&h, 1).is_empty(), "an indent-marker click placed a stop: {:?}", stops_of(&h, 1));

    // A right-button drag across the ruler places nothing.
    let (from, to) = (on_ruler(&mut h, 72.0), on_ruler(&mut h, 144.0));
    drag_events(&mut h, from, to, PointerButton::Secondary);
    assert!(stops_of(&h, 1).is_empty(), "a right-drag placed a stop: {:?}", stops_of(&h, 1));

    // Set a stop at 2" the intended way, then prove Escape cancels a move.
    let at = on_ruler(&mut h, 144.0);
    h.event(Event::PointerMoved(at));
    h.event(button(at, PointerButton::Primary, true));
    h.step();
    h.event(button(at, PointerButton::Primary, false));
    for _ in 0..32 {
        h.step();
    }
    assert_eq!(stops_of(&h, 1), vec![(144.0, TabAlign::Left)]);
    let (from, to) = (on_ruler(&mut h, 144.0), on_ruler(&mut h, 216.0));
    h.event(Event::PointerMoved(from));
    h.event(button(from, PointerButton::Primary, true));
    h.step();
    h.event(Event::PointerMoved(from + (to - from) * 0.5));
    h.step();
    h.event(Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
    h.step();
    h.event(button(to, PointerButton::Primary, false));
    for _ in 0..32 {
        h.step();
    }
    assert_eq!(stops_of(&h, 1), vec![(144.0, TabAlign::Left)], "Escape left the stop where it was");

    // A drag abandoned by moving the caret to another paragraph applies nowhere.
    let (from, to) = (on_ruler(&mut h, 144.0), on_ruler(&mut h, 216.0));
    h.event(Event::PointerMoved(from));
    h.event(button(from, PointerButton::Primary, true));
    h.step();
    h.event(Event::PointerMoved(from + (to - from) * 0.5));
    h.step();
    h.state_mut().run("caret.docStart", json!({})).unwrap();
    h.step();
    h.event(button(to, PointerButton::Primary, false));
    for _ in 0..3 {
        h.step();
    }
    assert_eq!(stops_of(&h, 1), vec![(144.0, TabAlign::Left)], "the old paragraph keeps its stop");
    assert!(stops_of(&h, 0).is_empty(), "the caret's new paragraph gained nothing: {:?}", stops_of(&h, 0));
}

/// Tab stops on the ruler: the corner selector picks the kind, a click sets a stop, dragging
/// moves it, dragging it off clears it, and each gesture is one undo step.
#[test]
fn ruler_tab_stops_are_set_moved_and_cleared_with_the_pointer() {
    use egui::{Event, PointerButton};
    use wordcraft_doc::props::TabAlign;
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Name\tScore"})).unwrap();
    a.run("view.ruler", json!({"value": true})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    let area = h.state().canvas.canvas_rect.expect("the canvas is laid out");
    // The middle of the ruler's strip, over a point `d` points from the paragraph's start.
    let on_ruler = |h: &mut egui_kittest::Harness<'_, WordApp>, d: f32| {
        let body_x = h.state_mut().session.layout().pages[0].body.x;
        let x = page_to_screen(h.state_mut(), 0, body_x + d, 0.0).expect("on screen").x;
        pos2(x, area.min.y - 7.0)
    };
    let button = |at: Pos2, pressed: bool| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    let click = |h: &mut egui_kittest::Harness<'_, WordApp>, at: Pos2| {
        h.event(Event::PointerMoved(at));
        h.event(button(at, true));
        h.step();
        h.event(button(at, false));
        for _ in 0..3 {
            h.step();
        }
        // Let the double-click time pass before the next press.
        for _ in 0..30 {
            h.step();
        }
    };
    let drag = |h: &mut egui_kittest::Harness<'_, WordApp>, from: Pos2, to: Pos2| {
        h.event(Event::PointerMoved(from));
        h.event(button(from, true));
        h.step();
        for k in 1..=6 {
            h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 6.0)));
            h.step();
        }
        h.event(button(to, false));
        for _ in 0..3 {
            h.step();
        }
    };
    let stops = |h: &egui_kittest::Harness<'_, WordApp>| {
        let s = &h.state().session;
        let para = s.doc.para_at(&s.sel.focus).expect("a paragraph");
        s.doc.styles.resolve_para(&para.props).tabs.iter().map(|t| (t.pos, t.align)).collect::<Vec<_>>()
    };

    // The selector starts at Left; a click moves it to Center.
    assert_eq!(h.state().canvas.tab_kind, TabAlign::Left);
    click(&mut h, pos2(area.min.x - RULER / 2.0, area.min.y - RULER / 2.0));
    assert_eq!(h.state().canvas.tab_kind, TabAlign::Center);

    // A click on the ruler sets a centre stop at 2 inches (on the 1/16 inch grid).
    let at = on_ruler(&mut h, 144.0);
    click(&mut h, at);
    assert_eq!(stops(&h), vec![(144.0, TabAlign::Center)]);

    // Dragging it to 3 inches moves it, keeping its kind.
    let (from, to) = (on_ruler(&mut h, 144.0), on_ruler(&mut h, 216.0));
    drag(&mut h, from, to);
    assert_eq!(stops(&h), vec![(216.0, TabAlign::Center)]);

    // Dragging it down into the page clears it.
    let from = on_ruler(&mut h, 216.0);
    drag(&mut h, from, from + vec2(0.0, 80.0));
    assert!(stops(&h).is_empty(), "the stop came off: {:?}", stops(&h));

    // Each gesture was one step: undo brings back the moved stop, then the first one.
    assert!(h.state_mut().session.undo());
    assert_eq!(stops(&h), vec![(216.0, TabAlign::Center)]);
    assert!(h.state_mut().session.undo());
    assert_eq!(stops(&h), vec![(144.0, TabAlign::Center)]);
}

#[test]
fn hiding_white_space_stacks_page_bands_and_keeps_hit_testing() {
    let scale = 1.0;
    let sizes = [(612.0, 792.0); 3];
    let (rects, _) = place_pages(&sizes, 0.0, scale, 1, vec2(800.0, 600.0));
    // Body from 72 to 720 pt on every page.
    let bodies = [(72.0, 720.0); 3];
    let (moved, bands, h) = collapse_white_space(&rects, &bodies, scale, 600.0);
    assert_eq!(bands[0].0, 0.0, "first page keeps its top");
    assert_eq!(bands[2].1, 792.0, "last page keeps its bottom");
    assert_eq!(bands[1], (66.0, 726.0));
    // Bands abut with one seam, so the content is much shorter than three full pages.
    assert_eq!(moved[1].min.y + bands[1].0, moved[0].min.y + bands[0].1 + SEAM);
    assert!(h < rects[2].bottom() - 2.0 * 66.0 * 2.0);
    // A point inside page 2's band maps to that page and the right page coordinates.
    let p = pos2(moved[1].center().x, moved[1].min.y + 300.0);
    assert_eq!(page_at_index(&moved, p), Some(1));
    // Just above the seam is page 1's text area, just below is page 2's, though the rects overlap.
    let seam = moved[0].min.y + bands[0].1;
    assert_eq!(nearest_page(&moved, pos2(moved[0].center().x, seam - 2.0)), Some(0));
    assert_eq!(nearest_page(&moved, pos2(moved[0].center().x, seam + SEAM + 2.0)), Some(1));
    assert!(on_page_gap(&moved, &bands, pos2(moved[0].center().x, seam + 0.5)));
    assert!(!on_page_gap(&moved, &bands, pos2(moved[0].center().x, seam - 40.0)));
    // The drawn I-beam takes the canvas colour in the gap and on a cut margin (#1746).
    assert!(on_visible_page(&moved, &bands, pos2(moved[0].center().x, seam - 40.0)));
    assert!(!on_visible_page(&moved, &bands, pos2(moved[0].center().x, seam + 0.5)));
    assert!(!on_visible_page(&moved, &bands, pos2(moved[0].min.x - 5.0, moved[0].center().y)));
    // Hostile numbers never panic.
    let _ = collapse_white_space(&rects, &[(f32::NAN, f32::INFINITY), (-1e9, 1e9)], f32::NAN, f32::NAN);
    let _ = collapse_white_space(&[], &[], 1.0, 1.0);
}

/// #1169: Space and Enter count as `Response::clicked` on the focused canvas, and
/// `request_focus` always interrupts IME. On X11 that recreates the XIM context and
/// fcitx drops every later key. Those keys still type, and they must not set
/// `should_interrupt_composition`. A real pointer click still does, so a composition
/// commits when the caret moves.
#[test]
fn space_and_enter_on_the_canvas_do_not_interrupt_ime() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("document.setText", json!({"text": "The"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.step();
    }
    assert!(h.state().canvas.focused, "the canvas takes focus when the window opens");
    let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    let ime_interrupted =
        |h: &egui_kittest::Harness<'_, WordApp>| h.output().platform_output.ime.as_ref().map(|ime| ime.should_interrupt_composition);
    h.event(key(egui::Key::Space));
    h.event(egui::Event::Text(" ".into()));
    h.step();
    assert_eq!(ime_interrupted(&h), Some(false), "space must not recreate the XIM context");
    let text = h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body);
    assert!(text.contains("The") && text.contains(' '), "the space is still inserted: {text:?}");
    assert!(h.state().canvas.focused, "the canvas keeps the keyboard");
    h.event(key(egui::Key::Enter));
    h.step();
    assert_eq!(ime_interrupted(&h), Some(false), "enter must not recreate the XIM context");
    assert!(h.state().canvas.focused, "enter leaves the canvas focused");

    let at = h.state().canvas.page_rects.first().map(|r| r.center()).expect("a page");
    h.event(egui::Event::PointerMoved(at));
    h.step();
    let mut pointer_interrupted = false;
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.step();
        pointer_interrupted |= ime_interrupted(&h) == Some(true);
    }
    assert!(pointer_interrupted, "a pointer click still interrupts composition");
}

/// Kittest's `MissingGlyphPolicy::Panic` is right: a missing glyph is a bug. But the interface
/// fonts need not carry Japanese (craft-fonts is optional, and a CI Linux runner has no
/// Japanese system font), and these IME tests compose Japanese through the interface font
/// chain. Draw a stand-in for exactly those characters; every other missing glyph still panics.
fn kana_test_glyphs(ctx: &egui::Context) {
    ctx.add_glyph_rasterizer(egui::GlyphRasterizer::new("wordcraft::canvas::tests::kana", |req| {
        let kana =
            *req.family == egui::FontFamily::Proportional && !req.cluster.is_empty() && req.cluster.chars().all(|c| matches!(c, 'に' | 'ほ' | 'ん'));
        kana.then(|| egui::RasterizedGlyph {
            bitmap: egui::GlyphBitmap {
                image: egui::ColorImage::new([1, 1], vec![egui::Color32::WHITE]),
                offset_px: egui::vec2(0.0, 0.0),
                is_color: false,
            },
            advance_px: (req.font_size_px * 0.5).max(1.0),
        })
    }));
}

/// The fixture above must work with no Japanese font anywhere: only a bundled Latin face, the
/// strict missing-glyph policy, and the kana stand-in.
#[test]
fn the_ime_fixture_draws_kana_without_a_japanese_font() {
    let ctx = egui::Context::default();
    ctx.set_missing_glyph_policy(egui::MissingGlyphPolicy::Panic);
    let mut fonts = egui::FontDefinitions::empty();
    fonts
        .font_data
        .insert("Latin".into(), std::sync::Arc::new(egui::FontData::from_static(include_bytes!("../../../../assets/fonts/Inter-Regular.ttf"))));
    fonts.families.insert(egui::FontFamily::Proportional, vec!["Latin".into()]);
    ctx.set_fonts(fonts);
    kana_test_glyphs(&ctx);
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let g = ui.painter().layout_no_wrap("にほん".into(), crate::theme::regular(14.0), egui::Color32::WHITE);
        assert_eq!(g.rows.first().map_or(0, |r| r.row.glyphs.len()), 3, "one glyph per composed kana");
    });
    output.textures_delta.clear();
}

/// While an input method composes, the caret and the candidate window sit at its cursor inside
/// the composition: the end of the range it reports, or the end of the text with no range. The
/// range comes from the OS and is clamped, whatever it holds.
#[test]
fn the_caret_follows_the_input_method_cursor_in_the_composition() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("document.setText", json!({"text": "The"})).unwrap();
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_context_setup(kana_test_glyphs)
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
    for _ in 0..8 {
        h.step();
    }
    let ime_x = |h: &egui_kittest::Harness<'_, WordApp>| h.output().platform_output.ime.as_ref().map(|ime| ime.cursor_rect.min.x);
    let start = ime_x(&h).expect("the canvas places the candidate window");
    let mut compose = |range: Option<std::ops::Range<usize>>| {
        h.event(egui::Event::Ime(egui::ImeEvent::Preedit { text: "にほん".into(), active_range_chars: range }));
        h.step();
        h.step();
        ime_x(&h).expect("the canvas places the candidate window")
    };
    let before = compose(Some(0..0));
    assert_eq!(before, start, "a cursor at the composition's start is the caret before composing");
    let end = compose(None);
    let mid = compose(Some(1..1));
    assert!(end > mid && mid > before, "the cursor moves through the composition: {before} {mid} {end}");
    assert_eq!(compose(Some(0..99)), end, "a range past the text ends at its end");
    assert_eq!(compose(Some(std::ops::Range { start: 5, end: 1 })), mid, "a reversed range uses its end");
    assert_eq!(compose(Some(usize::MAX..usize::MAX)), end);
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("にほん".into())));
    h.step();
    assert_eq!(h.state().canvas.ime_preedit_range, None, "the commit clears the range");
    assert!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("にほん"));
}

/// The outgoing IME descriptor is built from settled, visible logical text. A pre-edit does
/// not change the model; an accepted commit alone earns `InputMethod`; ordinary typing earns
/// `Other`. The commit mixes Korean, Japanese, ASCII and an emoji.
#[test]
fn ime_descriptor_reports_visible_logical_text_and_exact_cause() {
    use egui::ImeChangeCause;
    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("ab")), Default::default());
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_context_setup(kana_test_glyphs)
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
    for _ in 0..8 {
        h.step();
    }
    h.state_mut().session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 2));
    h.step();
    let text = |h: &egui_kittest::Harness<'_, WordApp>| {
        h.output().platform_output.ime.as_ref().and_then(|i| i.surrounding_text.available()).map(|p| p.text.clone())
    };
    let cause = |h: &egui_kittest::Harness<'_, WordApp>| h.output().platform_output.ime.as_ref().map(|i| i.change_cause);
    assert_eq!(text(&h), Some("ab".into()), "the committed text is available");
    h.event(egui::Event::Ime(egui::ImeEvent::Preedit { text: "に".into(), active_range_chars: None }));
    h.step();
    assert_eq!(text(&h), Some("ab".into()), "the pre-edit never enters the committed model");
    assert_eq!(cause(&h), Some(ImeChangeCause::Other), "a pre-edit is no mutation");
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("한日本A😀".into())));
    h.step();
    assert!(text(&h).unwrap_or_default().contains("한日本A😀"), "the commit is in the visible text");
    assert_eq!(cause(&h), Some(ImeChangeCause::InputMethod), "an accepted commit is the IME's change");
    h.event(egui::Event::Text("x".into()));
    h.step();
    assert_eq!(cause(&h), Some(ImeChangeCause::Other), "ordinary typing is an application change");
}

/// Tracked deletions never go to the input method. The caret may straddle one and the
/// selection direction survives the visible mapping.
#[test]
fn ime_descriptor_hides_tracked_deletions_and_preserves_reversed_selection() {
    use wordcraft_doc::props::CharProps;
    use wordcraft_doc::{Paragraph, Run, para_block};
    let mut p = Paragraph::new();
    p.text = "abDELcd".into();
    p.runs = vec![
        Run { len: 2, props: CharProps::default() },
        Run { len: 3, props: CharProps { del: Some(0), ..Default::default() } },
        Run { len: 2, props: CharProps::default() },
    ];
    let mut doc = wordcraft_doc::Document::from_text("x");
    doc.body = vec![para_block(p)];
    let app = WordApp::new(wordcraft_engine::Session::new(doc), Default::default());
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.step();
    }
    h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 2), focus: wordcraft_doc::Pos::body(0, 7) };
    h.step();
    let payload =
        |h: &egui_kittest::Harness<'_, WordApp>| h.output().platform_output.ime.as_ref().and_then(|i| i.surrounding_text.available()).cloned();
    let p = payload(&h).expect("the visible text is available");
    assert_eq!(p.text, "abcd");
    assert_eq!(p.cursor, 4);
    assert_eq!(p.anchor, 2, "the reversed selection stays reversed");
}

/// Byte deletion keeps the selection out of both erased sides, then the commit replaces it;
/// both accepted mutations are `InputMethod` receipts over bounded snapshots.
#[test]
fn ime_byte_deletion_preserves_the_selection_and_earns_input_method() {
    use egui::ImeChangeCause;
    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("abcd")), Default::default());
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.step();
    }
    h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 3), focus: wordcraft_doc::Pos::body(0, 1) };
    h.step();
    let cause = |h: &egui_kittest::Harness<'_, WordApp>| h.output().platform_output.ime.as_ref().map(|i| i.change_cause);
    h.event(egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 1 }));
    h.step();
    let body = |h: &egui_kittest::Harness<'_, WordApp>| h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end().to_string();
    assert_eq!(body(&h), "bc", "the selected text is preserved");
    assert_eq!(h.state().session.sel.anchor.off, 2, "the selection anchor moved with the erased prefix");
    assert_eq!(h.state().session.sel.focus.off, 0);
    assert_eq!(cause(&h), Some(ImeChangeCause::InputMethod), "an accepted deletion is the IME's change");
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("X".into())));
    h.step();
    assert_eq!(body(&h), "X", "the commit replaces the preserved selection");
    assert_eq!(cause(&h), Some(ImeChangeCause::InputMethod), "delete plus commit chain into one IME change");
}

/// A selection wider than the protocol limit is honestly unavailable, not truncated.
#[test]
fn ime_descriptor_is_unavailable_for_a_too_wide_selection() {
    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text(&"a".repeat(5000))), Default::default());
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.step();
    }
    h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 0), focus: wordcraft_doc::Pos::body(0, 5000) };
    h.step();
    let ime = h.output().platform_output.ime.clone().expect("the canvas owns the IME context");
    assert!(ime.surrounding_text.available().is_none(), "a 5000-byte selection is unavailable");
}

/// Replacing the document starts a new IME context; caret motion inside it does not.
#[test]
fn ime_generation_changes_on_document_replacement_but_not_on_caret_motion() {
    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("hello")), Default::default());
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.step();
    }
    let generation = |h: &egui_kittest::Harness<'_, WordApp>| h.output().platform_output.ime.as_ref().map(|i| i.context_generation);
    let first = generation(&h).expect("an IME context");
    h.state_mut().run("caret.right", json!({})).unwrap();
    h.step();
    assert_eq!(generation(&h), Some(first), "caret motion keeps the context");
    h.state_mut().session.set_document(wordcraft_doc::Document::from_text("new"));
    h.step();
    assert!(generation(&h) > Some(first), "a replaced document starts a new context");
}

/// An equation's structured tree has no reversible surrounding text: report `Unavailable`
/// while keeping the stable editing-session context and candidate geometry through caret
/// motion. A real content commit is proved by the engine's checked revision and earns
/// `InputMethod`.
#[test]
fn ime_equation_is_unavailable_with_a_stable_session_context() {
    use egui::ImeChangeCause;
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("insert.equation", json!({"linear": "x+y"})).unwrap();
    assert!(app.session.math.is_some(), "the equation is being edited");
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.step();
    }
    let ime = h.output().platform_output.ime.clone().expect("the equation places its candidate window");
    assert!(ime.surrounding_text.available().is_none(), "no fabricated equation surroundings");
    let generation = ime.context_generation;
    h.state_mut().run("equation.move", json!({"dir": "left"})).unwrap();
    h.step();
    let moved = h.output().platform_output.ime.clone().expect("still editing");
    assert_eq!(moved.context_generation, generation, "caret motion keeps the equation's IME context");
    assert!(moved.surrounding_text.available().is_none());
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("z".into())));
    h.step();
    let committed = h.output().platform_output.ime.clone().expect("still editing");
    assert_eq!(committed.change_cause, ImeChangeCause::InputMethod, "the engine revision proves the content commit");
    assert!(committed.surrounding_text.available().is_none());
}

/// A transformed equation commit (build-up or Math AutoCorrect) is `Other`: the engine's
/// per-command outcome marks it transformed even though the revision moved and content
/// changed. The equation really built up; no InputMethod receipt is fabricated.
#[test]
fn ime_transformed_equation_commits_are_other() {
    use egui::ImeChangeCause;
    for (typed, latex) in [("a/b ", "\\frac{a}{b}"), ("\\alpha ", "\\alpha")] {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("insert.equation", json!({})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        for _ in 0..8 {
            h.step();
        }
        h.event(egui::Event::Ime(egui::ImeEvent::Commit(typed.into())));
        h.step();
        let info = h.state_mut().run("equation.get", json!({})).unwrap();
        assert_eq!(info["latex"].as_str(), Some(latex), "the commit was transformed: {typed:?}");
        let ime = h.output().platform_output.ime.clone().expect("still editing the equation");
        assert_eq!(ime.change_cause, ImeChangeCause::Other, "an automatic equation rewrite is not the IME's own edit: {typed:?}");
    }
}

/// A literal IME commit after an earlier AutoCorrect is still the IME's own change. The
/// stale `"AutoFormat"` undo label left by that earlier step must not classify this command.
#[test]
fn ime_literal_commit_after_autoformat_is_input_method() {
    use egui::ImeChangeCause;
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    for text in ["Word", " ", "(c)", " "] {
        app.run("text.insert", json!({"text": text})).unwrap();
    }
    assert_eq!(app.session.undo_label(), Some("AutoFormat"), "the earlier AutoCorrect left its undo label");
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.step();
    }
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("x".into())));
    h.step();
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "Word \u{a9} x");
    let ime = h.output().platform_output.ime.clone().expect("an IME context");
    assert_eq!(ime.change_cause, ImeChangeCause::InputMethod, "the literal commit is not the earlier AutoFormat");
}

/// A real allocator exhaustion (not a hand-built stamp) keeps the engine revision absent;
/// an equation commit and a wide body deletion still happen but can never certify a receipt.
#[test]
fn exhausted_ime_revisions_never_certify_equation_or_wide_body_edits() {
    use egui::ImeChangeCause;
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("insert.equation", json!({"linear": "x"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.step();
    }
    h.state_mut().session.exhaust_ime_state_revision();
    h.step();
    assert_eq!(h.state_mut().session.ime_state_revision(), None, "the allocator is exhausted");
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("y".into())));
    h.step();
    let linear = h.state_mut().run("equation.get", json!({})).unwrap()["linear"].as_str().unwrap_or_default().to_string();
    assert_eq!(linear, "xy", "the real commit still happened");
    let ime = h.output().platform_output.ime.clone().expect("still editing the equation");
    assert_eq!(ime.change_cause, ImeChangeCause::Other, "without a revision the change cannot be certified");

    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text(&"a".repeat(5000))), Default::default());
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.step();
    }
    h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 1), focus: wordcraft_doc::Pos::body(0, 5000) };
    h.step();
    h.state_mut().session.exhaust_ime_state_revision();
    h.step();
    h.event(egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }));
    h.step();
    let body = h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body);
    assert_eq!(body.matches('a').count(), 4999, "the wide body delete still happened");
    let ime = h.output().platform_output.ime.clone().expect("an IME context");
    assert_eq!(ime.change_cause, ImeChangeCause::Other, "the exhausted revision cannot prove the delete");
}

/// A body replaced under the same document id starts a new IME context and interrupts the
/// live composition, even when the replacement text (and so the bounded window) is identical.
#[test]
fn ime_generation_changes_on_body_replacement_with_the_same_text() {
    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("hello")), Default::default());
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_context_setup(kana_test_glyphs)
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
    for _ in 0..8 {
        h.step();
    }
    h.event(egui::Event::Ime(egui::ImeEvent::Preedit { text: "に".into(), active_range_chars: None }));
    h.step();
    assert_eq!(h.state().canvas.ime_preedit.as_str(), "に");
    let first = h.output().platform_output.ime.as_ref().map(|i| i.context_generation).expect("an IME context");
    h.state_mut().run("document.setText", json!({"text": "hello"})).unwrap();
    h.step();
    let ime = h.output().platform_output.ime.clone().expect("the canvas still owns the IME context");
    assert!(ime.context_generation > first, "a replaced body starts a new context");
    assert!(ime.should_interrupt_composition, "the old composition is interrupted");
    assert!(h.state().canvas.ime_preedit.is_empty(), "the old pre-edit is not kept");
    assert!(ime.surrounding_text.available().is_some(), "the new body still has surroundings");
}

/// Undo (and a version restore) puts a whole body back: the IME generation follows it too.
#[test]
fn ime_generation_changes_on_undo_and_version_restore() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("one")), Default::default());
    app.run("file.versions", json!({"save": "v1"})).unwrap();
    app.run("document.setText", json!({"text": "two"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.step();
    }
    let generation = |h: &egui_kittest::Harness<'_, WordApp>| h.output().platform_output.ime.as_ref().map(|i| i.context_generation);
    let first = generation(&h).expect("an IME context");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.step();
    assert!(generation(&h) > Some(first), "Undo swaps the body back");
    let after_undo = generation(&h);
    h.state_mut().run("file.versions", json!({"restore": 0})).unwrap();
    h.step();
    assert!(generation(&h) > after_undo, "a version restore replaces the body");
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "one");
}

/// With smart quotes off the engine inserts the quote as typed: that is the input method's
/// own edit and earns an `InputMethod` receipt (it used to be forced to `Other`).
#[test]
fn ime_apostrophe_with_smart_quotes_off_is_an_input_method_change() {
    use egui::ImeChangeCause;
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("tools.autocorrect", json!({"smartQuotes": false})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.step();
    }
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("'".into())));
    h.step();
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "'", "the quote is not curled");
    let ime = h.output().platform_output.ime.clone().expect("an IME context");
    assert_eq!(ime.change_cause, ImeChangeCause::InputMethod, "the engine inserted it unchanged");
}

/// With smart quotes on, the same commit is AutoFormat work and stays `Other` (the existing
/// contract), so the fix distinguishes the two instead of always assuming a transformation.
#[test]
fn ime_apostrophe_with_smart_quotes_on_stays_other() {
    use egui::ImeChangeCause;
    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    // Smart quotes are on by default.
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.step();
    }
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("'".into())));
    h.step();
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "\u{2018}", "curled");
    let ime = h.output().platform_output.ime.clone().expect("an IME context");
    assert_eq!(ime.change_cause, ImeChangeCause::Other, "AutoFormat changed the text");
}

/// A real IME-only literal commit is the input method's own edit.
#[test]
fn ime_literal_commit_is_input_method() {
    use egui::ImeChangeCause;
    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.step();
    }
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("literal".into())));
    h.step();
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "literal");
    let ime = h.output().platform_output.ime.clone().expect("an IME context");
    assert_eq!(ime.change_cause, ImeChangeCause::InputMethod);
}

/// A formatting change before an IME commit in the same frame must force `Other`, even
/// though an available body stamp cannot see the style change.
#[test]
fn para_alignment_before_ime_commit_is_other() {
    use egui::ImeChangeCause;
    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("hello")), Default::default());
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.step();
    }
    h.state_mut().session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 5));
    h.step();
    h.state_mut().run("para.alignCenter", json!({})).unwrap();
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("x".into())));
    h.step();
    let ime = h.output().platform_output.ime.clone().expect("an IME context");
    assert_eq!(ime.change_cause, ImeChangeCause::Other, "an application change before the receipt poisons the frame");
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "hellox");
    assert_eq!(
        h.state().session.doc.para_at(&wordcraft_doc::Pos::body(0, 0)).and_then(|p| p.props.align),
        Some(wordcraft_doc::props::Align::Center),
        "the application command really ran"
    );
}

/// A formatting change after an accepted IME commit in the same frame must also force
/// `Other`: the pending application signal survives the receipt and is drained at observation.
#[test]
fn para_alignment_after_ime_commit_is_other() {
    use egui::ImeChangeCause;
    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("hello")), Default::default());
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.step();
    }
    h.state_mut().session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 5));
    h.step();
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("x".into())));
    h.event(egui::Event::Key {
        key: egui::Key::E,
        physical_key: Some(egui::Key::E),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    h.step();
    let ime = h.output().platform_output.ime.clone().expect("an IME context");
    assert_eq!(ime.change_cause, ImeChangeCause::Other, "an application change after the receipt poisons the frame");
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "hellox");
    assert_eq!(
        h.state().session.doc.para_at(&wordcraft_doc::Pos::body(0, 0)).and_then(|p| p.props.align),
        Some(wordcraft_doc::props::Align::Center),
        "the application command really ran"
    );
}

/// A selected non-empty SmartArt graphic consumes an IME commit as an application edit: its
/// item changes, the body does not, and the frame is `Other`.
#[test]
fn smart_art_item_ime_commit_is_other_and_does_not_reach_the_body() {
    use egui::ImeChangeCause;
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("insert.smartArt", json!({"items": ["A"]})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.step();
    }
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("x".into())));
    h.step();
    let body = h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end().to_string();
    assert_eq!(body, "", "the commit must not fall through into the body");
    let item = wordcraft_engine::cmd::smart_art::selected_smart_art(&h.state().session).and_then(|g| g.items.first().map(|i| i.text.clone()));
    assert_eq!(item.as_deref(), Some("Ax"), "the SmartArt item really changed");
    let ime = h.output().platform_output.ime.clone().expect("an IME context");
    assert_eq!(ime.change_cause, ImeChangeCause::Other);
}

/// A selected graphic with no items consumes the commit without editing anything. The body
/// stays empty and no InputMethod receipt is fabricated.
#[test]
fn empty_smart_art_ime_commit_is_consumed_without_body_insertion() {
    use egui::ImeChangeCause;
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("insert.smartArt", json!({"items": ["A"]})).unwrap();
    // `insert.smartArt` refuses an empty item list, but an imported/edited model may hold
    // none: make the selected graphic empty while keeping its identity.
    let at = app.session.sel.ordered().0;
    if let Some(wordcraft_doc::InlineObject::Graphic { graphic, .. }) =
        app.session.doc.para_mut(at.story, &at.path).expect("paragraph").object_at_mut(at.off)
        && let Some(spec) = std::sync::Arc::make_mut(graphic).smart_art.as_mut()
    {
        std::sync::Arc::make_mut(spec).items.clear();
    }
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.step();
    }
    let depth = h.state().session.undo_depth();
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("x".into())));
    h.step();
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "", "no body insertion");
    assert_eq!(h.state().session.undo_depth(), depth, "consuming an empty graphic changes no document");
    let ime = h.output().platform_output.ime.clone().expect("an IME context");
    assert_eq!(ime.change_cause, ImeChangeCause::Other, "no receipt and no application edit");
}

/// A wide selection's surrounding text is unavailable; deleting inside it still earns
/// `InputMethod` when both engine revisions are present and strictly increase.
#[test]
fn ime_wide_unavailable_deletion_uses_engine_revisions() {
    use egui::ImeChangeCause;
    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text(&"a".repeat(5000))), Default::default());
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.step();
    }
    // A wide selection with one character before it: the counted deletion is outside the
    // selection, so the selection stays wide (unavailable) after it.
    h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 1), focus: wordcraft_doc::Pos::body(0, 5000) };
    h.step();
    let before = h.output().platform_output.ime.clone().expect("an IME context");
    assert!(before.surrounding_text.available().is_none(), "the wide selection is unavailable");
    h.event(egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }));
    h.step();
    let body = h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body);
    assert_eq!(body.matches('a').count(), 4999);
    let after = h.output().platform_output.ime.clone().expect("an IME context");
    assert!(after.surrounding_text.available().is_none(), "the selection is still wide after the deletion");
    assert_eq!(after.change_cause, ImeChangeCause::InputMethod, "the engine revisions prove the change");
}

/// A transactional equation delete that cannot make progress is refused as a whole and
/// interrupts the composition without a receipt.
#[test]
fn ime_equation_delete_at_start_is_refused_without_a_receipt() {
    use egui::ImeChangeCause;
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("insert.equation", json!({"linear": "x+y"})).unwrap();
    app.run("equation.move", json!({"dir": "home"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.step();
    }
    h.event(egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { before_chars: 1, after_chars: 0 }));
    h.step();
    let linear = h.state_mut().run("equation.get", json!({})).unwrap()["linear"].as_str().unwrap_or_default().to_string();
    assert_eq!(linear, "x+y", "nothing is deleted");
    let ime = h.output().platform_output.ime.clone().expect("an IME context");
    assert_ne!(ime.change_cause, ImeChangeCause::InputMethod);
    assert!(ime.should_interrupt_composition, "the refused equation delete interrupts the composition");
}

/// A byte-counted replacement inside an equation is refused as a whole: nothing is deleted,
/// only the immediately paired stale `Text`/`Commit` is dropped and the composition is
/// interrupted. A later direct commit in the same raw batch or a later frame is fresh input
/// and applies, so the adjacent-pair bound never swallows it.
#[test]
fn ime_equation_byte_delete_drops_only_its_immediate_replacement() {
    use egui::ImeChangeCause;
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("insert.equation", json!({"linear": "x+y"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.step();
    }
    let linear = |h: &mut egui_kittest::Harness<'_, WordApp>| {
        h.state_mut().run("equation.get", json!({})).unwrap()["linear"].as_str().unwrap_or_default().to_string()
    };
    // Harness::event steps each queued event in a separate frame. Insert the complete
    // protocol replacement into RawInput to exercise a single native input batch.
    h.input_mut().events.extend([
        egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }),
        egui::Event::Ime(egui::ImeEvent::Commit("Z".into())),
        egui::Event::Ime(egui::ImeEvent::Commit("W".into())),
    ]);
    h.step();
    assert_eq!(linear(&mut h), "x+yW", "the adjacent stale commit is dropped and the next direct commit applies");
    let ime = h.output().platform_output.ime.clone().expect("still editing the equation");
    assert_eq!(ime.change_cause, ImeChangeCause::InputMethod, "the fresh direct commit is the input method's change");
    assert!(ime.should_interrupt_composition, "the refused byte deletion still interrupts the stale composition");

    // An adjacent `Text` + `Commit` pair is one replacement; a third commit is fresh.
    let base = linear(&mut h);
    h.input_mut().events.extend([
        egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }),
        egui::Event::Text("Q".into()),
        egui::Event::Ime(egui::ImeEvent::Commit("Q2".into())),
        egui::Event::Ime(egui::ImeEvent::Commit("R".into())),
    ]);
    h.step();
    assert_eq!(linear(&mut h), format!("{base}R"), "the adjacent pair is dropped, not every later commit");
    let ime = h.output().platform_output.ime.clone().expect("still editing the equation");
    assert_eq!(ime.change_cause, ImeChangeCause::InputMethod);
    assert!(ime.should_interrupt_composition);

    // A refused deletion with no replacement in its batch must not swallow a later direct
    // commit, even with no pre-edit before it.
    h.event(egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }));
    h.step();
    let base = linear(&mut h);
    assert_eq!(linear(&mut h), "x+yWR", "the refused deletion alone changed nothing");
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("Y".into())));
    h.step();
    assert_eq!(linear(&mut h), format!("{base}Y"), "a later direct commit applies without a pre-edit");
    let ime = h.output().platform_output.ime.clone().expect("still editing the equation");
    assert_eq!(ime.change_cause, ImeChangeCause::InputMethod, "the fresh commit is the input method's change");
    assert!(!ime.should_interrupt_composition, "the fresh commit is not treated as stale");
}

/// A satisfied transactional equation delete commits content and earns one `InputMethod`
/// receipt over the checked unavailable revisions.
#[test]
fn ime_equation_delete_is_one_input_method_change() {
    use egui::ImeChangeCause;
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("insert.equation", json!({"linear": "x+y"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.step();
    }
    h.event(egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { before_chars: 1, after_chars: 0 }));
    h.step();
    let linear = h.state_mut().run("equation.get", json!({})).unwrap()["linear"].as_str().unwrap_or_default().to_string();
    assert_eq!(linear, "x+");
    let ime = h.output().platform_output.ime.clone().expect("still editing the equation");
    assert_eq!(ime.change_cause, ImeChangeCause::InputMethod, "the engine revision proves the delete");
}

/// An application change in an equation context and an IME commit in the same frame are
/// `Other` in both orders, just as for body text.
#[test]
fn ime_equation_context_mixes_with_application_changes() {
    use egui::ImeChangeCause;
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("insert.equation", json!({"linear": "x"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.step();
    }
    h.state_mut().run("equation.justify", json!({"jc": "left"})).unwrap();
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("y".into())));
    h.step();
    let before = h.output().platform_output.ime.clone().expect("still editing");
    assert_eq!(before.change_cause, ImeChangeCause::Other, "an application change before the receipt poisons the frame");
    let linear = h.state_mut().run("equation.get", json!({})).unwrap()["linear"].as_str().unwrap_or_default().to_string();
    assert_eq!(linear, "xy");

    h.event(egui::Event::Ime(egui::ImeEvent::Commit("z".into())));
    h.step();
    let accepted = h.output().platform_output.ime.clone().expect("still editing");
    assert_eq!(accepted.change_cause, ImeChangeCause::InputMethod);
    h.state_mut().run("equation.justify", json!({"jc": "right"})).unwrap();
    h.step();
    let after = h.output().platform_output.ime.clone().expect("still editing");
    assert_eq!(after.change_cause, ImeChangeCause::Other, "an application change after the receipt poisons the frame");
}

/// Replacing the edited equation under the same document id gives it a fresh session token:
/// the IME context changes and the old composition is interrupted.
#[test]
fn ime_equation_replacement_starts_a_new_context() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("insert.equation", json!({"linear": "x+y"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.step();
    }
    let first = h.output().platform_output.ime.as_ref().map(|i| i.context_generation).expect("an IME context");
    h.state_mut().run("insert.equation", json!({"linear": "z"})).unwrap();
    h.step();
    let ime = h.output().platform_output.ime.clone().expect("the new equation owns the context");
    assert!(ime.context_generation > first, "a new equation editing session is a new context");
    assert!(ime.should_interrupt_composition, "the old equation composition is interrupted");
    assert!(h.state().session.math.is_some(), "the new equation is being edited");
}

#[test]
fn context_menu_crop_item_activates_crop_mode() {
    use egui_kittest::kittest::Queryable;
    use wordcraft_doc::para::{InlineObject, OBJ};
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    let pic = InlineObject::Image { media: "m".into(), w: 200.0, h: 100.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None };
    app.session.doc.insert_object(&Pos::body(0, 0), pic, &Default::default()).unwrap();
    let end = Pos::body(0, OBJ.len_utf8());
    app.run("select.range", json!({"anchor": Pos::body(0, 0), "focus": end})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..6 {
        h.step();
    }
    let (page, rect) = h.state_mut().session.layout().object(&Pos::body(0, 0), 0).map(|o| (o.page, o.rect)).expect("laid out");
    let at = page_to_screen(h.state_mut(), page, rect.x + rect.w / 2.0, rect.y + rect.h / 2.0).expect("on screen");
    h.event(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
        h.step();
    }
    h.step();
    assert!(h.state().canvas.context_menu_open, "context menu is open on image");
    h.get_by_label("Crop ").click();
    for _ in 0..3 {
        h.step();
    }
    assert!(crate::crop::active(h.state()).is_some(), "crop mode is active after clicking Crop in context menu");
}

#[test]
fn crop_is_disabled_when_cursor_is_adjacent_without_selection() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    let pic = wordcraft_doc::para::InlineObject::Image {
        media: "m".into(),
        w: 200.0,
        h: 100.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        ole: None,
    };
    app.session.doc.insert_object(&Pos::body(0, 0), pic, &Default::default()).unwrap();
    // Caret is collapsed right at pos 0 (adjacent to the picture).
    app.session.sel = wordcraft_engine::Selection::caret(Pos::body(0, 0));
    assert!(crate::ribbon::has_picture_selected(&app.session), "adjacent caret sees picture in ribbon");
    assert!(!crate::widgets::enabled(&app, "ui.crop"), "ui.crop is disabled when not selected as object");
    assert!(crate::crop::active(&app).is_none());
}

fn page_at_index(rects: &[Rect], p: Pos2) -> Option<usize> {
    nearest_page(rects, p)
}

/// A page drawn without a dragged object must not pass for the page with it: the drag's start
/// and end change the page's texture key once each, so the page is rendered again then and
/// only then. Without this, the old raster — the object still on it where it was — stays while
/// the preview follows the pointer, and the object shows twice.
#[test]
fn the_view_key_changes_when_a_drag_takes_an_object_off_the_page() {
    let doc = wordcraft_doc::Document::from_text("Hello WordCraft");
    let app = WordApp::new(wordcraft_engine::Session::new(doc), Default::default());
    let object = Pos::body(0, 5);
    let hidden = view_key(&app, false, Some(&object));
    assert_ne!(hidden, view_key(&app, false, None), "the page without the dragged object is rendered again");
    assert_eq!(hidden, view_key(&app, false, Some(&object)), "the same drag keeps the key: no re-render every frame");
}
