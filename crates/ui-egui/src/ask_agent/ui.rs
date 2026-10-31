//! The chip and the bubble, in egui's own visuals (each app sets them from its theme).
//!
//! Layout: the chip sits just under the selection, where the bubble then opens. The bubble has three
//! bands on one left edge: a header (who gets the request, close), the content (conversation, status,
//! answer), and a footer row (Actions on the left, the primary button on the right). Every control in
//! the bubble has the same height.

use egui::{Align, Color32, Key, Layout, Rect, RichText, Stroke, pos2, vec2};

use super::hub::{Snapshot, Status, Turn, colour};

fn tr(_ctx: &egui::Context, text: &'static str) -> &'static str {
    crate::i18n::t(text)
}

/// What the person did in the bubble.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// The instruction, or a follow-up after an answer.
    Send(String),
    Keep,
    Undo,
    Close,
}

/// A shortcut that only prepares an editable instruction; Send remains explicit.
#[derive(Clone, Copy, Debug)]
pub struct QuickAction {
    pub label: &'static str,
    pub group: &'static str,
    pub instruction: &'static str,
}

/// What the bubble shows: the current request, if any, and the agents listening.
pub struct View<'a> {
    pub snapshot: Option<&'a Snapshot>,
    pub agents: &'a [String],
    pub actions: &'a [QuickAction],
    pub history: &'a [Turn],
    pub instruction: &'a str,
}

const WIDTH: f32 = 360.0;
/// One height for every control in the bubble, so the rows line up.
const ROW: f32 = 28.0;

/// The chip just under the selection, on its left edge, where the bubble opens; true when clicked.
pub fn chip(ctx: &egui::Context, anchor: Rect) -> bool {
    egui::Area::new(egui::Id::unique("ask_agent_chip"))
        .order(egui::Order::Foreground)
        .fixed_pos(origin(ctx, anchor, 140.0))
        .constrain(true)
        .show(ctx, |ui| {
            let frame = egui::Frame::popup(ui.style()).corner_radius(14.0).inner_margin(egui::Margin::symmetric(10, 5));
            let inner = frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    sparkle(ui);
                    ui.label(RichText::new(tr(ctx, "Ask Agent")).strong());
                })
            });
            let response = ui.interact(inner.response.rect, ui.scope_id().with("chip"), egui::Sense::click());
            response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
        })
        .inner
}

/// The bubble under `anchor`: instruction, the agent at work, its answer with Keep, Undo and a
/// follow-up field.
pub fn bubble(ctx: &egui::Context, anchor: Rect, view: &View, draft: &mut String, picked: &mut Option<String>) -> Option<Action> {
    let mut action = None;
    // True on the first pass the bubble shows after at least one pass without it.
    let (id, pass) = (egui::Id::unique("ask_agent_bubble_pass"), ctx.cumulative_pass_nr());
    let last = ctx.data(|d| d.get_temp::<u64>(id));
    ctx.data_mut(|d| d.insert_temp(id, pass));
    let opened = last.is_none_or(|last| last + 1 < pass);
    egui::Area::new(egui::Id::unique("ask_agent_bubble"))
        .order(egui::Order::Foreground)
        .fixed_pos(origin(ctx, anchor, WIDTH + 40.0))
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).corner_radius(12.0).inner_margin(egui::Margin::symmetric(16, 14)).show(ui, |ui| {
                ui.set_width(WIDTH);
                let spacing = ui.spacing_mut();
                spacing.item_spacing = vec2(8.0, 10.0);
                spacing.interact_size.y = ROW;
                spacing.button_padding = vec2(12.0, 4.0);
                let status = view.snapshot.map(|s| s.status);

                // Header: who the request is for, then close; the agent choice sits on the right.
                row(ui, Layout::left_to_right(Align::Center), |ui| {
                    match view.snapshot.and_then(|s| s.agent.as_deref()) {
                        Some(name) => agent_label(ui, name),
                        None => {
                            sparkle(ui);
                            ui.label(RichText::new(tr(ctx, "Ask Agent")).strong());
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if close_button(ui).on_hover_text(tr(ctx, "Close")).clicked() {
                            action = Some(Action::Close);
                        }
                        if status.is_none() {
                            picker(ui, view.agents, picked);
                        }
                    });
                });

                if !view.instruction.is_empty() || !view.history.is_empty() {
                    egui::CollapsingHeader::new(tr(ctx, "Conversation"))
                        .id_salt(("ask_conversation", view.history.len()))
                        .default_open(!view.history.is_empty())
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical().id_salt("ask_conversation_scroll").max_height(160.0).show(ui, |ui| {
                                for turn in view.history {
                                    ui.label(RichText::new(tr(ctx, "You")).strong());
                                    ui.label(&turn.instruction);
                                    ui.label(RichText::new(tr(ctx, "Agent")).strong());
                                    ui.label(&turn.answer);
                                    ui.separator();
                                }
                                if !view.instruction.is_empty() {
                                    ui.label(RichText::new(tr(ctx, "Current instruction")).strong());
                                    ui.label(view.instruction);
                                }
                            });
                        });
                }

                if let Some(s) = view.snapshot {
                    for note in &s.notes {
                        ui.label(RichText::new(note).weak());
                    }
                }
                match status {
                    Some(Status::Queued) => _ = ui.label(RichText::new(tr(ctx, "Waiting for an agent…")).weak()),
                    Some(Status::Working { .. }) => {
                        row(ui, Layout::left_to_right(Align::Center), |ui| {
                            ui.spinner();
                            ui.label(RichText::new(tr(ctx, "Working on it…")).weak());
                        });
                    }
                    Some(Status::Failed) => {
                        let error = view.snapshot.and_then(|s| s.error.as_deref()).unwrap_or_default();
                        ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
                    }
                    Some(Status::Answered) => {
                        ui.label(view.snapshot.and_then(|s| s.answer.as_deref()).unwrap_or_default());
                        // The decision on the right, primary last, like Send below.
                        row(ui, Layout::right_to_left(Align::Center), |ui| {
                            if ui.add(primary(ui, tr(ctx, "Keep"))).clicked() {
                                action = Some(Action::Keep);
                            }
                            if ui.button(tr(ctx, "Undo")).on_hover_text(tr(ctx, "Undo all of the agent's edits")).clicked() {
                                action = Some(Action::Undo);
                            }
                        });
                        ui.separator();
                    }
                    None => {}
                }
                if matches!(status, None | Some(Status::Answered | Status::Failed)) {
                    let hint = if status.is_none() { tr(ctx, "What should the agent do?") } else { tr(ctx, "Reply to refine…") };
                    let field = ui.add(
                        egui::TextEdit::multiline(draft)
                            .hint_text(hint)
                            .desired_rows(3)
                            .desired_width(f32::INFINITY)
                            .margin(egui::Margin::symmetric(8, 6))
                            // Shift+Enter makes a new line; Enter alone sends, below.
                            .return_key(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, Key::Enter)),
                    );
                    // Focus the field once, when the bubble opens, so Tab can then reach the other controls.
                    if status.is_none() && opened {
                        field.request_focus();
                    }
                    let enter = field.has_focus() && ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift);
                    // Footer: starting points on the left, Send on the right.
                    row(ui, Layout::left_to_right(Align::Center), |ui| {
                        if matches!(status, None | Some(Status::Answered)) && !view.actions.is_empty() {
                            quick_actions(ui, view.actions, draft);
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let ready = !draft.trim().is_empty();
                            if (ui.add_enabled(ready, primary(ui, tr(ctx, "Send"))).clicked() || enter) && ready {
                                action = Some(Action::Send(std::mem::take(draft).trim().to_string()));
                            }
                        });
                    });
                }
            });
        });
    action
}

/// Where the chip and the bubble open: under the selection, on its left edge, kept on screen.
fn origin(ctx: &egui::Context, anchor: Rect, width: f32) -> egui::Pos2 {
    let screen = ctx.content_rect();
    pos2(anchor.left().clamp(8.0, (screen.right() - width).max(8.0)), anchor.bottom() + 8.0)
}

/// One row of controls, exactly `ROW` high, whatever the layout direction.
fn row(ui: &mut egui::Ui, layout: Layout, add: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(vec2(ui.available_width(), ROW), layout, add);
}

/// The one filled button of a row.
fn primary(ui: &egui::Ui, text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).strong()).fill(ui.visuals().selection.bg_fill).min_size(vec2(72.0, ROW))
}

/// Native menus keep less common actions out of the writing surface.
fn quick_actions(ui: &mut egui::Ui, actions: &[QuickAction], draft: &mut String) {
    ui.menu_button(tr(ui.ctx(), "Actions"), |ui| {
        for (index, action) in actions.iter().enumerate() {
            if action.group.is_empty() {
                if ui.button(tr(ui.ctx(), action.label)).clicked() {
                    *draft = action.instruction.into();
                    ui.close();
                }
            } else if !actions[..index].iter().any(|previous| previous.group == action.group) {
                ui.menu_button(tr(ui.ctx(), action.group), |ui| {
                    for action in actions.iter().filter(|item| item.group == action.group) {
                        if ui.button(tr(ui.ctx(), action.label)).clicked() {
                            *draft = action.instruction.into();
                            ui.close();
                        }
                    }
                });
            }
        }
    })
    .response
    .on_hover_text(tr(ui.ctx(), "Choose a starting instruction, then edit and send"));
}

/// Who the request goes to, when more than one agent listens. Called in a right-to-left row, so
/// the box comes first and its label lands on its left.
fn picker(ui: &mut egui::Ui, agents: &[String], picked: &mut Option<String>) {
    if agents.len() < 2 {
        return;
    }
    craft_ui::ComboBox::from_id_salt("ask_agent_picker").selected_text(picked.as_deref().unwrap_or(tr(ui.ctx(), "Any"))).show_ui(ui, |ui| {
        ui.selectable_value(picked, None, tr(ui.ctx(), "Any"));
        for name in agents {
            ui.selectable_value(picked, Some(name.clone()), name);
        }
    });
    ui.label(RichText::new(tr(ui.ctx(), "Agent")).weak());
}

/// The agent's colour dot and name.
pub fn agent_label(ui: &mut egui::Ui, name: &str) {
    let [r, g, b] = colour(name);
    let (dot, _) = ui.allocate_exact_size(vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle(dot.center(), 5.0, Color32::from_rgb(r, g, b), Stroke::NONE);
    ui.label(RichText::new(name).strong());
}

/// A drawn four-point sparkle in the selection colour, so it shows whatever the interface font.
fn sparkle(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(12.0, 12.0), egui::Sense::hover());
    let (c, colour) = (rect.center(), accent(ui.visuals()));
    for (long, short) in [(vec2(0.0, 6.0), vec2(1.6, 0.0)), (vec2(6.0, 0.0), vec2(0.0, 1.6))] {
        let points = vec![c - long, c + short, c + long, c - short];
        ui.painter().add(egui::Shape::convex_polygon(points, colour, Stroke::NONE));
    }
}

/// Of the selection's fill and text colours, the one that stands out more on the bubble.
fn accent(v: &egui::Visuals) -> Color32 {
    let luma = |c: Color32| 0.299 * f32::from(c.r()) + 0.587 * f32::from(c.g()) + 0.114 * f32::from(c.b());
    let (fill, text, bg) = (v.selection.bg_fill, v.selection.stroke.color, luma(v.window_fill));
    if (luma(fill) - bg).abs() > (luma(text) - bg).abs() { fill } else { text }
}

/// A drawn ✕, so it shows whatever the interface font.
fn close_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(20.0, 20.0), egui::Sense::click());
    let colour = if response.hovered() { ui.visuals().strong_text_color() } else { ui.visuals().weak_text_color() };
    let r = rect.shrink(5.0);
    for (a, b) in [(r.left_top(), r.right_bottom()), (r.right_top(), r.left_bottom())] {
        ui.painter().line_segment([a, b], Stroke::new(1.5, colour));
    }
    response
}
