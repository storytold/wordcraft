//! Data the reference dialogs share: caption labels, number formats and separators.

use crate::WordApp;

/// Labels every document has.
pub(crate) const BUILT_IN_LABELS: [&str; 3] = ["Equation", "Figure", "Table"];

/// Caption number formats: (`\*` switch value, example).
pub(crate) const NUMBER_FORMATS: [(&str, &str); 5] =
    [("ARABIC", "1, 2, 3, …"), ("alphabetic", "a, b, c, …"), ("ALPHABETIC", "A, B, C, …"), ("roman", "i, ii, iii, …"), ("ROMAN", "I, II, III, …")];

/// Chapter-number separators: (text, label).
pub(crate) const SEPARATORS: [(&str, &str); 5] =
    [("-", "- (hyphen)"), (".", ". (period)"), (":", ": (colon)"), ("\u{2014}", "\u{2014} (em dash)"), ("\u{2013}", "\u{2013} (en dash)")];

/// The labels the Caption dialog offers: the built-in ones and the person's own, kept with the
/// preferences. A label only a document's captions use is not the person's to delete, so it is not
/// here (the cross-reference and table-of-figures dialogs add it through [`caption_labels`]).
pub(crate) fn own_labels(app: &WordApp) -> Vec<String> {
    let mut labels: Vec<String> = BUILT_IN_LABELS.iter().map(ToString::to_string).collect();
    for l in &app.ui.caption_labels {
        if !labels.contains(l) {
            labels.push(l.clone());
        }
    }
    labels
}

/// The caption labels to offer: the built-in ones, the person's own (kept with the preferences) and
/// those the document's captions already use, each once.
pub(crate) fn caption_labels(app: &WordApp) -> Vec<String> {
    let mut labels: Vec<String> = BUILT_IN_LABELS.iter().map(ToString::to_string).collect();
    let in_document = wordcraft_engine::cmd::citations::caption_labels(&app.session);
    for l in app.ui.caption_labels.iter().chain(&in_document) {
        if !labels.contains(l) {
            labels.push(l.clone());
        }
    }
    labels
}
