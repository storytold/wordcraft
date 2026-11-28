use super::{DocLayout, PartKind, StoryRef, WordApp};

/// Whether the caret is in a text box's story.
pub fn in_text_box(app: &WordApp) -> bool {
    part_kind(app, app.session.sel.focus.story) == Some(PartKind::TextBox)
}

pub(crate) fn part_kind(app: &WordApp, s: StoryRef) -> Option<PartKind> {
    match s {
        StoryRef::Part(id) => app.session.doc.parts.get(&id).map(|p| p.kind),
        StoryRef::Body => None,
    }
}

/// The story a text box's story sits in (where the box is), when the caret is in a text box.
pub(crate) fn text_box_host(app: &WordApp, layout: &DocLayout) -> Option<StoryRef> {
    match app.session.sel.focus.story {
        StoryRef::Part(id) if in_text_box(app) => layout.text_box_story(id, app.session.page_hint),
        _ => None,
    }
}

/// Whether a press in a page's header/footer area (`zone`, from `header_footer_zone`) goes into
/// another header/footer than `story`: from the body, the other one of the two (#1026), or with
/// Different First Page on, page 1's header while editing page 2's (#2826).
pub(super) fn enters_header_footer(app: &WordApp, story: StoryRef, zone: Option<(Option<StoryRef>, bool)>) -> bool {
    zone.is_some_and(|(shown, header)| match part_kind(app, story) {
        _ if story == StoryRef::Body => true,
        Some(PartKind::Header) => !header || shown != Some(story),
        Some(PartKind::Footer) => header || shown != Some(story),
        _ => false,
    })
}

/// Editing a header or footer, or a text box in one.
pub fn editing_header_footer(app: &WordApp, layout: &DocLayout) -> bool {
    let story = text_box_host(app, layout).unwrap_or(app.session.sel.focus.story);
    matches!(part_kind(app, story), Some(PartKind::Header | PartKind::Footer))
}

/// Notes share the body editing context; other non-body stories dim the body.
pub(super) fn dims_body(app: &WordApp, layout: &DocLayout) -> bool {
    match app.session.sel.focus.story {
        StoryRef::Body => false,
        StoryRef::Part(_) if in_text_box(app) => editing_header_footer(app, layout),
        StoryRef::Part(_) => !matches!(part_kind(app, app.session.sel.focus.story), Some(PartKind::Footnote | PartKind::Endnote)),
    }
}
