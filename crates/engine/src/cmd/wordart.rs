//! WordArt: Insert › WordArt, Home › Font › Text Effects, and Shape Format › WordArt Styles (our
//! own styles in theme colours, text fill, text outline, Transform).
//!
//! Text effects are run properties ([`wordcraft_doc::wordart::TextEffects`]); a Transform is the
//! text warp of a shape ([`wordcraft_doc::wordart::TextWarp`]). With a shape selected, or the
//! caret in a text box, the effects apply to the whole box's text, as WordArt is formatted.

use serde_json::{Value, json};
use wordcraft_doc::effects::{Glow, Shadow};
use wordcraft_doc::para::{Anchor, Float, FloatAlign, InlineObject, OBJ, ShapeKind, Wrap};
use wordcraft_doc::props::{CharProps, Rgb};
use wordcraft_doc::wordart::{GradStop, Reflection, TextEffects, TextFill, TextOutline, TextWarp, art_style};
use wordcraft_doc::{Paragraph, PartKind, Pos, StoryRef, para_block};

use super::sel_result;
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

const EFFECT_PARAMS: &str = r#"{"style"?: "solid"|"outlined"|"hollow"|"gradient"|"shadowed"|"glowing"|"reflected"|"sunset", "clear"?: bool, "fill"?: "RRGGBB"|"none" (hollow)|{"color": "RRGGBB", "transparency"?: %}|{"gradient": ["RRGGBB", …], "angle"?: deg}|null (the run's colour), "outline"?: "RRGGBB"|"none"|{"color"?: "RRGGBB"|null, "width"?: pt, "transparency"?: %}|null, "shadow"?: preset|{"color"?, "transparency"?, "blur"?, "distance"?, "angle"?}|null, "glow"?: pt|{"color"?, "size"?, "transparency"?}|null, "reflection"?: bool|{"transparency"?: %, "size"?: %, "distance"?: pt}|null}  (an omitted key is left as it is; with a shape selected or the caret in a text box, the box's whole text)"#;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("insert.wordArt", "WordArt", "Insert › Text", insert).params(
            r#"{"text"?: string, "style"?: WordArt style id (see format.textEffects), "transform"?: "textArchUp"|"textArchDown"|"textCircle"|"textWave1"|… (a DrawingML preset text warp)}"#,
        ),
        CommandSpec::new("format.textEffects", "Text Effects", "Home › Font", text_effects).params(EFFECT_PARAMS),
        CommandSpec::new("wordArt.style", "WordArt Styles", "Shape Format › WordArt Styles", |s, v| {
            let id = p::req_str(v, "style")?;
            text_effects(s, &json!({"style": id}))
        })
        .params(r#"{"style": "solid"|"outlined"|"hollow"|"gradient"|"shadowed"|"glowing"|"reflected"|"sunset"}"#),
        CommandSpec::new("wordArt.textFill", "Text Fill", "Shape Format › WordArt Styles", |s, v| {
            text_effects(s, &json!({"fill": v.get("fill").or_else(|| v.get("color")).cloned().unwrap_or(Value::Null)}))
        })
        .params(r#"{"fill": "RRGGBB"|"none"|{"gradient": ["RRGGBB", …], "angle"?: deg}|null}"#),
        CommandSpec::new("wordArt.textOutline", "Text Outline", "Shape Format › WordArt Styles", |s, v| {
            text_effects(s, &json!({"outline": v.get("outline").cloned().unwrap_or_else(|| v.clone())}))
        })
        .params(r#"{"color"?: "RRGGBB"|null, "width"?: pt} or {"outline": "none"|null}"#),
        CommandSpec::new("wordArt.transform", "Transform", "Shape Format › WordArt Styles", transform)
            .params(r#"{"preset": "textArchUp"|"textArchDown"|"textCircle"|"textWave1"|"textSlantUp"|"textSlantDown"|"textInflate"|"textDeflate"|"textChevron"|"textChevronInverted"|"textTriangle"|"textTriangleInverted"|… (any DrawingML preset text warp)|"none", "adj"?: n (the preset's adjust value)}"#),
    ]
}

/// The theme's colours (dk1, lt1, dk2, lt2, accent1…6, …).
fn theme(s: &Session) -> Vec<Rgb> {
    s.doc.settings.theme_colors.clone()
}

/// The text box story the selection formats as a whole: the selected shape's, or the one the
/// caret is in.
fn box_story(s: &Session) -> Option<u32> {
    if let Some((_, InlineObject::Shape { story: Some(id), .. })) = super::objects::object_selection(s) {
        return Some(*id);
    }
    match s.sel.focus.story {
        StoryRef::Part(id) if s.sel.is_collapsed() && s.doc.parts.get(&id).is_some_and(|p| p.kind == PartKind::TextBox) => Some(id),
        _ => None,
    }
}

/// Apply `f` to the selection's characters (a text box's whole text: see [`box_story`]).
fn apply(s: &mut Session, f: &dyn Fn(&mut CharProps)) -> CmdResult {
    if let Some(id) = box_story(s) {
        let (a, b) = (s.doc.start_of(StoryRef::Part(id)), s.doc.end_of(StoryRef::Part(id)));
        s.doc.format_range(&a, &b, f)?;
        return sel_result(s);
    }
    super::format::apply(s, f)
}

fn color(v: &Value, k: &str) -> Option<Rgb> {
    p::str(v, k).and_then(Rgb::parse)
}

/// A change to one effect: `None` leaves it, `Some(None)` removes it.
type Change<T> = Option<Option<T>>;

fn parse_fill(v: Option<&Value>) -> Result<Change<TextFill>, CmdError> {
    let bad = || CmdError::Params("`fill`: a colour, \"none\", an object or null".into());
    Ok(match v {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(x)) if x == "none" => Some(Some(TextFill::None)),
        Some(Value::String(x)) => Some(Some(TextFill::Solid { color: Rgb::parse(x).ok_or_else(bad)?, transparency: 0.0 })),
        Some(o @ Value::Object(_)) => match o.get("gradient").and_then(Value::as_array) {
            Some(cols) => {
                let cols: Vec<Rgb> = cols.iter().filter_map(Value::as_str).filter_map(Rgb::parse).take(wordcraft_doc::wordart::MAX_STOPS).collect();
                let n = cols.len().max(2) as f32 - 1.0;
                let stops = cols.iter().enumerate().map(|(i, c)| GradStop { pos: 100.0 * i as f32 / n, color: *c, transparency: 0.0 }).collect();
                Some(TextFill::Gradient { stops, angle: p::f32(o, "angle").unwrap_or(90.0) }.sanitized())
            }
            None => Some(Some(TextFill::Solid { color: color(o, "color").ok_or_else(bad)?, transparency: p::f32(o, "transparency").unwrap_or(0.0) })),
        },
        Some(_) => return Err(bad()),
    })
}

fn parse_outline(v: Option<&Value>) -> Result<Change<TextOutline>, CmdError> {
    let d = TextOutline::default();
    Ok(match v {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(x)) if x == "none" => Some(Some(TextOutline { color: None, ..d })),
        Some(Value::String(x)) => {
            Some(Some(TextOutline { color: Some(Rgb::parse(x).ok_or_else(|| CmdError::Params("`outline`: a colour".into()))?), ..d }))
        }
        Some(o @ Value::Object(_)) => Some(Some(TextOutline {
            color: match o.get("color") {
                Some(Value::Null) => None,
                Some(_) => color(o, "color").or(d.color),
                None => d.color,
            },
            width: p::f32(o, "width").unwrap_or(d.width),
            transparency: p::f32(o, "transparency").unwrap_or(0.0),
        })),
        Some(_) => return Err(CmdError::Params("`outline`: a colour, \"none\", an object or null".into())),
    })
}

fn parse_shadow(v: Option<&Value>) -> Result<Change<Shadow>, CmdError> {
    let preset = |id: &str| Shadow::preset(id).ok_or_else(|| CmdError::Params(format!("unknown shadow preset `{id}`")));
    Ok(match v {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::Bool(b)) => Some(b.then(Shadow::default)),
        Some(Value::String(id)) if id == "none" => Some(None),
        Some(Value::String(id)) => Some(Some(preset(id)?)),
        Some(o @ Value::Object(_)) => {
            let mut sh = match p::str(o, "preset") {
                Some(id) => preset(id)?,
                None => Shadow::default(),
            };
            sh.color = color(o, "color").unwrap_or(sh.color);
            sh.transparency = p::f32(o, "transparency").unwrap_or(sh.transparency);
            sh.blur = p::f32(o, "blur").unwrap_or(sh.blur);
            sh.distance = p::f32(o, "distance").unwrap_or(sh.distance);
            sh.angle = p::f32(o, "angle").unwrap_or(sh.angle);
            Some(Some(sh.sanitized()))
        }
        Some(_) => return Err(CmdError::Params("`shadow`: a preset name, an object or null".into())),
    })
}

fn parse_glow(v: Option<&Value>, accent: Rgb) -> Result<Change<Glow>, CmdError> {
    let d = Glow { color: accent, ..Glow::default() };
    Ok(match v {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(x)) if x == "none" => Some(None),
        Some(n @ Value::Number(_)) => Some(Some(Glow { size: n.as_f64().unwrap_or(0.0) as f32, ..d }.sanitized()).filter(|g| g.size > 0.0)),
        Some(o @ Value::Object(_)) => Some(
            Some(
                Glow {
                    color: color(o, "color").unwrap_or(d.color),
                    size: p::f32(o, "size").unwrap_or(d.size),
                    transparency: p::f32(o, "transparency").unwrap_or(d.transparency),
                }
                .sanitized(),
            )
            .filter(|g| g.size > 0.0),
        ),
        Some(_) => return Err(CmdError::Params("`glow`: a size in points, an object or null".into())),
    })
}

fn parse_reflection(v: Option<&Value>) -> Result<Change<Reflection>, CmdError> {
    let d = Reflection::default();
    Ok(match v {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::Bool(b)) => Some(b.then_some(d)),
        Some(o @ Value::Object(_)) => Some(Some(Reflection {
            transparency: p::f32(o, "transparency").unwrap_or(d.transparency),
            size: p::f32(o, "size").unwrap_or(d.size),
            distance: p::f32(o, "distance").unwrap_or(d.distance),
            blur: p::f32(o, "blur").unwrap_or(d.blur),
        })),
        Some(_) => return Err(CmdError::Params("`reflection`: true, an object or null".into())),
    })
}

/// `format.textEffects`: set or clear the WordArt effects of the selection.
fn text_effects(s: &mut Session, v: &Value) -> CmdResult {
    let theme = theme(s);
    let accent = theme.get(4).copied().unwrap_or(Rgb(0x15, 0x60, 0x82));
    let style = match p::str(v, "style") {
        Some(id) => Some(art_style(id, &theme).ok_or_else(|| CmdError::Params(format!("unknown WordArt style `{id}`")))?),
        None => None,
    };
    let clear = p::bool(v, "clear").unwrap_or(false);
    let fill = parse_fill(v.get("fill"))?;
    let outline = parse_outline(v.get("outline"))?;
    let shadow = parse_shadow(v.get("shadow"))?;
    let glow = parse_glow(v.get("glow"), accent)?;
    let reflection = parse_reflection(v.get("reflection"))?;
    let f = |c: &mut CharProps| {
        let mut fx = if clear { TextEffects::default() } else { c.text_effects.take().map(|b| *b).unwrap_or_default() };
        if let Some(st) = &style {
            fx = st.clone();
        }
        if let Some(x) = &fill {
            fx.fill.clone_from(x);
        }
        if let Some(x) = outline {
            fx.outline = x;
        }
        if let Some(x) = shadow {
            fx.shadow = x;
        }
        if let Some(x) = glow {
            fx.glow = x;
        }
        if let Some(x) = reflection {
            fx.reflection = x;
        }
        let fx = fx.sanitized();
        c.text_effects = (!fx.is_empty()).then(|| Box::new(fx));
    };
    apply(s, &f)
}

/// The shape a Transform applies to: the selected one, or the one whose text box the caret is in.
fn warp_target(s: &Session) -> Option<Pos> {
    if let Some((pos, InlineObject::Shape { .. })) = super::objects::object_selection(s) {
        return Some(pos);
    }
    let StoryRef::Part(id) = s.sel.focus.story else { return None };
    let at = s.doc.text_box_anchor(id)?;
    Some(Pos { off: at.off.checked_sub(OBJ.len_utf8())?, ..at })
}

/// `wordArt.transform`: bend the shape's text along a preset warp, or straighten it.
fn transform(s: &mut Session, v: &Value) -> CmdResult {
    let preset = p::req_str(v, "preset")?;
    let warp = match preset {
        "none" => None,
        id => {
            let mut w = TextWarp::new(id).ok_or_else(|| CmdError::Params(format!("unknown text warp `{id}`")))?;
            if let Some(a) = v.get("adj").and_then(Value::as_f64).filter(|a| a.is_finite()) {
                w.set_adj("adj", a.clamp(-1e12, 1e12) as i64);
            }
            Some(w)
        }
    };
    let pos = warp_target(s).ok_or_else(|| CmdError::Disabled("select a shape or WordArt first".into()))?;
    let para = s.doc.para_mut(pos.story, &pos.path)?;
    match para.object_at_mut(pos.off) {
        Some(InlineObject::Shape { extra, .. }) => extra.warp = warp.clone(),
        _ => return Err(CmdError::Disabled("select a shape or WordArt first".into())),
    }
    para.touch();
    Ok(json!({"transform": warp.map(|w| w.preset)}))
}

/// How wide `text` is at `size` points in the document's body font, points.
fn text_width(s: &Session, text: &str, size: f32, bold: bool) -> f32 {
    let family = s.doc.styles.default_chr.font.clone().unwrap_or_else(|| s.doc.settings.minor_font.clone());
    let face = wordcraft_fonts::word::resolve(&family, bold, false).face;
    let k = size / face.upem.max(1.0) as f32;
    wordcraft_fonts::shape(&face, text, &[], |c| c).iter().map(|g| g.x_advance as f32 * k).sum::<f32>()
}

/// `insert.wordArt`: a floating text box without fill or line, its text in a WordArt style (and
/// optionally bent), centred on the column at the caret's paragraph; its text is selected so
/// typing replaces it.
fn insert(s: &mut Session, v: &Value) -> CmdResult {
    let text: String = p::str(v, "text").unwrap_or("Your text here").chars().filter(|c| !c.is_control()).take(1000).collect();
    let text = if text.trim().is_empty() { "Your text here".to_string() } else { text };
    let style = p::str(v, "style").unwrap_or("solid");
    let fx = art_style(style, &theme(s)).ok_or_else(|| CmdError::Params(format!("unknown WordArt style `{style}`")))?;
    let warp = match p::str(v, "transform") {
        None | Some("none") => None,
        Some(id) => Some(TextWarp::new(id).ok_or_else(|| CmdError::Params(format!("unknown text warp `{id}`")))?),
    };
    let size = 36.0;
    let props = CharProps { size: Some(size), bold: Some(true), text_effects: Some(Box::new(fx)), ..Default::default() };
    // A box the text fits on one line in (with the box's margins); a bent one is taller.
    let col = s.doc.sections().first().map(|(_, sp)| sp.text_width()).unwrap_or(468.0);
    let w = (text_width(s, &text, size, true) + 24.0).clamp(72.0, col.max(72.0));
    let mut h = size * 1.35 + 7.2;
    if let Some(wp) = &warp {
        h = match wp.preset.as_str() {
            "textCircle" => w,
            "textArchUp" | "textArchDown" => w * 0.5,
            _ => h * 1.6,
        };
    }
    let id = s.doc.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text(&text, props))]);
    if let Some(p) = s.doc.parts.get_mut(&id)
        && let Some(b) = p.blocks.first_mut()
        && let wordcraft_doc::Block::Para(para) = std::sync::Arc::make_mut(b)
    {
        para.props.align = Some(wordcraft_doc::props::Align::Center);
    }
    let obj = InlineObject::Shape {
        kind: ShapeKind::TextBox,
        w,
        h: h.clamp(18.0, 2000.0),
        fill: None,
        stroke: None,
        stroke_width: 0.0,
        float: Float {
            wrap: Wrap::InFrontOfText,
            h_rel: Anchor::Margin,
            h_align: Some(FloatAlign::Center),
            v_rel: Anchor::Paragraph,
            ..Default::default()
        },
        story: Some(id),
        freeform: None,
        effects: Default::default(),
        extra: wordcraft_doc::connector::ShapeExtra { warp, ..Default::default() },
    };
    let typing = s.typing_props();
    let at = super::delete_selection(s)?;
    s.doc.insert_object(&at, obj, &typing)?;
    s.sel = Selection { anchor: s.doc.start_of(StoryRef::Part(id)), focus: s.doc.end_of(StoryRef::Part(id)) };
    Ok(json!({"story": id}))
}
