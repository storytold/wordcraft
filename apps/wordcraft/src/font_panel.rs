//! The macOS system font panel (Format › Font…), with its text colour, underline and strikethrough,
//! and the system colour picker. The same file serves DeckCraft, GridCraft and WordCraft: the app shows it on a
//! [`Face`] and formats the selection with each [`Pick`].
//!
//! The panel sends `changeFont:` (family, size, bold, italic) and `changeAttributes:` (text colour,
//! underline, strikethrough) to the font manager's target, an object of ours. The colour panel, opened
//! from the font panel's Text Color or with its own pickers (colour wheel, sliders, palettes, crayons,
//! image, third-party pickers), sends `changeColor:` to the same object. Colours come in each picker's
//! colour space (grey, CMYK, catalogue lists) and are converted to sRGB.
//!
//! An exception to the workspace's no-`unsafe` rule (AGENTS.md): receiving those messages needs an
//! Objective-C class (`define_class!`), and setting the panels' targets is marked unsafe.

use std::cell::{Cell, RefCell};
use std::sync::mpsc::{Receiver, Sender, channel};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSColor, NSColorPanel, NSColorSpace, NSFont, NSFontManager, NSFontPanel, NSFontPanelModeMask, NSFontTraitMask, NSResponder,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString, ns_string};

/// An underline or strikethrough, as the panel sets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line {
    None,
    Single,
    Double,
}

impl Line {
    /// From an `NSUnderlineStyle`: its low byte is the line (0 none, 9 double, others single or thick);
    /// the rest is the pattern (dotted, dashed), which this ignores.
    fn from_style(style: isize) -> Self {
        match style & 0xFF {
            0 => Line::None,
            9 => Line::Double,
            _ => Line::Single,
        }
    }
}

/// What the user changed in the panel. Only that is applied, so a new size keeps each run's own
/// family, bold and italic in a mixed selection.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pick {
    pub family: Option<String>,
    pub size: Option<f64>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    /// sRGB.
    pub color: Option<[u8; 3]>,
    pub underline: Option<Line>,
    pub strike: Option<Line>,
}

/// The selection's font, as the panel shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Face {
    pub family: String,
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
}

impl Pick {
    fn between(before: &Face, after: &Face) -> Self {
        Pick {
            family: (after.family != before.family).then(|| after.family.clone()),
            size: (after.size != before.size).then_some(after.size),
            bold: (after.bold != before.bold).then_some(after.bold),
            italic: (after.italic != before.italic).then_some(after.italic),
            ..Pick::default()
        }
    }

    /// The panel's `changeAttributes:` answer, converted from no attributes: only what was set.
    fn from_attributes(changed: &NSDictionary<NSString, AnyObject>) -> Self {
        let line = |key: &NSString| changed.objectForKey(key)?.downcast::<NSNumber>().ok().map(|n| Line::from_style(n.as_isize()));
        Pick {
            color: changed.objectForKey(ns_string!("NSColor")).and_then(|c| c.downcast::<NSColor>().ok()).and_then(|c| srgb(&c)),
            underline: line(ns_string!("NSUnderline")),
            strike: line(ns_string!("NSStrikethrough")),
            ..Pick::default()
        }
    }
}

/// A colour in sRGB. `None` for one that has no single colour (a pattern image).
fn srgb(color: &NSColor) -> Option<[u8; 3]> {
    let c = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Some([byte(c.redComponent()), byte(c.greenComponent()), byte(c.blueComponent())])
}

struct Ivars {
    picks: Sender<Pick>,
    /// Wakes the app: a pick arrives through the panel's window, which isn't one the app watches.
    wake: Box<dyn Fn()>,
    /// The font the panel converts: the selection's, then each pick.
    current: RefCell<Retained<NSFont>>,
    /// The panel's controls the app takes picks from.
    modes: Cell<NSFontPanelModeMask>,
}

define_class!(
    // SAFETY: NSResponder has no subclassing requirements, and `FontTarget` doesn't implement Drop.
    #[unsafe(super(NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "CraftFontTarget"]
    #[ivars = Ivars]
    struct FontTarget;

    impl FontTarget {
        // SAFETY: `changeFont:` takes the sender (the font manager) and returns nothing.
        #[unsafe(method(changeFont:))]
        fn change_font(&self, _sender: Option<&AnyObject>) {
            let manager = NSFontManager::sharedFontManager(self.mtm());
            let before = self.ivars().current.borrow().clone();
            let font = manager.convertFont(&before);
            self.send(Pick::between(&face(&manager, &before), &face(&manager, &font)));
            self.ivars().current.replace(font);
        }

        // SAFETY: `changeAttributes:` takes the sender (the panel's effects) and returns nothing.
        #[unsafe(method(changeAttributes:))]
        fn change_attributes(&self, sender: Option<&AnyObject>) {
            if let Some(changed) = sender.and_then(convert_attributes) {
                self.send(Pick::from_attributes(&changed));
            }
        }

        // SAFETY: `validModesForFontPanel:` takes the panel and returns its mode mask.
        #[unsafe(method(validModesForFontPanel:))]
        fn valid_modes(&self, _panel: &NSFontPanel) -> NSFontPanelModeMask {
            self.ivars().modes.get()
        }

        // SAFETY: `changeColor:` takes the sender (the colour panel) and returns nothing.
        #[unsafe(method(changeColor:))]
        fn change_color(&self, _sender: Option<&AnyObject>) {
            let color = NSColorPanel::sharedColorPanel(self.mtm()).color();
            self.send(Pick { color: srgb(&color), ..Pick::default() });
        }
    }
);

#[allow(unsafe_code)]
impl FontTarget {
    fn new(mtm: MainThreadMarker, picks: Sender<Pick>, wake: Box<dyn Fn()>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            picks,
            wake,
            current: RefCell::new(NSFont::systemFontOfSize(12.0)),
            modes: Cell::new(NSFontPanelModeMask::ModesMaskStandardModes),
        });
        // SAFETY: NSResponder's designated initializer, on a freshly allocated object.
        unsafe { msg_send![super(this), init] }
    }

    fn send(&self, pick: Pick) {
        if pick != Pick::default() {
            let _ = self.ivars().picks.send(pick);
            (self.ivars().wake)();
        }
    }
}

/// What the sender of `changeAttributes:` changes, asked as `convertAttributes:` of no attributes.
#[allow(unsafe_code)]
fn convert_attributes(sender: &AnyObject) -> Option<Retained<NSDictionary<NSString, AnyObject>>> {
    // SAFETY: `respondsToSelector:` is NSObject's, which every sender of an action message is.
    let answers: bool = unsafe { msg_send![sender, respondsToSelector: sel!(convertAttributes:)] };
    if !answers {
        return None;
    }
    let none = NSDictionary::<NSString, AnyObject>::new();
    // SAFETY: `convertAttributes:` takes and returns an attribute dictionary (NSFontManager's and the
    // panel's effects both declare it so), and the sender answers it.
    unsafe { msg_send![sender, convertAttributes: &*none] }
}

fn face(manager: &NSFontManager, font: &NSFont) -> Face {
    let traits = manager.traitsOfFont(font);
    Face {
        family: font.familyName().map(|f| f.to_string()).unwrap_or_default(),
        size: font.pointSize(),
        bold: traits.contains(NSFontTraitMask::BoldFontMask),
        italic: traits.contains(NSFontTraitMask::ItalicFontMask),
    }
}

/// Where Format › Font… opens the panel and the picks arrive. Lives as long as the app: the panels
/// don't keep their target alive.
pub struct FontPanel {
    target: Retained<FontTarget>,
    picks: Receiver<Pick>,
    /// The font the panel shows, so it changes only when the selection's does.
    shown: RefCell<Option<Face>>,
    /// Whether the app has opened the panel; one open before that was restored by macOS.
    opened: Cell<bool>,
}

impl FontPanel {
    /// `wake` asks the app for a pass, in which it takes the [`picks`](Self::picks).
    pub fn new(mtm: MainThreadMarker, wake: impl Fn() + 'static) -> Self {
        let (tx, picks) = channel();
        FontPanel { target: FontTarget::new(mtm, tx, Box::new(wake)), picks, shown: RefCell::new(None), opened: Cell::new(false) }
    }

    /// Hides the panel's shadow controls, for an app whose text has no shadow.
    #[allow(dead_code)] // apps with a text shadow keep the controls
    pub fn hide_shadow(&self) {
        let modes = self.target.ivars().modes.get();
        self.target.ivars().modes.set(modes.difference(NSFontPanelModeMask::ModeMaskShadowEffect));
    }

    /// Shows the panel on the selection's font.
    pub fn show(&self, face: Face) {
        let mtm = self.target.mtm();
        self.opened.set(true);
        self.select(face);
        set_targets(mtm, &self.target);
        NSFontPanel::sharedFontPanel(mtm).orderFront(None);
    }

    /// While the panel is open, it shows the selection's font as that changes (another selection, or
    /// formatting from the ribbon), so a pick converts the selection's own font. `face` is asked only
    /// then.
    pub fn follow(&self, face: impl FnOnce() -> Face) {
        let mtm = self.target.mtm();
        // Before the app opens them, an open panel is one macOS restored from the last quit
        // (window restoration, which may come after the first frames). They open only when asked.
        if !self.opened.get() {
            if NSFontPanel::sharedFontPanelExists(mtm) && NSFontPanel::sharedFontPanel(mtm).isVisible() {
                NSFontPanel::sharedFontPanel(mtm).orderOut(None);
            }
            if NSColorPanel::sharedColorPanelExists(mtm) && NSColorPanel::sharedColorPanel(mtm).isVisible() {
                NSColorPanel::sharedColorPanel(mtm).orderOut(None);
            }
            return;
        }
        if !NSFontPanel::sharedFontPanelExists(mtm) || !NSFontPanel::sharedFontPanel(mtm).isVisible() {
            return;
        }
        let face = face();
        if self.shown.borrow().as_ref() != Some(&face) {
            self.select(face);
        }
    }

    fn select(&self, face: Face) {
        let manager = NSFontManager::sharedFontManager(self.target.mtm());
        let mut traits = NSFontTraitMask::empty();
        traits.set(NSFontTraitMask::BoldFontMask, face.bold);
        traits.set(NSFontTraitMask::ItalicFontMask, face.italic);
        // A family the system doesn't have (a document's, a font the app carries) shows the system
        // font, bold and italic as the selection is.
        let font = manager
            .fontWithFamily_traits_weight_size(&NSString::from_str(&face.family), traits, 5, face.size)
            .unwrap_or_else(|| manager.convertFont_toHaveTrait(&NSFont::systemFontOfSize(face.size), traits));
        self.target.ivars().current.replace(font.clone());
        manager.setSelectedFont_isMultiple(&font, false);
        self.shown.replace(Some(face));
    }

    /// The picks since the last call, oldest first. A colour can arrive twice for one change (from the
    /// font panel's colour well and from the colour panel); a repeat is dropped. The panel then shows
    /// the selection's font again at the next [`follow`](Self::follow), as a pick the app refused
    /// leaves the selection as it was.
    pub fn picks(&self) -> Vec<Pick> {
        let mut picks: Vec<Pick> = self.picks.try_iter().collect();
        picks.dedup();
        if !picks.is_empty() {
            self.shown.replace(None);
        }
        picks
    }

    /// Shows the panel on a given font, for `tests/mac_font_panel.rs`.
    #[allow(dead_code)]
    pub fn show_for_test(&self, family: &str, size: f64, bold: bool, italic: bool) {
        self.show(Face { family: family.into(), size, bold, italic });
    }

    /// What the panel does when the user picks a font, for `tests/mac_font_panel.rs`.
    #[allow(dead_code)]
    pub fn change_font_for_test(&self) {
        self.target.change_font(sel!(changeFont:), None);
    }

    /// What the colour panel does when the user picks a colour, for `tests/mac_font_panel.rs`.
    #[allow(dead_code)]
    pub fn change_color_for_test(&self, color: &NSColor) {
        NSColorPanel::sharedColorPanel(self.target.mtm()).setColor(color);
        self.target.change_color(sel!(changeColor:), None);
    }

    /// What the panel's effects do when the user changes them, for `tests/mac_font_panel.rs`.
    #[allow(dead_code)]
    pub fn change_attributes_for_test(&self, changed: &NSDictionary<NSString, AnyObject>) {
        self.target.send(Pick::from_attributes(changed));
    }

    /// What the font manager's Bigger, Smaller, Bold… items do, for `tests/mac_font_panel.rs`.
    #[allow(dead_code, unsafe_code)]
    pub fn modify_font_for_test(&self, action: objc2_app_kit::NSFontAction) {
        let mtm = self.target.mtm();
        let sender = objc2_app_kit::NSButton::new(mtm);
        sender.setTag(action.0 as isize);
        // SAFETY: `modifyFont:` reads only the sender's tag, and the sender is a control we made.
        unsafe { NSFontManager::sharedFontManager(mtm).modifyFont(Some(&sender)) };
    }

    /// The controls the panel would show, asked of the responder chain as the panel asks, for
    /// `tests/mac_font_panel.rs`.
    #[allow(dead_code, unsafe_code)]
    pub fn modes_for_test(&self) -> usize {
        let mtm = self.target.mtm();
        let panel = NSFontPanel::sharedFontPanel(mtm);
        // SAFETY: `targetForAction:` only looks the selector up; `validModesForFontPanel:` takes the
        // panel and returns its mode mask.
        unsafe {
            let Some(target) = NSApplication::sharedApplication(mtm).targetForAction(sel!(validModesForFontPanel:)) else { return 0 };
            msg_send![&target, validModesForFontPanel: &*panel]
        }
    }
}

#[allow(unsafe_code)]
fn set_targets(mtm: MainThreadMarker, target: &FontTarget) {
    // SAFETY: the target answers the font manager's actions (`changeFont:`, `changeAttributes:`), the
    // colour panel's (`changeColor:`) and the panel's `validModesForFontPanel:`, and `FontPanel` keeps
    // it alive for as long as the app runs, so the panels' and the app's unretained references stay
    // valid.
    unsafe {
        NSFontManager::sharedFontManager(mtm).setTarget(Some(target));
        let colors = NSColorPanel::sharedColorPanel(mtm);
        colors.setTarget(Some(target));
        colors.setAction(Some(sel!(changeColor:)));
        // A drag in a picker is one pick (one undo step), sent when it ends.
        colors.setContinuous(false);
        colors.setRestorable(false);
        NSFontPanel::sharedFontPanel(mtm).setRestorable(false);
        // The panel asks `validModesForFontPanel:` of the responder chain, not of the target; keep
        // whatever followed the app.
        let app = NSApplication::sharedApplication(mtm);
        if !app.nextResponder().is_some_and(|next| std::ptr::eq(&*next, &**target)) {
            target.setNextResponder(app.nextResponder().as_deref());
            app.setNextResponder(Some(target));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_changed_is_picked() {
        let before = Face { family: "Helvetica".into(), size: 12.0, bold: true, italic: false };
        let bigger = Face { size: 18.0, ..before.clone() };
        assert_eq!(Pick::between(&before, &bigger), Pick { size: Some(18.0), ..Pick::default() }, "bold stays each run's own");
        let plain = Face { bold: false, ..before.clone() };
        assert_eq!(Pick::between(&before, &plain), Pick { bold: Some(false), ..Pick::default() });
        assert_eq!(Pick::between(&before, &before), Pick::default());
    }

    #[test]
    fn underline_styles_are_lines() {
        assert_eq!(Line::from_style(0), Line::None);
        assert_eq!(Line::from_style(1), Line::Single);
        assert_eq!(Line::from_style(2), Line::Single, "thick");
        assert_eq!(Line::from_style(9), Line::Double);
        assert_eq!(Line::from_style(0x0201), Line::Single, "dotted single");
    }
}
