use super::format::fmt;
use super::*;

/// Most tab stops a paragraph keeps.
const MAX_TABS: usize = 64;

/// Tab stops: replace the direct list, set and clear stops, and change the default interval, in
/// one step. Clearing a stop the paragraph style defines records a clear stop over it.
pub(super) fn tabs(s: &mut Session, v: &Value) -> CmdResult {
    let stops = |k: &str| -> Result<Option<Vec<TabStop>>, CmdError> {
        let Some(x) = v.get(k).filter(|x| !x.is_null()) else { return Ok(None) };
        let list: Vec<TabStop> = serde_json::from_value(x.clone()).map_err(|e| CmdError::Params(format!("`{k}`: {e}")))?;
        if list.len() > MAX_TABS {
            return Err(CmdError::Params(format!("at most {MAX_TABS} tab stops")));
        }
        if list.iter().any(|t| !t.pos.is_finite() || t.pos.abs() > 1584.0) {
            return Err(CmdError::Params("tab positions are points within ±1584".into()));
        }
        Ok(Some(list))
    };
    let replace = stops("tabs")?;
    let set = stops("set")?.unwrap_or_default();
    let clear: Vec<f32> = match v.get("clear").filter(|x| !x.is_null()) {
        None => Vec::new(),
        Some(x) => {
            let list: Vec<f32> = serde_json::from_value(x.clone()).map_err(|e| CmdError::Params(format!("`clear`: {e}")))?;
            if list.len() > MAX_TABS || list.iter().any(|p| !p.is_finite()) {
                return Err(CmdError::Params(format!("`clear` is at most {MAX_TABS} positions in points")));
            }
            list
        }
    };
    let clear_all = p::bool(v, "clearAll").unwrap_or(false);
    if let Some(d) = p::f32(v, "default") {
        s.doc.settings.default_tab = d.clamp(1.0, 1584.0);
    }
    if replace.is_none() && set.is_empty() && clear.is_empty() && !clear_all {
        return sel_result(s);
    }
    let styles = s.doc.styles.clone();
    let near = |a: f32, b: f32| (a - b).abs() < wordcraft_doc::props::TAB_POS_EPS;
    fmt(s, &|p| {
        // The stops the paragraph inherits from its style.
        let inherited = styles.resolve_para(&ParaProps { tabs: None, ..p.clone() }).tabs;
        let mut list = replace.clone().unwrap_or_else(|| p.tabs.clone().unwrap_or_default());
        let clear_at = |list: &mut Vec<TabStop>, pos: f32| {
            list.retain(|t| !near(t.pos, pos));
            if inherited.iter().any(|t| near(t.pos, pos)) {
                list.push(TabStop { pos, align: TabAlign::Clear, leader: TabLeader::None });
            }
        };
        if clear_all {
            list.clear();
            for t in &inherited {
                clear_at(&mut list, t.pos);
            }
        }
        for pos in &clear {
            clear_at(&mut list, *pos);
        }
        for t in &set {
            list.retain(|x| !near(x.pos, t.pos));
            list.push(*t);
        }
        list.sort_by(|a, b| a.pos.total_cmp(&b.pos));
        list.truncate(MAX_TABS);
        p.tabs = Some(list);
    })
}

/// A `sides` object: exactly the sides it names, each `{"style", "width", "color"}`.
pub fn border_sides(v: &Value, space_tb: f32, space_lr: f32) -> Result<Borders, CmdError> {
    let obj = v.as_object().ok_or_else(|| CmdError::Params("`sides` is an object".into()))?;
    let side = |k: &str, space: f32| -> Option<Border> {
        let x = obj.get(k).filter(|x| x.is_object())?;
        let style = p::str(x, "style").map(BorderStyle::from_ooxml).unwrap_or(BorderStyle::Single);
        let width = p::f32(x, "width").unwrap_or(0.5).clamp(0.25, 6.0);
        Some(Border { style, width, color: p::str(x, "color").and_then(Rgb::parse), space })
    };
    Ok(Borders {
        top: side("top", space_tb),
        left: side("left", space_lr),
        bottom: side("bottom", space_tb),
        right: side("right", space_lr),
        between: side("between", space_tb),
        inside_v: None,
    })
}

pub(super) fn borders(s: &mut Session, v: &Value) -> CmdResult {
    if let Some(sides) = v.get("sides") {
        let b = border_sides(sides, 1.0, 4.0)?;
        // An explicit empty set keeps a style's borders off.
        return fmt(s, &|p| p.borders = Some(b));
    }
    let kind = p::str(v, "kind").unwrap_or("bottom");
    let width = p::f32(v, "width").unwrap_or(0.5).clamp(0.25, 6.0);
    let color = p::str(v, "color").and_then(Rgb::parse);
    let style = p::str(v, "style").map(BorderStyle::from_ooxml).unwrap_or(BorderStyle::Single);
    let b = Border { style, width, color, space: if kind == "left" || kind == "right" { 4.0 } else { 1.0 } };
    if kind == "horizontalLine" {
        // A paragraph with a bottom border, like Word's Horizontal Line.
        return fmt(s, &|p| p.borders = Some(Borders { bottom: Some(Border { width: 1.5, ..b }), ..Default::default() }));
    }
    fmt(s, &|p| {
        let mut cur = p.borders.unwrap_or_default();
        match kind {
            "none" => cur = Borders::default(),
            "all" => cur = Borders::all(b),
            "outside" => {
                cur.top = Some(b);
                cur.bottom = Some(b);
                cur.left = Some(b);
                cur.right = Some(b);
            }
            "inside" => cur.between = Some(b),
            "top" => cur.top = if cur.top.is_some_and(|x| x.is_visible()) { None } else { Some(b) },
            "bottom" => cur.bottom = if cur.bottom.is_some_and(|x| x.is_visible()) { None } else { Some(b) },
            "left" => cur.left = if cur.left.is_some_and(|x| x.is_visible()) { None } else { Some(b) },
            "right" => cur.right = if cur.right.is_some_and(|x| x.is_visible()) { None } else { Some(b) },
            _ => {}
        }
        p.borders = if cur.any_visible() { Some(cur) } else { Some(Borders::default()) };
    })
}
