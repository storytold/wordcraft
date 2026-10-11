//! Insert › SmartArt and the SmartArt Design tab: insert a SmartArt graphic, edit its items (the
//! Text Pane, Add Shape, Promote, Demote), change its layout and colours, reset it.
//!
//! A SmartArt graphic is an inline [`InlineObject::Graphic`] whose graphic keeps its model
//! ([`SmartArtSpec`]); WordCraft lays it out and draws it (see `wordcraft_docx::smart_art_items`)
//! and saves it as diagram parts. SmartArt made by other programs is shown as it is; these
//! commands only edit graphics with a model. Each command is one undo step.

use std::sync::Arc;

use serde_json::{Value, json};
use wordcraft_doc::graphic::{Graphic, GraphicKind};
use wordcraft_doc::para::{Float, InlineObject};
use wordcraft_doc::props::Rgb;
use wordcraft_doc::smart_art::{MAX_ITEMS, MAX_LEVEL, SmartArtColors, SmartArtItem, SmartArtLayout, SmartArtSpec, clean_text};

use super::delete_selection;
use super::objects::selected;
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

/// A new graphic's size: 6 × 3.5 inches, points (narrower when the text column is).
pub const DEFAULT_W: f32 = 432.0;
pub const DEFAULT_H: f32 = 252.0;

const LAYOUTS: &str = "basicList|process|cycle|hierarchy|pyramid|radial|matrix|venn";
const COLORS: &str = "accent1|accent2|accent3|accent4|accent5|accent6|colorful";
const ITEMS: &str = r#"[string | {"text": string, "level"?: 0-7}] (a string's leading tabs are its level)"#;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("insert.smartArt", "SmartArt", "Insert › Illustrations", insert).params(
            r#"{"layout"?: "basicList|process|cycle|hierarchy|pyramid|radial|matrix|venn", "items"?: [string | {"text": string, "level"?: 0-7}], "colors"?: "accent1…accent6|colorful", "width"?: pt, "height"?: pt} (no items: sample items for the layout)"#,
        ),
        CommandSpec::new("smartArt.items", "Edit Items", "SmartArt Design › Create Graphic", |s, v| {
            let items = items_of(v.get("items").ok_or_else(|| CmdError::Params(format!("`items` is required: {ITEMS}")))?)?;
            edit(s, move |g| {
                g.items = items.clone();
                Ok(())
            })
        })
        .params(r#"{"items": [string | {"text": string, "level"?: 0-7}]} (replaces the outline)"#)
        .when(has_smart_art),
        CommandSpec::new("smartArt.textPane", "Text Pane", "SmartArt Design › Create Graphic", |s, v| {
            let on = p::bool(v, "value").unwrap_or(!s.view.smart_art_pane);
            s.view.smart_art_pane = on;
            Ok(json!({"value": on}))
        })
        .params(r#"{"value"?: bool} (no value: show or hide the Text Pane)"#)
        .pure(),
        CommandSpec::new("smartArt.addShape", "Add Shape", "SmartArt Design › Create Graphic", |s, v| {
            let after = index(v, "after")?;
            let text = p::str(v, "text").map(clean_text).unwrap_or_default();
            let below = p::bool(v, "below").unwrap_or(false);
            edit(s, move |g| {
                if g.items.len() >= MAX_ITEMS {
                    return Err(CmdError::Failed(format!("a SmartArt graphic holds at most {MAX_ITEMS} items")));
                }
                let Some(last) = g.items.len().checked_sub(1) else {
                    g.items.push(SmartArtItem::new(text.clone(), 0));
                    return Ok(());
                };
                let at = after.unwrap_or(last).min(last);
                let level = g.items.get(at).map_or(0, |it| it.level);
                if below {
                    // Under the item: its first sub-item.
                    g.items.insert(at + 1, SmartArtItem::new(text.clone(), level.saturating_add(1).min(MAX_LEVEL)));
                } else {
                    // After the item and everything under it, at its level.
                    let end = g.descendants(at).end;
                    g.items.insert(end, SmartArtItem::new(text.clone(), level));
                }
                Ok(())
            })
        })
        .params(r#"{"after"?: item index (default: the last), "text"?: string, "below"?: bool (a sub-item instead of the next item)}"#)
        .when(has_smart_art),
        CommandSpec::new("smartArt.promote", "Promote", "SmartArt Design › Create Graphic", |s, v| {
            let i = index(v, "index")?;
            edit(s, move |g| {
                let i = item_at(g, i)?;
                if g.items.get(i).is_none_or(|it| it.level == 0) {
                    return Err(CmdError::Failed("this item is already at the top level".into()));
                }
                for j in std::iter::once(i).chain(g.descendants(i)) {
                    if let Some(it) = g.items.get_mut(j) {
                        it.level = it.level.saturating_sub(1);
                    }
                }
                Ok(())
            })
        })
        .params(r#"{"index"?: item index (default: the last)} (the item and the items under it move up a level)"#)
        .when(has_smart_art),
        CommandSpec::new("smartArt.demote", "Demote", "SmartArt Design › Create Graphic", |s, v| {
            let i = index(v, "index")?;
            edit(s, move |g| {
                let i = item_at(g, i)?;
                let prev = i.checked_sub(1).and_then(|p| g.items.get(p)).map(|it| it.level);
                let level = g.items.get(i).map_or(0, |it| it.level);
                let deepest = std::iter::once(i).chain(g.descendants(i)).filter_map(|j| g.items.get(j)).map(|it| it.level).max().unwrap_or(0);
                if prev.is_none_or(|p| p < level) || deepest >= MAX_LEVEL {
                    return Err(CmdError::Failed("this item can't go down a level (it needs an item before it at its level)".into()));
                }
                for j in std::iter::once(i).chain(g.descendants(i)) {
                    if let Some(it) = g.items.get_mut(j) {
                        it.level = it.level.saturating_add(1);
                    }
                }
                Ok(())
            })
        })
        .params(r#"{"index"?: item index (default: the last)} (the item and the items under it move down a level)"#)
        .when(has_smart_art),
        CommandSpec::new("smartArt.layout", "Change Layout", "SmartArt Design › Layouts", |s, v| {
            let layout = layout_of(v)?.ok_or_else(|| CmdError::Params(format!("`layout` is required: {LAYOUTS}")))?;
            edit(s, move |g| {
                g.layout = layout;
                Ok(())
            })
        })
        .params(r#"{"layout": "basicList|process|cycle|hierarchy|pyramid|radial|matrix|venn"}"#)
        .when(has_smart_art),
        CommandSpec::new("smartArt.colors", "Change Colors", "SmartArt Design › SmartArt Styles", |s, v| {
            let colors = colors_of(v)?.ok_or_else(|| CmdError::Params(format!("`variant` is required: {COLORS}")))?;
            edit(s, move |g| {
                g.colors = colors;
                Ok(())
            })
        })
        .params(r#"{"variant": "accent1|accent2|accent3|accent4|accent5|accent6|colorful"}"#)
        .when(has_smart_art),
        CommandSpec::new("smartArt.reset", "Reset Graphic", "SmartArt Design › Reset", |s, _| {
            edit(s, |g| {
                g.colors = SmartArtColors::default();
                Ok(())
            })
        })
        .params("{} (back to the default colours; the items and layout stay)")
        .when(has_smart_art),
    ]
}

/// The selected SmartArt graphic's model, when it has one.
pub fn selected_smart_art(s: &Session) -> Option<Arc<SmartArtSpec>> {
    match selected(s) {
        Some((_, InlineObject::Graphic { graphic, .. })) => graphic.smart_art.clone(),
        _ => None,
    }
}

fn has_smart_art(s: &Session) -> Option<&'static str> {
    match selected(s) {
        Some((_, InlineObject::Graphic { graphic, .. })) if graphic.smart_art.is_some() => None,
        Some((_, InlineObject::Graphic { graphic, .. })) if graphic.kind == GraphicKind::Diagram => {
            Some("this SmartArt was made in another program: WordCraft shows it but can't edit it")
        }
        _ => Some("select a SmartArt graphic first"),
    }
}

/// The graphic for `spec` laid out `w` × `h` points in the theme's colours.
pub fn smart_art_graphic(spec: SmartArtSpec, theme: &[Rgb], w: f32, h: f32) -> Graphic {
    let spec = spec.sanitized();
    let items = wordcraft_docx::smart_art_items(&spec, theme, w, h);
    Graphic { kind: GraphicKind::Diagram, items, w, h, source: None, smart_art: Some(Arc::new(spec)) }
}

/// Lay the SmartArt graphic of `o` out again at its current size (after a resize), if it has a model.
pub fn redraw(o: &mut InlineObject, theme: &[Rgb]) {
    if let InlineObject::Graphic { w, h, graphic, .. } = o
        && let Some(spec) = graphic.smart_art.as_deref()
    {
        *graphic = Arc::new(smart_art_graphic(spec.clone(), theme, *w, *h));
    }
}

/// `layout` of the params, if given.
fn layout_of(v: &Value) -> Result<Option<SmartArtLayout>, CmdError> {
    match v.get("layout") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(id)) => {
            SmartArtLayout::from_id(id).map(Some).ok_or_else(|| CmdError::Params(format!("unknown layout `{}`: {LAYOUTS}", clean_text(id))))
        }
        Some(_) => Err(CmdError::Params(format!("`layout`: {LAYOUTS}"))),
    }
}

/// `variant` (or `colors`) of the params, if given.
fn colors_of(v: &Value) -> Result<Option<SmartArtColors>, CmdError> {
    match v.get("variant").or_else(|| v.get("colors")) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(id)) => {
            SmartArtColors::from_id(id).map(Some).ok_or_else(|| CmdError::Params(format!("unknown colours `{}`: {COLORS}", clean_text(id))))
        }
        Some(_) => Err(CmdError::Params(format!("`variant`: {COLORS}"))),
    }
}

/// An item index param, if given.
fn index(v: &Value, k: &str) -> Result<Option<usize>, CmdError> {
    match v.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => p::u64(v, k)
            .map(|i| Some(usize::try_from(i).unwrap_or(usize::MAX)))
            .ok_or_else(|| CmdError::Params(format!("`{k}`: an item index (0 is the first)"))),
    }
}

/// Item `i` of `g` (default: the last).
fn item_at(g: &SmartArtSpec, i: Option<usize>) -> Result<usize, CmdError> {
    let last = g.items.len().checked_sub(1).ok_or_else(|| CmdError::Failed("the graphic has no items".into()))?;
    match i {
        None => Ok(last),
        Some(i) if i <= last => Ok(i),
        Some(i) => Err(CmdError::Params(format!("no item {i}: the graphic has {} items", last + 1))),
    }
}

/// Items from JSON (at most [`MAX_ITEMS`]): strings (leading tabs give the level) or
/// `{"text", "level"}` objects.
fn items_of(v: &Value) -> Result<Vec<SmartArtItem>, CmdError> {
    let a = v.as_array().ok_or_else(|| CmdError::Params(format!("`items`: {ITEMS}")))?;
    let items = a
        .iter()
        .take(MAX_ITEMS)
        .map(|it| match it {
            Value::String(t) => {
                let tabs = t.chars().take_while(|c| *c == '\t').count();
                Ok(SmartArtItem::new(clean_text(t), u8::try_from(tabs).unwrap_or(MAX_LEVEL).min(MAX_LEVEL)))
            }
            Value::Object(_) => {
                let text = match it.get("text") {
                    Some(Value::String(t)) => clean_text(t),
                    None | Some(Value::Null) => String::new(),
                    Some(other) => clean_text(&other.to_string()),
                };
                let level = p::u64(it, "level").unwrap_or(0).min(u64::from(MAX_LEVEL));
                Ok(SmartArtItem::new(text, u8::try_from(level).unwrap_or(MAX_LEVEL)))
            }
            _ => Err(CmdError::Params(format!("`items`: {ITEMS}"))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if items.is_empty() {
        return Err(CmdError::Params("`items` needs at least one item".into()));
    }
    Ok(items)
}

fn insert(s: &mut Session, v: &Value) -> CmdResult {
    let layout = layout_of(v)?.unwrap_or_default();
    let mut spec = SmartArtSpec::sample(layout);
    if let Some(items) = v.get("items").filter(|i| !i.is_null()) {
        spec.items = items_of(items)?;
    }
    if let Some(c) = colors_of(v)? {
        spec.colors = c;
    }
    // Fitted to the text width, as a new graphic is.
    let max_w = s.doc.sections().first().map(|(_, sp)| sp.text_width()).unwrap_or(468.0).max(36.0);
    let w = p::f32(v, "width").unwrap_or(DEFAULT_W).clamp(36.0, 2000.0).min(max_w);
    let h = p::f32(v, "height").unwrap_or(DEFAULT_H).clamp(36.0, 2000.0);
    let graphic = smart_art_graphic(spec, &s.doc.settings.theme_colors, w, h);
    let out = json!({"smartArt": graphic.smart_art.as_deref(), "width": w, "height": h});
    let props = s.typing_props();
    let at = delete_selection(s)?;
    let obj = InlineObject::Graphic { w, h, alt: String::new(), float: Float::default(), graphic: Arc::new(graphic) };
    let end = s.doc.insert_object(&at, obj, &props)?;
    s.sel = Selection { anchor: at, focus: end };
    Ok(out)
}

/// Change the selected graphic's model with `f`, then lay it out again at its size.
fn edit(s: &mut Session, f: impl Fn(&mut SmartArtSpec) -> Result<(), CmdError>) -> CmdResult {
    let (pos, obj) = selected(s).ok_or_else(|| CmdError::Disabled("select a SmartArt graphic first".into()))?;
    let InlineObject::Graphic { w, h, graphic, .. } = obj else { return Err(CmdError::Disabled("select a SmartArt graphic first".into())) };
    let mut spec = graphic.smart_art.as_deref().cloned().ok_or_else(|| CmdError::Disabled("this SmartArt graphic can't be edited".into()))?;
    f(&mut spec)?;
    let next = Arc::new(smart_art_graphic(spec, &s.doc.settings.theme_colors, w, h));
    let out = json!({"smartArt": next.smart_art.as_deref()});
    let para = s.doc.para_mut(pos.story, &pos.path)?;
    if let Some(InlineObject::Graphic { graphic, .. }) = para.object_at_mut(pos.off) {
        *graphic = next;
    }
    para.touch();
    Ok(out)
}
