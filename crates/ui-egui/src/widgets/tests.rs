use craft::ui::buttons::IconButton;
use craft::ui::contract::{Props as _, Show as _};
use egui_kittest::kittest::Queryable;

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
                && r.fill != egui::Color32::TRANSPARENT
                && r.corner_radius != egui::CornerRadius::ZERO =>
        {
            Some((r.fill, r.corner_radius))
        }
        _ => None,
    })
}

/// #305: drawn buttons must react like standard buttons — resting at idle without a fill plate,
/// fading in through an intermediate fill on pointer hover, settling on the theme's hover state
/// token, darkening/transitioning to the pressed state token while held down, and clearing when
/// unhovered.
#[test]
fn drawn_buttons_fade_in_on_hover_and_darken_while_pressed() {
    use egui::accesskit::Role;

    let mut h = crate::test_harness::builder().with_size(egui::vec2(200.0, 100.0)).with_step_dt(0.04).build_ui(|ui| {
        ui.horizontal(|ui| {
            let _ = IconButton::new(crate::icons::id("save")).label("Save").show(ui);
        });
    });
    h.ctx.all_styles_mut(|s| s.animation_time = egui::Style::default().animation_time);
    h.step();

    let env = craft::ui::runtime::env::Env::of_root(&h.ctx);
    let expected_hover = to_color32(env.tokens.color.state.hover);
    let expected_pressed = to_color32(env.tokens.color.state.pressed);

    let node = h.get_by_role_and_label(Role::Button, "Save");
    let rect = node.rect();
    assert!(rect.is_positive(), "Save button must have positive rect");

    // 1. Idle state: no hover or pressed fill plate
    let fill_before = find_button_fill(&h.output().shapes, rect);
    assert_eq!(fill_before, None, "Save button must have no hover fill before hover");

    // 2. Hover state: fades in through an intermediate fill, then settles on the hover token
    node.hover();
    h.run_steps(2);
    let fill_mid = find_button_fill(&h.output().shapes, rect);
    assert!(fill_mid.is_some(), "Save button must paint an intermediate fill while fading in");
    let (mid_fill, _) = fill_mid.unwrap();
    assert!(
        mid_fill != expected_hover && mid_fill.a() > 0 && mid_fill.a() < expected_hover.a(),
        "Save button hover should fade in through an intermediate alpha: got {mid_fill:?}, expected settled {expected_hover:?}"
    );

    h.run_steps(3);
    let fill_hovered = find_button_fill(&h.output().shapes, rect);
    assert!(fill_hovered.is_some(), "Save button must produce settled hover shape");
    let (hover_fill, radius) = fill_hovered.unwrap();
    assert_eq!(hover_fill, expected_hover, "Save button settled hover fill must match theme hover token");
    assert!(
        radius.nw > 0 && radius.nw == radius.ne && radius.nw == radius.sw && radius.nw == radius.se,
        "Save button corner radius must be uniformly rounded: {:?}",
        radius
    );

    // 3. Pressed state: pressing pointer button darkens/transitions to theme's pressed state token
    h.event(egui::Event::PointerButton { pos: rect.center(), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(4);
    let fill_pressed = find_button_fill(&h.output().shapes, rect);
    assert!(fill_pressed.is_some(), "Save button must have fill while pressed");
    let (pressed_fill, _) = fill_pressed.unwrap();
    assert_eq!(pressed_fill, expected_pressed, "Save button pressed fill must match theme pressed token");

    // 4. Release: releasing pointer button transitions back to hover state
    h.event(egui::Event::PointerButton {
        pos: rect.center(),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(4);
    let fill_released = find_button_fill(&h.output().shapes, rect);
    assert!(fill_released.is_some(), "Save button must remain filled upon release");
    let (released_fill, _) = fill_released.unwrap();
    assert_eq!(released_fill, expected_hover, "Save button fill after release should return to hover token");

    // 5. Unhover: pointer leaving the button clears the fill plate
    h.event(egui::Event::PointerMoved(egui::pos2(0.0, 0.0)));
    h.run_steps(4);
    let fill_after = find_button_fill(&h.output().shapes, rect);
    assert_eq!(fill_after, None, "Save button must clear hover fill after unhovering");
}
