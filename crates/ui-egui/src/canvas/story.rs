use super::{DocLayout, PartKind, Rect, StoryRef, WordApp, pos2, vec2};

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
        StoryRef::Part(id) if in_text_box(app) => layout.text_box(id, app.session.page_hint).map(|o| o.story),
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

/// The frame around the equation being edited, on screen: Word's outline, the grip at its top
/// left and the options strip inside its right edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EquationFrame {
    pub outline: Rect,
    pub grip: Rect,
    pub strip: Rect,
}

/// Lay the frame out around `bx` on the page drawn at `page` (`scale` screen pixels a point):
/// the equation's width and its line's height, plus the strip; the frame's pieces keep their
/// screen size at every zoom.
pub(super) fn equation_frame(page: Rect, bx: &wordcraft_layout::hit::EquationBox, scale: f32) -> EquationFrame {
    let (pad, strip_w) = (2.0, 10.0);
    let x0 = page.min.x + bx.x * scale - pad;
    let x1 = page.min.x + (bx.x + bx.math.width) * scale + pad;
    let (top, bottom) = (page.min.y + bx.top * scale - pad, page.min.y + bx.bottom * scale + pad);
    let outline = Rect::from_min_max(pos2(x0, top), pos2(x1 + strip_w, bottom));
    let strip = Rect::from_min_max(pos2(x1, top), pos2(x1 + strip_w, bottom));
    let grip = Rect::from_min_size(pos2(x0 - 5.0, top - 3.0), vec2(7.0, 14.0));
    EquationFrame { outline, grip, strip }
}
