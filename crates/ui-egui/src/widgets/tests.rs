use crate::WordApp;
use egui_kittest::kittest::Queryable;
use serde_json::json;

fn app_frame(ui: &mut egui::Ui, app: &mut WordApp) {
    let ctx = ui.ctx().clone();
    app.logic(&ctx);
    app.ui(ui);
}

fn new_app() -> WordApp {
    WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default())
}

fn to_color32(rgba: craft::theme::Rgba) -> egui::Color32 {
    egui::Color32::from(craft::theme::Srgba::from(rgba))
}

fn collect_shapes<'a>(shapes: &'a [egui::epaint::ClippedShape], out: &mut Vec<&'a egui::Shape>) {
    for clipped in shapes {
        fn walk<'a>(shape: &'a egui::Shape, out: &mut Vec<&'a egui::Shape>) {
            match shape {
                egui::Shape::Vec(children) => {
                    for child in children {
                        walk(child, out);
                    }
                }
                other => out.push(other),
            }
        }
        walk(&clipped.shape, out);
    }
}

fn find_button_fill(shapes: &[egui::epaint::ClippedShape], rect: egui::Rect) -> Option<(egui::Color32, egui::CornerRadius)> {
    let mut flattened = Vec::new();
    collect_shapes(shapes, &mut flattened);
    flattened.into_iter().find_map(|s| match s {
        egui::Shape::Rect(r)
            if (r.rect.min.x - rect.min.x).abs() <= 1.5
                && (r.rect.max.x - rect.max.x).abs() <= 1.5
                && (r.rect.min.y - rect.min.y).abs() <= 1.5
                && (r.rect.max.y - rect.max.y).abs() <= 1.5
                && r.fill != egui::Color32::TRANSPARENT =>
        {
            Some((r.fill, r.corner_radius))
        }
        _ => None,
    })
}

/// #305: drawn buttons must react like standard buttons — resting at idle without a fill plate,
/// fading in to the theme's hover state token on pointer hover, darkening/transitioning to the
/// pressed state token while held down, and clearing when unhovered.
#[test]
fn drawn_buttons_fade_in_on_hover_and_darken_while_pressed() {
    use egui::accesskit::Role;

    for theme in ["light", "dark"] {
        let mut a = new_app();
        a.run("text.insert", json!({"text": "Hello"})).unwrap();
        a.run("select.all", json!({})).unwrap();
        a.run("ui.theme", json!({"value": theme})).unwrap();
        let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 700.0)).with_step_dt(0.016).build_ui_state(app_frame, a);
        h.ctx.all_styles_mut(|s| s.animation_time = egui::Style::default().animation_time);
        crate::test_harness::frames(&mut h, 10);

        let env = craft::ui::runtime::env::Env::of_root(&h.ctx);
        let expected_hover = to_color32(env.tokens.color.state.hover);
        let expected_pressed = to_color32(env.tokens.color.state.pressed);

        // Test representative drawn icon buttons across the chrome and ribbon (Save in QAT, Cut in Ribbon, Zoom In in status bar).
        for (role, label, desc) in [
            (Role::Button, "Save", "Quick Access Toolbar button"),
            (Role::Button, "Cut", "Ribbon tool button"),
            (Role::Button, "Zoom In", "Status bar zoom-in button"),
        ] {
            let node = h.get_by_role_and_label(role, label);
            let rect = node.rect();
            assert!(rect.is_positive(), "{theme}: {desc} ({label}) must have positive rect");

            // 1. Idle state: no hover or pressed fill plate
            let fill_before = find_button_fill(&h.output().shapes, rect);
            assert_eq!(fill_before, None, "{theme}: {desc} ({label}) must have no hover fill before hover");

            // 2. Hover state: hover fades in and settles on the theme's hover state token
            node.hover();
            crate::test_harness::frames(&mut h, 8);
            let fill_hovered = find_button_fill(&h.output().shapes, rect);
            assert!(fill_hovered.is_some(), "{theme}: {desc} ({label}) must produce settled hover shape");
            let (hover_fill, radius) = fill_hovered.unwrap();
            assert_eq!(hover_fill, expected_hover, "{theme}: {desc} ({label}) settled hover fill must match theme hover token");
            assert!(
                radius.nw > 0 && radius.nw == radius.ne && radius.nw == radius.sw && radius.nw == radius.se,
                "{theme}: {desc} ({label}) corner radius must be uniformly rounded: {:?}",
                radius
            );

            // 3. Pressed state: pressing pointer button darkens/transitions to theme's pressed state token
            h.event(egui::Event::PointerButton {
                pos: rect.center(),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
            crate::test_harness::frames(&mut h, 8);
            let fill_pressed = find_button_fill(&h.output().shapes, rect);
            assert!(fill_pressed.is_some(), "{theme}: {desc} ({label}) must have fill while pressed");
            let (pressed_fill, _) = fill_pressed.unwrap();
            assert_eq!(pressed_fill, expected_pressed, "{theme}: {desc} ({label}) pressed fill must match theme pressed token");

            // 4. Release: releasing pointer button transitions back to hover state
            h.event(egui::Event::PointerButton {
                pos: rect.center(),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
            crate::test_harness::frames(&mut h, 8);
            let fill_released = find_button_fill(&h.output().shapes, rect);
            assert!(fill_released.is_some(), "{theme}: {desc} ({label}) must remain filled upon release");
            let (released_fill, _) = fill_released.unwrap();
            assert_eq!(released_fill, expected_hover, "{theme}: {desc} ({label}) fill after release should return to hover token");

            // 5. Unhover: pointer leaving the button clears the fill plate
            h.event(egui::Event::PointerMoved(egui::pos2(0.0, 0.0)));
            crate::test_harness::frames(&mut h, 8);
            let fill_after = find_button_fill(&h.output().shapes, rect);
            assert_eq!(fill_after, None, "{theme}: {desc} ({label}) must clear hover fill after unhovering");
        }
    }
}
