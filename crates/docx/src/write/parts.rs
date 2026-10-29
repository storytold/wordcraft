use super::bibliography::{bibliography_props_xml, bibliography_xml};
use super::settings::{
    ANONYMOUS_INITIALS, annotation_author, app_xml, core_xml, note_id_base, notes_part, settings_xml, theme_xml, write_comment_ids,
};
use super::styles::{numbering_xml, styles_xml};
use super::{
    BTreeMap, Blocks, CT_WML, Document, DocxError, Flavor, HashMap, MAX_VBA_RELATED, PartRels, VBA_PROJECT_PART, VBA_RELATED, W, Writer, embed,
    glossary, rt, settings_merge, watermark, xml,
};

/// The parts of the package [`write_as`] zips, in order, `[Content_Types].xml` first: (part name
/// without the leading `/`, bytes).
pub(crate) fn write_parts(doc: &Document, flavor: Flavor) -> Result<Vec<(String, Vec<u8>)>, DocxError> {
    // A range field must be whole in the file: drop markers that lost their partner in editing.
    let balanced;
    let doc = if doc.has_unbalanced_field_ranges() {
        let mut d = doc.clone();
        d.balance_field_ranges();
        balanced = d;
        &balanced
    } else {
        doc
    };
    let mut wr = Writer {
        eq_number: 0,
        doc,
        media_files: BTreeMap::new(),
        bookmarks: HashMap::new(),
        next_rev_id: 0,
        open_moves: Vec::new(),
        // Fresh drawing ids start past the shapes' own (connectors refer to those).
        docpr: doc.next_shape_id().saturating_sub(1),
        shape_ids: std::collections::HashSet::new(),
        z: 251_658_240,
        footnotes: Vec::new(),
        endnotes: Vec::new(),
        hf: Vec::new(),
        linked_headers: Default::default(),
        wm_header: (0..=u32::MAX).rev().find(|id| !doc.parts.contains_key(id)).unwrap_or(u32::MAX),
        pending_watermark: None,
        para_ids: HashMap::new(),
        pending_mark: None,
        current_note: None,
        toc_hold_end: false,
        toc_field: None,
        toc_begin_here: false,
        toc_end_here: false,
        inline_controls: Vec::new(),
        used_media: Default::default(),
        boxes: wordcraft_doc::BoxBudget::default(),
        written_boxes: std::collections::HashSet::new(),
        embeds: embed::EmbedWriter::new(doc),
        txbx_links: doc
            .text_box_chains()
            .iter()
            .enumerate()
            .flat_map(|(n, c)| c.iter().enumerate().map(move |(seq, id)| (*id, (n as u32 + 1, seq as u32))))
            .collect(),
    };
    wr.assign_media();
    wr.embeds.reserve_media(wr.media_files.values());
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    let mut overrides: Vec<(String, String)> = Vec::new();
    let mut rels = PartRels::default();
    let mut root = PartRels::default();
    let ns = xml::body_ns();
    let ns_refs: Vec<(&str, &str)> = ns.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

    // Comments get paragraph ids before any story is written.
    let mut comment_paras: Vec<(u32, String)> = Vec::new();
    for (i, (cid, c)) in doc.comments.iter().enumerate() {
        if let Some(part) = doc.parts.get(&c.part)
            && let Some(last) = part.blocks.iter().rev().find_map(|b| b.as_para())
        {
            let pid = format!("{:08X}", 0x1000_0000u32 + i as u32 + 1);
            wr.para_ids.insert(last as *const _ as usize, pid.clone());
            comment_paras.push((*cid, pid));
        }
    }

    let body = write_document_body(doc, &mut wr, &mut rels, &ns_refs);

    // Headers and footers (discovered while writing sections).
    let mut done = 0;
    while let Some((id, file, footer)) = wr.hf.get(done).cloned() {
        done += 1;
        let mut prels = PartRels::default();
        let mut w = W::new();
        let tag = if footer { "w:ftr" } else { "w:hdr" };
        w.open(tag, &ns_refs);
        let blocks: Blocks = doc.parts.get(&id).map(|p| p.blocks.clone()).unwrap_or_default();
        // The watermark goes in every header, at the start of its first paragraph.
        wr.pending_watermark = match &doc.settings.watermark {
            Some(wm) if !footer => watermark::run(wm, done, &doc.last_section),
            _ => None,
        };
        wr.blocks(&mut w, &blocks, &mut prels, false, 0);
        // Always taken: the story ends with a top-level paragraph.
        wr.pending_watermark = None;
        w.close(tag);
        push_part(
            &mut entries,
            &mut overrides,
            &format!("word/{file}"),
            w.into_bytes(),
            &format!("{CT_WML}{}+xml", if footer { "footer" } else { "header" }),
            prels,
        );
    }

    // The glossary: kept verbatim while its entries are as read and its parts were kept, else
    // written from the model (before the notes: references its entries hold are collected too).
    // When the privacy settings ask for changes, its kept parts are written without the names and
    // dates of its tracked changes and comments (#3692), or from the model if they can't be.
    let (names, dates) = (doc.settings.remove_personal_info, doc.settings.remove_date_time);
    if glossary::edited(doc)
        || (!doc.glossary.is_empty() && !wr.embeds.kept_glossary())
        || ((names || dates) && !wr.embeds.scrub_glossary(names, dates))
    {
        wr.embeds.drop_glossary = true;
        wr.glossary(&mut entries, &mut overrides, &mut rels);
    }

    write_notes(doc, &ns_refs, &mut wr, &mut entries, &mut overrides, &mut rels);

    write_comments(doc, &comment_paras, &ns_refs, &mut wr, &mut entries, &mut overrides, &mut rels);

    rels.add(rt::STYLES, "styles.xml", false);
    push_part(&mut entries, &mut overrides, "word/styles.xml", styles_xml(doc), &format!("{CT_WML}styles+xml"), PartRels::default());
    if !doc.numbering.nums.is_empty() || !doc.numbering.abstracts.is_empty() {
        rels.add(rt::NUMBERING, "numbering.xml", false);
        push_part(&mut entries, &mut overrides, "word/numbering.xml", numbering_xml(doc), &format!("{CT_WML}numbering+xml"), PartRels::default());
    }
    rels.add(rt::SETTINGS, "settings.xml", false);
    // The attached template is an external relationship of the settings part (§17.15.1.6).
    let mut settings_rels = PartRels::default();
    let template = doc.settings.attached_template.as_deref().map(str::trim).filter(|t| !t.is_empty());
    let template_rel = template.map(|t| settings_rels.add(rt::ATTACHED_TEMPLATE, t, true));
    push_part(
        &mut entries,
        &mut overrides,
        "word/settings.xml",
        {
            let generated = settings_xml(doc, notes_part(doc, &wr, true), notes_part(doc, &wr, false), template_rel.as_deref());
            match doc.passthrough.get(crate::package::SETTINGS_ORIG) {
                Some(orig) => settings_merge::merge(orig, &generated).unwrap_or(generated),
                None => generated,
            }
        },
        &format!("{CT_WML}settings+xml"),
        settings_rels,
    );
    rels.add(rt::THEME, "theme/theme1.xml", false);
    push_part(
        &mut entries,
        &mut overrides,
        "word/theme/theme1.xml",
        theme_xml(doc),
        "application/vnd.openxmlformats-officedocument.theme+xml",
        PartRels::default(),
    );

    wr.embeds.font_table(&mut rels);

    // Media referenced by some drawing (orphan parts would make the package suspect).
    let mut media_types: BTreeMap<String, &str> = BTreeMap::new();
    for (key, file) in &wr.media_files {
        if !wr.used_media.contains(key) {
            continue;
        }
        if let Some(bytes) = doc.media.get(key) {
            let ext = file.rsplit('.').next().unwrap_or("bin").to_ascii_lowercase();
            media_types.insert(ext, content_type_for(file));
            entries.push((format!("word/media/{file}"), bytes.to_vec()));
        }
    }

    wr.embeds.custom_xml_rels("D", &mut rels);
    wr.embeds.custom_xml_rels("P", &mut root);
    wr.embeds.unmodelled_rels("D", &mut rels);
    wr.embeds.unmodelled_rels("P", &mut root);
    // Parts of embedded objects and custom XML stores kept from the file.
    for (name, bytes, ct, prels) in std::mem::take(&mut wr.embeds.parts) {
        push_part(&mut entries, &mut overrides, &name, bytes, &ct, prels);
    }

    // The macro project and the parts it relates to (VBA data, signatures…), verbatim. A
    // macro-free package can't hold them (Word drops them too).
    if let Some(vba) = doc.passthrough.get(VBA_PROJECT_PART).filter(|b| !b.is_empty()) {
        if flavor.macros() {
            let mut vba_rels = PartRels::default();
            let manifest = doc.passthrough.get(VBA_RELATED).map(|m| String::from_utf8_lossy(m).into_owned()).unwrap_or_default();
            let mut written: Vec<String> = Vec::new();
            for line in manifest.lines().take(MAX_VBA_RELATED) {
                let mut f = line.split('\t');
                let (Some(kind), Some(path), Some(ct), None) = (f.next(), f.next(), f.next(), f.next()) else { continue };
                let Some(target) = path.strip_prefix("word/").filter(|t| !t.is_empty()) else { continue };
                let Some(bytes) = doc.passthrough.get(path) else { continue };
                let ours = written.iter().any(|w| w.eq_ignore_ascii_case(path));
                // Never shadow a part this writer produces (a hostile relationship may point at one).
                let clash = path.eq_ignore_ascii_case("word/document.xml")
                    || path.eq_ignore_ascii_case(VBA_PROJECT_PART)
                    || path.to_ascii_lowercase().ends_with(".rels")
                    || entries.iter().any(|(n, _)| n.eq_ignore_ascii_case(path));
                if clash && !ours {
                    continue;
                }
                // A relative reference whose first segment has a colon would read as a URI scheme.
                let target = if target.split('/').next().is_some_and(|seg| seg.contains(':')) { format!("./{target}") } else { target.to_string() };
                vba_rels.add(kind, &target, false);
                if !ours {
                    push_part(&mut entries, &mut overrides, path, bytes.to_vec(), ct, PartRels::default());
                    written.push(path.to_string());
                }
            }
            rels.add(rt::VBA_PROJECT, "vbaProject.bin", false);
            push_part(&mut entries, &mut overrides, VBA_PROJECT_PART, vba.to_vec(), "application/vnd.ms-office.vbaProject", vba_rels);
        } else {
            log::warn!("docx: {flavor:?} can't hold macros; the VBA project is left out");
        }
    }

    // Main part goes first in the zip after content types.
    entries.insert(0, ("word/document.xml".into(), body));
    overrides.insert(0, ("/word/document.xml".into(), flavor.main_content_type().into()));
    entries.insert(1, ("word/_rels/document.xml.rels".into(), rels.xml()));

    // Package-level parts.
    root.add(rt::OFFICE_DOC, "word/document.xml", false);
    root.add(rt::CORE, "docProps/core.xml", false);
    root.add(rt::APP, "docProps/app.xml", false);
    entries.push(("docProps/core.xml".into(), core_xml(doc)));
    overrides.push(("/docProps/core.xml".into(), "application/vnd.openxmlformats-package.core-properties+xml".into()));
    entries.push(("docProps/app.xml".into(), app_xml(doc)));
    overrides.push(("/docProps/app.xml".into(), "application/vnd.openxmlformats-officedocument.extended-properties+xml".into()));
    if !doc.custom_props.is_empty() {
        root.add(rt::CUSTOM, "docProps/custom.xml", false);
        entries.push(("docProps/custom.xml".into(), crate::custom::write(&doc.custom_props)));
        overrides.push(("/docProps/custom.xml".into(), "application/vnd.openxmlformats-officedocument.custom-properties+xml".into()));
    }
    if !doc.sources.is_empty() {
        root.add(rt::CUSTOM_XML, "customXml/item1.xml", false);
        let mut item_rels = PartRels::default();
        item_rels.add(rt::CUSTOM_XML_PROPS, "itemProps1.xml", false);
        push_part(
            &mut entries,
            &mut overrides,
            "customXml/item1.xml",
            bibliography_xml(doc),
            "application/vnd.openxmlformats-officedocument.customXml+xml",
            item_rels,
        );
        push_part(
            &mut entries,
            &mut overrides,
            "customXml/itemProps1.xml",
            bibliography_props_xml(),
            "application/vnd.openxmlformats-officedocument.customXmlProperties+xml",
            PartRels::default(),
        );
    }
    entries.insert(0, ("_rels/.rels".into(), root.xml()));

    // Content types.
    let mut w = W::new();
    w.open("Types", &[("xmlns", "http://schemas.openxmlformats.org/package/2006/content-types")]);
    w.empty("Default", &[("Extension", "rels"), ("ContentType", "application/vnd.openxmlformats-package.relationships+xml")]);
    w.empty("Default", &[("Extension", "xml"), ("ContentType", "application/xml")]);
    for (ext, ct) in &media_types {
        if ext != "xml" && ext != "rels" {
            w.empty("Default", &[("Extension", ext), ("ContentType", ct)]);
        }
    }
    for (part, ct) in &overrides {
        w.empty("Override", &[("PartName", part), ("ContentType", ct)]);
    }
    w.close("Types");
    entries.insert(0, ("[Content_Types].xml".into(), w.into_bytes()));
    Ok(entries)
}

fn write_document_body(doc: &Document, wr: &mut Writer<'_>, rels: &mut PartRels, ns_refs: &[(&str, &str)]) -> Vec<u8> {
    let mut w = W::new();
    w.open("w:document", ns_refs);
    let pic = doc.settings.page_picture.as_ref().filter(|p| wr.media_files.contains_key(&p.media) && doc.media.contains_key(&p.media));
    if pic.is_some() || doc.settings.page_color.is_some() {
        let color = doc.settings.page_color.map(|c| c.hex());
        let attrs: Vec<(&str, &str)> = color.iter().map(|c| ("w:color", c.as_str())).collect();
        w.open("w:background", &attrs);
        if let Some(p) = pic
            && let Some(file) = wr.media_files.get(&p.media)
        {
            let rid = rels.add(rt::IMAGE, &format!("media/{file}"), false);
            wr.used_media.insert(p.media.clone());
            w.open("v:background", &[("id", "_x0000_s1025"), ("o:bwmode", "white")]);
            let mut fa: Vec<(&str, &str)> = vec![("r:id", &rid)];
            if !p.fill_type.is_empty() {
                fa.push(("type", &p.fill_type));
            }
            if !p.title.is_empty() {
                fa.push(("o:title", &p.title));
            }
            w.empty("v:fill", &fa);
            w.close("v:background");
        }
        w.close("w:background");
    }
    w.open("w:body", &[]);
    wr.blocks(&mut w, &doc.body, rels, true, 0);
    let last = doc.last_section.clone();
    wr.sectpr(&mut w, &last, rels);
    w.close("w:body");
    w.close("w:document");
    w.into_bytes()
}

fn write_notes(
    doc: &Document,
    ns_refs: &[(&str, &str)],
    wr: &mut Writer<'_>,
    entries: &mut Vec<(String, Vec<u8>)>,
    overrides: &mut Vec<(String, String)>,
    rels: &mut PartRels,
) {
    for foot in [true, false] {
        let seps = if foot { &doc.footnote_separators } else { &doc.endnote_separators };
        if !notes_part(doc, wr, foot) {
            continue;
        }
        let (root, tag, file) = if foot { ("w:footnotes", "w:footnote", "footnotes.xml") } else { ("w:endnotes", "w:endnote", "endnotes.xml") };
        let mut prels = PartRels::default();
        let mut w = W::new();
        w.open(root, ns_refs);
        // The separators and continuation notice: the document's own, else the defaults (no
        // notice). Word numbers them -1, 0 and 1.
        for (id, kind, own) in [
            ("-1", "separator", &seps.separator),
            ("0", "continuationSeparator", &seps.continuation_separator),
            ("1", "continuationNotice", &seps.continuation_notice),
        ] {
            match own {
                Some(blocks) => {
                    w.open(tag, &[("w:type", kind), ("w:id", id)]);
                    if blocks.is_empty() {
                        w.empty("w:p", &[]);
                    } else {
                        wr.blocks(&mut w, blocks, &mut prels, false, 0);
                    }
                    w.close(tag);
                }
                None if kind != "continuationNotice" => {
                    w.open(tag, &[("w:type", kind), ("w:id", id)]);
                    w.open("w:p", &[]);
                    w.open("w:pPr", &[]);
                    w.empty("w:spacing", &[("w:after", "0"), ("w:line", "240"), ("w:lineRule", "auto")]);
                    w.close("w:pPr");
                    w.open("w:r", &[]);
                    w.empty(if kind == "separator" { "w:separator" } else { "w:continuationSeparator" }, &[]);
                    w.close("w:r");
                    w.close("w:p");
                    w.close(tag);
                }
                None => {}
            }
        }
        let base = note_id_base(doc, foot);
        let mut k = 0;
        while let Some(pid) = (if foot { &wr.footnotes } else { &wr.endnotes }).get(k).copied() {
            let nid = (k + base).to_string();
            k += 1;
            w.open(tag, &[("w:id", &nid)]);
            let blocks: Blocks = doc.parts.get(&pid).map(|p| p.blocks.clone()).unwrap_or_default();
            wr.note_blocks(&mut w, &blocks, &mut prels, foot, pid);
            w.close(tag);
        }
        w.close(root);
        let kind = if foot { rt::FOOTNOTES } else { rt::ENDNOTES };
        rels.add(kind, file, false);
        push_part(
            entries,
            overrides,
            &format!("word/{file}"),
            w.into_bytes(),
            &format!("{CT_WML}{}+xml", if foot { "footnotes" } else { "endnotes" }),
            prels,
        );
    }
}

fn write_comments(
    doc: &Document,
    comment_paras: &[(u32, String)],
    ns_refs: &[(&str, &str)],
    wr: &mut Writer<'_>,
    entries: &mut Vec<(String, Vec<u8>)>,
    overrides: &mut Vec<(String, String)>,
    rels: &mut PartRels,
) {
    if !doc.comments.is_empty() {
        let mut prels = PartRels::default();
        let mut w = W::new();
        w.open("w:comments", ns_refs);
        for (cid, c) in &doc.comments {
            let id = cid.to_string();
            let author = annotation_author(doc, &c.author);
            let mut a = vec![("w:id", id.as_str()), ("w:author", author)];
            if !c.date.is_empty() && !doc.settings.remove_date_time {
                a.push(("w:date", c.date.as_str()));
            }
            if doc.settings.remove_personal_info {
                a.push(("w:initials", ANONYMOUS_INITIALS));
            } else if !c.initials.is_empty() {
                a.push(("w:initials", c.initials.as_str()));
            }
            w.open("w:comment", &a);
            let blocks: Blocks = doc.parts.get(&c.part).map(|p| p.blocks.clone()).unwrap_or_default();
            wr.blocks(&mut w, &blocks, &mut prels, false, 0);
            w.close("w:comment");
        }
        w.close("w:comments");
        rels.add(rt::COMMENTS, "comments.xml", false);
        push_part(entries, overrides, "word/comments.xml", w.into_bytes(), &format!("{CT_WML}comments+xml"), prels);

        let mut w = W::new();
        w.open(
            "w15:commentsEx",
            &[
                ("xmlns:w15", "http://schemas.microsoft.com/office/word/2012/wordml"),
                ("xmlns:mc", "http://schemas.openxmlformats.org/markup-compatibility/2006"),
            ],
        );
        let pid_of = |cid: u32| comment_paras.iter().find(|(c, _)| *c == cid).map(|(_, p)| p.clone());
        for (cid, c) in &doc.comments {
            let Some(pid) = pid_of(*cid) else { continue };
            let done = if c.resolved { "1" } else { "0" };
            match c.parent.and_then(pid_of) {
                Some(parent) => w.empty("w15:commentEx", &[("w15:paraId", &pid), ("w15:paraIdParent", &parent), ("w15:done", done)]),
                None => w.empty("w15:commentEx", &[("w15:paraId", &pid), ("w15:done", done)]),
            }
        }
        w.close("w15:commentsEx");
        rels.add(rt::COMMENTS_EX, "commentsExtended.xml", false);
        push_part(entries, overrides, "word/commentsExtended.xml", w.into_bytes(), &format!("{CT_WML}commentsExtended+xml"), PartRels::default());
        write_comment_ids(doc, comment_paras, entries, overrides, rels);
    }
}

pub(super) fn push_part(
    entries: &mut Vec<(String, Vec<u8>)>,
    overrides: &mut Vec<(String, String)>,
    path: &str,
    bytes: Vec<u8>,
    ct: &str,
    rels: PartRels,
) {
    if !rels.is_empty() {
        let (dir, file) = path.rsplit_once('/').unwrap_or(("", path));
        entries.push((format!("{dir}/_rels/{file}.rels"), rels.xml()));
    }
    entries.push((path.to_string(), bytes));
    overrides.push((format!("/{path}"), ct.to_string()));
}

/// Content type by file extension (or sniffed name).
pub(super) fn content_type_for(file: &str) -> &'static str {
    match file.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" | "jpe" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        "tif" | "tiff" => "image/tiff",
        "svg" => "image/svg+xml",
        "emf" => "image/x-emf",
        "wmf" => "image/x-wmf",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

/// Guess an image extension from magic bytes.
pub(super) fn sniff(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG") {
        "png"
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        "jpeg"
    } else if bytes.starts_with(b"GIF8") {
        "gif"
    } else if bytes.starts_with(b"BM") {
        "bmp"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "webp"
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        "tiff"
    } else if bytes.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) {
        "wmf"
    } else if bytes.get(40..44) == Some(b" EMF") {
        "emf"
    } else if bytes.starts_with(b"<svg") || bytes.starts_with(b"<?xml") {
        "svg"
    } else {
        "bin"
    }
}
