//! Layout commands beside the selected picture, using the shared task bar and menu.

use crate::WordApp;
use craft::behavior::{IconId, overlay::Side};
use craft::pages::ui::PageBody;
use craft::ui::buttons::IconButton;
use craft::ui::canvas_controls::{TaskBar, TaskBarDismiss};
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, Trigger};
use craft::ui::menu::Menu;
use egui::{Rect, Ui};
use wordcraft_doc::{Pos, para::InlineObject};
use wordcraft_layout::DocLayout;

fn anchor(app: &WordApp, layout: &DocLayout, body: &PageBody<'_>) -> Option<(Pos, Rect)> {
    if app.session.view.read_mode
        || app.modal_open()
        || app.ui.backstage
        || app.canvas.context_menu_open
        || app.canvas.crop.is_some()
        || app.canvas.object_gesture.is_some()
        || app.canvas.frame_held
        || app.keytips_active
        || app.session.cell_range().is_some()
        || wordcraft_engine::cmd::canvas::selected_member(&app.session).is_some()
    {
        return None;
    }
    let (pos, InlineObject::Image { .. }) = crate::objects::selected(app)? else { return None };
    let object = layout.object(&pos, app.session.page_hint)?;
    let page = body.pages.iter().find(|p| p.page == object.page)?;
    let bounds = object.bounds();
    let scale = page.scale as f32;
    let picture = Rect::from_min_size(page.rect.min + egui::vec2(bounds.x, bounds.y) * scale, egui::vec2(bounds.w, bounds.h) * scale);
    (picture.is_finite() && picture.intersect(body.view.rect).is_positive()).then_some((pos, picture))
}

#[derive(Clone, Default)]
struct MenuState {
    picture: Option<(u64, Pos)>,
    frame: u64,
    open: bool,
}

pub(crate) fn show(app: &mut WordApp, ui: &mut Ui, layout: &DocLayout, body: &PageBody<'_>) {
    let key = ui.make_persistent_id("picture_layout_menu");
    let Some((pos, picture)) = anchor(app, layout, body) else {
        ui.data_mut(|d| d.remove::<MenuState>(key));
        return;
    };
    let mut state = ui.data(|d| d.get_temp::<MenuState>(key)).unwrap_or_default();
    let selected = Some((app.session.document_id(), pos.clone()));
    let frame = ui.ctx().cumulative_frame_nr();
    // A hidden canvas (Read Mode or File) must not bring back a menu left open on its last frame.
    if state.picture != selected || frame.saturating_sub(state.frame) > 1 {
        state.open = false;
    }
    state.picture = selected;
    state.frame = frame;
    // Hang from the picture's top edge: the middle of its right edge belongs to the resize and rotation handles.
    let corners: Vec<_> = [picture.left_top(), picture.right_top()].into_iter().map(|p| body.view.to_doc(p)).collect();
    TaskBar::selection(tl!("Picture").to_string(), &corners).side(Side::End).dismiss(TaskBarDismiss::Keep).show(ui, body.view.rect, |ui, _| {
        let enabled = crate::app::command_board::command_enabled(app, "arrange.wrap");
        let opener = IconButton::new(IconId::new("wrapText"))
            .label(tl!("Layout Options").to_string())
            .id_salt((app.session.document_id(), &pos))
            .enabled(enabled)
            .show(ui);
        if opener.clicked() {
            state.open = !state.open;
        }
        Menu::new(tl!("Layout Options").to_string())
            .open(Bind::bound(&mut state.open))
            .opened_by(&opener.response, Trigger::Click)
            .show(ui, |ui, _| crate::ribbon::wrap_text_items(ui, app));
    });
    ui.data_mut(|d| d.insert_temp(key, state));
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;
    use serde_json::json;
    use wordcraft_doc::para::Wrap;

    use crate::table_resize::tests::{H, steps};
    use crate::test_harness::AppFrames as _;
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
        let mut h = crate::test_harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(
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
        h.app_frame();
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
            h.app_frame();
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
        steps(&mut h, 3);
        h.get_by_label("Layout Options").click();
        steps(&mut h, 3);
        assert!(h.query_all_by_label("Square").next().is_some());
        h.state_mut().run("view.readMode", json!({})).unwrap();
        steps(&mut h, 3);
        assert!(h.query_all_by_label("Layout Options").next().is_none());
        h.state_mut().run("view.printLayout", json!({})).unwrap();
        steps(&mut h, 3);
        assert!(h.query_all_by_label("Layout Options").next().is_some());
        assert!(h.query_all_by_label("Square").next().is_none(), "leaving Read Mode must not reopen the old menu");
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
        let viewport = h.state().canvas.view_rect().unwrap();
        let picture = egui::Rect::from_min_max(min, max);
        let actual = h.get_by_label("Layout Options").rect();
        assert!(
            actual.center().y >= picture.top() - actual.height() && actual.center().y <= picture.bottom() + actual.height(),
            "the button follows the selected picture"
        );
        assert!(
            actual.left() >= picture.right() || actual.right() <= picture.left() || picture.width() > viewport.width() - actual.width() * 2.0,
            "the button sits beside the picture when there is room"
        );
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
        let viewport = h.state().canvas.view_rect().unwrap();
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
        let mut h = window();
        h.state_mut().run("arrange.wrap", json!({"wrap":"inFrontOfText"})).unwrap();
        for x in [0.0, 500.0] {
            h.state_mut().run("arrange.bounds", json!({"x":x,"y":100})).unwrap();
            steps(&mut h, 6);
            let viewport = h.state().canvas.view_rect().unwrap();
            let actual = h.get_by_label("Layout Options").rect();
            assert!(viewport.contains_rect(actual), "edge controls stay in the viewport");
        }
    }
}
