//! Ask Agent's chip and bubble. Shared as is by the four apps: egui's own visuals (each app sets
//! them from its theme) and one translation function, the only line that differs between apps.

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

const WIDTH: f32 = 340.0;

/// The chip in the margin beside the selection's first line; true when clicked.
pub fn chip(ctx: &egui::Context, anchor: Rect) -> bool {
    let (pos, pivot) = match anchor.left() > 140.0 {
        true => (pos2(anchor.left() - 10.0, anchor.top()), egui::Align2::RIGHT_TOP),
        false => (pos2(anchor.left(), anchor.bottom() + 6.0), egui::Align2::LEFT_TOP),
    };
    egui::Area::new("ask_agent_chip".into())
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .pivot(pivot)
        .show(ctx, |ui| {
            let frame = egui::Frame::popup(ui.style()).corner_radius(14.0).inner_margin(egui::Margin::symmetric(12, 5));
            let inner = frame.show(ui, |ui| ui.label(RichText::new(tr(ctx, "Ask Agent")).strong()));
            let response = ui.interact(inner.response.rect, ui.id().with("chip"), egui::Sense::click());
            response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
        })
        .inner
}

/// The bubble under `anchor`: instruction, the agent at work, its answer with Keep, Undo and a
/// follow-up field.
pub fn bubble(ctx: &egui::Context, anchor: Rect, view: &View, draft: &mut String, picked: &mut Option<String>) -> Option<Action> {
    let screen = ctx.content_rect();
    let pos = pos2(anchor.left().clamp(8.0, (screen.right() - WIDTH - 40.0).max(8.0)), anchor.bottom() + 8.0);
    let mut action = None;
    egui::Area::new("ask_agent_bubble".into()).order(egui::Order::Foreground).fixed_pos(pos).constrain(true).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).corner_radius(10.0).inner_margin(16.0).show(ui, |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.horizontal(|ui| {
                match view.snapshot.and_then(|s| s.agent.as_deref()) {
                    Some(name) => agent_label(ui, name),
                    None => _ = ui.label(RichText::new(tr(ctx, "Ask Agent")).strong()),
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if close_button(ui).on_hover_text(tr(ctx, "Close")).clicked() {
                        action = Some(Action::Close);
                    }
                });
            });
            let status = view.snapshot.map(|s| s.status);
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
                    _ = ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new(tr(ctx, "Working on it…")).weak());
                    })
                }
                Some(Status::Failed) => {
                    let error = view.snapshot.and_then(|s| s.error.as_deref()).unwrap_or_default();
                    ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
                }
                Some(Status::Answered) => {
                    ui.label(view.snapshot.and_then(|s| s.answer.as_deref()).unwrap_or_default());
                    ui.horizontal(|ui| {
                        if ui.add(egui::Button::new(RichText::new(tr(ctx, "Keep")).strong()).fill(ui.visuals().selection.bg_fill)).clicked() {
                            action = Some(Action::Keep);
                        }
                        if ui.button(tr(ctx, "Undo")).on_hover_text(tr(ctx, "Undo all of the agent's edits")).clicked() {
                            action = Some(Action::Undo);
                        }
                    });
                    ui.separator();
                }
                None => picker(ui, view.agents, picked),
            }
            if matches!(status, None | Some(Status::Answered | Status::Failed)) {
                if matches!(status, None | Some(Status::Answered)) && !view.actions.is_empty() {
                    quick_actions(ui, view.actions, draft);
                }
                let hint = if status.is_none() { tr(ctx, "What should the agent do?") } else { tr(ctx, "Reply to refine…") };
                let field = ui.add(egui::TextEdit::multiline(draft).hint_text(hint).desired_rows(2).desired_width(f32::INFINITY));
                if status.is_none() && !field.has_focus() && draft.is_empty() {
                    field.request_focus();
                }
                let enter = field.has_focus() && ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let ready = !draft.trim().is_empty();
                    if (ui.add_enabled(ready, egui::Button::new(tr(ctx, "Send"))).clicked() || enter) && ready {
                        action = Some(Action::Send(std::mem::take(draft).trim().to_string()));
                    }
                });
            }
        });
    });
    action
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

/// Who the request goes to, when more than one agent listens.
fn picker(ui: &mut egui::Ui, agents: &[String], picked: &mut Option<String>) {
    if agents.len() < 2 {
        return;
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr(ui.ctx(), "Agent")).weak());
        egui::ComboBox::from_id_salt("ask_agent_picker").selected_text(picked.as_deref().unwrap_or(tr(ui.ctx(), "Any"))).show_ui(ui, |ui| {
            ui.selectable_value(picked, None, tr(ui.ctx(), "Any"));
            for name in agents {
                ui.selectable_value(picked, Some(name.clone()), name);
            }
        });
    });
}

/// The agent's colour dot and name.
pub fn agent_label(ui: &mut egui::Ui, name: &str) {
    let [r, g, b] = colour(name);
    let (dot, _) = ui.allocate_exact_size(vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle(dot.center(), 5.0, Color32::from_rgb(r, g, b), Stroke::NONE);
    ui.label(RichText::new(name).strong());
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
