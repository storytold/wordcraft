//! Paragraph formatting, lists and styles (Home › Paragraph, Home › Styles).

use serde_json::{Value, json};
use wordcraft_doc::numbering::ListKind;
use wordcraft_doc::props::{Align, Border, BorderStyle, Borders, LineSpacing, NumRef, ParaProps, Rgb, TabStop};
use wordcraft_doc::styles::{Style, StyleKind};
use wordcraft_doc::{Block, Pos};

use super::sel_result;
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("para.alignLeft", "Align Left", "Home › Paragraph", |s, _| align(s, Align::Left)).key("Mod+L"),
        CommandSpec::new("para.alignCenter", "Center", "Home › Paragraph", |s, _| align(s, Align::Center)).key("Mod+E"),
        CommandSpec::new("para.alignRight", "Align Right", "Home › Paragraph", |s, _| align(s, Align::Right)).key("Mod+R"),
        CommandSpec::new("para.justify", "Justify", "Home › Paragraph", |s, _| align(s, Align::Justify)).key("Mod+J"),
        CommandSpec::new("para.distribute", "Distributed", "Home › Paragraph", |s, _| align(s, Align::Distribute)).key("Mod+Shift+J"),
        CommandSpec::new("para.align", "Alignment", "Home › Paragraph", |s, v| {
            let a = match p::req_str(v, "value")? {
                "left" => Align::Left,
                "center" => Align::Center,
                "right" => Align::Right,
                "justify" | "both" => Align::Justify,
                "distribute" => Align::Distribute,
                // Logical edges, whatever the paragraph's direction.
                "start" => return fmt(s, &|p| p.align = Some(Align::Left)),
                "end" => return fmt(s, &|p| p.align = Some(Align::Right)),
                x => return Err(CmdError::Params(format!("unknown alignment `{x}`"))),
            };
            align(s, a)
        })
        .params(r#"{"value": "left|center|right|justify|distribute (as seen on the page) | start|end (reading direction)"}"#),
        CommandSpec::new("para.rtl", "Right-to-Left Text Direction", "Home › Paragraph", |s, v| direction(s, v, true))
            .params(r#"{"value"?: bool (false = left to right)}"#),
        CommandSpec::new("para.ltr", "Left-to-Right Text Direction", "Home › Paragraph", |s, v| direction(s, v, false))
            .params(r#"{"value"?: bool (false = right to left)}"#),
        CommandSpec::new("para.indent", "Increase Indent", "Home › Paragraph", indent).key("Mod+M"),
        CommandSpec::new("para.outdent", "Decrease Indent", "Home › Paragraph", outdent).key("Mod+Shift+M"),
        CommandSpec::new("para.hangingIndent", "Hanging Indent", "Home › Paragraph › Paragraph", |s, _| {
            fmt(s, &|p| {
                let l = p.indent_left.unwrap_or(0.0) + 36.0;
                p.indent_left = Some(l);
                p.indent_first = Some(-36.0);
            })
        })
        .key("Mod+T"),
        CommandSpec::new("para.removeHanging", "Reduce Hanging Indent", "Home › Paragraph › Paragraph", |s, _| {
            fmt(s, &|p| {
                p.indent_left = Some((p.indent_left.unwrap_or(0.0) - 36.0).max(0.0));
                p.indent_first = Some(0.0);
            })
        })
        .key("Mod+Shift+T"),
        CommandSpec::new("para.indents", "Indents", "Layout › Paragraph", |s, v| {
            let (l, r, f) = (p::f32(v, "left"), p::f32(v, "right"), p::f32(v, "firstLine"));
            fmt(s, &|p| {
                if let Some(x) = l {
                    p.indent_left = Some(x.clamp(-1584.0, 1584.0));
                }
                if let Some(x) = r {
                    p.indent_right = Some(x.clamp(-1584.0, 1584.0));
                }
                if let Some(x) = f {
                    p.indent_first = Some(x.clamp(-1584.0, 1584.0));
                }
            })
        })
        .params(r#"{"left"?: pt, "right"?: pt, "firstLine"?: pt (negative = hanging)}"#),
        CommandSpec::new("para.lineSpacing", "Line and Paragraph Spacing", "Home › Paragraph", line_spacing)
            .params(r#"{"value": number (multiple) } | {"atLeast": pt} | {"exactly": pt}"#),
        CommandSpec::new("para.single", "Single Spacing", "Home › Paragraph › Line Spacing", |s, _| ls(s, LineSpacing::Multiple(1.0))).key("Mod+1"),
        CommandSpec::new("para.double", "Double Spacing", "Home › Paragraph › Line Spacing", |s, _| ls(s, LineSpacing::Multiple(2.0))).key("Mod+2"),
        CommandSpec::new("para.oneAndHalf", "1.5 Line Spacing", "Home › Paragraph › Line Spacing", |s, _| ls(s, LineSpacing::Multiple(1.5))).key("Mod+5"),
        CommandSpec::new("para.spacing", "Paragraph Spacing", "Layout › Paragraph", |s, v| {
            let (b, a) = (p::f32(v, "before"), p::f32(v, "after"));
            fmt(s, &|p| {
                if let Some(x) = b {
                    p.space_before = Some(x.clamp(0.0, 1584.0));
                }
                if let Some(x) = a {
                    p.space_after = Some(x.clamp(0.0, 1584.0));
                }
            })
        })
        .params(r#"{"before"?: pt, "after"?: pt}"#),
        CommandSpec::new("para.addSpaceBefore", "Add Space Before Paragraph", "Home › Paragraph › Line Spacing", |s, _| {
            let has = cur(s).space_before > 0.0;
            fmt(s, &|p| p.space_before = Some(if has { 0.0 } else { 12.0 }))
        })
        .key("Mod+0"),
        CommandSpec::new("para.removeSpaceAfter", "Remove Space After Paragraph", "Home › Paragraph › Line Spacing", |s, _| {
            let has = cur(s).space_after > 0.0;
            fmt(s, &|p| p.space_after = Some(if has { 0.0 } else { 8.0 }))
        }),
        CommandSpec::new("para.style", "Apply Style", "Home › Styles", apply_style).params(r#"{"style": string (name or id)}"#).key("Mod+Shift+S"),
        CommandSpec::new("para.normal", "Normal Style", "Home › Styles", |s, _| apply_style(s, &json!({"style": "Normal"}))).key("Mod+Shift+N"),
        CommandSpec::new("para.heading1", "Heading 1", "Home › Styles", |s, _| apply_style(s, &json!({"style": "Heading1"}))).key("Mod+Alt+1"),
        CommandSpec::new("para.heading2", "Heading 2", "Home › Styles", |s, _| apply_style(s, &json!({"style": "Heading2"}))).key("Mod+Alt+2"),
        CommandSpec::new("para.heading3", "Heading 3", "Home › Styles", |s, _| apply_style(s, &json!({"style": "Heading3"}))).key("Mod+Alt+3"),
        CommandSpec::new("para.bullets", "Bullets", "Home › Paragraph", |s, v| list(s, v, ListKind::Bullet)).params(r#"{"kind"?: "bullet" | single char, "off"?: bool}"#).key("Mod+Shift+L"),
        CommandSpec::new("para.numbering", "Numbering", "Home › Paragraph", |s, v| list(s, v, ListKind::Numbered))
            .params(r#"{"kind"?: "numbered|numberedParen|upperLetter|lowerLetter|lowerRoman|outline", "off"?: bool}"#),
        CommandSpec::new("para.multilevel", "Multilevel List", "Home › Paragraph", |s, v| list(s, v, ListKind::Legal)).params(r#"{"kind"?: "legal|outline"}"#),
        CommandSpec::new("para.listLevel", "Change List Level", "Home › Paragraph › Bullets", |s, v| {
            let lvl = p::u64(v, "level").unwrap_or(0).min(8) as u8;
            fmt(s, &|p| {
                if let Some(n) = p.numbering.as_mut() {
                    n.level = lvl;
                }
            })
        })
        .params(r#"{"level": 0-8}"#),
        CommandSpec::new("para.restartNumbering", "Restart at 1", "Home › Paragraph › Numbering", restart_numbering),
        CommandSpec::new("para.shading", "Shading", "Home › Paragraph", |s, v| {
            let c = p::str(v, "color").and_then(Rgb::parse);
            fmt(s, &|p| p.shading = c)
        })
        .params(r#"{"color": "RRGGBB" | null}"#),
        CommandSpec::new("para.borders", "Borders", "Home › Paragraph", borders)
            .params(r#"{"kind": "bottom|top|left|right|none|all|outside|inside|horizontalLine", "width"?: pt, "color"?: "RRGGBB", "style"?: "single|double|dotted|dashed|thick"}"#),
        CommandSpec::new("para.sort", "Sort", "Home › Paragraph", sort).params(r#"{"descending"?: bool}"#),
        CommandSpec::new("para.keepNext", "Keep with Next", "Home › Paragraph › Line and Page Breaks", |s, v| tog(s, v, |p| p.keep_next, |p, b| p.keep_next = Some(b))),
        CommandSpec::new("para.keepLines", "Keep Lines Together", "Home › Paragraph › Line and Page Breaks", |s, v| tog(s, v, |p| p.keep_lines, |p, b| p.keep_lines = Some(b))),
        CommandSpec::new("para.pageBreakBefore", "Page Break Before", "Home › Paragraph › Line and Page Breaks", |s, v| {
            tog(s, v, |p| p.page_break_before, |p, b| p.page_break_before = Some(b))
        }),
        CommandSpec::new("para.widowControl", "Widow/Orphan Control", "Home › Paragraph › Line and Page Breaks", |s, v| {
            tog(s, v, |p| p.widow_control, |p, b| p.widow_control = Some(b))
        }),
        CommandSpec::new("para.tabs", "Tabs", "Home › Paragraph › Paragraph", |s, v| {
            let tabs: Vec<TabStop> = serde_json::from_value(v.get("tabs").cloned().unwrap_or(Value::Null)).map_err(|e| CmdError::Params(e.to_string()))?;
            if tabs.len() > 64 {
                return Err(CmdError::Params("at most 64 tab stops".into()));
            }
            fmt(s, &|p| p.tabs = Some(tabs.clone()))
        })
        .params(r#"{"tabs": [{"pos": pt, "align": "left|center|right|decimal|bar", "leader": "none|dot|hyphen|underscore"}]}"#),
        CommandSpec::new("para.outlineLevel", "Outline Level", "Home › Paragraph › Paragraph", |s, v| {
            let l = p::u64(v, "level").map(|x| x.min(9) as u8);
            fmt(s, &|p| p.outline_level = l)
        }),
        CommandSpec::new("para.set", "Set Paragraph Formatting", "Home › Paragraph › Paragraph", |s, v| {
            let props: ParaProps = serde_json::from_value(v.get("props").cloned().unwrap_or(Value::Null)).map_err(|e| CmdError::Params(e.to_string()))?;
            fmt(s, &|p| p.overlay(&props))
        })
        .params(r#"{"props": ParaProps}"#),
        CommandSpec::new("para.dialog", "Paragraph Settings", "Home › Paragraph", |s, _| {
            s.ui_requests.push(json!({"open": "paragraph"}));
            sel_result(s)
        })
        .pure(),
        CommandSpec::new("styles.create", "Create a Style", "Home › Styles", create_style).params(r#"{"name": string, "basedOn"?: string, "fromSelection"?: bool}"#),
        CommandSpec::new("styles.modify", "Modify Style", "Home › Styles", modify_style).params(r#"{"style": string, "chr"?: CharProps, "para"?: ParaProps, "name"?: string, "next"?: string}"#),
        CommandSpec::new("styles.updateToMatch", "Update Style to Match Selection", "Home › Styles", update_to_match).params(r#"{"style"?: string}"#),
        CommandSpec::new("styles.delete", "Delete Style", "Home › Styles", |s, v| {
            let id = style_id(s, p::req_str(v, "style")?)?;
            if s.doc.styles.get(&id).is_some_and(|st| st.builtin) {
                return Err(CmdError::Failed("built-in styles can't be deleted".into()));
            }
            s.doc.styles.styles.retain(|st| st.id != id);
            sel_result(s)
        }),
        CommandSpec::new("styles.list", "Styles", "Home › Styles", |s, _| {
            Ok(Value::Array(
                s.doc.styles.styles.iter().map(|st| json!({"id": st.id, "name": st.name, "kind": st.kind, "quick": st.quick, "basedOn": st.based_on})).collect(),
            ))
        })
        .pure(),
        CommandSpec::new("styles.pane", "Styles Pane", "Home › Styles", |s, _| {
            s.view.styles_pane = !s.view.styles_pane;
            sel_result(s)
        })
        .key("Mod+Alt+Shift+S")
        .pure(),
        CommandSpec::new("styles.addToGallery", "Add to Style Gallery", "Home › Styles", |s, v| {
            let id = style_id(s, p::req_str(v, "style")?)?;
            let on = p::bool(v, "value").unwrap_or(true);
            if let Some(st) = s.doc.styles.get_mut(&id) {
                st.quick = on;
            }
            sel_result(s)
        }),
    ]
}

/// Resolved paragraph props at the caret.
fn cur(s: &Session) -> wordcraft_doc::resolve::ResolvedPara {
    let props = s.doc.para_at(&s.sel.focus).map(|p| p.props.clone()).unwrap_or_default();
    s.doc.styles.resolve_para(&props)
}

pub fn fmt(s: &mut Session, f: &dyn Fn(&mut ParaProps)) -> CmdResult {
    let (a, b) = s.sel.ordered();
    s.doc.format_paragraphs(&a, &b, f)?;
    sel_result(s)
}

/// `a` is the alignment as seen on the page (the buttons): in a right-to-left paragraph
/// Align Left is its end edge.
fn align(s: &mut Session, a: Align) -> CmdResult {
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

/// Paragraph reading order (Word's Right-to-Left / Left-to-Right Text Direction buttons). Only
/// the direction changes: alignment and indents are logical, so a start-aligned paragraph moves
/// to the other side, as in Word. `rtl` is what the command turns on; `value: false` turns it off.
fn direction(s: &mut Session, v: &Value, rtl: bool) -> CmdResult {
    let on = p::bool(v, "value").unwrap_or(true);
    let want = if on { rtl } else { !rtl };
    fmt(s, &|p| p.bidi = Some(want))
}

fn tog(s: &mut Session, v: &Value, get: fn(&ParaProps) -> Option<bool>, set: fn(&mut ParaProps, bool)) -> CmdResult {
    let on = p::bool(v, "value").unwrap_or_else(|| {
        let r = cur(s);
        let props = s.doc.para_at(&s.sel.focus).map(|p| p.props.clone()).unwrap_or_default();
        let _ = r;
        !get(&props).unwrap_or(false)
    });
    fmt(s, &|p| set(p, on))
}

pub fn indent(s: &mut Session, _: &Value) -> CmdResult {
    let (a, b) = s.sel.ordered();
    let paths = s.doc.paths_between(&a, &b);
    let tab = s.doc.settings.default_tab.max(1.0);
    for path in paths {
        let (num, left) = {
            let Some(p) = s.doc.para(a.story, &path) else { continue };
            (p.props.numbering.filter(|n| n.num != 0), s.doc.styles.resolve_para(&p.props).indent_left)
        };
        let para = s.doc.para_mut(a.story, &path)?;
        match num {
            Some(n) => para.props.numbering = Some(NumRef { num: n.num, level: (n.level + 1).min(8) }),
            None => para.props.indent_left = Some(((left / tab).floor() + 1.0) * tab),
        }
        para.touch();
    }
    sel_result(s)
}

pub fn outdent(s: &mut Session, _: &Value) -> CmdResult {
    let (a, b) = s.sel.ordered();
    let paths = s.doc.paths_between(&a, &b);
    let tab = s.doc.settings.default_tab.max(1.0);
    for path in paths {
        let (num, left) = {
            let Some(p) = s.doc.para(a.story, &path) else { continue };
            (p.props.numbering.filter(|n| n.num != 0), s.doc.styles.resolve_para(&p.props).indent_left)
        };
        let para = s.doc.para_mut(a.story, &path)?;
        match num {
            Some(n) => para.props.numbering = Some(NumRef { num: n.num, level: n.level.saturating_sub(1) }),
            None => para.props.indent_left = Some((((left / tab).ceil() - 1.0) * tab).max(0.0)),
        }
        para.touch();
    }
    sel_result(s)
}

fn ls(s: &mut Session, l: LineSpacing) -> CmdResult {
    fmt(s, &|p| p.line_spacing = Some(l))
}

fn line_spacing(s: &mut Session, v: &Value) -> CmdResult {
    let l = if let Some(x) = p::f32(v, "atLeast") {
        LineSpacing::AtLeast(x.clamp(0.0, 1584.0))
    } else if let Some(x) = p::f32(v, "exactly") {
        LineSpacing::Exactly(x.clamp(0.7, 1584.0))
    } else {
        LineSpacing::Multiple(p::req_f32(v, "value")?.clamp(0.06, 132.0))
    };
    ls(s, l)
}

fn style_id(s: &Session, name: &str) -> Result<String, CmdError> {
    s.doc.styles.find(name).map(|x| x.id.clone()).ok_or_else(|| CmdError::Params(format!("no style `{name}`")))
}

fn apply_style(s: &mut Session, v: &Value) -> CmdResult {
    let name = p::req_str(v, "style")?;
    let id = style_id(s, name)?;
    let kind = s.doc.styles.get(&id).map(|x| x.kind).unwrap_or_default();
    if kind == StyleKind::Character {
        return super::format::apply(s, &|c| c.style = Some(id.clone()));
    }
    let (a, b) = s.sel.ordered();
    s.doc.format_paragraphs(&a, &b, &|p| {
        p.style = Some(id.clone());
        // Applying a heading or Normal removes direct list numbering (except list styles).
        if (id.starts_with("Heading") || id == "Title" || id == "Normal") && p.numbering.is_some_and(|n| n.num != 0) {
            p.numbering = None;
        }
    })?;
    // A paragraph style resets direct character formatting that matches nothing (Word keeps
    // direct formatting unless it covers most of the paragraph; we clear font/size/colour).
    if a.path == b.path || a == b {
        let paths = s.doc.paths_between(&a, &b);
        for path in paths {
            let para = s.doc.para_mut(a.story, &path)?;
            let len = para.len();
            para.format(0, len, &|c| {
                c.font = None;
                c.font_cs = None;
                c.size = None;
                c.size_cs = None;
                c.color = None;
            })?;
            para.mark.font = None;
            para.mark.font_cs = None;
            para.mark.size = None;
            para.mark.size_cs = None;
            para.mark.color = None;
        }
    }
    s.pending = None;
    sel_result(s)
}

fn list(s: &mut Session, v: &Value, default: ListKind) -> CmdResult {
    let kind = p::str(v, "kind").and_then(ListKind::parse).unwrap_or(default);
    let off = p::bool(v, "off").unwrap_or(false);
    let (a, b) = s.sel.ordered();
    // Toggle: if every selected paragraph already has this kind of list, remove it.
    let current: Vec<Option<NumRef>> =
        s.doc.paths_between(&a, &b).iter().map(|p| s.doc.para(a.story, p).and_then(|x| x.props.numbering).filter(|n| n.num != 0)).collect();
    let same_kind = |n: &NumRef| {
        s.doc.numbering.level(n.num, 0).is_some_and(|l| {
            let want = wordcraft_doc::numbering::levels_for(kind);
            want.first().is_some_and(|w| {
                (w.format == wordcraft_doc::section::NumFormat::Bullet) == (l.format == wordcraft_doc::section::NumFormat::Bullet)
                    && (p::str(v, "kind").is_none() || w.text == l.text)
            })
        })
    };
    let remove = off || (!current.is_empty() && current.iter().all(|n| n.as_ref().is_some_and(same_kind)));
    if remove {
        return fmt(s, &|p| {
            p.numbering = Some(NumRef { num: 0, level: 0 });
            p.indent_left = None;
            p.indent_first = None;
            if p.style.as_deref() == Some("ListParagraph") {
                p.style = None;
            }
        });
    }
    // Continue the list of the previous paragraph if it has the same kind; else a new list.
    let prev_num = s
        .doc
        .prev_para(a.story, &a.path)
        .and_then(|q| s.doc.para(a.story, &q).and_then(|x| x.props.numbering))
        .filter(|n| n.num != 0 && same_kind(n));
    let num = match prev_num {
        Some(n) => n.num,
        None => match kind {
            ListKind::Bullet | ListKind::BulletChar(_) => s.doc.numbering.find_kind(kind).unwrap_or_else(|| s.doc.numbering.add_list(kind)),
            _ => s.doc.numbering.add_list(kind),
        },
    };
    fmt(s, &|p| {
        let level = p.numbering.filter(|n| n.num != 0).map(|n| n.level).unwrap_or(0);
        p.numbering = Some(NumRef { num, level });
        p.indent_left = None;
        p.indent_first = None;
        if p.style.is_none() || p.style.as_deref() == Some("Normal") {
            p.style = Some("ListParagraph".into());
        }
    })
}

fn restart_numbering(s: &mut Session, _: &Value) -> CmdResult {
    let f = s.sel.focus.clone();
    let Some(n) = s.doc.para_at(&f).and_then(|p| p.props.numbering).filter(|n| n.num != 0) else {
        return Err(CmdError::Failed("not in a numbered list".into()));
    };
    let new = s.doc.numbering.restart(n.num).ok_or_else(|| CmdError::Failed("bad list".into()))?;
    // This paragraph and the following ones of the same list use the new instance.
    let paths: Vec<_> = s.doc.para_paths(f.story).into_iter().filter(|p| *p >= f.path).collect();
    for path in paths {
        let para = s.doc.para_mut(f.story, &path)?;
        match para.props.numbering {
            Some(x) if x.num == n.num => {
                para.props.numbering = Some(NumRef { num: new, level: x.level });
                para.touch();
            }
            _ => {
                if para.props.numbering.is_none() && !para.is_empty() {
                    break;
                }
            }
        }
    }
    sel_result(s)
}

fn borders(s: &mut Session, v: &Value) -> CmdResult {
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

/// Sort the selected paragraphs (same container) alphabetically.
fn sort(s: &mut Session, v: &Value) -> CmdResult {
    let desc = p::bool(v, "descending").unwrap_or(false);
    let (a, b) = s.sel.ordered();
    if a.path.parent() != b.path.parent() {
        return Err(CmdError::Failed("select paragraphs in one place to sort".into()));
    }
    let (i0, i1) = (a.path.last(), b.path.last());
    let bl = s.doc.container_mut(a.story, &a.path)?;
    let Some(slice) = bl.get_mut(i0..=i1) else { return Err(CmdError::Failed("bad range".into())) };
    if slice.iter().any(|b| !matches!(**b, Block::Para(_))) {
        return Err(CmdError::Failed("sorting tables isn't supported here; use Table › Sort".into()));
    }
    let key = |b: &std::sync::Arc<Block>| b.as_para().map(|p| p.plain_text().to_lowercase()).unwrap_or_default();
    slice.sort_by(|x, y| {
        let o = natural_cmp(&key(x), &key(y));
        if desc { o.reverse() } else { o }
    });
    // Keep a section break on the last paragraph of the range where it was.
    s.sel = crate::Selection { anchor: Pos { off: 0, ..a }, focus: b };
    sel_result(s)
}

/// Compare strings with embedded numbers numerically ("item 2" < "item 10").
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let num = |s: &str| s.trim().split(|c: char| !c.is_ascii_digit()).next().and_then(|n| n.parse::<f64>().ok());
    match (num(a), num(b)) {
        (Some(x), Some(y)) if x != y => x.total_cmp(&y),
        _ => a.cmp(b),
    }
}

fn create_style(s: &mut Session, v: &Value) -> CmdResult {
    let name = p::req_str(v, "name")?.trim().to_string();
    if name.is_empty() {
        return Err(CmdError::Params("name is empty".into()));
    }
    if s.doc.styles.find(&name).is_some() {
        return Err(CmdError::Failed(format!("a style named `{name}` already exists")));
    }
    let id = s.doc.styles.new_id(&name);
    let based = p::str(v, "basedOn").and_then(|b| s.doc.styles.find(b).map(|x| x.id.clone())).or(Some("Normal".into()));
    let mut st = Style {
        id: id.clone(),
        name,
        kind: StyleKind::Paragraph,
        based_on: based,
        next: Some(id.clone()),
        quick: true,
        priority: Some(50),
        ..Default::default()
    };
    if p::bool(v, "fromSelection").unwrap_or(true)
        && let Some(para) = s.doc.para_at(&s.sel.focus)
    {
        st.para = para.props.clone();
        st.para.style = None;
        st.para.numbering = None;
        st.chr = para.props_at(s.sel.focus.off).clone();
        st.chr.style = None;
        st.chr.link = None;
        st.chr.ins = None;
        st.chr.del = None;
    }
    s.doc.styles.upsert(st);
    apply_style(s, &json!({"style": id}))?;
    Ok(json!({"id": id}))
}

fn modify_style(s: &mut Session, v: &Value) -> CmdResult {
    let id = style_id(s, p::req_str(v, "style")?)?;
    let chr: Option<wordcraft_doc::CharProps> =
        v.get("chr").map(|c| serde_json::from_value(c.clone())).transpose().map_err(|e| CmdError::Params(e.to_string()))?;
    let para: Option<ParaProps> =
        v.get("para").map(|c| serde_json::from_value(c.clone())).transpose().map_err(|e| CmdError::Params(e.to_string()))?;
    let next = p::str(v, "next").map(str::to_string);
    let name = p::str(v, "name").map(str::to_string);
    let st = s.doc.styles.get_mut(&id).ok_or_else(|| CmdError::Params("no such style".into()))?;
    if let Some(c) = chr {
        st.chr.overlay(&c);
    }
    if let Some(pp) = para {
        st.para.overlay(&pp);
    }
    if let Some(n) = next {
        st.next = Some(n);
    }
    if let Some(n) = name.filter(|n| !n.trim().is_empty()) {
        st.name = n;
    }
    // Every paragraph's layout depends on styles: bump all revisions.
    touch_all(s);
    sel_result(s)
}

fn update_to_match(s: &mut Session, v: &Value) -> CmdResult {
    let f = s.sel.focus.clone();
    let Some(para) = s.doc.para_at(&f).cloned() else { return sel_result(s) };
    let id = match p::str(v, "style") {
        Some(n) => style_id(s, n)?,
        None => para.props.style.clone().unwrap_or_else(|| "Normal".into()),
    };
    let mut chr = para.props_at(f.off).clone();
    chr.style = None;
    chr.link = None;
    chr.ins = None;
    chr.del = None;
    let mut pp = para.props.clone();
    pp.style = None;
    pp.numbering = None;
    if let Some(st) = s.doc.styles.get_mut(&id) {
        st.chr.overlay(&chr);
        st.para.overlay(&pp);
    }
    // Remove the now-redundant direct formatting from the paragraph.
    let p = s.doc.para_mut(f.story, &f.path)?;
    let len = p.len();
    p.format(0, len, &|c| *c = c.cleared())?;
    let style = p.props.style.clone();
    p.props = ParaProps { style, numbering: p.props.numbering, ..Default::default() };
    touch_all(s);
    sel_result(s)
}

/// Style edits re-lay out every paragraph: the layout cache is keyed on the style sheet, so
/// nothing needs touching here (kept as the one place to hook style-change side effects).
pub fn touch_all(s: &mut Session) {
    s.dirty = true;
}
