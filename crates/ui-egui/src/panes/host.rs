use super::state::{SavedDock, labels, move_before, open, pane_state};
use super::*;

pub(super) struct Content<'a> {
    pub(super) app: &'a mut WordApp,
}

/// The pane host's name. Its focus and drag state is per window ([`WordApp::window_id`]); where
/// each pane sits is the app's.
const HOST: &str = "task_panes";

/// Whether a dialog, file browser, colors dialog, content-control popup, Zotero alert, keytips
/// or egui popup is open: the keyboard belongs to it, not to the panes or the document.
pub(crate) fn overlay_open(app: &WordApp, ctx: &egui::Context) -> bool {
    app.dialog.is_some()
        || app.file_dialog_open()
        || app.colors.is_some()
        || app.controls.popup_open()
        || app.zotero.alert_open()
        || app.ui.keytips != crate::keytips::Phase::Off
        || egui::Popup::is_any_open(ctx)
}

/// F6 and Shift+F6 move the keyboard through the panes and back to the document. The key stays
/// with an open dialog, file browser, popup or keytips (Customize Keyboard records it), and an
/// F6 the user bound to a command runs that command instead.
fn route_f6(app: &WordApp, ctx: &egui::Context) {
    if overlay_open(app, ctx) {
        return;
    }
    let reg = &app.session.registry;
    let bound = |m: egui::Modifiers| app.session.keymap.resolve(reg, &crate::keys::combo(m, "F6", true)).is_some();
    let mut cycle = None;
    ctx.input_mut(|input| {
        input.events.retain(|e| match e {
            egui::Event::Key { key: egui::Key::F6, pressed: true, modifiers: m, .. } if !m.command && !m.ctrl && !m.mac_cmd && !bound(*m) => {
                cycle = Some(m.shift);
                false
            }
            _ => true,
        });
    });
    if let Some(reverse) = cycle {
        craft_ui::panes::focus_next(ctx, app.window_id(HOST), reverse);
    }
}

/// Draw the open panes around the document. Call before the canvas's `CentralPanel`.
pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let open = open(app);
    if !open.contains(&Pane::FormatShape) {
        crate::format_shape::forget_drag(ui.ctx());
    }
    route_f6(app, ui.ctx());
    let states: Vec<(Pane, PaneState)> = open.iter().map(|pane| (*pane, pane_state(app, *pane))).collect();
    let out = PaneHost::new(app.window_id(HOST))
        .policy(SidePolicy::Columns)
        .f6(false)
        .labels(labels())
        .frame(egui::Frame::NONE.fill(t.panel).stroke(Stroke::new(1.0, t.border)))
        .body_margin(egui::Margin::same(10))
        .show(ui, &open, |pane| states.iter().find(|(p, _)| p == pane).map(|(_, state)| *state), &mut Content { app });
    for request in out.requests {
        apply(app, request);
    }
    if out.to_document {
        app.canvas.want_focus = true;
    }
}

/// Run a request from the pane host through the pane's commands and the UI preferences.
pub(crate) fn apply(app: &mut WordApp, request: PaneRequest<Pane>) {
    match request {
        PaneRequest::Close(pane) => close(app, pane),
        PaneRequest::Place(Pane::Styles, place) => {
            let dock = match place {
                Place::Docked(Edge::Start) => "left",
                Place::Docked(Edge::End) => "right",
                Place::Floating => "float",
                Place::Docked(Edge::Bottom) => return,
            };
            let _ = app.run("view.stylesPaneDock", json!({"dock": dock}));
        }
        PaneRequest::Resize(Pane::Styles, width) => app.ui.styles_pane_width = Some(width),
        PaneRequest::FloatRect(Pane::Styles, rect) => app.ui.styles_pane_float = Some(rect),
        PaneRequest::Place(pane, place) => {
            let dock = match place {
                Place::Docked(Edge::Start) => "left",
                Place::Docked(Edge::End) => "right",
                Place::Floating => "float",
                Place::Docked(Edge::Bottom) => return,
            };
            let _ = app.run("ui.paneDock", json!({"pane": pane.key(), "dock": dock}));
        }
        PaneRequest::Resize(pane, width) => app.ui.pane_places.entry(pane.key().into()).or_default().width = Some(width),
        PaneRequest::FloatRect(pane, rect) => app.ui.pane_places.entry(pane.key().into()).or_default().float = Some(rect),
        PaneRequest::Before(pane, before) => {
            let _ = app.run("ui.paneDock", json!({"pane": pane.key(), "before": before.map(Pane::key)}));
        }
        PaneRequest::Focus(_) => {}
    }
}

/// `ui.paneDock {"pane": key, "dock"?: "left" | "right" | "float", "before"?: key | null}` (#1756):
/// docks or floats a task pane, as its menu's Dock Left, Dock Right and Float do; without `dock`
/// it reads the pane's place. `pane` is the pane's `panePlaces` key; Styles goes through
/// `view.stylesPaneDock`. `before` puts the pane directly before that pane in the order, outside
/// it on their edge, as a drop over a docked pane does; `null` puts it after every pane.
pub(crate) fn dock_command(app: &mut WordApp, p: &Value) -> Result<Value, String> {
    let key = p.get("pane").and_then(Value::as_str).ok_or("`pane` must be a task pane's key, such as \"navigation\"")?;
    let pane = Pane::from_key(key).ok_or_else(|| format!("unknown task pane `{key}`"))?;
    let dock = match p.get("dock") {
        None => None,
        Some(v) => Some(match v.as_str() {
            Some("left") => SavedDock::Left,
            Some("right") => SavedDock::Right,
            Some("float") => SavedDock::Float,
            _ => return Err("`dock` must be left, right or float".into()),
        }),
    };
    let before = match p.get("before") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(v) => match v.as_str().and_then(Pane::from_key) {
            Some(before) if before != pane => Some(Some(before)),
            _ => return Err("`before` must be another task pane's key or null".into()),
        },
    };
    if let Some(before) = before {
        move_before(app, pane, before);
    }
    if pane == Pane::Styles {
        let params = p.get("dock").map_or_else(|| json!({}), |d| json!({"dock": d}));
        let result = app.run("view.stylesPaneDock", params)?;
        return Ok(json!({"pane": key, "dock": result.get("dock")}));
    }
    if let Some(dock) = dock {
        app.ui.pane_places.entry(key.into()).or_default().dock = Some(dock);
    }
    let name = match pane_state(app, pane).place {
        Place::Docked(Edge::Start) => "left",
        Place::Floating => "float",
        Place::Docked(_) => "right",
    };
    Ok(json!({"pane": key, "dock": name}))
}

fn close(app: &mut WordApp, pane: Pane) {
    match pane.close_command() {
        Some(id) => {
            let _ = app.run(id, json!({"value": false}));
        }
        None => app.restrict.open = false,
    }
}
