//! View tab and status bar: views, zoom, show/hide, panes.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wordcraft_layout::ViewMode;

use super::sel_result;
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

fn toggle(s: &mut Session, v: &Value, f: fn(&mut crate::ViewState) -> &mut bool) -> CmdResult {
    let cur = *f(&mut s.view);
    *f(&mut s.view) = p::bool(v, "value").unwrap_or(!cur);
    Ok(json!({"value": *f(&mut s.view)}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("view.marks", "Show/Hide ¶", "Home › Paragraph", |s, v| {
            let r = toggle(s, v, |x| &mut x.marks);
            s.relayout();
            r
        })
        .key("Mod+Shift+8 / Mod+8")
        .pure(),
        CommandSpec::new("view.printLayout", "Print Layout", "View › Views", |s, _| mode(s, ViewMode::Print, false)).pure(),
        CommandSpec::new("view.webLayout", "Web Layout", "View › Views", |s, _| mode(s, ViewMode::Web, false)).pure(),
        CommandSpec::new("view.draft", "Draft", "View › Views", |s, _| mode(s, ViewMode::Draft, false)).pure(),
        CommandSpec::new("view.outline", "Outline", "View › Views", |s, _| {
            mode(s, ViewMode::Draft, false)?;
            s.view.nav_pane = true;
            sel_result(s)
        })
        .pure(),
        CommandSpec::new("view.readMode", "Read Mode", "View › Views", |s, _| mode(s, ViewMode::Print, true)).pure(),
        CommandSpec::new("view.focus", "Focus", "View › Immersive", |s, v| toggle(s, v, |x| &mut x.focus_mode)).pure(),
        CommandSpec::new("view.ruler", "Ruler", "View › Show", |s, v| toggle(s, v, |x| &mut x.ruler)).pure(),
        // The drawing grid over the page (screen only; never printed or exported).
        CommandSpec::new("view.gridlines", "Gridlines", "View › Show", |s, v| toggle(s, v, |x| &mut x.gridlines))
            .params(r#"{"value"?: bool}"#)
            .pure(),
        // Outlines of table cells (screen only; never printed or exported).
        CommandSpec::new("table.viewGridlines", "View Gridlines", "Table Layout › Table", |s, v| toggle(s, v, |x| &mut x.table_gridlines))
            .params(r#"{"value"?: bool}"#)
            .pure(),
        CommandSpec::new("view.navigationPane", "Navigation Pane", "View › Show", |s, v| toggle(s, v, |x| &mut x.nav_pane)).pure(),
        CommandSpec::new("view.zoom", "Zoom", "View › Zoom", zoom)
            .params(r#"{"value": percent (10-500) | "pageWidth" | "onePage" | "multiplePages"}"#)
            .pure(),
        CommandSpec::new("view.zoom100", "100%", "View › Zoom", |s, _| zoom(s, &json!({"value": 100}))).pure(),
        CommandSpec::new("view.zoomIn", "Zoom In", "View › Zoom", |s, _| {
            let z = ((s.view.zoom * 10.0).round() / 10.0 + 0.1).min(5.0);
            zoom(s, &json!({"value": z * 100.0}))
        })
        .pure(),
        CommandSpec::new("view.zoomOut", "Zoom Out", "View › Zoom", |s, _| {
            let z = ((s.view.zoom * 10.0).round() / 10.0 - 0.1).max(0.1);
            zoom(s, &json!({"value": z * 100.0}))
        })
        .pure(),
        CommandSpec::new("view.onePage", "One Page", "View › Zoom", |s, _| zoom(s, &json!({"value": "onePage"}))).pure(),
        CommandSpec::new("view.multiplePages", "Multiple Pages", "View › Zoom", |s, _| zoom(s, &json!({"value": "multiplePages"}))).pure(),
        CommandSpec::new("view.pageWidth", "Page Width", "View › Zoom", |s, _| zoom(s, &json!({"value": "pageWidth"}))).pure(),
        CommandSpec::new("view.darkMode", "Switch Modes", "View › Dark Mode", |s, v| toggle(s, v, |x| &mut x.dark_mode)).pure(),
        CommandSpec::new("view.stylesPane", "Styles Pane", "Home › Styles", |s, v| toggle(s, v, |x| &mut x.styles_pane)).pure(),
        CommandSpec::new("view.commentsPane", "Comments Pane", "Review › Comments", |s, v| toggle(s, v, |x| &mut x.comments_pane)).pure(),
        CommandSpec::new("view.newWindow", "New Window", "View › Window", |s, _| {
            s.ui_requests.push(json!({"open": "newWindow"}));
            sel_result(s)
        })
        .pure(),
        CommandSpec::new("view.split", "Split", "View › Window", |s, _| {
            s.ui_requests.push(json!({"toggle": "split"}));
            sel_result(s)
        })
        .pure(),
        // View › Window across WordCraft windows (#322): each window is its own process; the
        // desktop app performs these (see `Windows`), elsewhere they are disabled.
        CommandSpec::new("view.switchWindows", "Switch Windows", "View › Window", |s, v| {
            let id = pick_window(s, v)?;
            s.ui_requests.push(json!({"windows": "focus", "id": id}));
            Ok(json!({"window": id}))
        })
        .params(r#"{"window"?: id from ui.inspect `windows.others` (optional when one other window is open)}"#)
        .when(has_other_window)
        .pure(),
        CommandSpec::new("view.arrangeAll", "Arrange All", "View › Window", |s, _| {
            s.ui_requests.push(json!({"windows": "arrange"}));
            Ok(json!({"windows": s.windows.others.len() + 1}))
        })
        .when(windows_available)
        .pure(),
        CommandSpec::new("view.sideBySide", "View Side by Side", "View › Window", side_by_side)
            .params(r#"{"value"?: bool, "window"?: id of the other window (optional when one other window is open)}"#)
            .when(|s| match windows_available(s) {
                None if s.windows.side_by_side.is_none() => has_other_window(s),
                r => r,
            })
            .pure(),
        CommandSpec::new("view.syncScroll", "Synchronous Scrolling", "View › Window", |s, v| {
            let Some(with) = s.windows.side_by_side else { return Err(CmdError::Disabled("turn on View Side by Side first".into())) };
            s.windows.sync_scroll = p::bool(v, "value").unwrap_or(!s.windows.sync_scroll);
            s.ui_requests.push(json!({"windows": "syncScroll", "with": with, "value": s.windows.sync_scroll}));
            Ok(json!({"value": s.windows.sync_scroll}))
        })
        .params(r#"{"value"?: bool}"#)
        .when(|s| if s.windows.side_by_side.is_some() { None } else { Some("turn on View Side by Side first") })
        .pure(),
        CommandSpec::new("view.state", "View State", "View", |s, _| serde_json::to_value(&s.view).map_err(|e| CmdError::Failed(e.to_string())))
            .pure(),
    ]
}

/// View › Window (#322): the other WordCraft windows as the desktop app last saw them, and the
/// Side by Side pairing. The front end keeps `available` and `others` current; elsewhere (web,
/// CLI, MCP without the app) they stay empty and the window commands are disabled.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Windows {
    /// The host can list and move its windows (the desktop app).
    pub available: bool,
    /// The other open windows, by id.
    pub others: Vec<OtherWindow>,
    /// View Side by Side is on, with this window.
    pub side_by_side: Option<u64>,
    /// Synchronous Scrolling (only while Side by Side is on).
    pub sync_scroll: bool,
}

/// Another open WordCraft window.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OtherWindow {
    pub id: u64,
    pub title: String,
}

fn windows_available(s: &Session) -> Option<&'static str> {
    if s.windows.available { None } else { Some("only the desktop app has several windows") }
}

fn has_other_window(s: &Session) -> Option<&'static str> {
    windows_available(s).or(if s.windows.others.is_empty() { Some("no other WordCraft window is open") } else { None })
}

/// The window a command means: `window`, or the only other one.
fn pick_window(s: &Session, v: &Value) -> Result<u64, CmdError> {
    let others = &s.windows.others;
    match p::u64(v, "window") {
        Some(id) if others.iter().any(|w| w.id == id) => Ok(id),
        Some(id) => Err(CmdError::Params(format!("no open window {id}"))),
        None => match others.as_slice() {
            [only] => Ok(only.id),
            [] => Err(CmdError::Disabled("no other WordCraft window is open".into())),
            several => {
                let list: Vec<String> = several.iter().map(|w| format!("{} ({})", w.id, w.title)).collect();
                Err(CmdError::Params(format!("pass `window`: one of {}", list.join(", "))))
            }
        },
    }
}

/// View Side by Side: on with a window (left this one, right that one, scrolling together), or
/// off.
fn side_by_side(s: &mut Session, v: &Value) -> CmdResult {
    let on = p::bool(v, "value").unwrap_or(s.windows.side_by_side.is_none() || v.get("window").is_some());
    if !on {
        if let Some(id) = s.windows.side_by_side.take() {
            s.ui_requests.push(json!({"windows": "sideBySide", "off": id}));
        }
        s.windows.sync_scroll = false;
        return Ok(json!({"value": false}));
    }
    let id = match (s.windows.side_by_side, v.get("window")) {
        (Some(id), None) => id,
        _ => pick_window(s, v)?,
    };
    if let Some(old) = s.windows.side_by_side.filter(|old| *old != id) {
        s.ui_requests.push(json!({"windows": "sideBySide", "off": old}));
    }
    s.windows.side_by_side = Some(id);
    s.windows.sync_scroll = true;
    s.ui_requests.push(json!({"windows": "sideBySide", "with": id}));
    Ok(json!({"value": true, "window": id, "syncScroll": true}))
}

fn mode(s: &mut Session, m: ViewMode, read: bool) -> CmdResult {
    s.view.mode = m;
    s.view.read_mode = read;
    s.relayout();
    Ok(json!({"mode": m, "readMode": read}))
}

fn zoom(s: &mut Session, v: &Value) -> CmdResult {
    match v.get("value") {
        Some(Value::String(f)) if ["pageWidth", "onePage", "multiplePages", "textWidth"].contains(&f.as_str()) => {
            s.view.fit = f.clone();
            s.view.multi_page = f == "multiplePages";
        }
        Some(x) => {
            let pct = x.as_f64().ok_or_else(|| CmdError::Params("zoom percent or fit mode".into()))? as f32;
            if !(10.0..=500.0).contains(&pct) {
                return Err(CmdError::Params("zoom must be 10–500%".into()));
            }
            s.view.zoom = pct / 100.0;
            s.view.fit.clear();
            s.view.multi_page = false;
        }
        None => {
            s.ui_requests.push(json!({"open": "zoom"}));
        }
    }
    Ok(json!({"zoom": s.view.zoom, "fit": s.view.fit}))
}
