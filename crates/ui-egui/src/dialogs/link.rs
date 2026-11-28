use craft::text::ui::hyperlink::{CurrentLink, HyperlinkAnswer, HyperlinkDialog, HyperlinkParts, LinkPlace, LinkPlaceGroup, LinkTarget};
use craft::ui::dialog::DialogId;
use craft::ui::runtime::env::Label;

use super::host::{DialogHooks, Keyed, Settle, open_with};
use super::{Value, WordApp};
use serde_json::json;

/// What a place of the Place in This Document list is, in list order across the groups.
#[derive(Clone, Debug, PartialEq)]
enum PlaceRef {
    /// A heading, by body block.
    Heading(u64),
    /// A bookmark, by name.
    Bookmark(String),
}

/// The groups the dialog lists and the target of each place, from `link.targets`. The index of a
/// place in the second is its `LinkTarget::Place` (title rows are not counted).
fn link_places(app: &mut WordApp) -> Option<(Vec<LinkPlaceGroup>, Vec<PlaceRef>)> {
    let targets = app.run("link.targets", json!({})).ok()?;
    let mut refs = Vec::new();
    let mut headings = Vec::new();
    for h in targets.get("headings").and_then(Value::as_array).into_iter().flatten() {
        let level = h.get("level").and_then(Value::as_u64).unwrap_or(1).min(255) as u8;
        let text = h.get("text").and_then(Value::as_str).unwrap_or_default();
        refs.push(PlaceRef::Heading(h.get("block").and_then(Value::as_u64).unwrap_or(0)));
        headings.push(LinkPlace::new(text, level.saturating_sub(1)));
    }
    let mut bookmarks = Vec::new();
    for b in targets.get("bookmarks").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        refs.push(PlaceRef::Bookmark(b.to_string()));
        bookmarks.push(LinkPlace::new(b, 0));
    }
    let groups = vec![LinkPlaceGroup::new(Label::from(tl!("Headings")), headings), LinkPlaceGroup::new(Label::from(tl!("Bookmarks")), bookmarks)];
    Some((groups, refs))
}

/// `s` percent-encoded for a `mailto:` query.
fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The `insert.link` params for a dialog target, or `None` for a place the list doesn't hold.
fn link_params(target: &LinkTarget, text: Option<&str>, places: &[PlaceRef]) -> Option<Value> {
    let mut params = match target {
        LinkTarget::Url(url) => json!({"url": url}),
        LinkTarget::Email { address, subject, .. } => {
            let query = subject.as_deref().filter(|s| !s.is_empty()).map(|s| format!("?subject={}", percent_encode(s))).unwrap_or_default();
            json!({"url": format!("mailto:{address}{query}")})
        }
        LinkTarget::Top => json!({"top": true}),
        LinkTarget::Place(i) => match places.get(*i)? {
            PlaceRef::Heading(block) => json!({"heading": block}),
            PlaceRef::Bookmark(name) => json!({"bookmark": name}),
        },
        _ => return None,
    };
    if let Some(text) = text {
        params["text"] = json!(text);
    }
    Some(params)
}

/// Runs the answer the dialog filed and tells it the outcome.
fn settle(app: &mut WordApp, _ctx: &egui::Context, id: DialogId) -> Settle {
    let Some(dialog) = app.dialog_host.get_mut_by_id::<Keyed<HyperlinkDialog>>(id) else {
        return Settle::Done;
    };
    let Some(answer) = dialog.inner_mut().take_ok() else {
        return Settle::Keep;
    };
    let result = match answer {
        HyperlinkAnswer::Set { link, text, .. } => {
            let places = link_places(app).map(|(_, refs)| refs).unwrap_or_default();
            match link_params(&link.target, text.as_deref(), &places) {
                Some(params) => app.run("insert.link", params).map(|_| ()).map_err(Label::from),
                None => Err(Label::from(tl!("That place is no longer in the document."))),
            }
        }
        _ => Ok(()),
    };
    if let Some(dialog) = app.dialog_host.get_mut_by_id::<Keyed<HyperlinkDialog>>(id) {
        dialog.inner_mut().answer_ok(result);
    }
    Settle::Keep
}

/// Opens Insert › Link on the selected text.
pub(crate) fn open(app: &mut WordApp, _name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    let (groups, _) = link_places(app)?;
    let dialog =
        HyperlinkDialog::new(CurrentLink::None, Some(app.session.selected_text())).place_groups(groups).parts(HyperlinkParts::NoScreenTip).held_ok();
    Some(open_with(app, parent, Keyed::new("link", dialog), DialogHooks { prepare: None, settle }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_answer_maps_to_insert_link_params_for_every_target_kind() {
        // `[Headings: Intro, Method] [Bookmarks: fig1]`: fig1 is place 2, the title rows are not counted.
        let places = [PlaceRef::Heading(3), PlaceRef::Heading(9), PlaceRef::Bookmark("fig1".into())];
        let url = link_params(&LinkTarget::Url("https://example.org".into()), Some("Site"), &places).unwrap();
        assert_eq!(url, json!({"url": "https://example.org", "text": "Site"}));
        let mail = link_params(&LinkTarget::email("a@b.org", Some("Hi there".into())), None, &places).unwrap();
        assert_eq!(mail, json!({"url": "mailto:a@b.org?subject=Hi%20there"}));
        let bare = link_params(&LinkTarget::email("a@b.org", None), None, &places).unwrap();
        assert_eq!(bare, json!({"url": "mailto:a@b.org"}));
        assert_eq!(link_params(&LinkTarget::Top, None, &places), Some(json!({"top": true})));
        assert_eq!(link_params(&LinkTarget::Place(1), None, &places), Some(json!({"heading": 9})));
        assert_eq!(link_params(&LinkTarget::Place(2), None, &places), Some(json!({"bookmark": "fig1"})));
        assert_eq!(link_params(&LinkTarget::Place(3), None, &places), None, "a place past the list is refused");
    }
}
