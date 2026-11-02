use super::buttons::enabled;
use super::{Rect, Response, Ui, WordApp};

/// Shortcut text for a command (`⌘B` on macOS, `Ctrl+B` elsewhere): its first key in effect,
/// custom keys included (Customize Keyboard).
pub fn shortcut_text(app: &WordApp, id: &str) -> String {
    let keys = app.session.keymap.keys_for(&app.session.registry, id);
    keys.first().map(|k| key_text(k)).unwrap_or_default()
}

/// A key as the platform writes it (`⌘⇧K` on macOS, `Ctrl+Shift+K` elsewhere).
pub fn key_text(sc: &str) -> String {
    if sc.is_empty() {
        return String::new();
    }
    if cfg!(target_os = "macos") {
        sc.replace("Mod+", "⌘").replace("Shift+", "⇧").replace("Alt+", "⌥").replace("Ctrl+", "⌃")
    } else {
        sc.replace("Mod+", "Ctrl+")
    }
}

/// The lines of a control's hover hint: the title (`Name (Ctrl+B)`), then the group it lives in
/// and, while the control is off, why. Empty parts are left out; `reason` falls back to a generic
/// line when the command gave none.
pub(crate) fn tooltip_lines(label: &str, shortcut: &str, desc: &str, disabled: Option<&str>) -> (String, Vec<String>) {
    let title = if shortcut.is_empty() || names_a_shortcut(label) { label.to_string() } else { format!("{label} ({shortcut})") };
    let mut notes = Vec::new();
    if !desc.is_empty() {
        notes.push(desc.to_string());
    }
    if let Some(reason) = disabled {
        notes.push(if reason.is_empty() { tl!("Not available right now").to_string() } else { capitalised(tl!(reason)) });
    }
    (title, notes)
}

/// Whether `label` already ends in a parenthesised key ("Paste (⌘V)"), so none is appended again.
fn names_a_shortcut(label: &str) -> bool {
    let Some(inner) = label.strip_suffix(')').and_then(|l| l.rsplit_once(" (")).map(|(_, k)| k) else {
        return false;
    };
    ["⌘", "⌃", "⌥", "⇧", "Ctrl+", "Alt+", "Shift+", "Mod+"].iter().any(|m| inner.contains(m))
}

/// `text` with its first letter in capitals (the engine's reasons are lower case).
fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Show the hover hint on `resp`. A control reached with the keyboard shows it too, after the
/// usual delay (egui itself only hints for the pointer), unless the pointer is busy elsewhere or a
/// menu is open, whose hint would otherwise be drawn over the menu.
pub(super) fn show_hint(resp: &Response, title: &str, notes: &[String]) {
    let add = |ui: &mut Ui| {
        ui.set_max_width(260.0);
        ui.label(egui::RichText::new(title).font(crate::theme::semibold(12.0)));
        for n in notes {
            ui.label(egui::RichText::new(n).small().weak());
        }
    };
    let ctx = &resp.ctx;
    let since_key = id_since_focus(resp);
    // The pass on which the pointer was last over any hinted control (see the focus hint below).
    let hover_key = egui::Id::unique("hint_pointer_over_a_control");
    let pass = ctx.cumulative_pass_nr();
    // A disabled response never reports hover, so test the pointer against its rectangle, as
    // `Tooltip::for_disabled` does.
    let over = resp.hovered() || (!resp.enabled() && ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| resp.rect.contains(p)));
    if over {
        ctx.data_mut(|d| d.insert_temp(hover_key, pass));
    }
    if resp.hovered() || !resp.has_focus() {
        // A response from a disabled `Ui` only shows hints through `for_disabled`.
        if resp.enabled() {
            egui::Tooltip::for_enabled(resp).show(add);
        } else {
            egui::Tooltip::for_disabled(resp).show(add);
        }
    }
    if let Some(waited) = since_key {
        let delay = ctx.global_style().interaction.tooltip_delay;
        let pointer_idle = !ctx.input(|i| i.pointer.has_pointer()) || ctx.input(|i| i.pointer.time_since_last_movement()) > 2.0;
        // Another control under the pointer (seen on this or the previous pass) owns the hint.
        let pointer_over_other = ctx.data(|d| d.get_temp::<u64>(hover_key)).is_some_and(|p| p + 1 >= pass);
        if waited < delay {
            ctx.request_repaint_after_secs(delay - waited);
        } else if !resp.hovered() && pointer_idle && !pointer_over_other && !egui::Popup::is_any_open(ctx) {
            egui::Tooltip::for_widget(resp).show(add);
        }
    }
}

/// Seconds `resp` has had the keyboard focus (None when it hasn't), tracked in egui memory.
fn id_since_focus(resp: &Response) -> Option<f32> {
    let key = resp.id.with("hint_focus_since");
    let now = resp.ctx.input(|i| i.time);
    if !resp.has_focus() {
        resp.ctx.data_mut(|d| d.remove::<f64>(key));
        return None;
    }
    let since = resp.ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(key, || now));
    Some((now - since) as f32)
}

/// `label` is already in the interface language.
pub(super) fn tooltip(app: &WordApp, resp: Response, label: &str, id: &str) -> Response {
    let spec = app.session.registry.get(id);
    let desc = spec.map(|s| crate::i18n::location(s.location)).unwrap_or_default();
    // `enabled` also covers what the registry doesn't know (no document open, `ui.*` commands).
    let reason = if enabled(app, id) {
        None
    } else if !app.has_document() && !crate::works_without_document(id) {
        Some(tl!("Open a document first"))
    } else {
        Some(app.session.why_disabled(id).unwrap_or(""))
    };
    let (title, notes) = tooltip_lines(label, &shortcut_text(app, id), &desc, reason);
    show_hint(&resp, &title, &notes);
    resp
}

/// Record a keytip badge over a control's rect when keytips are showing commands. `ui` reaches
/// the phase and badge list through egui memory, so widgets without `&mut WordApp` can record too.
pub(super) fn keytip_badge(ui: &Ui, rect: Rect, id: &str, name: &str) {
    if ui.data(|d| d.get_temp::<crate::keytips::Phase>(crate::keytips::phase_id())) != Some(crate::keytips::Phase::Commands) {
        return;
    }
    let tab = ui.data(|d| d.get_temp::<String>(crate::keytips::tab_id())).unwrap_or_default();
    let controls = crate::keytips::controls_public(&tab);
    // Match by command id first; fall back to tooltip prefix, since widget tooltips carry
    // extra text ("Paste (⌘V)") the canonical control name does not.
    let Some(i) = controls
        .iter()
        .position(|c| c.id == id && id != "ui.dialog")
        .or_else(|| controls.iter().position(|c| c.tip == name))
        .or_else(|| controls.iter().position(|c| name.starts_with(c.tip)))
    else {
        return;
    };
    let letter = ui.data(|d| d.get_temp::<Vec<String>>(crate::keytips::letters_id().with(&tab))).and_then(|l| l.get(i).cloned());
    if let Some(l) = letter {
        ui.data_mut(|d| {
            let mut v: Vec<(Rect, String)> = d.get_temp::<Vec<(Rect, String)>>(crate::keytips::rects_id()).unwrap_or_default();
            v.push((rect, l));
            d.insert_temp(crate::keytips::rects_id(), v);
        });
    }
}
