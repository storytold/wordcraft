use super::dispatch::body;
use super::{Dialog, Sense, Tokens, Ui, WordApp, vec2};

/// A colour menu: a swatch with the colour grid and No Color. `value` is hex, empty for none.
pub(crate) fn color_menu(ui: &mut Ui, theme: &[wordcraft_doc::Rgb], value: &mut String) {
    color_field(ui, theme, value, "No Color", "");
}

/// A colour field: a swatch and a drop-down with `none_label` (which sets `none_value`), the
/// colour grid, Recent Colors and More Colors… (#606). `value` is hex or `none_value`. An empty
/// `none_label` leaves the none option out: the field always holds a colour (#1869).
pub(crate) fn color_field(ui: &mut Ui, theme: &[wordcraft_doc::Rgb], value: &mut String, none_label: &str, none_value: &str) {
    ui.horizontal(|ui| {
        let id = ui.scope_id().with("colorField");
        crate::colors::take_field(ui.ctx(), id, value);
        let c = wordcraft_doc::Rgb::parse(value);
        let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
        let t = Tokens::get(ui.ctx());
        match c {
            Some(c) => {
                ui.painter().rect_filled(r, 2.0, crate::theme::c32(c));
            }
            None => {
                ui.painter().line_segment([r.left_bottom(), r.right_top()], egui::Stroke::new(1.0, t.text_dim));
            }
        }
        ui.painter().rect_stroke(r, 2.0, egui::Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
        let label = if c.is_some() || none_label.is_empty() { value.clone() } else { tl!(none_label).to_string() };
        ui.menu_button(label, |ui| {
            if !none_label.is_empty() && ui.button(tl!(none_label)).clicked() {
                *value = none_value.to_string();
                ui.close();
            }
            crate::colors::field_menu(ui, theme, id, value);
        });
    });
}

/// The open dialog, as (window id, frames it has been centred for).
fn dialog_top_key() -> egui::Id {
    egui::Id::unique("dialog_top")
}

/// Frames a dialog stays centred: the first one measures it, the second centres that size.
const DIALOG_CENTRED_FRAMES: u8 = 2;

/// Places a dialog: centred for its first frames, then left where it is with its top-centre point
/// as the pivot, so a taller or shorter tab (Paragraph, Font, Borders and Shading…) only moves
/// the bottom edge and the tab strip stays under the mouse (#507). It is no longer anchored after
/// that, so dragging the title bar moves it (#732). egui keeps it on screen if it grows past the
/// bottom.
fn place_dialog<'a>(ctx: &egui::Context, id: egui::Id, w: egui::Window<'a>) -> egui::Window<'a> {
    let w = w.pivot(egui::Align2::CENTER_TOP);
    match ctx.data(|d| d.get_temp::<(egui::Id, u8)>(dialog_top_key())) {
        Some((open, frames)) if open == id && frames >= DIALOG_CENTRED_FRAMES => w,
        _ => w.anchor(egui::Align2::CENTER_CENTER, vec2(0.0, -40.0)),
    }
}

/// After a frame of a dialog: counts the frames it has been centred for.
fn count_dialog_frame(ctx: &egui::Context, id: egui::Id) {
    let frames = match ctx.data(|d| d.get_temp::<(egui::Id, u8)>(dialog_top_key())) {
        Some((open, frames)) if open == id => frames,
        _ => 0,
    };
    if frames < DIALOG_CENTRED_FRAMES {
        ctx.data_mut(|d| d.insert_temp(dialog_top_key(), (id, frames + 1)));
    }
}

/// Takes the active dialog, resolves its translated title, hosts it as a non-resizable window, draws
/// its body, keeps it open when it stays, and gives the Save Changes prompt the AccessKit
/// alert-dialog role.
pub fn show(app: &mut WordApp, ctx: &egui::Context) {
    let Some(mut d) = app.dialog.take() else {
        ctx.data_mut(|d| d.remove::<(egui::Id, u8)>(dialog_top_key()));
        return;
    };
    let mut close = false;
    let title = match &d {
        Dialog::RenameDocument { .. } => "Rename Document",
        Dialog::Font { .. } => "Font",
        Dialog::Paragraph { .. } => "Paragraph",
        Dialog::Tabs { .. } => "Tabs",
        Dialog::Borders { .. } => "Borders and Shading",
        Dialog::Find { .. } => "Find and Replace",
        Dialog::InsertTable { .. } => "Insert Table",
        Dialog::PageSetup { .. } => "Page Setup",
        Dialog::Link { .. } => "Insert Hyperlink",
        Dialog::Bookmark { .. } => "Bookmark",
        Dialog::WordCount { .. } => "Word Count",
        Dialog::Zoom { .. } => "Zoom",
        Dialog::Watermark { .. } => "Custom Watermark",
        Dialog::Phonetic { .. } => "Phonetic Guide",
        Dialog::NewStyle { .. } => "Create New Style",
        Dialog::ModifyStyle { .. } => "Modify Style",
        Dialog::ManageStyles { .. } => "Manage Styles",
        Dialog::TableStyle { id: None, .. } => "New Table Style",
        Dialog::TableStyle { .. } => "Modify Table Style",
        Dialog::Commands { .. } => "Search Commands",
        Dialog::Insert(d) => d.title(),
        Dialog::Refs(d) => d.title(),
        Dialog::Icons { .. } => "Icons",
        Dialog::PasteSpecial { .. } => "Paste Special",
        Dialog::About { .. } => "About WordCraft",
        Dialog::SaveChanges { .. } => "WordCraft",
        Dialog::DiscardRecovery { .. } => "Discard recovery copy",
        Dialog::RecipientList { .. } => "Recipient List",
        Dialog::SelectTable { .. } => "Select Table",
        Dialog::InsertMergeField { .. } => "Insert Merge Field",
        Dialog::FindRecipient { .. } => "Find Recipient",
        Dialog::MergeRecords { .. } => "Finish & Merge",
        Dialog::FieldCode { edit: false, .. } => "Field",
        Dialog::FieldCode { .. } => "Edit Field Code",
        Dialog::MergeRule { rule, .. } if rule == "SKIPIF" => "Skip Record If",
        Dialog::MergeRule { .. } => "If…Then…Else",
        Dialog::MatchFields { .. } => "Match Fields",
        Dialog::CheckErrors { .. } => "Check for Errors",
        Dialog::TableProperties { .. } => "Table Properties",
        Dialog::CellMargins { .. } => "Cell Margins",
        Dialog::Password { .. } => "Password",
        Dialog::EncryptPassword { .. } => "Encrypt with Password",
        Dialog::DefineList { .. } => "Define New Multilevel List",
        Dialog::TrackOptions { .. } => "Track Changes Options",
        Dialog::Review(d) => d.title(),
        Dialog::CustomizeKeyboard { .. } => "Customize Keyboard",
        Dialog::CustomizeRibbon { .. } => "Customize Ribbon",
        Dialog::Control { .. } => "Content Control Properties",
        Dialog::Enclose { .. } => "Enclose Characters",
        Dialog::NoteOptions { .. } => "Footnote and Endnote",
        Dialog::Templates { .. } => "Templates and Add-ins",
        Dialog::Organizer { .. } => "Organizer",
        Dialog::CustomProperties { .. } => "Custom Document Properties",
        Dialog::Layout(d) => d.title(),
        Dialog::Themes(d) => d.title(),
    };
    let id = egui::Id::unique(("dialog", title));
    let window = crate::window_close::window(ctx, egui::Window::new(tl!(title)).id(id).collapsible(false).resizable(false));
    let shown = place_dialog(ctx, id, window).show(ctx, |ui| {
        ui.set_min_width(340.0);
        close = body(app, ui, &mut d);
    });
    if let Some(shown) = &shown {
        if let Dialog::SaveChanges { name, message, .. } = &d {
            let question = crate::i18n::fmt(tl!("Do you want to save changes to {name}?"), &[("name", name)]);
            let mut description = tl!("Your changes will be lost if you don't save them.").to_string();
            if !message.is_empty() {
                description.push('\n');
                description.push_str(message);
            }
            // This is an urgent decision, not another application window. Keep its context with
            // the focused choice, including when a failed Save brings the same prompt back.
            ctx.accesskit_node_builder(shown.response.id, |node| {
                node.set_role(egui::accesskit::Role::AlertDialog);
                node.set_label(question);
                node.set_description(description);
            });
        }
        count_dialog_frame(ctx, id);
        close |= crate::window_close::show(ctx, &shown.response, &Tokens::get(ctx));
    }
    if close {
        app.canvas.want_focus = true;
    } else {
        app.dialog = Some(d);
    }
}

/// Cancel key for dialogs: Escape everywhere, plus ⌘. (Command+Period) on macOS.
pub(crate) fn cancel_key_pressed(ui: &Ui) -> bool {
    ui.input(|i| i.key_pressed(egui::Key::Escape) || (cfg!(target_os = "macos") && i.modifiers.command && i.key_pressed(egui::Key::Period)))
}

/// Lays out an OK and a Cancel button right-to-left, in the platform's order:
/// macOS puts Cancel left of the default OK button; Windows and Linux put OK left of Cancel.
/// Returns `(ok, cancel)` responses. Call inside a right-to-left layout.
pub(crate) fn ok_cancel_responses(ui: &mut Ui, draw_ok: impl FnOnce(&mut Ui) -> egui::Response, cancel: &str) -> (egui::Response, egui::Response) {
    if cfg!(target_os = "macos") {
        let okb = draw_ok(ui);
        (okb, ui.button(cancel))
    } else {
        let cancelb = ui.button(cancel);
        (draw_ok(ui), cancelb)
    }
}

pub(crate) fn buttons(ui: &mut Ui, ok: &str) -> (bool, bool) {
    let mut r = (false, false);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Enter and Escape belong to the Colors dialog while it is open above this one.
            let keys = !crate::colors::is_open(ui.ctx());
            let (okb, cancelb) = ok_cancel_responses(ui, |ui| ui.add(crate::widgets::primary_button(ui, ok)), tl!("Cancel"));
            if cancelb.clicked() || (keys && cancel_key_pressed(ui)) {
                r.1 = true;
            }
            if okb.clicked() || (keys && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                r.0 = true;
            }
        });
    });
    r
}

/// A dialog's single Close button (Escape too).
pub(super) fn close_button(ui: &mut Ui) -> bool {
    ui.add_space(8.0);
    let mut close = ui.input(|i| i.key_pressed(egui::Key::Escape));
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        close |= ui.add(crate::widgets::primary_button(ui, tl!("Close"))).clicked();
    });
    close
}
