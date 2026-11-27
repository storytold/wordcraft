use std::collections::BTreeMap;

use craft::ui::patterns::dock::{PanePlace, PaneState};
use craft::ui::runtime::env::PaneEdge;
use egui::{Rect, pos2, vec2};
use serde_json::Value;
use wordcraft_engine::StylesPaneDock;

use super::{Pane, WordApp};

/// Every pane in the order the user left them (`paneOrder` in `ui.json`), then the panes that
/// list lacks in their default order. Each edge shows its panes in this order, the first outermost.
fn order(app: &WordApp) -> Vec<Pane> {
    let mut order: Vec<Pane> = app.ui.pane_order.iter().filter_map(|key| Pane::from_key(key)).collect();
    for pane in Pane::ALL {
        if !order.contains(&pane) {
            order.push(pane);
        }
    }
    order
}

/// Put `pane` directly before `before` in the order, outside it on their edge (`None`: after
/// every pane).
pub(super) fn move_before(app: &mut WordApp, pane: Pane, before: Option<Pane>) {
    let mut order = order(app);
    order.retain(|p| *p != pane);
    let at = before.and_then(|b| order.iter().position(|p| *p == b)).unwrap_or(order.len());
    order.insert(at, pane);
    app.ui.pane_order = order.iter().map(|p| p.key().to_string()).collect();
}

/// `paneOrder` as saved: unknown and repeated keys are dropped, not the whole file.
pub(crate) fn lenient_order<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let v = <Value as serde::Deserialize>::deserialize(d)?;
    let mut keys: Vec<String> = Vec::new();
    for key in v.as_array().map(Vec::as_slice).unwrap_or_default().iter().take(Pane::ALL.len() * 4).filter_map(Value::as_str) {
        if Pane::from_key(key).is_some() && !keys.iter().any(|k| k == key) {
            keys.push(key.to_string());
        }
    }
    Ok(keys)
}

/// The panes open now, from the session's view flags, in the user's order.
pub fn open(app: &WordApp) -> Vec<Pane> {
    let v = &app.session.view;
    let smart_art = v.smart_art_pane && wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session).is_some();
    let mut open = [
        (Pane::SmartArtText, smart_art),
        (Pane::Navigation, v.nav_pane),
        (Pane::Clipboard, v.clipboard_pane),
        (Pane::Styles, v.styles_pane),
        (Pane::StyleInspector, v.style_inspector),
        (Pane::Accessibility, v.accessibility_pane),
        (Pane::AltText, v.alt_text_pane),
        (Pane::FormatShape, v.format_shape_pane),
        (Pane::RestrictEditing, app.restrict.open),
        (Pane::Proofing, app.spelling_review.is_some()),
        (Pane::Comments, v.comments_pane),
    ]
    .into_iter()
    .filter_map(|(pane, shown)| shown.then_some(pane))
    .collect::<Vec<_>>();
    let order = order(app);
    open.sort_by_key(|pane| order.iter().position(|p| p == pane).unwrap_or(usize::MAX));
    open
}

/// Where a task pane other than Styles sits, kept in `ui.json` (#1756). Styles keeps its place in
/// `view.stylesPaneDock` and `stylesPaneFloat`.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SavedPane {
    /// `None`: the pane's default edge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dock: Option<SavedDock>,
    /// `[x, y, width, height]` of the floating pane, also kept while it is docked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub float: Option<[f32; 4]>,
    /// A floating pane shows its title row only.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub minimized: bool,
    /// The docked width this run; like the Styles pane's, not kept between runs.
    #[serde(skip)]
    pub width: Option<f32>,
}

/// A saved place: `left` and `right` are the window's physical sides.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SavedDock {
    Left,
    Right,
    Float,
}

impl SavedDock {
    /// The word `ui.paneDock` and `view.stylesPaneDock` use for the place.
    pub(super) fn word(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Float => "float",
        }
    }
}

/// The physical place a pane area place stands for: its `Start` edge is the window's left only in a left-to-right
/// interface. `None` for a place no task pane takes (top, bottom).
pub(super) fn saved_dock(place: PanePlace, rtl: bool) -> Option<SavedDock> {
    match place {
        PanePlace::Floating => Some(SavedDock::Float),
        PanePlace::Docked(PaneEdge::Start) => Some(if rtl { SavedDock::Right } else { SavedDock::Left }),
        PanePlace::Docked(PaneEdge::End) => Some(if rtl { SavedDock::Left } else { SavedDock::Right }),
        _ => None,
    }
}

/// The pane area place of a saved physical place.
fn dock_place(dock: SavedDock, rtl: bool) -> PanePlace {
    match (dock, rtl) {
        (SavedDock::Left, false) | (SavedDock::Right, true) => PanePlace::Docked(PaneEdge::Start),
        (SavedDock::Right, false) | (SavedDock::Left, true) => PanePlace::Docked(PaneEdge::End),
        (SavedDock::Float, _) => PanePlace::Floating,
    }
}

/// `panePlaces` as saved: unknown panes, places and rects are dropped, not the whole file.
pub(crate) fn lenient_places<'de, D: serde::Deserializer<'de>>(d: D) -> Result<BTreeMap<String, SavedPane>, D::Error> {
    let v = <Value as serde::Deserialize>::deserialize(d)?;
    let Some(map) = v.as_object() else { return Ok(BTreeMap::new()) };
    let dock = |v: &Value| match v.as_str() {
        Some("left") => Some(SavedDock::Left),
        Some("right") => Some(SavedDock::Right),
        Some("float") => Some(SavedDock::Float),
        _ => None,
    };
    Ok(Pane::ALL
        .iter()
        .filter(|pane| **pane != Pane::Styles)
        .filter_map(|pane| {
            let saved = map.get(pane.key())?;
            let place = SavedPane {
                dock: saved.get("dock").and_then(dock),
                float: saved.get("float").and_then(float_rect),
                minimized: saved.get("minimized").and_then(Value::as_bool).unwrap_or(false),
                width: None,
            };
            Some((pane.key().to_string(), place))
        })
        .collect())
}

/// Four finite numbers as `[x, y, width, height]`; the pane area brings a saved rect back
/// inside the window.
pub(crate) fn float_rect(v: &Value) -> Option<[f32; 4]> {
    let [x, y, w, h] = v.as_array().map(Vec::as_slice)? else { return None };
    let n = |v: &Value| v.as_f64().map(|n| n as f32).filter(|n| n.is_finite());
    n(x).zip(n(y)).zip(n(w)).zip(n(h)).map(|(((x, y), w), h)| [x, y, w, h])
}

/// A saved `[x, y, width, height]` as a rect.
fn rect_of([x, y, w, h]: [f32; 4]) -> Rect {
    Rect::from_min_size(pos2(x, y), vec2(w, h))
}

/// A rect as the saved `[x, y, width, height]`.
pub(super) fn rect_to(rect: Rect) -> [f32; 4] {
    [rect.min.x, rect.min.y, rect.width(), rect.height()]
}

/// A pane's place and size: Styles' from `view.stylesPaneDock` and its own preferences, the
/// others' from `panePlaces`.
///
/// `left` / `right` in the docking API and in saved prefs are **physical** window sides (and the
/// Dock Left / Dock Right menus mean the same); the pane area's edges are logical, so `rtl` maps one to the
/// other. A pane that was never docked keeps its logical default, which mirrors by itself.
pub(super) fn pane_state(app: &WordApp, pane: Pane, rtl: bool) -> PaneState {
    if pane == Pane::Styles {
        return styles_state(app, rtl);
    }
    let mut state = pane.default_state();
    let Some(saved) = app.ui.pane_places.get(pane.key()) else { return state };
    if let Some(dock) = saved.dock {
        state.place = dock_place(dock, rtl);
    }
    state.size = saved.width.or(state.size);
    state.float_rect = saved.float.map(rect_of);
    state.minimized = saved.minimized;
    state
}

/// The Styles pane's place and size, from `view.stylesPaneDock` and the UI preferences (#1756).
fn styles_state(app: &WordApp, rtl: bool) -> PaneState {
    let dock = match app.session.view.styles_pane_dock {
        StylesPaneDock::Right => SavedDock::Right,
        StylesPaneDock::Left => SavedDock::Left,
        StylesPaneDock::Floating => SavedDock::Float,
    };
    let mut state = Pane::Styles.default_state();
    state.place = dock_place(dock, rtl);
    state.size = app.ui.styles_pane_width.or(state.size);
    state.float_rect = app.ui.styles_pane_float.map(rect_of);
    state
}
