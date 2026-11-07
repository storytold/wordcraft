//! The restricted embedded editor (`?embed=1` on the web app, together with a host).
//!
//! A page that embeds WordCraft to edit one document it supplied itself — an app that
//! keeps contract files, say — doesn't want the standalone app's document tools: New,
//! Open, Save As, templates, Print, Close, mail merge, the citation tools, the Options
//! and recovery pages, and any way to change who signs the document's edits, while Track
//! Changes stays on. [`Rules`] is what is refused, and [`WordApp::embed`] carries it.
//!
//! Refusal is layered so the editor can't be talked past its host: the Backstage side
//! menu hides the pages ([`Rules::keeps_page`]), the Mailings tab is gone
//! ([`Rules::hides_tab`]), ribbon and context-menu buttons grey out
//! (`widgets::enabled`), and what still reaches [`WordApp::run`] or
//! [`WordApp::handle_request`] — a shortcut, a keytip, an agent over the control
//! channel — is refused with a message on the status line.

/// The Backstage pages the embedded editor's side menu keeps.
const KEPT_PAGES: [&str; 3] = ["info", "save", "export"];

/// The Backstage pages it hides: routing to one is refused, and a hidden page showing
/// (a stale default) redirects to Info.
const HIDDEN_PAGES: [&str; 8] = ["home", "new", "open", "recover", "saveAs", "print", "close", "options"];

/// Command ids that take a file in. In the embedded editor these stay allowed: only the
/// host's delivered files (and a Password or Save Changes dialog retrying one of them)
/// produce them, the dialog ids that pick from the file system are denied, and the web app
/// has no control channel. `?embed=1` is web-only by construction ([`WordApp::embed`] is
/// set by `apps/wordcraft-web` with its host bridge up).
const INTAKE_ALLOWED: [&str; 2] = ["file.open", "insert.picture"];

/// Command ids the embedded editor refuses, however they are opened: at [`WordApp::run`]
/// (with a status line message), at [`WordApp::execute`] (which the control channel and the
/// Save Changes dialog's held-back command reach), and at the control channel's
/// document-addressed path.
const DENIED: [&str; 52] = [
    "file.new",
    "file.newFromTemplate",
    "file.open",
    "file.saveAs",
    "file.saveTemplate",
    "file.rename",
    "file.encrypt",
    "file.close",
    "file.closeDocument",
    "file.print",
    "file.printPdf",
    "file.printMarkup",
    "file.exportPng",
    "file.recover",
    "file.protect",
    "file.setAuthor",
    "file.autosave",
    "file.share",
    "file.versions",
    "file.compatibility",
    "file.privacy",
    "file.privacyOptions",
    "file.inspect",
    "review.compare",
    "review.combine",
    "insert.object",
    "insert.textFromFile",
    "insert.picture",
    "picture.change",
    "view.newWindow",
    "file.options",
    "ui.quickAccessToolbar",
    "font.add",
    "font.loadSystem",
    "review.trackChanges",
    "review.restrict",
    "review.trackingOptions",
    "ui.fileBrowser",
    "ui.saveAsTemplate",
    "ui.startScreen",
    "ui.changePicture",
    "ui.openFileDialog",
    "ui.openRecipientList",
    "ui.print",
    "ui.printSubmit",
    "ui.printSettings",
    "tools.autocorrect",
    "tools.templates",
    "tools.macros",
    "styles.organizer",
    "tools.customizeRibbon",
    "tools.customizeKeyboard",
];

/// Command id prefixes it refuses: everything under them.
const DENIED_PREFIXES: [&str; 2] = ["mailings.", "ui.zotero."];

/// UI request kinds (an `{"open": …}` request of [`WordApp::handle_request`]) it refuses.
const DENIED_REQUESTS: [&str; 11] = [
    "openFile",
    "saveAs",
    "print",
    "options",
    "newWindow",
    "insertPicture",
    "addFont",
    "loadSystemFonts",
    "exportPng",
    "customizeRibbon",
    "customizeKeyboard",
];

/// Dialog names (`ui.dialog`) it refuses: the dialogs of denied functionality —
/// customization, the document tools, the author name, restrictions, compare and the
/// mail-merge set.
const DENIED_DIALOGS: [&str; 25] = [
    "customizeRibbon",
    "customizeQuickAccess",
    "customizeKeyboard",
    "trackChangesOptions",
    "encryptPassword",
    "renameDocument",
    "autoCorrect",
    "restrictEditing",
    "compare",
    "combine",
    "templates",
    "organizer",
    "macros",
    "checkErrors",
    "findRecipient",
    "insertMergeField",
    "matchFields",
    "mergeRecords",
    "recipientList",
    "ruleIf",
    "ruleSkipIf",
    "newRecipientList",
    "printRecords",
    "envelopes",
    "labels",
];

/// What the embedded editor refuses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rules;

impl Rules {
    /// Whether the command `id` is refused.
    pub fn denies(&self, id: &str) -> bool {
        DENIED.contains(&id) || DENIED_PREFIXES.iter().any(|p| id.starts_with(p))
    }

    /// Whether the Backstage page stays in the side menu.
    pub fn keeps_page(&self, page: &str) -> bool {
        KEPT_PAGES.contains(&page)
    }

    /// Whether routing to this Backstage page is refused.
    pub fn hidden_page(&self, page: &str) -> bool {
        HIDDEN_PAGES.contains(&page)
    }

    /// Whether the UI request kind is refused.
    pub fn denies_request(&self, kind: &str) -> bool {
        DENIED_REQUESTS.contains(&kind)
    }

    /// Whether a denied id takes the host's delivered files in (and so stays allowed).
    pub fn intake_allows(&self, id: &str) -> bool {
        INTAKE_ALLOWED.contains(&id)
    }

    /// Whether the dialog named `name` (`ui.dialog`) is refused.
    pub fn denies_dialog(&self, name: &str) -> bool {
        DENIED_DIALOGS.contains(&name)
    }

    /// Whether the ribbon tab is taken out.
    pub fn hides_tab(&self, tab: &str) -> bool {
        tab == "Mailings"
    }
}

#[cfg(test)]
mod tests;
