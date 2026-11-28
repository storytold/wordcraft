use std::cell::RefCell;

use craft::theme::SizeTokens;
use craft::ui::contract::BodyCx;
use craft::ui::patterns::dock::{PaneArea, PaneRequest, SidePolicy};
use craft::ui::runtime::env::Env;
use egui::Ui;
use serde_json::{Value, json};

use super::state::{SavedDock, move_before, open, pane_state, rect_to, saved_dock};
use super::{Pane, WordApp};

/// The pane bodies' view of the app. The pane area runs the bodies and the document centre one after the other
/// inside one pass, so both reach the app through one cell that lives for that pass.
pub(super) struct Content<'a, 'b> {
    pub(super) app: &'a RefCell<&'b mut WordApp>,
    /// Whether the interface reads right to left: the area's edges are logical, the saved places physical.
    pub(super) rtl: bool,
    /// The theme's sizes, from which each pane's least width and first float size come.
    pub(super) size: SizeTokens,
}

/// Draw the open panes around the document and run `centre` in what they leave. Call in place of the document's
/// central panel.
pub fn show(app: &mut WordApp, ui: &mut Ui, mut centre: impl FnMut(&mut Ui, &mut BodyCx<'_>, &mut WordApp)) {
    let (rtl, size) = {
        let env = Env::of(ui);
        (env.direction.is_rtl(), env.tokens.size)
    };
    let open = open(app);
    let requests = {
        let cell = RefCell::new(&mut *app);
        let mut content = Content { app: &cell, rtl, size };
        PaneArea::new(tl!("Task Panes").to_owned(), &open, &mut content)
            .policy(SidePolicy::Columns)
            .show(ui, |ui, cx| centre(ui, cx, &mut cell.borrow_mut()))
            .requests
    };
    for request in requests {
        apply(app, request, rtl);
    }
}

/// Run a request from the pane area through the pane's commands and the UI preferences.
pub(crate) fn apply(app: &mut WordApp, request: PaneRequest<Pane>, rtl: bool) {
    match request {
        PaneRequest::Close(pane) => close(app, pane),
        PaneRequest::Place(Pane::Styles, place) => {
            // `left` and `right` are the window's physical sides (same as `ui.paneDock` / `view.stylesPaneDock`).
            let Some(dock) = saved_dock(place, rtl) else { return };
            let _ = app.run("view.stylesPaneDock", json!({"dock": dock.word()}));
        }
        PaneRequest::Resize(Pane::Styles, width) => app.ui.styles_pane_width = Some(width),
        PaneRequest::FloatRect(Pane::Styles, rect) => app.ui.styles_pane_float = Some(rect_to(rect)),
        // Styles keeps no minimized flag: its place lives in the view state.
        PaneRequest::Minimize(Pane::Styles, _) => {}
        PaneRequest::Place(pane, place) => {
            let Some(dock) = saved_dock(place, rtl) else { return };
            let _ = app.run("ui.paneDock", json!({"pane": pane.key(), "dock": dock.word()}));
        }
        PaneRequest::Resize(pane, width) => app.ui.pane_places.entry(pane.key().into()).or_default().width = Some(width),
        PaneRequest::FloatRect(pane, rect) => app.ui.pane_places.entry(pane.key().into()).or_default().float = Some(rect_to(rect)),
        PaneRequest::Minimize(pane, minimized) => app.ui.pane_places.entry(pane.key().into()).or_default().minimized = minimized,
        PaneRequest::Before(pane, before) => {
            let _ = app.run("ui.paneDock", json!({"pane": pane.key(), "before": before.map(Pane::key)}));
        }
        // A focus change moves no pane.
        _ => {}
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
    // Report the physical side the pane occupies (matches what was stored / requested).
    let rtl = crate::i18n::current().rtl();
    let name = saved_dock(pane_state(app, pane, rtl).place, rtl).map_or("right", SavedDock::word);
    Ok(json!({"pane": key, "dock": name}))
}

fn close(app: &mut WordApp, pane: Pane) {
    match pane.close_command() {
        Some(id) => {
            let _ = app.run(id, json!({"value": false}));
        }
        None if pane == Pane::Proofing => app.spelling_review = None,
        None => app.restrict.open = false,
    }
}
