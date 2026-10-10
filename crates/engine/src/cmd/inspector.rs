//! Style Inspector (Home › Styles): the paragraph style and the character style at the caret,
//! the direct formatting layered on each, and a way to clear each level.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use wordcraft_doc::props::{CharProps, ParaProps};

use super::sel_result;
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// The levels `styles.inspectorClear` can reset.
pub const LEVELS: [&str; 4] = ["paragraphStyle", "paragraphFormatting", "characterStyle", "characterFormatting"];

/// Direct character properties that aren't formatting (links, revisions, proofing, direction).
const CHAR_SKIP: [&str; 8] = ["style", "link", "ins", "del", "fmtChange", "lang", "noProof", "rtl"];
/// Direct paragraph properties the inspector doesn't list (lists have their own UI).
const PARA_SKIP: [&str; 4] = ["style", "numbering", "numChange", "fmtChange"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("styles.inspector", "Style Inspector", "Home › Styles", |s, v| {
            s.view.style_inspector = p::bool(v, "value").unwrap_or(!s.view.style_inspector);
            let mut out = inspect(s);
            if let Value::Object(m) = &mut out {
                m.insert("pane".into(), json!(s.view.style_inspector));
            }
            Ok(out)
        })
        .params(r#"{"value"?: bool (show/hide the pane)}"#)
        .pure(),
        CommandSpec::new("styles.inspect", "Inspect Styles at Selection", "Home › Styles › Style Inspector", |s, _| Ok(inspect(s))).pure(),
        CommandSpec::new("styles.inspectorClear", "Clear Formatting Level", "Home › Styles › Style Inspector", clear)
            .params(r#"{"level": "paragraphStyle|paragraphFormatting|characterStyle|characterFormatting"}"#),
    ]
}

/// The paragraph props at the caret (the selection start for a selection).
fn para_props(s: &Session) -> ParaProps {
    let (a, _) = s.sel.ordered();
    s.doc.para_at(&a).map(|p| p.props.clone()).unwrap_or_default()
}

/// The direct character props at the caret (the first selected character for a selection).
fn char_props(s: &Session) -> CharProps {
    let (a, b) = s.sel.ordered();
    if a == b {
        return s.typing_props();
    }
    match s.doc.para_at(&a) {
        Some(p) if a.off < p.len() => p.props_of_char(a.off).clone(),
        _ => s.typing_props(),
    }
}

/// Each set field of `direct` (minus `skip`) whose value changes the result of `resolve`, given
/// a props value holding only that field. Returns `[{"prop", "value"}]` in field order.
fn differences<T: Serialize + DeserializeOwned>(direct: &T, skip: &[&str], base: &Value, resolve: &dyn Fn(T) -> Value) -> Vec<Value> {
    let Ok(Value::Object(fields)) = serde_json::to_value(direct) else { return Vec::new() };
    let mut out = Vec::new();
    for (k, v) in fields {
        if skip.contains(&k.as_str()) || v.is_null() {
            continue;
        }
        let mut one = Map::new();
        one.insert(k.clone(), v.clone());
        let Ok(single) = serde_json::from_value::<T>(Value::Object(one)) else { continue };
        if resolve(single) != *base {
            out.push(json!({"prop": k, "value": v}));
        }
    }
    out
}

fn style_name(s: &Session, id: &str) -> String {
    s.doc.styles.get(id).map(|st| st.name.clone()).unwrap_or_else(|| id.to_string())
}

/// What the Style Inspector shows, as JSON.
pub fn inspect(s: &Session) -> Value {
    let pp = para_props(s);
    let pstyle = pp.style.clone().unwrap_or_else(|| "Normal".into());
    let styles = &s.doc.styles;
    let para_base = ParaProps { style: Some(pstyle.clone()), ..Default::default() };
    let pbase = serde_json::to_value(styles.resolve_para(&para_base)).unwrap_or(Value::Null);
    let pdiff = differences(&pp, &PARA_SKIP, &pbase, &|mut one: ParaProps| {
        one.style = Some(pstyle.clone());
        serde_json::to_value(styles.resolve_para(&one)).unwrap_or(Value::Null)
    });

    let cp = char_props(s);
    let cstyle = cp.style.clone();
    let char_base = CharProps { style: cstyle.clone(), ..Default::default() };
    let cbase = serde_json::to_value(styles.resolve_char(Some(&pstyle), &char_base)).unwrap_or(Value::Null);
    let cdiff = differences(&cp, &CHAR_SKIP, &cbase, &|mut one: CharProps| {
        one.style = cstyle.clone();
        serde_json::to_value(styles.resolve_char(Some(&pstyle), &one)).unwrap_or(Value::Null)
    });

    json!({
        "paragraph": {
            "style": pstyle,
            "styleName": style_name(s, &pstyle),
            "direct": pdiff,
        },
        "character": {
            "style": cstyle,
            "styleName": cstyle.as_deref().map(|c| style_name(s, c)),
            "direct": cdiff,
        },
    })
}

fn clear(s: &mut Session, v: &Value) -> CmdResult {
    let level = p::req_str(v, "level")?;
    let (a, b) = s.sel.ordered();
    match level {
        "paragraphStyle" => {
            // Reset to Normal, keeping direct formatting.
            s.doc.format_paragraphs(&a, &b, &|p| p.style = None)?;
        }
        "paragraphFormatting" => {
            s.doc.format_paragraphs(&a, &b, &|p| {
                *p = ParaProps { style: p.style.clone(), numbering: p.numbering, ..Default::default() };
            })?;
        }
        "characterStyle" => return super::format::apply(s, &|c| c.style = None),
        "characterFormatting" => {
            return super::format::apply(s, &|c| {
                *c = CharProps {
                    style: c.style.clone(),
                    link: c.link.clone(),
                    ins: c.ins,
                    del: c.del,
                    lang: c.lang.clone(),
                    no_proof: c.no_proof,
                    rtl: c.rtl,
                    ..Default::default()
                };
            });
        }
        other => return Err(CmdError::Params(format!("unknown level `{other}`; expected one of {}", LEVELS.join(", ")))),
    }
    sel_result(s)
}
