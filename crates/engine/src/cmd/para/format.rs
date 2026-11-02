use super::{Align, CmdError, CmdResult, LineSpacing, NumRef, ParaProps, Pos, Session, Value, json, p, sel_result};

/// Resolved paragraph props at the caret.
pub(super) fn cur(s: &Session) -> wordcraft_doc::resolve::ResolvedPara {
    // A selection ending at the next paragraph's start doesn't format that paragraph (#418), so
    // its state comes from the last paragraph it does cover.
    let (a, b) = s.sel.ordered();
    let mut at = s.sel.focus.clone();
    if let Some(last) = s.doc.para_paths_in(&a, &b).pop()
        && last != at.path
        && at == b
    {
        at = Pos { path: last, off: 0, ..at };
    }
    let props = s.doc.para_at(&at).map(|p| p.props.clone()).unwrap_or_default();
    s.doc.styles.resolve_para(&props)
}

pub fn fmt(s: &mut Session, f: &dyn Fn(&mut ParaProps)) -> CmdResult {
    let story = s.sel.focus.story;
    for path in crate::cmd::selected_paths(s) {
        let para = s.doc.para_mut(story, &path)?;
        f(&mut para.props);
        para.touch();
    }
    sel_result(s)
}

/// `a` is the alignment as seen on the page (the buttons): in a right-to-left paragraph
/// Align Left is its end edge.
pub(super) fn align(s: &mut Session, a: Align) -> CmdResult {
    let r = cur(s);
    // Clicking the active alignment again returns to the start edge (Word).
    let logical = a.visual(r.bidi);
    let target = if r.align == logical && logical != Align::Left { None } else { Some(a) };
    let focus_rtl = r.bidi;
    fmt(s, &|p| {
        let rtl = p.bidi.unwrap_or(focus_rtl);
        p.align = Some(target.map_or(Align::Left, |a| a.visual(rtl)));
    })
}

/// Set the alignment as seen on the page in every selected paragraph, in its own direction.
pub(super) fn set_align(s: &mut Session, a: Align) -> CmdResult {
    let focus_rtl = cur(s).bidi;
    fmt(s, &|p| p.align = Some(a.visual(p.bidi.unwrap_or(focus_rtl))))
}

/// Paragraph reading order (Word's Right-to-Left / Left-to-Right Text Direction buttons). Only
/// the direction changes: alignment and indents are logical, so a start-aligned paragraph moves
/// to the other side, as in Word. `rtl` is what the command turns on; `value: false` turns it off.
pub(super) fn direction(s: &mut Session, v: &Value, rtl: bool) -> CmdResult {
    let on = p::bool(v, "value").unwrap_or(true);
    let want = if on { rtl } else { !rtl };
    fmt(s, &|p| p.bidi = Some(want))
}

pub(super) fn tog(s: &mut Session, v: &Value, get: fn(&ParaProps) -> Option<bool>, set: fn(&mut ParaProps, bool)) -> CmdResult {
    let on = p::bool(v, "value").unwrap_or_else(|| {
        let r = cur(s);
        let props = s.doc.para_at(&s.sel.focus).map(|p| p.props.clone()).unwrap_or_default();
        let _ = r;
        !get(&props).unwrap_or(false)
    });
    fmt(s, &|p| set(p, on))
}

/// Asian Typography (the Paragraph dialog's tab): sets the given flags on every selected
/// paragraph and returns the resolved flags at the caret.
pub(super) fn asian_typography(s: &mut Session, v: &Value) -> CmdResult {
    let flag = |k: &str| p::bool(v, k);
    let (kinsoku, word_wrap, overflow, top_line, de, dn) =
        (flag("kinsoku"), flag("wordWrap"), flag("overflowPunct"), flag("topLinePunct"), flag("autoSpaceDE"), flag("autoSpaceDN"));
    let mut out = if [kinsoku, word_wrap, overflow, top_line, de, dn].iter().any(Option::is_some) {
        fmt(s, &|p| {
            let set = |field: &mut Option<bool>, val: Option<bool>| {
                if val.is_some() {
                    *field = val;
                }
            };
            set(&mut p.kinsoku, kinsoku);
            set(&mut p.word_wrap, word_wrap);
            set(&mut p.overflow_punct, overflow);
            set(&mut p.top_line_punct, top_line);
            set(&mut p.auto_space_de, de);
            set(&mut p.auto_space_dn, dn);
        })?
    } else {
        sel_result(s)?
    };
    let r = cur(s);
    if let Some(o) = out.as_object_mut() {
        o.insert(
            "asianTypography".into(),
            json!({
                "kinsoku": r.kinsoku, "wordWrap": r.word_wrap, "overflowPunct": r.overflow_punct,
                "topLinePunct": r.top_line_punct, "autoSpaceDE": r.auto_space_de, "autoSpaceDN": r.auto_space_dn,
            }),
        );
    }
    Ok(out)
}

/// Set Paragraph Formatting, the Paragraph dialog's OK as one undo step (#1346): `props` on
/// every selected paragraph, then `onPage`'s alignment and indents as seen on the page, each
/// paragraph's own direction (once `props` is applied) saying which edge is its start.
pub(super) fn set_props(s: &mut Session, v: &Value) -> CmdResult {
    let props: ParaProps = serde_json::from_value(v.get("props").cloned().unwrap_or(Value::Null)).map_err(|e| CmdError::Params(e.to_string()))?;
    let page = v.get("onPage").cloned().unwrap_or(Value::Null);
    let align = match p::str(&page, "align") {
        None => None,
        Some("left") => Some(Align::Left),
        Some("center") => Some(Align::Center),
        Some("right") => Some(Align::Right),
        Some("justify" | "both") => Some(Align::Justify),
        Some("distribute") => Some(Align::Distribute),
        Some(x) => return Err(CmdError::Params(format!("unknown alignment `{x}`"))),
    };
    let indent = |k: &str| p::f32(&page, k).map(|x| x.clamp(-1584.0, 1584.0));
    let (left, right) = (indent("left"), indent("right"));
    let story = s.sel.focus.story;
    for path in crate::cmd::selected_paths(s) {
        let Some(para) = s.doc.para(story, &path) else { continue };
        let rtl = s.doc.styles.resolve_para(&para.props.clone().overlaid(&props)).bidi;
        let para = s.doc.para_mut(story, &path)?;
        para.props.overlay(&props);
        if let Some(a) = align {
            para.props.align = Some(a.visual(rtl));
        }
        let (start, end) = if rtl { (right, left) } else { (left, right) };
        if let Some(x) = start {
            para.props.indent_left = Some(x);
        }
        if let Some(x) = end {
            para.props.indent_right = Some(x);
        }
        para.touch();
    }
    sel_result(s)
}

pub fn indent(s: &mut Session, _: &Value) -> CmdResult {
    let a = s.sel.ordered().0;
    let paths = crate::cmd::selected_paths(s);
    let tab = s.doc.settings.default_tab.max(1.0);
    for path in paths {
        let (num, left) = {
            let Some(p) = s.doc.para(a.story, &path) else { continue };
            (p.props.numbering.filter(|n| n.num != 0), s.doc.styles.resolve_para(&p.props).indent_left)
        };
        let para = s.doc.para_mut(a.story, &path)?;
        match num {
            Some(n) => para.props.numbering = Some(NumRef { num: n.num, level: n.level.saturating_add(1).min(8) }),
            None => {
                para.props.indent_left = Some(((left / tab).floor() + 1.0) * tab);
                para.props.cancel_char_indents(true, false, false);
            }
        }
        para.touch();
    }
    sel_result(s)
}

pub fn outdent(s: &mut Session, _: &Value) -> CmdResult {
    let a = s.sel.ordered().0;
    let paths = crate::cmd::selected_paths(s);
    let tab = s.doc.settings.default_tab.max(1.0);
    for path in paths {
        let (num, left) = {
            let Some(p) = s.doc.para(a.story, &path) else { continue };
            (p.props.numbering.filter(|n| n.num != 0), s.doc.styles.resolve_para(&p.props).indent_left)
        };
        let para = s.doc.para_mut(a.story, &path)?;
        match num {
            Some(n) => para.props.numbering = Some(NumRef { num: n.num, level: n.level.saturating_sub(1) }),
            None => {
                para.props.indent_left = Some((((left / tab).ceil() - 1.0) * tab).max(0.0));
                para.props.cancel_char_indents(true, false, false);
            }
        }
        para.touch();
    }
    sel_result(s)
}

pub(super) fn ls(s: &mut Session, l: LineSpacing) -> CmdResult {
    fmt(s, &|p| p.line_spacing = Some(l))
}

pub(super) fn line_spacing(s: &mut Session, v: &Value) -> CmdResult {
    let l = if let Some(x) = p::f32(v, "atLeast") {
        LineSpacing::AtLeast(x.clamp(0.0, 1584.0))
    } else if let Some(x) = p::f32(v, "exactly") {
        LineSpacing::Exactly(x.clamp(0.7, 1584.0))
    } else {
        LineSpacing::Multiple(p::req_f32(v, "value")?.clamp(0.06, 132.0))
    };
    ls(s, l)
}
