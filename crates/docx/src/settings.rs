//! `adjustLineHeightInTable` is not in the pinned shared DOCX model yet. Its writer preserves the
//! original settings XML, so this adapter reads the flag there and updates it before a save.

use std::{collections::BTreeMap, sync::Arc};

use craft_docx::DocxError;

const SETTINGS_ORIG: &str = "wordcraft:settings.orig";
const MINIMAL_SETTINGS: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
    "<w:settings xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">",
    "<w:compat><w:adjustLineHeightInTable/></w:compat></w:settings>"
);
const ADJUST: &str = "w:adjustLineHeightInTable";
const AFTER: [&str; 17] = [
    "w:autoSpaceLikeWord95",
    "w:noSpaceRaiseLower",
    "w:doNotUseHTMLParagraphAutoSpacing",
    "w:layoutRawTableWidth",
    "w:layoutTableRowsApart",
    "w:useWord97LineBreakRules",
    "w:doNotBreakWrappedTables",
    "w:doNotSnapToGridInCell",
    "w:selectFldWithFirstOrLastChar",
    "w:applyBreakingRules",
    "w:doNotWrapTextWithPunct",
    "w:doNotUseEastAsianBreakRules",
    "w:useWord2002TableStyleRules",
    "w:growAutofit",
    "w:useFELayout",
    "w:allowSpaceOfSameStyleInTable",
    "w:compatSetting",
];

/// `w:adjustLineHeightInTable` defaults off when it isn't present.
pub(crate) fn read_adjust_line_height_in_table(passthrough: &BTreeMap<String, Arc<Vec<u8>>>) -> bool {
    let Some(bytes) = passthrough.get(SETTINGS_ORIG) else { return false };
    let Ok(root) = craft_xml::parse(bytes) else { return false };
    if root.local() != "settings" {
        return false;
    }
    root.child("w:compat").is_some_and(|compat| compat.children(ADJUST).any(|element| !is_switched_off(element.attr("w:val"))))
}

/// Make the preserved settings match the WordCraft model. The shared DOCX writer merges this
/// original part into its generated settings and already treats the switch as an unowned option.
pub(crate) fn write_adjust_line_height_in_table(passthrough: &mut BTreeMap<String, Arc<Vec<u8>>>, enabled: bool) -> Result<(), DocxError> {
    let had_original = passthrough.contains_key(SETTINGS_ORIG);
    let original = passthrough.get(SETTINGS_ORIG).map(|bytes| bytes.as_slice());
    if !had_original && !enabled {
        return Ok(());
    }
    if had_original && read_adjust_line_height_in_table(passthrough) == enabled {
        return Ok(());
    }
    let original = original.unwrap_or(MINIMAL_SETTINGS.as_bytes());
    let xml = std::str::from_utf8(original).map_err(|_| DocxError::Xml("settings.xml is not UTF-8".into()))?;
    // The parser validates the XML before the byte-preserving scanner below edits it.
    let root = craft_xml::parse(original).map_err(|_| DocxError::Xml("settings.xml is malformed".into()))?;
    if root.local() != "settings" {
        return Err(DocxError::Xml("settings.xml has an unexpected root".into()));
    }
    let updated = sync_adjust_line_height(xml, enabled).ok_or_else(|| DocxError::Xml("could not update adjustLineHeightInTable".into()))?;
    if !had_original || updated.as_bytes() != original {
        passthrough.insert(SETTINGS_ORIG.into(), Arc::new(updated.into_bytes()));
    }
    Ok(())
}

/// Is an ST_OnOff value false? An absent value means true.
fn is_switched_off(value: Option<&str>) -> bool {
    value.is_some_and(|v| matches!(v.trim(), "0" | "false" | "off" | "False" | "FALSE" | "Off" | "OFF"))
}

/// Keep the original bytes except for the owned switch, inserting it in CT_Compat order.
fn sync_adjust_line_height(xml: &str, enabled: bool) -> Option<String> {
    let (_, close, settings_children) = children(xml, "w:settings")?;
    let Some(compat) = settings_children.iter().find(|child| child.name == "w:compat") else {
        return if enabled {
            Some(format!("{}<w:compat><w:adjustLineHeightInTable/></w:compat>{}", xml.get(..close)?, xml.get(close..)?))
        } else {
            Some(xml.to_string())
        };
    };
    let original_compat = xml.get(compat.start..compat.end)?;
    let updated_compat = sync_compat(original_compat, enabled)?;
    if updated_compat == original_compat {
        return Some(xml.to_string());
    }
    Some(format!("{}{}{}", xml.get(..compat.start)?, updated_compat, xml.get(compat.end..)?))
}

fn sync_compat(compat: &str, enabled: bool) -> Option<String> {
    let open_end = tag_end(compat)? + 1;
    if compat.get(open_end - 2..open_end)? == "/>" {
        if !enabled {
            return Some(compat.to_string());
        }
        let open = compat.get(..open_end - 2)?.trim_end();
        return Some(format!("{open}><w:adjustLineHeightInTable/></w:compat>"));
    }
    let (_, _, compat_children) = children(compat, "w:compat")?;
    let matches: Vec<&Child> = compat_children.iter().filter(|child| child.name == ADJUST).collect();
    let mut out = compat.to_string();
    for child in matches.iter().rev() {
        out.replace_range(child.start..child.end, "");
    }
    if !enabled {
        return Some(out);
    }
    let (_, close, children) = children(&out, "w:compat")?;
    let at = AFTER.iter().filter_map(|name| children.iter().find(|child| child.name == *name).map(|child| child.start)).min().unwrap_or(close);
    out.insert_str(at, "<w:adjustLineHeightInTable/>");
    Some(out)
}

/// Direct children of `root`, with byte spans. This keeps comments, attributes and unknown options
/// byte-for-byte when a single switch changes.
fn children(xml: &str, root: &str) -> Option<(usize, usize, Vec<Child>)> {
    let open = xml.find(&format!("<{root}"))?;
    let open_end = open + tag_end(xml.get(open..)?)? + 1;
    if xml.get(open_end - 2..open_end)? == "/>" {
        return None;
    }
    let close = xml.rfind(&format!("</{root}>"))?;
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut i = open_end;
    let mut current: Option<(String, usize)> = None;
    while i < close {
        let rest = xml.get(i..close)?;
        let Some(next_tag) = rest.find('<') else {
            if rest.chars().all(|c| matches!(c, ' ' | '\t' | '\r' | '\n')) {
                break;
            }
            return None;
        };
        let at = i + next_tag;
        let tail = xml.get(at..close)?;
        if tail.starts_with("<!--") {
            i = at + tail.find("-->")? + 3;
            continue;
        }
        if tail.starts_with("<![CDATA[") || tail.starts_with("<!") {
            return None;
        }
        if tail.starts_with("<?") {
            i = at + tail.find("?>")? + 2;
            continue;
        }
        let end = at + tag_end(tail)? + 1;
        let tag = xml.get(at..end)?;
        if tag.starts_with("</") {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                let (name, start) = current.take()?;
                out.push(Child { name, start, end });
            }
        } else {
            let name: String = tag.get(1..)?.chars().take_while(|c| !c.is_whitespace() && *c != '>' && *c != '/').collect();
            if depth == 0 {
                if tag.ends_with("/>") {
                    out.push(Child { name, start: at, end });
                } else {
                    current = Some((name, at));
                    depth = 1;
                }
            } else if !tag.ends_with("/>") {
                depth += 1;
            }
        }
        i = end;
    }
    (depth == 0).then_some((open_end, close, out))
}

fn tag_end(s: &str) -> Option<usize> {
    let mut quote = None;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

struct Child {
    name: String,
    start: usize,
    end: usize,
}
