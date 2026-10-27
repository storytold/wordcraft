use super::*;

/// The simple field at or just before `pos` when it shows its code (View › Field Codes, or its
/// own toggle): where it is.
pub(super) fn field_showing_code(app: &WordApp, pos: &wordcraft_doc::Pos) -> Option<wordcraft_doc::Pos> {
    let para = app.session.doc.para_at(pos)?;
    let width = wordcraft_doc::para::OBJ.len_utf8();
    [Some(pos.off), pos.off.checked_sub(width)].into_iter().flatten().find_map(|off| match para.object_at(off) {
        Some(wordcraft_doc::para::InlineObject::Field { code, .. }) if *code != app.session.view.field_codes => {
            Some(wordcraft_doc::Pos { off, ..pos.clone() })
        }
        _ => None,
    })
}

/// The hyperlink under the pointer at (x, y) on `page`, where `pos` is the caret position the
/// point hits. The hit snaps to the nearest text, so the link only counts when the point is on
/// the linked character itself, not beside, above or below it (issue #413).
pub(super) fn link_under(app: &WordApp, layout: &DocLayout, page: usize, x: f32, y: f32, pos: &Pos) -> Option<String> {
    let doc = &app.session.doc;
    let para = doc.para_at(pos)?;
    let at = wordcraft_geom::Point::new(x, y);
    // The caret falls between characters: the one under the point is on either side of it.
    [(pos.off, para.next_boundary(pos.off)), (para.prev_boundary(pos.off), pos.off)].into_iter().filter(|(a, b)| a < b).find_map(|(a, b)| {
        let link = para.props_of_char(a).link.clone().filter(|l| !l.is_empty()).or_else(|| field_link(para.object_at(a)?))?;
        let (from, to) = (Pos { off: a, ..pos.clone() }, Pos { off: b, ..pos.clone() });
        layout.selection_rects(doc, &from, &to, page).into_iter().any(|(pi, r)| pi == page && r.contains(at)).then_some(link)
    })
}

/// The link a cross-reference field makes: `REF name \h` and `PAGEREF name \h` lead to the
/// bookmark `name` (`#name`), as in Word; without `\h` the result is plain text (#2945).
pub(super) fn field_link(obj: &wordcraft_doc::InlineObject) -> Option<String> {
    let wordcraft_doc::InlineObject::Field { instr, .. } = obj else { return None };
    let code = wordcraft_engine::cmd::references::FieldCode::parse(instr);
    let name = code.args.first().filter(|n| !n.is_empty())?;
    (matches!(code.name.as_str(), "REF" | "PAGEREF") && code.has('h')).then(|| format!("#{name}"))
}

/// A link to a hidden bookmark (`#_Toc…`, `#_Ref…`): a name the program made, which nobody
/// chose or sees, so the hover tip says where it goes instead of showing it (#2177).
pub(super) fn is_generated_bookmark_link(link: &str) -> bool {
    link.strip_prefix("#_").is_some()
}

/// Whether Ctrl/⌘+click may hand a document's hyperlink to the system: web and email links only,
/// so a document can't launch `file:` paths, programs or custom-scheme handlers.
pub(super) fn is_followable_link(url: &str) -> bool {
    wordcraft_doc::link::is_web_or_mail_link(url)
}
