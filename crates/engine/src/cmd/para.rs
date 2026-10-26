//! Paragraph formatting, lists and styles (Home › Paragraph, Home › Styles).

use serde_json::{Value, json};
use wordcraft_doc::collate::{SortKey, sort_key};
use wordcraft_doc::numbering::ListKind;
use wordcraft_doc::props::{Align, Border, BorderStyle, Borders, LineSpacing, NumRef, ParaProps, Rgb, TabAlign, TabLeader, TabStop};
use wordcraft_doc::styles::{Style, StyleKind};
use wordcraft_doc::{Block, CharProps, Pos};

use super::sel_result;
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

mod format;
mod lists;
mod outline;
mod styles;
mod tabs_borders;

use format::{align, asian_typography, cur, direction, line_spacing, ls, set_align, set_props, tog};
pub use format::{fmt, indent, outdent};
pub use lists::{ListButtons, list_buttons, manage_list, touch_all};
use lists::{list, restart_numbering};
pub(crate) use outline::{ensure_heading_style, set_heading};
use outline::{move_paras, outline_step, sort};
use styles::{apply_style, apply_styles, create_style, delete_style, modify_style, set_visibility, style_id, update_to_match};
pub use tabs_borders::border_sides;
use tabs_borders::{borders, tabs};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("para.alignLeft", "Align Left", "Home › Paragraph", |s, _| align(s, Align::Left)).key("Mod+L"),
        CommandSpec::new("para.alignCenter", "Center", "Home › Paragraph", |s, _| align(s, Align::Center)).key("Mod+E"),
        CommandSpec::new("para.alignRight", "Align Right", "Home › Paragraph", |s, _| align(s, Align::Right)).key("Mod+R"),
        CommandSpec::new("para.justify", "Justify", "Home › Paragraph", |s, _| align(s, Align::Justify)).key("Mod+J"),
        CommandSpec::new("para.distribute", "Distributed", "Home › Paragraph", |s, _| align(s, Align::Distribute)).key("Mod+Shift+J"),
        CommandSpec::new("para.thaiDistribute", "Thai Distributed", "Home › Paragraph", |s, _| align(s, Align::ThaiDistribute)),
        CommandSpec::new("para.khmerJustify", "Khmer Justify", "Home › Paragraph", |s, _| align(s, Align::KhmerJustify)).key("Mod+Alt+J"),
        CommandSpec::new("para.align", "Alignment", "Home › Paragraph", |s, v| {
            let a = match p::req_str(v, "value")? {
                "left" => Align::Left,
                "center" => Align::Center,
                "right" => Align::Right,
                "justify" | "both" => Align::Justify,
                "distribute" => Align::Distribute,
                "thaiDistribute" => Align::ThaiDistribute,
                "khmerJustify" => Align::KhmerJustify,
                // Logical edges, whatever the paragraph's direction.
                "start" => return fmt(s, &|p| p.align = Some(Align::Left)),
                "end" => return fmt(s, &|p| p.align = Some(Align::Right)),
                x => return Err(CmdError::Params(format!("unknown alignment `{x}`"))),
            };
            // An explicit value sets it (no toggling back to the start edge like the buttons).
            set_align(s, a)
        })
        .params(r#"{"value": "left|center|right|justify|distribute|thaiDistribute|khmerJustify (as seen on the page) | start|end (reading direction)"}"#),
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
                p.cancel_char_indents(true, false, true);
            })
        })
        .key("Mod+T"),
        CommandSpec::new("para.removeHanging", "Reduce Hanging Indent", "Home › Paragraph › Paragraph", |s, _| {
            fmt(s, &|p| {
                p.indent_left = Some((p.indent_left.unwrap_or(0.0) - 36.0).max(0.0));
                p.indent_first = Some(0.0);
                p.cancel_char_indents(true, false, true);
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
                p.cancel_char_indents(l.is_some(), r.is_some(), f.is_some());
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
            let (b_auto, a_auto) = (p::bool(v, "beforeAuto"), p::bool(v, "afterAuto"));
            // A number replaces automatic spacing, as typing one in Word's Paragraph dialog does.
            let rp = cur(s);
            let b_auto = b_auto.or((b.is_some() && rp.before_auto).then_some(false));
            let a_auto = a_auto.or((a.is_some() && rp.after_auto).then_some(false));
            fmt(s, &|p| {
                if let Some(x) = b {
                    p.space_before = Some(x.clamp(0.0, 1584.0));
                }
                if let Some(x) = a {
                    p.space_after = Some(x.clamp(0.0, 1584.0));
                }
                if let Some(on) = b_auto {
                    p.space_before_auto = Some(on);
                }
                if let Some(on) = a_auto {
                    p.space_after_auto = Some(on);
                }
            })
        })
        .params(r#"{"before"?: pt, "after"?: pt, "beforeAuto"?: bool, "afterAuto"?: bool (automatic, HTML-style spacing: 14 pt)}"#),
        CommandSpec::new("para.addSpaceBefore", "Add Space Before Paragraph", "Home › Paragraph › Line Spacing", |s, _| {
            let rp = cur(s);
            let has = rp.space_before > 0.0;
            fmt(s, &|p| {
                p.space_before = Some(if has { 0.0 } else { 12.0 });
                if rp.before_auto {
                    p.space_before_auto = Some(false);
                }
            })
        })
        .key("Mod+0"),
        CommandSpec::new("para.removeSpaceAfter", "Remove Space After Paragraph", "Home › Paragraph › Line Spacing", |s, _| {
            let rp = cur(s);
            let has = rp.space_after > 0.0;
            fmt(s, &|p| {
                p.space_after = Some(if has { 0.0 } else { 8.0 });
                if rp.after_auto {
                    p.space_after_auto = Some(false);
                }
            })
        }),
        // Back to the paragraph style: direct indents, spacing, alignment… go; the style, list
        // membership and drop cap stay. ⌘Q quits on macOS, so Word for Mac uses ⌃Q.
        CommandSpec::new("para.reset", "Reset Paragraph Formatting", "Home › Paragraph", |s, _| {
            fmt(s, &|p| {
                *p = ParaProps {
                    style: p.style.take(),
                    numbering: p.numbering.take(),
                    drop_cap: p.drop_cap.take(),
                    drop_cap_margin: p.drop_cap_margin.take(),
                    drop_cap_distance: p.drop_cap_distance.take(),
                    drop_cap_font: p.drop_cap_font.take(),
                    ..Default::default()
                }
            })
        })
        .key(super::mac_or("Ctrl+Q", "Mod+Q")),
        CommandSpec::new("para.style", "Apply Style", "Home › Styles", apply_style).params(r#"{"style": string (name or id)}"#),
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
        CommandSpec::new("para.moveUp", "Move Paragraph Up", "Home › Paragraph", |s, _| move_paras(s, true))
            .key(if cfg!(target_os = "macos") { "Ctrl+Shift+Up" } else { "Alt+Shift+Up" }),
        CommandSpec::new("para.moveDown", "Move Paragraph Down", "Home › Paragraph", |s, _| move_paras(s, false))
            .key(if cfg!(target_os = "macos") { "Ctrl+Shift+Down" } else { "Alt+Shift+Down" }),
        CommandSpec::new("para.promote", "Promote", "Home › Paragraph", |s, _| outline_step(s, true))
            .key(if cfg!(target_os = "macos") { "Ctrl+Shift+Left" } else { "Alt+Shift+Left" }),
        CommandSpec::new("para.demote", "Demote", "Home › Paragraph", |s, _| outline_step(s, false))
            .key(if cfg!(target_os = "macos") { "Ctrl+Shift+Right" } else { "Alt+Shift+Right" }),
        CommandSpec::new("para.shading", "Shading", "Home › Paragraph", |s, v| {
            let c = p::str(v, "color").and_then(Rgb::parse);
            fmt(s, &|p| p.shading = c)
        })
        .params(r#"{"color": "RRGGBB" | null}"#),
        CommandSpec::new("para.borders", "Borders", "Home › Paragraph", borders).params(
            r#"{"kind": "bottom|top|left|right|none|all|outside|inside|horizontalLine", "width"?: pt, "color"?: "RRGGBB", "style"?: "single|double|dotted|dashed|thick|triple|dotDash|wave", "sides"?: Sides} where Sides = {"top"|"left"|"bottom"|"right"|"between": {"style"?, "width"?: pt, "color"?: "RRGGBB"} | null}: exactly these sides (others off), instead of `kind`"#,
        ),
        CommandSpec::new("para.sort", "Sort", "Home › Paragraph", sort).params(r#"{"descending"?: bool}"#),
        CommandSpec::new("para.keepNext", "Keep with Next", "Home › Paragraph › Line and Page Breaks", |s, v| tog(s, v, |p| p.keep_next, |p, b| p.keep_next = Some(b)))
            .params(r#"{"value"?: bool}"#),
        CommandSpec::new("para.keepLines", "Keep Lines Together", "Home › Paragraph › Line and Page Breaks", |s, v| tog(s, v, |p| p.keep_lines, |p, b| p.keep_lines = Some(b)))
            .params(r#"{"value"?: bool}"#),
        CommandSpec::new("para.pageBreakBefore", "Page Break Before", "Home › Paragraph › Line and Page Breaks", |s, v| {
            tog(s, v, |p| p.page_break_before, |p, b| p.page_break_before = Some(b))
        })
        .params(r#"{"value"?: bool}"#),
        CommandSpec::new("para.widowControl", "Widow/Orphan Control", "Home › Paragraph › Line and Page Breaks", |s, v| {
            tog(s, v, |p| p.widow_control, |p, b| p.widow_control = Some(b))
        })
        .params(r#"{"value"?: bool}"#),
        CommandSpec::new("para.asianTypography", "Asian Typography", "Home › Paragraph › Asian Typography", asian_typography).params(
            r#"{"kinsoku"?: bool (East Asian line-breaking rules), "wordWrap"?: bool (false: Latin words may break at any character), "overflowPunct"?: bool (hanging punctuation), "topLinePunct"?: bool (compress punctuation at the start of a line), "autoSpaceDE"?: bool (space between Asian and Latin text), "autoSpaceDN"?: bool (space between Asian text and numbers)} — flags left out stay as they are; returns the flags at the caret"#,
        ),
        CommandSpec::new("para.tabs", "Tabs", "Home › Paragraph › Paragraph", tabs).params(
            r#"{"tabs"?: [TabStop] (replace the direct stops), "set"?: [TabStop], "clear"?: [pt], "clearAll"?: bool, "default"?: pt} where TabStop = {"pos": pt, "align"?: "left|center|right|decimal|bar", "leader"?: "none|dot|hyphen|underscore|middleDot"}"#,
        ),
        CommandSpec::new("para.outlineLevel", "Outline Level", "Home › Paragraph › Paragraph", |s, v| {
            let l = p::u64(v, "level").map(|x| x.min(9) as u8);
            fmt(s, &|p| p.outline_level = l)
        }),
        CommandSpec::new("para.set", "Set Paragraph Formatting", "Home › Paragraph › Paragraph", set_props).params(
            r#"{"props": ParaProps, "onPage"?: {"align"?: "left|center|right|justify|distribute|thaiDistribute|khmerJustify", "left"?: pt, "right"?: pt} (alignment and indents as seen on the page: each paragraph's own direction, after `props`, says which edge is its start)}"#,
        ),
        CommandSpec::new("para.dialog", "Paragraph Settings", "Home › Paragraph", |s, _| {
            s.push_ui_request(json!({"open": "paragraph"}));
            sel_result(s)
        })
        .pure(),
        CommandSpec::new("styles.create", "Create a Style", "Home › Styles", create_style).params(r#"{"name": string, "basedOn"?: string, "fromSelection"?: bool}"#),
        CommandSpec::new("styles.modify", "Modify Style", "Home › Styles", modify_style).params(r#"{"style": string, "chr"?: CharProps, "para"?: ParaProps, "name"?: string, "next"?: string}"#),
        CommandSpec::new("styles.updateToMatch", "Update Style to Match Selection", "Home › Styles", update_to_match).params(r#"{"style"?: string}"#),
        CommandSpec::new("styles.delete", "Delete Style", "Home › Styles", delete_style).params(r#"{"style": string (name or id; custom styles only)}"#),
        CommandSpec::new("styles.manage", "Manage Styles", "Home › Styles", |s, v| {
            let sort = p::str(v, "sort").unwrap_or("recommended");
            if !matches!(sort, "recommended" | "alphabetical") {
                return Err(CmdError::Params(format!("unknown sort `{sort}` (recommended|alphabetical)")));
            }
            let list = manage_list(s, sort == "alphabetical");
            s.push_ui_request(json!({"open": "manageStyles"}));
            Ok(list)
        })
        .params(r#"{"sort"?: "recommended|alphabetical"} → [{id, name, type: "paragraph|character|linked|table", builtIn, inGallery, hidden, basedOn, priority, format}]"#)
        .pure(),
        CommandSpec::new("styles.setVisibility", "Show or Hide Style", "Home › Styles", set_visibility)
            .params(r#"{"style": string, "gallery"?: bool (Styles gallery, w:qFormat), "hidden"?: bool (hidden from style lists, w:semiHidden)}"#),
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
        CommandSpec::new("styles.apply", "Apply Styles", "Home › Styles", apply_styles)
            .params(r#"{"style"?: string (name or id)}"#)
            // On macOS ⌘⇧S is Save As, as in Word for Mac.
            .key(super::non_mac("Mod+Shift+S"))
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
