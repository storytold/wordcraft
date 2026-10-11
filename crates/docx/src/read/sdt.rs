//! Content control properties (`w:sdtPr`, ECMA-376 §17.5.2; check boxes and repeating
//! sections from the [MS-DOCX] `w14`/`w15` extensions).

use wordcraft_doc::control::{CHECKED, ContentControl, ControlKind, ControlLock, ListItem, MAX_CONTROL_TEXT, MAX_LIST_ITEMS, UNCHECKED};

use crate::xml::El;

/// Most bytes of `w:sdtPr` children kept verbatim per control.
const MAX_EXTRA: usize = 64 * 1024;

fn text(v: Option<&str>) -> String {
    let v = v.unwrap_or("");
    v.chars().filter(|c| *c != wordcraft_doc::para::OBJ && !c.is_control()).take(MAX_CONTROL_TEXT).collect()
}

/// An on/off element: on unless its value (`w:val`, or the extension's own) says off.
fn on(e: &El) -> bool {
    e.attr("w:val").or_else(|| e.attr("w14:val")).or_else(|| e.attr("w15:val")).is_none_or(|v| !matches!(v, "0" | "false" | "off"))
}

fn symbol(e: Option<&El>, default: char) -> (char, String) {
    let Some(e) = e else { return (default, String::new()) };
    let ch = e
        .attr("w14:val")
        .and_then(|h| u32::from_str_radix(h.trim(), 16).ok())
        .and_then(char::from_u32)
        .filter(|c| *c as u32 >= 0x20 && *c != wordcraft_doc::para::OBJ);
    (ch.unwrap_or(default), text(e.attr("w14:font")))
}

fn items(e: &El) -> Vec<ListItem> {
    e.children("w:listItem")
        .take(MAX_LIST_ITEMS)
        .map(|i| ListItem { display: text(i.attr("w:displayText")), value: text(i.attr("w:value")) })
        .collect()
}

/// The control described by `w:sdtPr` (`None`: an `w:sdt` without one, a rich text control).
pub fn sdt_pr(pr: Option<&El>, block: bool) -> ContentControl {
    let mut c = ContentControl { block, ..Default::default() };
    let Some(pr) = pr else { return c };
    let mut extra_len = 0;
    for k in pr.els() {
        match k.name.as_str() {
            "w:rPr" => c.rpr_xml = k.to_xml(),
            "w:alias" => c.title = text(k.attr("w:val")),
            "w:tag" => c.tag = text(k.attr("w:val")),
            "w:id" => c.id = k.attr("w:val").and_then(|v| v.trim().parse::<i64>().ok()),
            "w:lock" => c.lock = ControlLock::from_ooxml(k.attr("w:val").unwrap_or("")),
            "w:placeholder" => c.placeholder = text(k.child("w:docPart").and_then(|d| d.attr("w:val"))),
            "w:temporary" => c.temporary = on(k),
            "w:showingPlcHdr" => c.showing_placeholder = on(k),
            "w:richText" => c.kind = ControlKind::RichText,
            "w:text" => c.kind = ControlKind::Text { multi_line: k.attr("w:multiLine").is_some_and(|v| !matches!(v, "0" | "false" | "off")) },
            "w14:checkbox" => {
                let checked = k.child("w14:checked").is_some_and(on);
                let (checked_char, checked_font) = symbol(k.child("w14:checkedState"), CHECKED);
                let (unchecked_char, unchecked_font) = symbol(k.child("w14:uncheckedState"), UNCHECKED);
                c.kind = ControlKind::CheckBox { checked, checked_char, checked_font, unchecked_char, unchecked_font };
            }
            "w:comboBox" => c.kind = ControlKind::ComboBox { items: items(k), last_value: text(k.attr("w:lastValue")) },
            "w:dropDownList" => c.kind = ControlKind::DropDown { items: items(k), last_value: text(k.attr("w:lastValue")) },
            "w:date" => {
                let v = |n: &str| text(k.child_val(n));
                c.kind = ControlKind::Date {
                    full_date: text(k.attr("w:fullDate")),
                    format: v("w:dateFormat"),
                    lid: v("w:lid"),
                    calendar: v("w:calendar"),
                    store_as: v("w:storeMappedDataAs"),
                };
            }
            "w:picture" => c.kind = ControlKind::Picture,
            "w:docPartObj" | "w:docPartList" => {
                c.kind = ControlKind::Gallery {
                    list: k.name == "w:docPartList",
                    gallery: text(k.child_val("w:docPartGallery")),
                    category: text(k.child_val("w:docPartCategory")),
                    unique: k.child("w:docPartUnique").is_some_and(on),
                };
            }
            "w15:repeatingSection" => {
                c.kind = ControlKind::RepeatingSection {
                    title: text(k.child("w15:sectionTitle").and_then(|t| t.attr("w:val").or_else(|| t.attr("w15:val")))),
                    no_insert_delete: k.child("w15:doNotAllowInsertDeleteSection").is_some_and(on),
                };
            }
            "w15:repeatingSectionItem" => c.kind = ControlKind::RepeatingSectionItem,
            _ => {
                let x = k.to_xml();
                if !x.is_empty() && extra_len + x.len() <= MAX_EXTRA {
                    extra_len += x.len();
                    c.extra.push(x);
                }
            }
        }
    }
    c
}

/// `w:sdtEndPr`, kept verbatim.
pub fn sdt_end_pr(e: Option<&El>) -> String {
    e.map(|e| e.to_xml()).filter(|x| x.len() <= MAX_EXTRA).unwrap_or_default()
}
