use super::body::Body;
use super::limits::{MAX_ENTRY, MAX_EXTRA_SPACES, MAX_SECTIONS};
use super::styles::{Styles, column_sep, columns_of};
use super::xml::{Attrs, Ev, events, get, length, local};
use super::{Columns, Cursor, Document, FBlock, Flow, HashMap, Meta, Para, Read, SectionProps, model, page_parts};
use std::sync::Arc;
use wordcraft_doc::odf_profile::OdfProfile;

pub(super) fn read_entry(z: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Option<Vec<u8>> {
    let f = z.by_name(name).ok()?;
    if f.size() > MAX_ENTRY {
        return None;
    }
    let mut out = Vec::new();
    f.take(MAX_ENTRY).read_to_end(&mut out).ok()?;
    Some(out)
}

/// The pictures a package's frames point at, within one byte budget. Each entry is inflated
/// at most once, and only when the size it declares fits in what is left; the bytes inflated
/// (even from an entry bigger than it declares) and every later copy are charged, so repeated
/// references and lying entries can't go past the budget, and nothing is inflated after it.
pub(super) struct Pictures {
    /// Entry name → its bytes (`None`: missing, unreadable or over the budget; not retried).
    seen: HashMap<String, Option<Vec<u8>>>,
    /// Bytes still allowed.
    pub(super) room: u64,
}

impl Pictures {
    pub(super) fn new(budget: u64) -> Self {
        Pictures { seen: HashMap::new(), room: budget }
    }

    /// The bytes of entry `name`, or `None` when it can't be read or doesn't fit.
    pub(super) fn get(&mut self, z: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Option<Vec<u8>> {
        if let Some(hit) = self.seen.get(name) {
            let d = hit.as_ref().filter(|d| d.len() as u64 <= self.room)?;
            self.room = self.room.saturating_sub(d.len() as u64);
            return Some(d.clone());
        }
        let d = self.inflate(z, name);
        self.seen.insert(name.to_string(), d.clone());
        d
    }

    /// Inflate entry `name`, charging the bytes inflated; `None` when it doesn't fit or fails.
    fn inflate(&mut self, z: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Option<Vec<u8>> {
        let room = self.room;
        if room == 0 {
            return None;
        }
        let f = z.by_name(name).ok()?;
        if f.size() > room {
            return None;
        }
        let mut out = Vec::new();
        let read = f.take(room.saturating_add(1)).read_to_end(&mut out);
        self.room = room.saturating_sub(out.len() as u64);
        (read.is_ok() && out.len() as u64 <= room).then_some(out)
    }
}

/// What `meta.xml` says of the document's changes: who changed it last (`dc:creator`), when
/// (`dc:date`, kept as written, with `Z` when it gives no zone) and how often it was saved
/// (`meta:editing-cycles`).
pub(super) fn read_changes(xml: &[u8]) -> (String, String, u32) {
    let mut cur: Option<String> = None;
    let (mut by, mut date, mut cycles) = (String::new(), String::new(), String::new());
    events(xml, |ev| match ev {
        Ev::Start(q, _) => cur = Some(q),
        Ev::End(_) => cur = None,
        Ev::Text(t) => match cur.as_deref() {
            Some("dc:creator") if by.len() < 256 => by.push_str(&t),
            Some("dc:date") if date.len() < 64 => date.push_str(&t),
            Some("meta:editing-cycles") if cycles.len() < 16 => cycles.push_str(&t),
            _ => {}
        },
    });
    let date = date.trim();
    // Zoned already: `Z`, or an offset (`+hh:mm`, or `-hh:mm` in the time part), as `read_meta`.
    let zoned = date.ends_with(['Z', 'z']) || date.contains('+') || date.split_once('T').is_some_and(|(_, time)| time.contains('-'));
    let date = if date.is_empty() || zoned { date.to_string() } else { format!("{}Z", date.split('.').next().unwrap_or("")) };
    (by.trim().to_string(), date, cycles.trim().parse::<u32>().unwrap_or(0))
}

pub(super) fn read_meta(xml: &[u8]) -> Meta {
    let mut m = Meta::default();
    let mut cur: Option<String> = None;
    let mut keywords: Vec<String> = Vec::new();
    let mut creator = String::new();
    let mut mod_date = String::new();
    events(xml, |ev| match ev {
        Ev::Start(q, _) => cur = Some(q),
        Ev::End(_) => cur = None,
        Ev::Text(t) => match cur.as_deref() {
            Some("dc:title") => m.title.push_str(&t),
            Some("dc:subject") => m.subject.push_str(&t),
            Some("dc:description") => m.description.push_str(&t),
            Some("meta:initial-creator") => m.author.push_str(&t),
            Some("dc:creator") => creator.push_str(&t),
            Some("meta:keyword") => keywords.push(t),
            Some("meta:creation-date") => m.created.push_str(&t),
            Some("dc:date") => mod_date.push_str(&t),
            _ => {}
        },
    });
    if m.author.is_empty() {
        m.author = creator;
    }
    if m.created.is_empty() {
        m.created = mod_date;
    }
    m.keywords = keywords.join(", ");
    let created = m.created.trim();
    if !created.is_empty() {
        let has_tz =
            created.ends_with(['Z', 'z']) || created.contains('+') || created.split_once('T').map(|(_, time)| time.contains('-')).unwrap_or(false);
        if !has_tz {
            let base = created.split('.').next().unwrap_or("");
            m.created = format!("{base}Z");
        } else {
            m.created = created.to_string();
        }
    } else {
        m.created.clear();
    }
    m
}

/// Every paragraph of `blocks`, in table cells, notes and comments too.
fn each_para(blocks: &mut [FBlock], depth: usize, f: &mut dyn FnMut(&mut Para)) {
    if depth > model::MAX_DEPTH {
        return;
    }
    for b in blocks {
        match b {
            FBlock::Para(p) => {
                f(p);
                for i in &mut p.inlines {
                    each_inline_para(i, depth + 1, f);
                }
            }
            FBlock::Table(t) => {
                for c in t.rows.iter_mut().flatten() {
                    each_para(&mut c.blocks, depth + 1, f);
                }
            }
        }
    }
}

/// [`each_para`] in the paragraphs `i` holds: a note's, a comment's, a text box's, or those of
/// the text boxes in a group.
fn each_inline_para(i: &mut model::Inline, depth: usize, f: &mut dyn FnMut(&mut Para)) {
    if depth > model::MAX_DEPTH {
        return;
    }
    match i {
        model::Inline::Note { blocks, .. } | model::Inline::Shape { blocks, .. } => each_para(blocks, depth, f),
        model::Inline::Comment { comment, .. } => each_para(&mut comment.blocks, depth, f),
        model::Inline::Group { members, .. } => {
            for m in members {
                each_inline_para(&mut m.inline, depth + 1, f);
            }
        }
        _ => {}
    }
}

/// The custom character styles the text of `blocks` names (by ODF name, in [`model::Fmt::extra`])
/// with their custom parents, parents first, each with its document style id, added to `found`;
/// the text then names them by id. `para_ids` are the paragraph styles' ids, which a character
/// style's id doesn't take.
fn custom_char_styles(blocks: &mut [FBlock], styles: &Styles, para_ids: &[String], found: &mut Vec<(String, String)>) {
    let id_for = |odt: &str| {
        let id = super::styles::style_id(odt);
        if para_ids.contains(&id) { format!("{id}Char") } else { id }
    };
    each_para(blocks, 0, &mut |p| {
        for i in &mut p.inlines {
            let fmt = match i {
                model::Inline::Text(_, f) | model::Inline::Field { fmt: f, .. } | model::Inline::Ruby { fmt: f, .. } => f,
                _ => continue,
            };
            let Some(odt) = fmt.extra.as_mut().and_then(|x| x.style.take()) else { continue };
            let mut chain: Vec<String> = Vec::new();
            let mut cur = Some(odt.clone());
            while let Some(n) = cur.take() {
                if found.iter().any(|(o, _)| *o == n) || chain.contains(&n) || chain.len() >= 16 {
                    break;
                }
                cur = styles.named_text(&n).and_then(|(_, parent, ..)| parent);
                chain.push(n);
            }
            for n in chain.into_iter().rev() {
                let id = id_for(&n);
                found.push((n, id));
            }
            if let Some(x) = fmt.extra.as_mut() {
                x.style = found.iter().find(|(o, _)| *o == odt).map(|(_, id)| id.clone());
            }
        }
    });
}

/// Add the custom character styles `found` (ODF name, document id; parents first) built from
/// `defs` (name, parent's ODF name, own formatting), alongside.
fn add_char_styles(doc: &mut Document, found: &[(String, String)], defs: &[(String, Option<String>, model::Fmt, [bool; 4])]) {
    for ((_, id), (name, parent, fmt, off)) in found.iter().zip(defs) {
        let based_on = parent.as_ref().and_then(|p| found.iter().find(|(o, _)| o == p).map(|(_, id)| id.clone())).filter(|b| b != id);
        Arc::make_mut(&mut doc.styles).upsert(wordcraft_doc::styles::Style {
            id: id.clone(),
            name: name.clone(),
            kind: wordcraft_doc::styles::StyleKind::Character,
            based_on,
            chr: turned_off(model::char_props(fmt), *off),
            ..Default::default()
        });
    }
}

/// The custom styles the paragraphs of `blocks` name (by ODF name, [`super::styles::ParaLook::named`])
/// with their parents, parents first; the paragraphs then name them by document style id.
/// `odt` holds the styles already found (in other parts), which aren't added again.
fn custom_styles(blocks: &mut [FBlock], styles: &Styles, odt: Vec<String>) -> (Vec<String>, Vec<super::styles::NamedStyle>) {
    let mut odt = odt;
    let mut defs: Vec<super::styles::NamedStyle> = Vec::new();
    each_para(blocks, 0, &mut |p| {
        let Some(name) = p.extra.as_mut().and_then(|x| x.style.take()) else { return };
        // The style and its custom parents, parents first, each once (a loop ends the chain).
        let mut chain: Vec<(String, super::styles::NamedStyle)> = Vec::new();
        let mut cur = Some(name.clone());
        while let Some(n) = cur.take() {
            if odt.contains(&n) || chain.iter().any(|(m, _)| *m == n) || chain.len() >= 16 {
                break;
            }
            let Some(def) = styles.named_style(&n) else { break };
            if let super::styles::Parent::Named(p) = &def.parent {
                cur = Some(p.clone());
            }
            chain.push((n, def));
        }
        for (n, def) in chain.into_iter().rev() {
            odt.push(n);
            defs.push(def);
        }
        // A custom style, or an index style WordCraft has under its own id (TOC 1…).
        if (odt.contains(&name) || super::writer::generating_style(&name).is_some())
            && let Some(x) = p.extra.as_mut()
        {
            x.style = Some(super::styles::style_id(&name));
        }
    });
    (odt, defs)
}

/// Add the custom styles `named` (ODF names `odt`, parents first) to the document: each with its
/// name, the style it is based on and its own formatting. A style WordCraft has under the same id
/// takes the file's definition.
fn add_custom_styles(doc: &mut Document, named: &[super::styles::NamedStyle], odt: &[String], tabs_from_indent: bool) {
    use super::styles::{Parent, style_id};
    for (def, name) in named.iter().zip(odt) {
        let id = style_id(name);
        let based_on = match &def.parent {
            Parent::Normal => "Normal".to_string(),
            Parent::Kind(model::Kind::Heading(n)) => format!("Heading{}", (*n).clamp(1, 6)),
            Parent::Kind(model::Kind::Title) => "Title".into(),
            Parent::Kind(model::Kind::Quote) => "Quote".into(),
            Parent::Kind(model::Kind::Code) if doc.styles.get(model::CODE_STYLE).is_some() => model::CODE_STYLE.into(),
            Parent::Kind(_) => "Normal".into(),
            Parent::Named(p) => style_id(p),
        };
        let look = &def.look;
        let sp = &look.spacing;
        let para = wordcraft_doc::props::ParaProps {
            space_before: sp.before,
            space_after: sp.after,
            line_spacing: sp.line,
            indent_left: sp.left,
            indent_right: sp.right,
            indent_first: sp.first,
            align: look.align,
            keep_next: look.keep_next.then_some(true),
            borders: look.borders,
            shading: look.shading,
            // LibreOffice measures tabs from the indent (`TabsRelativeToIndent`); Word from the
            // margin.
            tabs: look.tabs.clone().map(|mut tabs| {
                if tabs_from_indent {
                    let indent = sp.left.unwrap_or(0.0);
                    for t in &mut tabs {
                        t.pos = (t.pos + indent).clamp(-1584.0, 1584.0);
                    }
                }
                tabs
            }),
            contextual_spacing: look.contextual,
            bidi: look.rtl.filter(|r| *r),
            ..Default::default()
        };
        let name = super::writer::generating_style(name).map_or_else(|| def.name.clone(), |(_, n)| n);
        let style = wordcraft_doc::styles::Style {
            id: id.clone(),
            name,
            kind: wordcraft_doc::styles::StyleKind::Paragraph,
            based_on: (based_on != id).then_some(based_on),
            para,
            chr: turned_off(model::char_props(&look.fmt), def.off),
            ..Default::default()
        };
        Arc::make_mut(&mut doc.styles).upsert(style);
    }
}

/// `chr` with what a style turns off (`off`: bold, italic, underline, strike-through) said so:
/// otherwise the style it is based on would show through (LibreOffice's agenda dates, not bold
/// under bold titles).
fn turned_off(mut chr: wordcraft_doc::props::CharProps, off: [bool; 4]) -> wordcraft_doc::props::CharProps {
    let [bold, italic, underline, strike] = off;
    if bold {
        chr.bold = Some(false);
    }
    if italic {
        chr.italic = Some(false);
    }
    if underline {
        chr.underline = Some(wordcraft_doc::props::Underline::None);
    }
    if strike {
        chr.strike = Some(false);
    }
    chr
}

/// Give the document's style for paragraph kind `kind` the look the file gives its own: its text
/// formatting, spacing, alignment and keep with next replace WordCraft's (a heading the file
/// leaves black isn't blue). Its outline level and other settings stay.
fn apply_look(doc: &mut Document, kind: model::Kind, look: &super::styles::ParaLook) {
    let id = match kind {
        model::Kind::Heading(n) => format!("Heading{}", n.clamp(1, 6)),
        model::Kind::Title => "Title".into(),
        model::Kind::Quote => "Quote".into(),
        model::Kind::Code => model::CODE_STYLE.into(),
        model::Kind::Normal | model::Kind::Rule => return,
    };
    let Some(st) = Arc::make_mut(&mut doc.styles).get_mut(&id) else { return };
    st.chr = model::char_props(&look.fmt);
    let sp = &look.spacing;
    let p = &mut st.para;
    (p.space_before, p.space_after, p.line_spacing) = (sp.before, sp.after, sp.line);
    (p.indent_left, p.indent_right, p.indent_first) = (sp.left, sp.right, sp.first);
    p.align = look.align;
    p.keep_next = look.keep_next.then_some(true);
    if look.contextual.is_some() {
        p.contextual_spacing = look.contextual;
    }
}

/// Number the headings as the outline style `levels` does (ODF 1.3 §16.34): one list whose
/// levels carry the outline's formats, each linked to its heading style, which numbers its
/// paragraphs through it, as Word's numbered headings do. Levels without a number are left so.
fn number_headings(doc: &mut Document, levels: &[super::styles::ListLevel]) {
    use wordcraft_doc::numbering::{AbstractNum, ListKind, Num, levels_for};
    use wordcraft_doc::props::NumRef;
    let numbered: Vec<bool> = (0..6).map(|i| levels.get(i).is_some_and(|l| l.ordered)).collect();
    if !numbered.contains(&true) {
        return;
    }
    let mut out = levels_for(ListKind::Numbered);
    for (i, level) in out.iter_mut().enumerate() {
        let src = levels.get(i).copied().unwrap_or_default();
        // Headings sit where the outline level puts them, else at the margin; their number is
        // followed by a tab.
        (level.indent, level.hanging) = src.label.and_then(|l| l.position()).unwrap_or((0.0, 0.0));
        if let Some(a) = src.label.and_then(|l| l.align) {
            level.align = a;
        }
        match src.label.filter(|_| src.ordered) {
            Some(l) => {
                level.format = l.format;
                level.text = l.text(i as u8);
                level.start = src.start;
            }
            None => {
                level.format = wordcraft_doc::section::NumFormat::None;
                level.text = String::new();
            }
        }
        if i < 6 {
            level.style = Some(format!("Heading{}", i + 1));
        }
    }
    let numbering = Arc::make_mut(&mut doc.numbering);
    let aid = numbering.abstracts.iter().map(|a| a.id + 1).max().unwrap_or(0);
    numbering.abstracts.push(AbstractNum { id: aid, name: None, levels: out, ..Default::default() });
    let nid = numbering.nums.iter().map(|n| n.id + 1).max().unwrap_or(1).max(1);
    numbering.nums.push(Num { id: nid, abstract_id: aid, ..Default::default() });
    for (i, on) in numbered.iter().enumerate() {
        if *on && let Some(st) = Arc::make_mut(&mut doc.styles).get_mut(&format!("Heading{}", i + 1)) {
            st.para.numbering = Some(NumRef { num: nid, level: i as u8 });
        }
    }
}

/// The boolean configuration item `name` of `settings.xml` (ODF 1.3 §3.10), when it has one.
fn setting(xml: &[u8], name: &str) -> Option<bool> {
    let mut inside = false;
    let mut value = None;
    events(xml, |ev| match ev {
        Ev::Start(q, a) => inside = q == "config:config-item" && get(&a, "config:name") == Some(name) && value.is_none(),
        Ev::Text(t) if inside => value = Some(t.trim() == "true"),
        _ => inside = false,
    });
    value
}

/// Parse an `.odt` package into the flow model.
pub fn parse(bytes: &[u8]) -> Result<Flow, String> {
    parse_full(bytes).map(|(flow, _)| flow)
}

/// [`parse`], with the last section's properties (the others are on the paragraphs that end them).
fn parse_full(bytes: &[u8]) -> Result<(Flow, SectionProps), String> {
    parse_tracked(bytes, default_paper(), &OdfProfile::default()).map(|p| (p.flow, p.last))
}

/// The paragraphs kept as read that their links tie together (see `model::Group`), in `blocks`:
/// each group given its id and size when every link it needs is there, its paragraphs written
/// from the model otherwise (a comment's end in a paragraph not kept, say), and a run kept
/// whole (a list) with them.
fn resolve_groups(blocks: &mut [FBlock]) {
    // The linked paragraphs, in order, and the paragraphs holding each key.
    let mut links: Vec<Vec<(String, bool)>> = Vec::new();
    walk_grouped(blocks, 0, &mut |g| links.push(g.links.clone()));
    if links.is_empty() {
        return;
    }
    let mut holders: std::collections::HashMap<&str, Vec<usize>> = std::collections::HashMap::new();
    for (i, l) in links.iter().enumerate() {
        for (k, _) in l {
            let h = holders.entry(k.as_str()).or_default();
            if h.last() != Some(&i) {
                h.push(i);
            }
        }
    }
    // Groups: paragraphs sharing a key one of them needs.
    let mut root: Vec<usize> = (0..links.len()).collect();
    fn find(root: &mut [usize], mut i: usize) -> usize {
        while let Some(&r) = root.get(i) {
            if r == i {
                break;
            }
            i = r;
        }
        i
    }
    let needed: std::collections::HashSet<&str> = links.iter().flatten().filter(|(_, n)| *n).map(|(k, _)| k.as_str()).collect();
    for (k, h) in &holders {
        if needed.contains(k)
            && let Some(&first) = h.first()
        {
            for &i in h.iter().skip(1) {
                let (a, b) = (find(&mut root, first), find(&mut root, i));
                if let Some(r) = root.get_mut(b) {
                    *r = a;
                }
            }
        }
    }
    // A group is whole when each key one of its paragraphs needs another holds.
    let mut whole: std::collections::HashMap<usize, bool> = std::collections::HashMap::new();
    let mut size: std::collections::HashMap<usize, u32> = std::collections::HashMap::new();
    for (i, l) in links.iter().enumerate() {
        let r = find(&mut root, i);
        *size.entry(r).or_default() += 1;
        let ok = l.iter().filter(|(_, n)| *n).all(|(k, _)| holders.get(k.as_str()).is_some_and(|h| h.len() >= 2));
        let w = whole.entry(r).or_insert(true);
        *w &= ok;
    }
    let mut ids: std::collections::HashMap<usize, u32> = std::collections::HashMap::new();
    let mut verdicts: Vec<Option<Option<(u32, u32)>>> = Vec::with_capacity(links.len());
    for i in 0..links.len() {
        let r = find(&mut root, i);
        let s = size.get(&r).copied().unwrap_or(1);
        let needs = links.get(i).is_some_and(|l| l.iter().any(|(_, n)| *n));
        verdicts.push(match (whole.get(&r).copied().unwrap_or(false), s) {
            // Alone, needing nothing: its own (what it has, nobody needed).
            (true, 1) if !needs => Some(None),
            (true, s) if s > 1 => {
                let next = u32::try_from(ids.len()).unwrap_or(u32::MAX);
                Some(Some((*ids.entry(r).or_insert(next), s)))
            }
            _ => None,
        });
    }
    let mut at = 0usize;
    apply_groups(blocks, 0, &verdicts, &mut at);
}

/// Each paragraph kept as read with links in `blocks` (its [`model::Group`]), in order.
fn walk_grouped(blocks: &[FBlock], depth: usize, f: &mut dyn FnMut(&crate::model::Group)) {
    for b in blocks {
        match b {
            FBlock::Para(p) => {
                if let (Some(_), Some(g)) = (&p.kept, &p.group) {
                    f(g);
                }
            }
            FBlock::Table(t) if depth < crate::model::MAX_DEPTH => {
                for c in t.rows.iter().flatten() {
                    walk_grouped(&c.blocks, depth + 1, f);
                }
            }
            FBlock::Table(_) => {}
        }
    }
}

/// The groups' `verdicts` (in [`walk_grouped`]'s order, from `at`) applied to `blocks`: a group's
/// id and size, none, or the paragraph written from the model (with a run kept whole holding it).
fn apply_groups(blocks: &mut [FBlock], depth: usize, verdicts: &[Option<Option<(u32, u32)>>], at: &mut usize) {
    let mut dropped = Vec::new();
    for (i, b) in blocks.iter_mut().enumerate() {
        match b {
            FBlock::Para(p) => {
                if p.kept.is_some() && p.group.is_some() {
                    match verdicts.get(*at).copied().flatten() {
                        Some(None) => p.group = None,
                        Some(Some(id)) => {
                            if let Some(g) = &mut p.group {
                                g.links.clear();
                                g.id = Some(id);
                            }
                        }
                        None => {
                            (p.kept, p.group) = (None, None);
                            dropped.push(i);
                        }
                    }
                    *at += 1;
                } else if p.kept.is_none() {
                    p.group = None;
                }
            }
            FBlock::Table(t) if depth < crate::model::MAX_DEPTH => {
                for c in t.rows.iter_mut().flatten() {
                    apply_groups(&mut c.blocks, depth + 1, verdicts, at);
                }
            }
            FBlock::Table(_) => {}
        }
    }
    // A run kept whole (a list) holding a paragraph written from the model is too.
    if !dropped.is_empty() {
        for (i, b) in blocks.iter_mut().enumerate() {
            if let FBlock::Para(p) = b
                && let Some(ix) = p.index.as_deref()
                && dropped.iter().any(|d| *d >= i && *d < i + ix.1)
            {
                p.index = None;
            }
        }
    }
}

/// The markup kept as read in `blocks` (paragraphs', tables', the paragraphs' in tables not
/// kept whole) that links to package files, added to `out`.
fn kept_markup(blocks: &[FBlock], depth: usize, out: &mut Vec<(String, std::sync::Arc<str>)>) {
    for b in blocks {
        match b {
            FBlock::Para(p) => out.extend(p.kept.iter().filter(|x| x.contains("xlink:href")).map(|x| (String::new(), x.clone()))),
            FBlock::Table(t) => match &t.kept {
                Some(x) => out.extend(x.contains("xlink:href").then(|| (String::new(), x.clone()))),
                None if depth < crate::model::MAX_DEPTH => {
                    for c in t.rows.iter().flatten() {
                        kept_markup(&c.blocks, depth + 1, out);
                    }
                }
                None => {}
            },
        }
    }
}

/// Drop the markup kept as read (of paragraphs and tables, as [`kept_markup`] finds it) that
/// links to one of the package files in `missing`.
fn unkeep_linking(blocks: &mut [FBlock], depth: usize, missing: &std::collections::BTreeSet<String>) {
    if missing.is_empty() {
        return;
    }
    for b in blocks {
        match b {
            FBlock::Para(p) => {
                if p.kept.as_deref().is_some_and(|x| super::kept::links_to(x, missing)) {
                    p.kept = None;
                }
            }
            FBlock::Table(t) => {
                if t.kept.as_deref().is_some_and(|x| super::kept::links_to(x, missing)) {
                    t.kept = None;
                }
                if t.kept.is_none() && depth < crate::model::MAX_DEPTH {
                    for c in t.rows.iter_mut().flatten() {
                        unkeep_linking(&mut c.blocks, depth + 1, missing);
                    }
                }
            }
        }
    }
}

/// What [`parse_tracked`] reads besides the flow.
struct Parsed {
    flow: Flow,
    /// Who changed it last, when, and how often it was saved (`meta.xml`).
    changes: (String, String, u32),
    /// The style and font definitions kept paragraphs may refer to (see `super::kept`).
    kept_defs: Vec<(String, std::sync::Arc<str>)>,
    /// The package files those definitions refer to.
    kept_files: Vec<(String, Vec<u8>)>,
    /// Each section's master page as the file named it.
    section_masters: Vec<Option<String>>,
    /// The space between paragraphs is their spacings' sum (else the larger).
    sum_spacing: bool,
    /// The last section's properties.
    last: SectionProps,
    /// Whether change tracking is on (`None`: the document has no tracked changes).
    track: Option<bool>,
    /// The document's default font.
    font: Option<String>,
    /// The document's default font size, points.
    size: Option<f32>,
    /// The document's default text colour.
    color: Option<wordcraft_doc::props::Rgb>,
    /// The custom paragraph styles the paragraphs use, parents included, parents first.
    named: Vec<super::styles::NamedStyle>,
    /// Their ODF names, alongside.
    named_odt: Vec<String>,
    /// The custom character styles the text uses: ODF name and document id, parents first.
    char_styles: Vec<(String, String)>,
    /// Their names, parents and own formatting, alongside.
    char_defs: Vec<(String, Option<String>, model::Fmt, [bool; 4])>,
    /// The file's `settings.xml`, kept for the next save.
    settings: Option<Vec<u8>>,
    /// The heading numbering (`text:outline-style`), level by level.
    outline: Option<Vec<super::styles::ListLevel>>,
    /// How the file's heading, title, quotation and preformatted styles look.
    looks: Vec<(model::Kind, super::styles::ParaLook)>,
    /// The document's default paragraph spacing.
    spacing: model::Spacing,
    /// Whether the document kerns pairs of letters by default.
    kern: bool,
    /// Widow and orphan control the document states for every paragraph, if it does.
    widows: Option<bool>,
    /// The document's default Asian and complex-script fonts and languages.
    scripts: wordcraft_doc::props::CharProps,
    /// The default tab interval, points.
    default_tab: Option<f32>,
    /// Tab stops are measured from the indent (LibreOffice's `TabsRelativeToIndent`).
    tabs_from_indent: bool,
    /// The page settings that are the document's.
    page_doc: PageDoc,
    parts: page_parts::Parts,
    /// Some paragraph hyphenates: the document hyphenates (see [`hyphenation`]).
    hyphenate: bool,
    /// Custom document properties (`meta:user-defined`) and user fields.
    custom: Vec<wordcraft_doc::CustomProp>,
    /// The bibliography sources the citations carry.
    sources: Vec<wordcraft_doc::Source>,
    /// The package's parts WordCraft doesn't read itself (macros…), kept for the next save.
    kept: super::parts::Kept,
    /// Hidden form controls, kept for the next save.
    hidden_controls: Vec<String>,
    /// The page background picture's bytes and fill.
    page_image: Option<(Vec<u8>, &'static str)>,
}

/// Hyphenation is a paragraph's in ODF (`fo:hyphenate`) and the document's in WordCraft (as in
/// Word): when any paragraph of `blocks` hyphenates (its `suppress_hyphens` read as
/// `Some(false)`), the document does, and every other paragraph opts out; otherwise nothing is
/// marked. Whether the document hyphenates.
fn hyphenation(blocks: &mut [FBlock]) -> bool {
    let mut any = false;
    each_para(blocks, 0, &mut |p| any |= p.extra.as_ref().is_some_and(|x| x.suppress_hyphens == Some(false)));
    each_para(blocks, 0, &mut |p| {
        let hyphenates = p.extra.as_ref().is_some_and(|x| x.suppress_hyphens == Some(false));
        if any && !hyphenates {
            p.extra.get_or_insert_with(Default::default).suppress_hyphens = Some(true);
        } else if let Some(x) = &mut p.extra {
            x.suppress_hyphens = None;
        }
    });
    any
}

/// The content of a package that has none: an empty text.
const EMPTY_CONTENT: &str = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:text><text:p/></office:text></office:body></office:document-content>"#;

/// The page size of a page layout that states none: WordCraft's default page's.
fn default_paper() -> (f32, f32) {
    let d = SectionProps::default();
    (d.page_w, d.page_h)
}

/// [`parse_full`], plus what the document holds outside the flow. A page layout that states no
/// size (§20.215 `fo:page-width` is optional) is `paper` (width, height) in size.
fn parse_tracked(bytes: &[u8], paper: (f32, f32), profile: &OdfProfile) -> Result<Parsed, String> {
    wordcraft_doc::zip_guard::check(bytes, crate::MAX_ZIP_ENTRIES).map_err(|e| format!("ODT: {e}"))?;
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("ODT: not a valid package ({e})"))?;
    if let Some(mt) = read_entry(&mut z, "mimetype") {
        let mt = String::from_utf8_lossy(&mt);
        if !mt.trim().starts_with("application/vnd.oasis.opendocument.text") {
            // Its media type named only when it reads as one (a damaged package's may be a picture).
            let mt = mt.trim();
            let shown = mt.len() <= 100 && mt.chars().all(|c| c.is_ascii_graphic());
            return Err(if shown { format!("ODT: not a text document ({mt})") } else { "ODT: not a text document".to_string() });
        }
    }
    let styles_xml = read_entry(&mut z, "styles.xml");
    // A template of styles only (LibreOffice's style templates) has no content: an empty text.
    let content = match read_entry(&mut z, "content.xml") {
        Some(c) => c,
        None if styles_xml.is_some() => EMPTY_CONTENT.as_bytes().to_vec(),
        None => return Err("ODT: content.xml is missing".into()),
    };
    let profile = profile.effective();
    let mut styles = Styles::with_profile(&profile);
    let mut pages = Pages::default();
    if let Some(s) = &styles_xml {
        styles.read(s, false);
        pages = read_pages(s, paper, &profile);
    }
    // No page layout at all (no styles.xml, or no master page): the default paper, with the
    // profile's margins.
    if pages.first.is_none() {
        pages.first = Some(SectionProps {
            page_w: paper.0,
            page_h: paper.1,
            margin_top: profile.page_margin,
            margin_bottom: profile.page_margin,
            margin_left: profile.page_margin,
            margin_right: profile.page_margin,
            ..SectionProps::default()
        });
    }
    styles.read(&content, true);
    let settings = read_entry(&mut z, "settings.xml");
    // Where tab stops are measured from: as the document's settings say (LibreOffice's own), else
    // the profile's choice (the standard ties it only to indexes, §19.512.3).
    styles.tabs_from_indent = settings.as_deref().and_then(|s| setting(s, "TabsRelativeToIndent")).unwrap_or(profile.tabs_from_indent);
    // How the space between two paragraphs adds up: as LibreOffice's `AddParaTableSpacing` says
    // (false: the larger of the two, as Word), else the profile's choice (the standard doesn't say).
    let sum_spacing = settings.as_deref().and_then(|s| setting(s, "AddParaTableSpacing")).unwrap_or(profile.sum_paragraph_spacing);
    let meta_xml = read_entry(&mut z, "meta.xml");
    let meta = meta_xml.as_deref().map(read_meta).unwrap_or_default();
    let changes = meta_xml.as_deref().map(read_changes).unwrap_or_default();
    // Custom document properties, then the user fields not among them.
    let mut custom = meta_xml.as_deref().map(super::fields::meta_props).unwrap_or_default();
    for f in super::fields::user_fields(&content) {
        if custom.len() < super::fields::MAX_PROPS && !custom.iter().any(|c| c.name == f.name) {
            custom.push(f);
        }
    }
    let entries: Vec<String> = z.file_names().take(100_000).map(str::to_string).collect();
    let manifest = read_entry(&mut z, "META-INF/manifest.xml").map(|m| super::objects::manifest(&m)).unwrap_or_default();
    let kept = super::parts::read(&mut z, &entries, &manifest);
    let mut pictures = Pictures::new(MAX_ENTRY);
    let mut fetch = |path: &str| -> Option<Vec<u8>> {
        // A frame may point at any entry, as often as it likes: `Pictures` keeps that bounded.
        pictures.get(&mut z, path)
    };
    // The page background picture's bytes.
    let page_image = pages.doc.page_image.clone().and_then(|(href, fill)| fetch(&href).map(|b| (b, fill)));
    let objects = (entries.as_slice(), &manifest);
    // One `text:s` budget for the headers and footers and then the body.
    let mut spaces_left = MAX_EXTRA_SPACES;
    let mut parts =
        styles_xml.as_deref().map(|x| page_parts::read(x, &styles, &mut pages, &mut fetch, objects, &mut spaces_left)).unwrap_or_default();
    first_pages(&mut pages);
    let mut body = Body::new(&styles, &pages, &mut fetch);
    (body.entries, body.manifest) = (objects.0, Some(objects.1));
    body.raw = &content;
    body.spaces_left = spaces_left;
    super::xml::events_at(&content, |ev, at| body.feed_at(ev, at));
    let mut blocks = body.take_blocks();
    // The file's own definitions: what kept paragraphs refer to, and the style definitions
    // written as read while they stay unchanged.
    let mut kept_defs = super::kept::defs(&content, true);
    kept_defs.extend(meta_xml.as_deref().map(super::kept::meta_defs).unwrap_or_default());
    kept_defs.extend(styles_xml.as_deref().map(|x| super::kept::defs(x, false)).unwrap_or_default());
    let hidden_controls = body.take_hidden_controls();
    let (mut named_odt, mut named) = custom_styles(&mut blocks, &styles, Vec::new());
    for b in parts.blocks_mut() {
        let (o, n) = custom_styles(b, &styles, named_odt);
        named_odt = o;
        named.extend(n);
    }
    let para_ids: Vec<String> = named_odt.iter().map(|n| super::styles::style_id(n)).collect();
    let mut char_styles = Vec::new();
    custom_char_styles(&mut blocks, &styles, &para_ids, &mut char_styles);
    for b in parts.blocks_mut() {
        custom_char_styles(b, &styles, &para_ids, &mut char_styles);
    }
    let char_defs = char_styles.iter().filter_map(|(o, _)| styles.named_text(o)).collect();
    // An open section with nothing in it (after a column section at the end) isn't one: the
    // section before it ends the document.
    let mut last = body.sec;
    if blocks.len() <= body.sec_from
        && let Some(FBlock::Para(p)) = blocks.last_mut()
        && let Some(s) = p.section.take()
    {
        last = *s;
    }
    if blocks.is_empty() {
        blocks.push(FBlock::Para(Para::default()));
    }
    let hyphenate = hyphenation(&mut blocks);
    let track = body.track;
    let sources = std::mem::take(&mut body.sources);
    resolve_groups(&mut blocks);
    let section_masters = std::mem::take(&mut body.section_masters);
    // And the pictures paragraphs and tables kept as read show.
    let mut linking: Vec<(String, std::sync::Arc<str>)> = kept_defs.clone();
    kept_markup(&blocks, 0, &mut linking);
    for b in parts.blocks_mut() {
        kept_markup(b, 0, &mut linking);
    }
    let (kept_files, missing) = super::kept::files(&linking, &entries, &mut fetch);
    // Markup linking to a file that couldn't be read is written from the model: as read, it
    // would point at a file the saved package hasn't got.
    unkeep_linking(&mut blocks, 0, &missing);
    for b in parts.blocks_mut() {
        unkeep_linking(b, 0, &missing);
    }
    // Their manifest entries, and their folders' (an object's says what it is).
    for (path, _) in &kept_files {
        let dir = path.rsplit_once('/').map(|(d, _)| format!("{d}/"));
        for p in std::iter::once(path.clone()).chain(dir) {
            if let Some(mime) = manifest.get(&p).or_else(|| manifest.get(p.trim_end_matches('/'))) {
                kept_defs.push((format!("manifest:{p}"), std::sync::Arc::from(mime.as_str())));
            }
        }
    }
    Ok(Parsed {
        flow: Flow { blocks, meta },
        last,
        track,
        font: styles.doc_font(),
        size: styles.doc_size(),
        color: styles.doc_color(),
        spacing: styles.doc_spacing(),
        looks: styles.kind_looks(),
        named,
        named_odt,
        char_styles,
        char_defs,
        settings: settings.filter(|s| s.len() <= super::package::MAX_SETTINGS),
        outline: styles.outline.clone(),
        kern: styles.default_kern,
        widows: styles.stated_widows(),
        scripts: styles.default_scripts.clone(),
        default_tab: styles.default_tab,
        tabs_from_indent: styles.tabs_from_indent,
        page_doc: pages.doc.clone(),
        parts,
        hyphenate,
        custom,
        sources,
        kept,
        hidden_controls,
        page_image,
        sum_spacing,
        changes,
        kept_defs,
        kept_files,
        section_masters,
    })
}

/// Parse an `.odt` package into a document.
pub fn import(bytes: &[u8]) -> Result<Document, String> {
    import_on(bytes, &SectionProps::default())
}

/// [`import`], with `page` the default page (the user's region's): a page layout that states no
/// size is `page`'s size, as LibreOffice makes it its default paper's.
pub fn import_on(bytes: &[u8], page: &SectionProps) -> Result<Document, String> {
    import_with(bytes, page, &OdfProfile::default())
}

/// [`import_on`], filling in what the standard leaves open as `profile` says (see
/// [`wordcraft_doc::odf_profile`]).
pub fn import_with(bytes: &[u8], page: &SectionProps, profile: &OdfProfile) -> Result<Document, String> {
    model::begin_import();
    let Parsed {
        flow,
        last,
        track,
        font,
        size,
        color,
        spacing,
        looks,
        named,
        named_odt,
        char_styles,
        char_defs,
        settings,
        outline,
        kern,
        widows,
        scripts,
        default_tab,
        tabs_from_indent,
        page_doc,
        parts,
        hyphenate,
        custom,
        sources,
        kept,
        hidden_controls,
        page_image,
        sum_spacing,
        changes,
        kept_defs,
        kept_files,
        section_masters,
    } = parse_tracked(bytes, (page.page_w, page.page_h), profile)?;
    let mut doc = model::to_doc(&flow);
    kept.store(&mut doc);
    // Elements can't hold a NUL: it parts them.
    if !hidden_controls.is_empty() {
        doc.passthrough.insert(super::forms::HIDDEN_PART.into(), std::sync::Arc::new(hidden_controls.join("\0").into_bytes()));
    }
    doc.custom_props = custom;
    (doc.core.last_modified_by, doc.core.modified, doc.core.revision) = changes;
    doc.sources = sources;
    doc.settings.auto_hyphenation |= hyphenate;
    if font.is_some() {
        Arc::make_mut(&mut doc.styles).default_chr.font = font;
    }
    if size.is_some() {
        Arc::make_mut(&mut doc.styles).default_chr.size = size;
    }
    if let Some(c) = color {
        Arc::make_mut(&mut doc.styles).default_chr.color = Some(wordcraft_doc::props::TextColor::Rgb(c));
    }
    Arc::make_mut(&mut doc.styles).default_chr.kern = kern.then_some(1.0);
    Arc::make_mut(&mut doc.styles).default_chr.overlay(&scripts);
    if let Some(t) = default_tab {
        doc.settings.default_tab = t;
    }
    doc.settings.page_color = page_doc.page_color;
    // Laid out with the standard's rules, and the profile's choices made as it is laid out.
    doc.settings.odf = Some(wordcraft_doc::odf_profile::OdfLayout { tabs_from_indent, ..profile.layout() });
    doc.settings.sum_paragraph_spacing = sum_spacing;
    // The page background picture (Design › Page Color › Fill Effects › Picture).
    if let Some((bytes, fill)) = page_image
        && let Some(ext) = model::sniff_image(&bytes)
    {
        let media = doc.add_media(bytes, ext);
        doc.settings.page_picture = Some(wordcraft_doc::PagePicture { media, fill_type: fill.to_string(), title: String::new() });
    }
    if let Some((fmt, props)) = page_doc.notes[0] {
        (doc.settings.footnote_format, doc.settings.footnote_pr) = (fmt, props);
    }
    if let Some((fmt, props)) = page_doc.notes[1] {
        (doc.settings.endnote_format, doc.settings.endnote_pr) = (fmt, props);
    }
    let d = &mut Arc::make_mut(&mut doc.styles).default_para;
    (d.space_before, d.space_after, d.line_spacing) = (spacing.before, spacing.after, spacing.line);
    (d.indent_left, d.indent_right, d.indent_first) = (spacing.left, spacing.right, spacing.first);
    // Unstated, as the profile says (`widowControl`).
    d.widow_control = Some(widows.unwrap_or(profile.widow_control));
    for (kind, look) in &looks {
        apply_look(&mut doc, *kind, look);
    }
    if let Some(s) = settings {
        doc.passthrough.insert(super::package::SETTINGS_PART.into(), std::sync::Arc::new(s));
    }
    add_custom_styles(&mut doc, &named, &named_odt, tabs_from_indent);
    add_char_styles(&mut doc, &char_styles, &char_defs);
    if let Some(levels) = &outline {
        number_headings(&mut doc, levels);
    }
    doc.last_section = last;
    page_parts::add(&mut doc, parts);
    if let Some(t) = track {
        doc.settings.track_changes = t;
    }
    // The markup kept for paragraphs nothing changed while reading, filed for the writer.
    if doc.kept.get().is_none() {
        doc.kept = wordcraft_doc::kept::Kept::new(Default::default());
    }
    if let Some(k) = doc.kept.get_mut() {
        k.format = "odt".into();
        for (key, xml) in kept_defs {
            k.keep_def(key, xml);
        }
        for (path, bytes) in kept_files {
            k.keep_file(path, bytes);
        }
        k.section_masters = section_masters;
    }
    doc.seal_kept();
    let (fp, pages) = (wordcraft_doc::kept::styles_fingerprint(&doc), wordcraft_doc::kept::pages_fingerprint(&doc));
    if let Some(k) = doc.kept.get_mut() {
        (k.styles_fp, k.pages_fp) = (Some(fp), Some(pages));
    }
    Ok(doc)
}

/// The page setup of each master page in `styles.xml` (ODF 1.3 §16.9), and of the first one,
/// which pages use until a paragraph names another.
#[derive(Default)]
pub(super) struct Pages {
    pub(super) masters: HashMap<String, SectionProps>,
    pub(super) first: Option<SectionProps>,
    pub(super) first_name: Option<String>,
    /// Each master page's next one (§19.511 `style:next-style-name`): the page style the pages
    /// after its first take.
    pub(super) next: HashMap<String, String>,
    /// What the document's first page style and its note and line numbering settings say for
    /// the whole document.
    pub(super) doc: PageDoc,
}

/// Page settings that are the document's in WordCraft (and Word): the page colour (the first
/// page style's background), footnote and endnote numbering (`text:notes-configuration`).
#[derive(Clone, Default)]
pub(super) struct PageDoc {
    pub(super) page_color: Option<wordcraft_doc::props::Rgb>,
    /// The first page style's background picture (§17.3 `style:background-image`): its package
    /// path and WordCraft's fill for its repeat (tiled, stretched, or centred at its size).
    pub(super) page_image: Option<(String, &'static str)>,
    /// Footnotes', then endnotes', number format and options.
    pub(super) notes: [Option<(wordcraft_doc::section::NumFormat, wordcraft_doc::section::NoteProps)>; 2],
}

/// A `text:notes-configuration` (ODF 1.3 §16.29): which notes it is for (endnotes: true), their
/// number format, first number (ODF counts from 0), restart, and footnotes at the page's foot
/// or the document's end.
fn notes_config(a: &Attrs) -> (bool, wordcraft_doc::section::NumFormat, wordcraft_doc::section::NoteProps) {
    use wordcraft_doc::section::{MAX_NOTE_START, NotePos, NoteProps, NoteRestart};
    let endnote = get(a, "text:note-class").is_some_and(|c| c.trim() == "endnote");
    let fmt = super::styles::num_format(get(a, "style:num-format").unwrap_or("1"));
    let num_start =
        get(a, "text:start-value").and_then(|v| v.trim().parse::<u64>().ok()).map(|v| v.saturating_add(1).min(u64::from(MAX_NOTE_START)) as u32);
    let num_restart = get(a, "text:start-numbering-at").map(|v| match v.trim() {
        "page" => NoteRestart::EachPage,
        "chapter" => NoteRestart::EachSect,
        _ => NoteRestart::Continuous,
    });
    let pos = match (endnote, get(a, "text:footnotes-position").map(str::trim)) {
        (false, Some("document")) => Some(NotePos::DocEnd),
        (false, Some("page")) => Some(NotePos::PageBottom),
        _ => None,
    };
    (endnote, fmt, NoteProps { pos, num_fmt: None, num_start, num_restart })
}

/// A `text:linenumbering-configuration` (ODF 1.3 §16.31) that numbers lines: every how many,
/// how far from the text, restarting on each page or not.
fn line_numbering(a: &Attrs) -> Option<wordcraft_doc::section::LineNumbering> {
    use wordcraft_doc::section::{LineNumberRestart, LineNumbering};
    // On unless it says otherwise (§19.845).
    if get(a, "text:number-lines").is_some_and(|v| v.trim() == "false") {
        return None;
    }
    let count_by = get(a, "text:increment").and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(1).clamp(1, 100);
    let distance = get(a, "text:offset").and_then(length).filter(|v| v.is_finite()).map_or(0.0, |v| v.clamp(0.0, 1584.0));
    let restart =
        if get(a, "text:restart-on-page").is_some_and(|v| v.trim() == "true") { LineNumberRestart::Page } else { LineNumberRestart::Continuous };
    Some(LineNumbering { count_by, start: 1, distance, restart })
}

/// A page layout's header or footer area (`style:header-style`/`style:footer-style`, ODF 1.3
/// §16.6–16.7): its minimum height and its spacing from the text, in points.
#[derive(Clone, Copy, Default)]
struct Band {
    /// Its height (`fo:min-height`), when stated: the standard gives no default (the profile's
    /// [`OdfProfile::header_min_height`]).
    height: Option<f32>,
    spacing: f32,
    /// Content taller than the band takes its spacing first (§20.264 `style:dynamic-spacing`,
    /// as Word does), instead of keeping it below; `None` when unstated (the profile's
    /// [`OdfProfile::dynamic_spacing`]).
    dynamic: Option<bool>,
}

impl Band {
    /// Where the text starts below the band's start (the page margin), and the gap kept below
    /// content taller than the band, as `profile` reads what the standard leaves open.
    fn room(&self, profile: &OdfProfile) -> (f32, f32) {
        let height = self.height.unwrap_or(profile.header_min_height);
        let room = if profile.header_height_includes_spacing { height.max(self.spacing) } else { height + self.spacing };
        let gap = if self.dynamic.unwrap_or(profile.dynamic_spacing) { 0.0 } else { self.spacing };
        (room, gap)
    }
}

/// A page layout read from `styles.xml`: its properties, columns, and header and footer areas.
#[derive(Default)]
struct Layout {
    attrs: Attrs,
    /// Its background picture: package path and fill (see [`PageDoc::page_image`]).
    image: Option<(String, &'static str)>,
    columns: Option<Columns>,
    column_specs: Vec<super::styles::ColumnSpec>,
    header: Band,
    footer: Band,
}

fn read_pages(xml: &[u8], paper: (f32, f32), profile: &OdfProfile) -> Pages {
    let mut layouts: HashMap<String, Layout> = HashMap::new();
    let mut cur: Option<String> = None;
    // Inside a layout's footer style (else its header style).
    let mut in_footer = false;
    // Master pages: name, layout, whether a header and a footer show.
    let mut masters: Vec<(String, String, bool, bool)> = Vec::new();
    let mut in_master = false;
    let mut nexts: HashMap<String, String> = HashMap::new();
    let mut doc = PageDoc::default();
    let mut lines = None;
    events(xml, |ev| match ev {
        Ev::Start(q, a) => match local(&q) {
            "notes-configuration" if q.starts_with("text:") => {
                let (endnote, fmt, props) = notes_config(&a);
                if let Some(slot) = doc.notes.get_mut(usize::from(endnote)) {
                    *slot = Some((fmt, props));
                }
            }
            "linenumbering-configuration" if q.starts_with("text:") => lines = line_numbering(&a),
            "page-layout" if q.starts_with("style:") => {
                cur = get(&a, "style:name").map(String::from);
                // A layout stating nothing is still one: the defaults' page.
                if let Some(n) = &cur
                    && layouts.len() < MAX_SECTIONS
                {
                    layouts.entry(n.clone()).or_default();
                }
            }
            // A page background picture (§17.3, in `style:page-layout-properties`): §20.249
            // `style:repeat` (repeat is the default) as WordCraft's tile, stretch, or centre.
            "background-image" if q.starts_with("style:") => {
                if let (Some(l), Some(href)) =
                    (cur.as_ref().and_then(|n| layouts.get_mut(n)), get(&a, "xlink:href").map(str::trim).filter(|h| !h.is_empty() && h.len() <= 1024))
                {
                    let fill = match get(&a, "style:repeat").map(str::trim) {
                        Some("no-repeat") => "center",
                        Some("stretch") => "frame",
                        _ => "tile",
                    };
                    l.image = Some((href.trim_start_matches("./").to_string(), fill));
                }
            }
            // A band whose height isn't stated is LibreOffice's default, 0.5 cm (measured; see
            // `Band::default`).
            "header-style" => in_footer = false,
            "footer-style" => in_footer = true,
            "header-footer-properties" => {
                if let Some(l) = cur.as_ref().and_then(|n| layouts.get_mut(n)) {
                    let len = |k: &str| get(&a, k).and_then(length).filter(|v| *v >= 0.0).map(|v| v.min(1584.0));
                    let band = if in_footer { &mut l.footer } else { &mut l.header };
                    band.height = len("fo:min-height").or_else(|| len("svg:height"));
                    band.spacing = len(if in_footer { "fo:margin-top" } else { "fo:margin-bottom" }).unwrap_or(0.0);
                    band.dynamic = get(&a, "style:dynamic-spacing").map(|v| v.trim() == "true");
                }
            }
            "page-layout-properties" => {
                if let Some(l) = cur.as_ref().and_then(|n| layouts.get_mut(n)) {
                    l.attrs = a;
                }
            }
            "columns" if q.starts_with("style:") => {
                if let Some(l) = cur.as_ref().and_then(|n| layouts.get_mut(n)) {
                    l.columns = Some(columns_of(&a, profile.column_gap));
                }
            }
            "column" if q.starts_with("style:") => {
                if let Some(l) = cur
                    .as_ref()
                    .and_then(|n| layouts.get_mut(n))
                    .filter(|l| l.columns.is_some() && l.column_specs.len() < super::styles::MAX_COLUMN_SPECS)
                    && let Some(spec) = super::styles::column_spec(&a)
                {
                    l.column_specs.push(spec);
                }
            }
            "column-sep" => {
                if let Some(c) = cur.as_ref().and_then(|n| layouts.get_mut(n)).and_then(|l| l.columns.as_mut()) {
                    c.separator = column_sep(&a);
                }
            }
            "master-page" if masters.len() < MAX_SECTIONS => {
                in_master = false;
                if let (Some(n), Some(l)) = (get(&a, "style:name"), get(&a, "style:page-layout-name")) {
                    masters.push((n.to_string(), l.to_string(), false, false));
                    in_master = true;
                    if let Some(next) = get(&a, "style:next-style-name").map(str::trim).filter(|x| !x.is_empty() && *x != n) {
                        nexts.insert(n.to_string(), next.to_string());
                    }
                }
            }
            "header" | "footer" if in_master && q.starts_with("style:") && get(&a, "style:display") != Some("false") => {
                if let Some(m) = masters.last_mut() {
                    if local(&q) == "header" {
                        m.2 = true;
                    } else {
                        m.3 = true;
                    }
                }
            }
            _ => {}
        },
        Ev::End(q) => match local(&q) {
            "page-layout" => cur = None,
            "master-page" => in_master = false,
            _ => {}
        },
        Ev::Text(_) => {}
    });
    let mut pages = Pages { doc, next: nexts, ..Default::default() };
    for (name, layout, header, footer) in masters {
        let Some(Layout { attrs: a, columns: cols, column_specs, header: hb, footer: fb, image }) = layouts.get(&layout) else { continue };
        // What a layout leaves unsaid: the default paper, and the profile's margins (§20.206–20.209
        // refer to XSL's margins; whether XSL's initial 0 comes with them the standard doesn't say).
        let mut sec = SectionProps {
            page_w: paper.0,
            page_h: paper.1,
            margin_top: profile.page_margin,
            margin_bottom: profile.page_margin,
            margin_left: profile.page_margin,
            margin_right: profile.page_margin,
            ..SectionProps::default()
        };
        apply_layout(a, &mut sec);
        sec.line_numbers.clone_from(&lines);
        if pages.first.is_none() {
            pages.doc.page_color = get(a, "fo:background-color").and_then(model::parse_color);
            pages.doc.page_image.clone_from(image);
        }
        // ODF starts a header at the page margin and the text after the header's band; Word
        // measures the header's distance and the text's margin from the edge. How the band's
        // height and spacing add up, and what an unstated height or spacing rule is, the
        // standard leaves open: the profile's (see `Band::room`).
        let limit = sec.page_w.max(sec.page_h);
        if header {
            let (room, gap) = hb.room(profile);
            sec.header = sec.margin_top;
            sec.margin_top = (sec.margin_top + room).min(limit);
            // Content taller than the band pushes the text down, the spacing below it kept or not.
            sec.header_gap = gap;
        }
        if footer {
            let (room, gap) = fb.room(profile);
            sec.footer = sec.margin_bottom;
            sec.margin_bottom = (sec.margin_bottom + room).min(limit);
            sec.footer_gap = gap;
        }
        // A one-column layout keeps the default columns (and their gap).
        if let Some(c) = cols.as_ref().filter(|c| c.count > 1) {
            sec.columns = super::styles::with_widths(c.clone(), column_specs, sec.text_width());
        }
        if pages.first.is_none() {
            pages.first = Some(sec.clone());
            pages.first_name = Some(name.clone());
        }
        pages.masters.entry(name).or_insert(sec);
    }
    pages
}

/// A master page followed by another (`style:next-style-name`, LibreOffice's "First Page"
/// before "Default") is a section with a different first page: its own headers and footers on
/// the first page (none there is an empty one), the next master's on the pages after it. Its
/// page size and margins are its own (the first page's).
fn first_pages(pages: &mut Pages) {
    let names: Vec<(String, String)> = pages.next.iter().map(|(a, b)| (a.clone(), b.clone())).collect();
    // The masters as read: a master the loop has already changed (Left and Right pages naming each
    // other) gives the next one its own headers, whatever order the map yields them in.
    let read = pages.masters.clone();
    for (name, next) in names {
        let Some(after) = read.get(&next).cloned() else { continue };
        let Some(sec) = pages.masters.get_mut(&name) else { continue };
        let (own_header, own_footer) = (sec.headers.default, sec.footers.default);
        sec.headers = after.headers;
        sec.footers = after.footers;
        sec.headers.first = own_header;
        sec.footers.first = own_footer;
        sec.title_page = true;
        if pages.first_name.as_deref() == Some(name.as_str()) {
            pages.first = Some(sec.clone());
        }
    }
}

/// Replace `sec` with the page setup of the first master page in `styles.xml`, if any.
#[cfg(test)]
pub(super) fn apply_page_layout(xml: &[u8], sec: &mut SectionProps) {
    if let Some(first) = read_pages(xml, default_paper(), &OdfProfile::default()).first {
        *sec = first;
    }
}

/// A `style:page-layout-properties` element's page size, orientation and margins into `sec`.
fn apply_layout(a: &Attrs, sec: &mut SectionProps) {
    let len = |k: &str| get(a, k).and_then(length).filter(|v| *v >= 0.0);
    // Like the DOCX reader: a page is at least half an inch; at most 22 inches (Word's limit), so a hostile
    // `1e30pt` can't reach layout.
    let page = |k: &str| len(k).filter(|v| *v >= 36.0).map(|v| v.min(1584.0));
    if let (Some(w), Some(h)) = (page("fo:page-width"), page("fo:page-height")) {
        sec.page_w = w;
        sec.page_h = h;
    }
    sec.landscape = match get(a, "style:print-orientation") {
        Some("landscape") => true,
        Some("portrait") => false,
        _ => sec.page_w > sec.page_h,
    };
    for (k, slot) in [
        ("fo:margin-top", &mut sec.margin_top),
        ("fo:margin-bottom", &mut sec.margin_bottom),
        ("fo:margin-left", &mut sec.margin_left),
        ("fo:margin-right", &mut sec.margin_right),
    ] {
        if let Some(v) = len(k) {
            // No margin is larger than the page's longer side (as `w:pgMar` in the DOCX reader).
            *slot = v.min(sec.page_w.max(sec.page_h));
        }
    }
    // Page numbers' format (§19.500 `style:num-format`; an empty one shows the numbers anyway).
    if let Some(f) = get(a, "style:num-format").filter(|f| !f.trim().is_empty()) {
        sec.page_num_format = super::styles::num_format(f);
    }
    // Page borders (§20.183 `fo:border`, §20.210 `fo:padding`: the space inside them).
    let side = |s: &str| {
        let b = get(a, &format!("fo:border-{s}")).or_else(|| get(a, "fo:border")).and_then(super::styles::parse_border)?;
        let pad = get(a, &format!("fo:padding-{s}")).or_else(|| get(a, "fo:padding")).and_then(length).filter(|v| v.is_finite());
        (b.style != wordcraft_doc::props::BorderStyle::None)
            // Any distance on the page (Word's 31 pt limit is for its own measure, from the edge).
            .then(|| wordcraft_doc::props::Border { space: pad.map_or(0.0, |p| p.clamp(0.0, 1584.0)), ..b })
    };
    // ODF's border is at the margin, the text inside its padding; WordCraft measures the border
    // from the page edge and the text margin from the edge too.
    let limit = sec.page_w.max(sec.page_h);
    let inset = |b: Option<wordcraft_doc::props::Border>, margin: &mut f32| {
        b.map(|b| {
            let edge = *margin;
            *margin = (edge + b.width + b.space).min(limit);
            wordcraft_doc::props::Border { space: edge, ..b }
        })
    };
    let borders = wordcraft_doc::props::Borders {
        top: inset(side("top"), &mut sec.margin_top),
        bottom: inset(side("bottom"), &mut sec.margin_bottom),
        left: inset(side("left"), &mut sec.margin_left),
        right: inset(side("right"), &mut sec.margin_right),
        ..Default::default()
    };
    if borders.any_visible() {
        sec.page_borders = Some(borders);
    }
}
