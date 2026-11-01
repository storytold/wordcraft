use super::parts::push_part;
use super::{CT_WML, Document, PartRels, W, Writer, math, rt, story, xml};

/// The file id of the first footnote or endnote: after the separators (-1, 0) and the
/// continuation notice (1), when the document has one.
pub(super) fn note_id_base(doc: &Document, foot: bool) -> usize {
    let seps = if foot { &doc.footnote_separators } else { &doc.endnote_separators };
    if seps.continuation_notice.is_some() { 2 } else { 1 }
}

/// Whether the footnotes (or endnotes) part is written: there are notes, or separators of the
/// document's own to keep.
pub(super) fn notes_part(doc: &Document, wr: &Writer<'_>, foot: bool) -> bool {
    let (ids, seps) = if foot { (&wr.footnotes, &doc.footnote_separators) } else { (&wr.endnotes, &doc.endnote_separators) };
    !ids.is_empty() || !seps.is_empty()
}

/// `w:settings`, its children in the schema's order (CT_Settings is a sequence). `template_rel`
/// is the id of the attached template's relationship, when there is one.
pub(super) fn settings_xml(doc: &Document, footnotes: bool, endnotes: bool, template_rel: Option<&str>) -> Vec<u8> {
    let s = &doc.settings;
    let s_notes = (&doc.footnote_separators, &doc.endnote_separators);
    let mut w = W::new();
    let ns = xml::body_ns();
    let ns_refs: Vec<(&str, &str)> = ns.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    w.open("w:settings", &ns_refs);
    let zoom = s.zoom.unwrap_or(wordcraft_doc::Zoom { percent: 100, fit: None });
    let percent = zoom.percent.clamp(10, 500).to_string();
    match zoom.fit {
        Some(fit) => {
            let val = match fit {
                wordcraft_doc::ZoomFit::OnePage => "fullPage",
                wordcraft_doc::ZoomFit::PageWidth => "bestFit",
                wordcraft_doc::ZoomFit::TextWidth => "textFit",
            };
            w.empty("w:zoom", &[("w:val", val), ("w:percent", &percent)]);
        }
        None => w.empty("w:zoom", &[("w:percent", &percent)]),
    }
    // §17.15.1.70 / §17.15.1.69, right after `w:zoom` in CT_Settings.
    if s.remove_personal_info {
        w.empty("w:removePersonalInformation", &[]);
    }
    if s.remove_date_time {
        w.empty("w:removeDateAndTime", &[]);
    }
    if s.page_color.is_some() || s.page_picture.is_some() {
        w.empty("w:displayBackgroundShape", &[]);
    }
    if s.mirror_margins {
        w.empty("w:mirrorMargins", &[]);
    }
    if let Some(id) = template_rel {
        w.empty("w:attachedTemplate", &[("r:id", id)]);
    }
    if s.link_styles {
        w.empty("w:linkStyles", &[]);
    }
    if s.track_changes {
        w.empty("w:trackRevisions", &[]);
    }
    if !s.track_moves {
        w.empty("w:doNotTrackMoves", &[]);
    }
    if let Some(p) = &s.protection {
        let mut a: Vec<(&str, &str)> = Vec::new();
        if p != "none" {
            a.push(("w:edit", p));
        }
        if s.protect_formatting {
            a.push(("w:formatting", "1"));
        }
        a.push(("w:enforcement", "1"));
        // Only the hash attributes the reader keeps, each once.
        for name in crate::read::PROTECTION_HASH_ATTRS {
            if let Some((_, v)) = s.protect_hash.iter().find(|(k, _)| k == name) {
                a.push((name, v));
            }
        }
        w.empty("w:documentProtection", &a);
    }
    w.val("w:defaultTabStop", &crate::units::twips(s.default_tab.clamp(1.0, 1584.0)));
    if s.auto_hyphenation {
        w.empty("w:autoHyphenation", &[]);
    }
    // Schema order: consecutiveHyphenLimit, hyphenationZone, doNotHyphenateCaps.
    if s.consecutive_hyphen_limit > 0 {
        w.val("w:consecutiveHyphenLimit", &s.consecutive_hyphen_limit.min(32_767).to_string());
    }
    if s.hyphenation_zone.is_finite() && (s.hyphenation_zone - wordcraft_doc::DEFAULT_HYPHENATION_ZONE).abs() > 0.01 {
        w.val("w:hyphenationZone", &crate::units::twips(s.hyphenation_zone.clamp(0.0, 1584.0)));
    }
    if !s.hyphenate_caps {
        w.empty("w:doNotHyphenateCaps", &[]);
    }
    if s.even_odd_headers {
        w.empty("w:evenAndOddHeaders", &[]);
    }
    for (tag, v) in [("w:drawingGridHorizontalSpacing", s.grid_h), ("w:drawingGridVerticalSpacing", s.grid_v)] {
        if v.is_finite() && (v - wordcraft_doc::DEFAULT_GRID).abs() > 0.01 {
            w.val(tag, &crate::units::twips(v.clamp(0.5, 1584.0)));
        }
    }
    w.val("w:characterSpacingControl", "doNotCompress");
    write_note_settings(&mut w, s, s_notes, footnotes, endnotes);
    write_compatibility_settings(&mut w, doc);
    if let Some(m) = &s.math {
        math::math_pr(&mut w, m);
    }
    w.close("w:settings");
    w.into_bytes()
}

fn write_note_settings(
    w: &mut W,
    settings: &wordcraft_doc::Settings,
    separators: (&wordcraft_doc::NoteSeparators, &wordcraft_doc::NoteSeparators),
    footnotes: bool,
    endnotes: bool,
) {
    for (tag, fmt, pr, used, el) in [
        ("w:footnotePr", settings.footnote_format, &settings.footnote_pr, footnotes, "w:footnote"),
        ("w:endnotePr", settings.endnote_format, &settings.endnote_pr, endnotes, "w:endnote"),
    ] {
        w.open(tag, &[]);
        story::note_pr_inner(w, &wordcraft_doc::section::NoteProps { num_fmt: Some(fmt), ..*pr });
        if used {
            w.empty(el, &[("w:id", "-1")]);
            w.empty(el, &[("w:id", "0")]);
            let seps = if el == "w:footnote" { separators.0 } else { separators.1 };
            if seps.continuation_notice.is_some() {
                w.empty(el, &[("w:id", "1")]);
            }
        }
        w.close(tag);
    }
}

fn write_compatibility_settings(w: &mut W, doc: &Document) {
    w.open("w:compat", &[]);
    // CT_Compat: the older switches come before the `w:compatSetting`s.
    if doc.settings.do_not_expand_shift_return {
        w.empty("w:doNotExpandShiftReturn", &[]);
    }
    if doc.settings.sum_paragraph_spacing {
        w.empty("w:doNotUseHTMLParagraphAutoSpacing", &[]);
    }
    if doc.settings.allow_space_of_same_style_in_table {
        w.empty("w:allowSpaceOfSameStyleInTable", &[]);
    }
    let mode = doc.settings.compat_mode.clamp(11, 15).to_string();
    w.empty("w:compatSetting", &[("w:name", "compatibilityMode"), ("w:uri", "http://schemas.microsoft.com/office/word"), ("w:val", &mode)]);
    w.close("w:compat");
}

/// Is `d` a usable `w16cid:durableId`: 1–8 hex digits, below 0x7FFFFFFF, not 0?
pub(crate) fn durable_id_ok(d: &str) -> bool {
    d.len() <= 8 && u32::from_str_radix(d, 16).is_ok_and(|v| v != 0 && v < 0x7FFF_FFFF)
}

/// `commentsIds.xml` (paragraph id → durable id) and `commentsExtensible.xml` (durable id → UTC
/// date) for the comments that got paragraph ids. Durable ids read from the file are kept (unique
/// ones); comments without one get the next free id. Word uses them to keep tracking a comment
/// across saves and for its UTC time.
pub(super) fn write_comment_ids(
    doc: &Document,
    comment_paras: &[(u32, String)],
    entries: &mut Vec<(String, Vec<u8>)>,
    overrides: &mut Vec<(String, String)>,
    rels: &mut PartRels,
) {
    if comment_paras.is_empty() {
        return;
    }
    // Keep the existing exact-string reservation rule, including comments without a written part.
    let reserved: std::collections::HashSet<&str> = doc.comments.values().filter_map(|c| c.durable_id.as_deref()).collect();
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut next = 0x1000_0000u32;
    let mut rows: Vec<(String, String, Option<String>)> = Vec::new();
    for (cid, pid) in comment_paras {
        let Some(c) = doc.comments.get(cid) else { continue };
        let kept = c.durable_id.as_deref().filter(|d| durable_id_ok(d)).map(|d| format!("{:08X}", u32::from_str_radix(d, 16).unwrap_or(0)));
        let durable = match kept.filter(|d| !used.contains(d)) {
            Some(d) => d,
            None => loop {
                next = next.saturating_add(1);
                let d = format!("{next:08X}");
                if !used.contains(&d) && !reserved.contains(d.as_str()) {
                    break d;
                }
            },
        };
        used.insert(durable.clone());
        // Only a known UTC time: `w:date` is local time, often with a `Z` it doesn't mean.
        // Remove date and time on save (#1062): no comment date here either.
        let utc = if doc.settings.remove_date_time { None } else { c.date_utc.clone() };
        rows.push((pid.clone(), durable, utc));
    }
    let mc = "http://schemas.openxmlformats.org/markup-compatibility/2006";
    let mut w = W::new();
    w.open(
        "w16cid:commentsIds",
        &[("xmlns:mc", mc), ("xmlns:w16cid", "http://schemas.microsoft.com/office/word/2016/wordml/cid"), ("mc:Ignorable", "w16cid")],
    );
    for (pid, durable, _) in &rows {
        w.empty("w16cid:commentId", &[("w16cid:paraId", pid), ("w16cid:durableId", durable)]);
    }
    w.close("w16cid:commentsIds");
    rels.add(rt::COMMENTS_IDS, "commentsIds.xml", false);
    push_part(entries, overrides, "word/commentsIds.xml", w.into_bytes(), &format!("{CT_WML}commentsIds+xml"), PartRels::default());

    let mut w = W::new();
    w.open(
        "w16cex:commentsExtensible",
        &[("xmlns:mc", mc), ("xmlns:w16cex", "http://schemas.microsoft.com/office/word/2018/wordml/cex"), ("mc:Ignorable", "w16cex")],
    );
    for (_, durable, utc) in &rows {
        match utc {
            Some(d) => w.empty("w16cex:commentExtensible", &[("w16cex:durableId", durable), ("w16cex:dateUtc", d)]),
            None => w.empty("w16cex:commentExtensible", &[("w16cex:durableId", durable)]),
        }
    }
    w.close("w16cex:commentsExtensible");
    rels.add(rt::COMMENTS_EXTENSIBLE, "commentsExtensible.xml", false);
    push_part(entries, overrides, "word/commentsExtensible.xml", w.into_bytes(), &format!("{CT_WML}commentsExtensible+xml"), PartRels::default());
}

pub(super) fn theme_xml(doc: &Document) -> Vec<u8> {
    let s = &doc.settings;
    let names = ["dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hlink", "folHlink"];
    let mut w = W::new();
    let name = if s.theme_name.is_empty() { "Craft" } else { s.theme_name.as_str() };
    w.open("a:theme", &[("xmlns:a", "http://schemas.openxmlformats.org/drawingml/2006/main"), ("name", name)]);
    w.open("a:themeElements", &[]);
    w.open("a:clrScheme", &[("name", name)]);
    for (i, n) in names.iter().enumerate() {
        let c = s.theme_colors.get(i).or_else(|| wordcraft_doc::THEME_COLORS.get(i)).copied().unwrap_or_default();
        w.open(&format!("a:{n}"), &[]);
        w.empty("a:srgbClr", &[("val", &c.hex())]);
        w.close(&format!("a:{n}"));
    }
    w.close("a:clrScheme");
    w.open("a:fontScheme", &[("name", name)]);
    for (tag, face) in [("a:majorFont", &s.major_font), ("a:minorFont", &s.minor_font)] {
        w.open(tag, &[]);
        w.empty("a:latin", &[("typeface", face)]);
        w.empty("a:ea", &[("typeface", "")]);
        w.empty("a:cs", &[("typeface", "")]);
        w.close(tag);
    }
    w.close("a:fontScheme");
    // A minimal format scheme (three entries per list, as the schema requires).
    w.open("a:fmtScheme", &[("name", name)]);
    w.open("a:fillStyleLst", &[]);
    for _ in 0..3 {
        w.raw("<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>");
    }
    w.close("a:fillStyleLst");
    w.open("a:lnStyleLst", &[]);
    for width in ["6350", "12700", "19050"] {
        w.raw(&format!(
            "<a:ln w=\"{width}\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/><a:miter lim=\"800000\"/></a:ln>"
        ));
    }
    w.close("a:lnStyleLst");
    w.open("a:effectStyleLst", &[]);
    for _ in 0..3 {
        w.raw("<a:effectStyle><a:effectLst/></a:effectStyle>");
    }
    w.close("a:effectStyleLst");
    w.open("a:bgFillStyleLst", &[]);
    for _ in 0..3 {
        w.raw("<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>");
    }
    w.close("a:bgFillStyleLst");
    w.close("a:fmtScheme");
    w.close("a:themeElements");
    w.empty("a:objectDefaults", &[]);
    w.empty("a:extraClrSchemeLst", &[]);
    w.close("a:theme");
    w.into_bytes()
}

/// The author the saved file gives a comment or tracked change when the document removes personal
/// information on save (`w:removePersonalInformation`), as Word does.
pub(crate) const ANONYMOUS_AUTHOR: &str = "Author";

pub(crate) const ANONYMOUS_INITIALS: &str = "A";

/// The `w:author` written for an annotation by `author`: "Author" when the document removes
/// personal information on save, "Unknown" when it has none.
pub(crate) fn annotation_author<'a>(doc: &Document, author: &'a str) -> &'a str {
    if doc.settings.remove_personal_info {
        ANONYMOUS_AUTHOR
    } else if author.is_empty() {
        "Unknown"
    } else {
        author
    }
}

pub(super) fn core_xml(doc: &Document) -> Vec<u8> {
    let c = &doc.core;
    // Remove personal information (§17.15.1.70): no creator or last-modified-by in the file.
    let personal = |v: &String| if doc.settings.remove_personal_info { String::new() } else { v.clone() };
    let (creator, last_modified_by) = (personal(&c.creator), personal(&c.last_modified_by));
    let mut w = W::new();
    w.open(
        "cp:coreProperties",
        &[
            ("xmlns:cp", "http://schemas.openxmlformats.org/package/2006/metadata/core-properties"),
            ("xmlns:dc", "http://purl.org/dc/elements/1.1/"),
            ("xmlns:dcterms", "http://purl.org/dc/terms/"),
            ("xmlns:dcmitype", "http://purl.org/dc/dcmitype/"),
            ("xmlns:xsi", "http://www.w3.org/2001/XMLSchema-instance"),
        ],
    );
    for (tag, v) in [
        ("dc:title", &c.title),
        ("dc:subject", &c.subject),
        ("dc:creator", &creator),
        ("cp:keywords", &c.keywords),
        ("dc:description", &c.description),
        ("cp:lastModifiedBy", &last_modified_by),
        ("cp:category", &c.category),
    ] {
        if !v.is_empty() {
            w.leaf(tag, &[], v);
        }
    }
    if c.revision > 0 {
        w.leaf("cp:revision", &[], &c.revision.to_string());
    }
    for (tag, v) in [("dcterms:created", &c.created), ("dcterms:modified", &c.modified)] {
        if !v.is_empty() {
            w.leaf(tag, &[("xsi:type", "dcterms:W3CDTF")], v);
        }
    }
    w.close("cp:coreProperties");
    w.into_bytes()
}

pub(super) fn app_xml(doc: &Document) -> Vec<u8> {
    let mut w = W::new();
    w.open(
        "Properties",
        &[
            ("xmlns", "http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"),
            ("xmlns:vt", "http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"),
        ],
    );
    w.leaf("Application", &[], "WordCraft");
    w.leaf("Words", &[], &doc.word_count().to_string());
    w.leaf("Paragraphs", &[], &doc.paragraph_count().to_string());
    w.leaf("DocSecurity", &[], "0");
    // Kept properties, except the personal ones when the file asks Word to remove personal
    // information on save.
    let private = doc.settings.remove_personal_info;
    if let Some(kept) = doc.passthrough.get(crate::package::APP_KEPT) {
        for line in String::from_utf8_lossy(kept).lines().take(crate::package::APP_KEPT_PROPS.len()) {
            let Some((name, value)) = line.split_once('\t') else { continue };
            let Some(name) = crate::package::APP_KEPT_PROPS.iter().find(|n| **n == name) else { continue };
            if private && matches!(*name, "Company" | "Manager") {
                continue;
            }
            w.leaf(name, &[], value);
        }
    }
    w.close("Properties");
    w.into_bytes()
}
