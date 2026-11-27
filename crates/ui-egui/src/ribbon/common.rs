use std::ops::RangeInclusive;

use craft::keymap::CommandRef;
use craft::menu::ribbon::{RibbonGroup, RibbonRow};
use craft::ui::buttons::{Arrangement, Button};
use craft::ui::contract::{Bind, BodyCx, Gap, Props as _, Show as _, ShowContainer as _, TooltipSpec, Trigger};
use craft::ui::controls::disclosure::DisclosureGlyph;
use craft::ui::display::Divider;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::ThumbAspect;
use craft::ui::menu::{Menu, MenuItem, MenuOutput};
use craft::ui::primitives::{FacePaint, IconSpec};
use craft::ui::runtime::env::Label;
use craft::ui::text::NumberField;

use serde_json::Value;

use super::{Ui, WordApp, json};
use crate::icons;
use crate::undo_marks::apply_edits;

/// The arrow after the label of a ribbon menu button: the small filled triangle of the old ribbon.
pub(crate) static MENU_ARROW: DisclosureGlyph = DisclosureGlyph::new(1.0);

/// A button that opens a menu, greyed out (and not opening) while the command the menu is about cannot run.
/// `face` is the caller's button; `about` is that command's id, `None` for a menu about no command.
pub(crate) fn menu_button(
    ui: &mut Ui,
    mut face: Button<'_>,
    name: impl Into<Label>,
    about: Option<&str>,
    body: impl FnMut(&mut Ui, &mut BodyCx<'_>),
) -> MenuOutput {
    let resolved = about.filter(|id| !id.is_empty()).map(|id| craft::ui::runtime::commands::resolve(ui.ctx(), CommandRef::new(id)));
    let enabled = resolved.as_ref().is_none_or(|cmd| cmd.enabled);
    if !enabled && let Some(hint) = resolved.as_ref().and_then(|cmd| cmd.hint.clone()) {
        face = face.tooltip(TooltipSpec::new(hint.clone()).disabled_text(hint));
    }
    let opener = face.trailing_icon(IconSpec::glyph(&MENU_ARROW)).enabled(enabled).show(ui);
    if !ui.is_sizing_pass() {
        crate::keytips::note_menu_button(ui.ctx(), opener.response.id, about.filter(|id| !id.is_empty()));
    }
    Menu::new(name).opened_by(&opener.response, Trigger::Click).show(ui, body).output
}

/// Shows `add` with its inline buttons as icons alone (the compact group level): buttons the old
/// ribbon drew without their labels. The labels stay their accessible names and tooltips.
pub(crate) fn icon_only<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    craft::ui::runtime::env::with_env(
        ui,
        |overrides| {
            overrides.values.set(craft::ui::buttons::GroupLevel::Compact);
        },
        add,
    )
}

/// Custom groups (Customize Ribbon, #379): up to three commands as stacked buttons, more as inline
/// buttons in columns of three. Each shows the command's own icon and label.
pub(super) fn custom_groups(row: &mut RibbonRow<'_>, app: &mut WordApp, groups: &[wordcraft_engine::ribbon::CustomGroup]) {
    for g in groups {
        let ids: Vec<&'static str> = g.commands.iter().filter_map(|id| app.session.registry.get(id).map(|s| s.id)).collect();
        let Some(first) = ids.first() else { continue };
        let group = RibbonGroup::new(&g.name, g.name.clone(), icons::for_command(first));
        row.group(group, |ui| {
            if ids.len() <= 3 {
                Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
                    for id in &ids {
                        Button::command(CommandRef::new(*id)).icon(icons::for_command(id)).arrangement(Arrangement::Stacked).show(ui);
                    }
                });
                return;
            }
            Row::new().show(ui, |ui, _| {
                for column in ids.chunks(3) {
                    Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
                        for id in column {
                            Button::command(CommandRef::new(*id)).icon(icons::for_command(id)).show(ui);
                        }
                    });
                }
            });
        });
    }
}

/// A W or H field of Picture, Shape, Chart or SmartArt Format › Size, `pt` shown in the interface unit.
/// A size typed in any unit applies once typed, never per keystroke; a drag is one undo step and
/// Esc cancels. A value outside `range` (points) is shown and writes nothing. `lock_aspect` false
/// resizes the one side only.
pub(super) fn size_field(ui: &mut Ui, app: &mut WordApp, label: &str, key: &str, pt: f32, lock_aspect: bool, range: RangeInclusive<f32>) {
    let field = NumberField::length(tl!(label).to_string(), Bind::given(Some(f64::from(pt))), crate::options::lengths(app.session.units()))
        .range(f64::from(*range.start())..=f64::from(*range.end()))
        .id_salt(("size_field", key));
    let events = field.show(ui).edit.events;
    let key = key.to_string();
    apply_edits(app, events, move |app, v: Option<f64>| {
        let Some(v) = v.map(|v| v as f32).filter(|v| range.contains(v)) else { return };
        let params = if lock_aspect { json!({ key.as_str(): v }) } else { json!({ key.as_str(): v, "lockAspect": false }) };
        let _ = app.run("picture.size", params);
    });
}

/// Layout › Size: the named paper sizes, then More Paper Sizes… for a custom size in Page
/// Setup (#1433).
pub(crate) fn size_menu(ui: &mut Ui, app: &mut WordApp) {
    // The current paper size is checked.
    let x = wordcraft_engine::cmd::page::sect(&app.session);
    let current = wordcraft_geom::paper_name(x.page_w, x.page_h);
    for (n, w, h) in wordcraft_geom::PAPER_SIZES {
        // Paper names are the industry's (Letter, A4, B5 (JIS)) and stay as they are; the
        // envelopes say what they are. Sizes in the measurement unit, written the region's
        // way (`21 cm × 29,7 cm`, #838).
        let name = if n.starts_with("Envelope ") { tl!(n) } else { n };
        let (width, height) = (app.session.format_length(*w), app.session.format_length(*h));
        let label = crate::i18n::fmt(tl!("{paper}   {width} × {height}"), &[("paper", name), ("width", &width), ("height", &height)]);
        MenuItem::command(CommandRef::with_params("layout.size", json!({"name": n}))).label(label).check(current == Some(*n)).show(ui);
    }
    Divider::horizontal().show(ui);
    MenuItem::command(CommandRef::with_params("ui.dialog", json!({"name": "pageSetup"}))).label(tl!(crate::dialogs::PAPER_LABELS.more)).show(ui);
}

/// The outline weights Word offers, in points: Text Outline, Shape Outline and the border dialogs.
const OUTLINE_WEIGHTS: [f32; 9] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.25, 3.0, 4.5, 6.0];

/// An outline's Weight submenu (#3917): each weight as its label and a line sample drawn at that
/// weight, the `current` one checked. Choosing one runs `id` with `params(weight)`.
pub(crate) fn weight_menu(ui: &mut Ui, _app: &mut WordApp, id: &str, current: Option<f32>, params: impl Fn(f32) -> Value) {
    // A hairline under a device pixel would vanish, so the thinnest samples get one.
    let pixel = 1.0 / ui.ctx().pixels_per_point();
    for w in OUTLINE_WEIGHTS {
        let label = crate::i18n::fmt(tl!("{n} pt"), &[("n", &w.to_string())]);
        let checked = current.is_some_and(|c| (c - w).abs() < 0.01);
        MenuItem::command(CommandRef::with_params(id, params(w)))
            .label(label)
            .check(checked)
            .sample(
                ThumbAspect::new(3, 1),
                FacePaint::new(u64::from(w.to_bits()), move |cx, rect| {
                    let y = rect.center().y;
                    cx.painter.line_segment([egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)], egui::Stroke::new(w.max(pixel), cx.ink()));
                }),
            )
            .show(ui);
    }
}
