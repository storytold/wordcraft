//! Quick access to the selected picture's text-wrapping options (#6064).

use egui::{Context, Order, Popup, Pos2, Rect, vec2};
use wordcraft_doc::{Pos, para::InlineObject};
use wordcraft_layout::DocLayout;

use crate::{WordApp, theme, widgets};

const SIZE: f32 = 28.0;
const GAP: f32 = 8.0;

/// Place the button beside the picture's visible bounds, keeping it in the document viewport.
/// Leave a gap for the resize handles; prefer the right edge, falling back to the left.
fn button_position(picture: Rect, viewport: Rect) -> Option<Pos2> {
    if !picture.is_finite()
        || !viewport.is_finite()
        || viewport.width() < SIZE
        || viewport.height() < SIZE
        || !picture.intersect(viewport).is_positive()
    {
        return None;
    }
    let right = picture.right() + GAP;
    let x = if right + SIZE <= viewport.right() { right } else { picture.left() - GAP - SIZE };
    Some(egui::pos2(x.clamp(viewport.left(), viewport.right() - SIZE), picture.top().clamp(viewport.top(), viewport.bottom() - SIZE)))
}

fn anchor(app: &WordApp, layout: &DocLayout, pages: &[Rect], scale: f32, viewport: Rect) -> Option<(Pos, Pos2)> {
    // Editing overlays must not cover a modal, a context menu or crop/drag handles, nor offer
    // formatting in Read Mode. A picture selected inside a Drawing Canvas has its own controls.
    if app.session.view.read_mode
        || app.dialog.is_some()
        || app.colors.is_some()
        || app.ui.backstage
        || app.canvas.context_menu_open
        || app.canvas.crop.is_some()
        || app.canvas.obj_drag.is_some()
        || app.ui.keytips != crate::keytips::Phase::Off
        || app.session.cell_range().is_some()
        || wordcraft_engine::cmd::canvas::selected_member(&app.session).is_some()
    {
        return None;
    }
    let (pos, InlineObject::Image { .. }) = crate::objects::selected(app)? else { return None };
    let object = layout.object(&pos, app.session.page_hint)?;
    let page = *pages.get(object.page)?;
    let picture = crate::frame::to_screen(page, scale, object.bounds());
    button_position(picture, viewport).map(|at| (pos, at))
}

/// A page overlay on the Middle layer: above the document, below menus, with its own hit area
/// so opening the popup cannot deselect the picture by clicking the underlying paragraph.
pub(crate) fn show(app: &mut WordApp, ctx: &Context, layout: &DocLayout, pages: &[Rect], scale: f32, viewport: Rect) {
    let id = app.window_id("picture_layout_options");
    let popup = id.with("popup");
    let owner = id.with("picture");
    let Some((pos, at)) = anchor(app, layout, pages, scale, viewport) else {
        Popup::close_id(ctx, popup);
        ctx.data_mut(|d| d.remove::<Pos>(owner));
        return;
    };
    let previous = ctx.data_mut(|d| {
        let previous = d.get_temp::<Pos>(owner);
        d.insert_temp(owner, pos.clone());
        previous
    });
    // A popup belongs to one selected picture; changing the selection must not retarget it.
    if previous.as_ref() != Some(&pos) {
        Popup::close_id(ctx, popup);
    }
    egui::Area::new(id).order(Order::Middle).fixed_pos(at).constrain_to(viewport).show(ctx, |ui| {
        ui.spacing_mut().button_padding = vec2(6.0, 6.0);
        let response = ui
            .add_enabled_ui(widgets::enabled(app, "arrange.wrap"), |ui| {
                widgets::icon_only_button(ui, "wrapText", tl!("Layout Options"), theme::regular(12.0), false)
            })
            .inner;
        Popup::menu(&response).id(popup).show(|ui| {
            crate::rtl::root(ui, |ui| crate::ribbon::wrap_text_items(ui, app));
        });
    });
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;
    use serde_json::json;
    use wordcraft_doc::para::Wrap;

    use crate::table_resize::tests::{H, steps};
    use crate::{Services, WordApp};

    fn window() -> H {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        app.run("view.commentsPane", json!({"value": false})).unwrap();
        let image = image::RgbaImage::from_pixel(12, 8, image::Rgba([30, 120, 180, 255]));
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let data = wordcraft_engine::cmd::insert::base64_encode(png.get_ref());
        app.run("insert.picture", json!({"data": data})).unwrap();
        app.run("picture.size", json!({"width": 180, "height": 120})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        steps(&mut h, 6);
        h
    }

    /// The picture stays selected while the popup opens, and its choices use the existing
    /// undoable arrange commands rather than moving the caret into the document underneath.
    #[test]
    fn layout_options_changes_wrapping_without_losing_the_picture() {
        let mut h = window();
        let selection = h.state().session.sel.clone();
        h.state_mut().run("caret.docEnd", json!({})).unwrap();
        h.state_mut().run("text.insert", json!({"text": "Text must stay editable in front of a picture. ".repeat(16)})).unwrap();
        h.state_mut().session.sel = selection.clone();
        steps(&mut h, 6);
        // Start inline, then exercise every choice through the popup, ending inline again.
        for (label, expected) in [
            ("Square", Wrap::Square),
            ("Tight", Wrap::Tight),
            ("Through", Wrap::Through),
            ("Top and Bottom", Wrap::TopAndBottom),
            ("Behind Text", Wrap::BehindText),
            ("In Front of Text", Wrap::InFrontOfText),
            ("In Line with Text", Wrap::Inline),
        ] {
            let before = h.state().session.doc.clone();
            let depth = h.state().session.undo_depth();
            h.get_by_label("Layout Options").click();
            steps(&mut h, 3);
            assert_eq!(h.state().session.sel, selection);
            assert_eq!(h.state().session.doc, before);
            assert_eq!(h.state().session.undo_depth(), depth);
            h.get_by_label(label).click();
            steps(&mut h, 3);
            assert_eq!(h.state().session.sel, selection);
            let (_, object) = crate::objects::selected(h.state()).unwrap();
            assert_eq!(object.frame().unwrap().2.wrap, expected, "{label}");
            assert_eq!(h.state().session.undo_depth(), depth + 1);
            assert!(h.query_all_by_label(label).next().is_none(), "choosing {label} closes the popup");
            let after = h.state().session.doc.clone();
            h.state_mut().run("edit.undo", json!({})).unwrap();
            assert_eq!(h.state().session.doc, before, "undo {label}");
            h.state_mut().run("edit.redo", json!({})).unwrap();
            assert_eq!(h.state().session.doc, after, "redo {label}");
            steps(&mut h, 3);
        }
    }

    #[test]
    fn clicking_a_picture_shows_layout_options_and_escape_dismisses_the_popup() {
        let mut h = window();
        let selection = h.state().session.sel.clone();
        let (pos, _) = crate::objects::selected(h.state()).unwrap();
        h.state_mut().run("caret.docEnd", json!({})).unwrap();
        steps(&mut h, 3);
        assert!(h.query_all_by_label("Layout Options").next().is_none());
        let layout = h.state_mut().session.layout();
        let object = layout.object(&pos, h.state().session.page_hint).unwrap();
        let rect = object.rect;
        let at = crate::canvas::page_to_screen(h.state_mut(), object.page, rect.x + rect.w / 2.0, rect.y + rect.h / 2.0).unwrap();
        h.event(egui::Event::PointerMoved(at));
        crate::table_resize::tests::button(&h, at, true);
        h.step();
        crate::table_resize::tests::button(&h, at, false);
        steps(&mut h, 3);
        assert_eq!(h.state().session.sel, selection);
        let before = h.state().session.doc.clone();
        let depth = h.state().session.undo_depth();
        h.get_by_label("Layout Options").click();
        steps(&mut h, 3);
        assert!(h.query_all_by_label("Square").next().is_some());
        h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
        steps(&mut h, 3);
        assert!(h.query_all_by_label("Square").next().is_none());
        assert!(h.query_all_by_label("Layout Options").next().is_some());
        assert_eq!(h.state().session.sel, selection);
        assert_eq!(h.state().session.doc, before);
        assert_eq!(h.state().session.undo_depth(), depth);
    }

    #[test]
    fn a_behind_text_picture_can_be_reselected_on_its_uncovered_area() {
        let mut h = window();
        let selection = h.state().session.sel.clone();
        let (pos, _) = crate::objects::selected(h.state()).unwrap();
        h.state_mut().run("caret.docEnd", json!({})).unwrap();
        h.state_mut().run("text.insert", json!({"text": "Text in front of the picture"})).unwrap();
        h.state_mut().session.sel = selection.clone();
        steps(&mut h, 6);
        h.get_by_label("Layout Options").click();
        steps(&mut h, 3);
        h.get_by_label("Behind Text").click();
        steps(&mut h, 3);
        assert_eq!(h.state().session.sel, selection, "changing wrapping keeps the picture selected");

        let click = |h: &mut H, at| {
            h.event(egui::Event::PointerMoved(at));
            crate::table_resize::tests::button(h, at, true);
            h.step();
            crate::table_resize::tests::button(h, at, false);
            steps(h, 3);
        };
        let layout = h.state_mut().session.layout();
        let text = wordcraft_doc::Pos { off: pos.off + wordcraft_doc::para::OBJ.len_utf8(), ..pos.clone() };
        let caret = layout.caret(&text).unwrap();
        let on_text = crate::canvas::page_to_screen(h.state_mut(), caret.page, caret.x + 2.0, caret.top + caret.height / 2.0).unwrap();
        click(&mut h, on_text);
        assert!(crate::objects::selected(h.state()).is_none(), "text in front of the picture must still take a plain click");
        assert!(h.query_all_by_label("Layout Options").next().is_none());

        let object = layout.object(&pos, h.state().session.page_hint).unwrap();
        let rect = object.rect;
        let on_picture = crate::canvas::page_to_screen(h.state_mut(), object.page, rect.x + rect.w / 2.0, rect.y + rect.h * 0.75).unwrap();
        click(&mut h, on_picture);
        assert_eq!(h.state().session.sel, selection, "an uncovered part of the picture must allow selecting it again");
        h.get_by_label("Layout Options").click();
        steps(&mut h, 3);
        h.get_by_label("In Front of Text").click();
        steps(&mut h, 3);
        assert_eq!(crate::objects::selected(h.state()).unwrap().1.frame().unwrap().2.wrap, Wrap::InFrontOfText);
    }

    #[test]
    fn layout_options_closes_when_the_picture_is_no_longer_selected() {
        let mut h = window();
        let selection = h.state().session.sel.clone();
        h.get_by_label("Layout Options").click();
        steps(&mut h, 3);
        assert!(h.query_all_by_label("Square").next().is_some());
        h.state_mut().run("caret.docEnd", json!({})).unwrap();
        steps(&mut h, 3);
        assert!(h.query_all_by_label("Layout Options").next().is_none());
        assert!(h.query_all_by_label("Square").next().is_none());
        h.state_mut().session.sel = selection;
        steps(&mut h, 3);
        assert!(h.query_all_by_label("Layout Options").next().is_some());
        assert!(h.query_all_by_label("Square").next().is_none(), "reselecting must not reopen a stale popup");
        h.state_mut().run("ui.crop", json!({})).unwrap();
        steps(&mut h, 3);
        assert!(h.query_all_by_label("Layout Options").next().is_none());
        h.state_mut().run("ui.crop", json!({})).unwrap();
        h.state_mut().run("view.readMode", json!({})).unwrap();
        steps(&mut h, 3);
        assert!(h.query_all_by_label("Layout Options").next().is_none());
        h.state_mut().run("view.printLayout", json!({})).unwrap();
        h.state_mut().run("insert.shape", json!({"kind": "rectangle"})).unwrap();
        steps(&mut h, 3);
        assert!(h.query_all_by_label("Layout Options").next().is_none(), "this control belongs only to pictures");
    }

    fn assert_next_to_picture(h: &mut H) {
        let (pos, _) = crate::objects::selected(h.state()).unwrap();
        let layout = h.state_mut().session.layout();
        let object = layout.object(&pos, h.state().session.page_hint).unwrap();
        let bounds = object.bounds();
        let min = crate::canvas::page_to_screen(h.state_mut(), object.page, bounds.x, bounds.y).unwrap();
        let max = crate::canvas::page_to_screen(h.state_mut(), object.page, bounds.right(), bounds.bottom()).unwrap();
        let viewport = h.state().canvas.viewport.unwrap();
        let expected = super::button_position(egui::Rect::from_min_max(min, max), viewport).unwrap();
        let actual = h.get_by_label("Layout Options").rect();
        assert!(actual.min.distance(expected) < 1.0, "button {actual:?} must follow picture at {expected:?}");
        assert!(viewport.contains_rect(actual), "the button cannot cover the ribbon or status bar");
    }

    #[test]
    fn layout_options_follows_zoom_rotation_and_scrolling() {
        let mut h = window();
        assert_next_to_picture(&mut h);
        h.state_mut().run("view.zoom", json!({"value": 150})).unwrap();
        steps(&mut h, 6);
        assert_next_to_picture(&mut h);
        h.state_mut().run("arrange.rotation", json!({"degrees": 30})).unwrap();
        steps(&mut h, 6);
        assert_next_to_picture(&mut h);
        let old = h.get_by_label("Layout Options").rect().min;
        let viewport = h.state().canvas.viewport.unwrap();
        h.event(egui::Event::PointerMoved(viewport.center()));
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -40.0),
            modifiers: Default::default(),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
        });
        steps(&mut h, 12);
        assert_next_to_picture(&mut h);
        assert!(h.get_by_label("Layout Options").rect().min.y < old.y - 10.0, "the button scrolls with the picture");
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -1600.0),
            modifiers: Default::default(),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
        });
        steps(&mut h, 18);
        assert!(h.query_all_by_label("Layout Options").next().is_none(), "an offscreen picture has no floating button");
    }

    #[test]
    fn layout_options_stays_inside_the_viewport_at_the_picture_edges() {
        use egui::{Rect, pos2, vec2};
        let viewport = Rect::from_min_size(pos2(100.0, 200.0), vec2(400.0, 300.0));
        let right_edge = Rect::from_min_size(pos2(430.0, 250.0), vec2(60.0, 80.0));
        let at = super::button_position(right_edge, viewport).unwrap();
        assert!(at.x + super::SIZE < right_edge.left(), "no room on the right: use the left");
        let partly_above = Rect::from_min_size(pos2(200.0, 150.0), vec2(60.0, 80.0));
        assert_eq!(super::button_position(partly_above, viewport).unwrap().y, viewport.top());
        let offscreen = right_edge.translate(vec2(0.0, -500.0));
        assert!(super::button_position(offscreen, viewport).is_none());
        assert!(super::button_position(right_edge, Rect::NOTHING).is_none());
    }
}
