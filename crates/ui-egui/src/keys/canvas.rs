// PLAN: the Insert key has no editor input, so the overtype switch is the shortcut layer's (shell-w6).
use craft::behavior::editor::{EditorInput, SurroundingUnit};
use craft::behavior::intent::{EditCommand, Granularity, MoveMods, NavMove};
use craft::platform::clipboard::{RGBA8, URI_LIST};
use craft::ui::canvas::ToolPass;
use craft::ui::runtime::commands::{ClipItem, ClipboardHandle};

use super::ime::{
    MAX_IME_COMMIT_BYTES, MAX_IME_PREEDIT_BYTES, delete_in_equation, delete_surrounding, delete_surrounding_bytes, ime_report, ime_stamp,
    note_automatic_change,
};
use super::{Key, Modifiers, WordApp, json};

/// What a refused or interrupted IME replacement still has to drop from the same input list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReplacementDrop {
    /// Nothing waits.
    None,
    /// The next input may be the replacement: an `ImeCommit` or a `Text` starting a pair.
    Awaiting,
    /// The replacement's `Text` was dropped; an `ImeCommit` after it belongs to the same replacement.
    TextDropped,
    /// The replacement's `ImeCommit` was dropped; the `Text` that follows it is its text.
    CommitDropped,
}

/// State carried across the inputs of one pass's list; never stored.
struct Batch {
    drop_equation_replacement: ReplacementDrop,
    interrupted_preedit: bool,
    interrupted_replacement: ReplacementDrop,
    /// An `ImeCommit` was seen: the `Text` after it is the committed string.
    committing: bool,
}

/// The page's editor input of one pass: text, editing keys, clipboard and IME, in the order they
/// arrived. While keytips are showing every key belongs to them, so nothing here runs.
pub fn page_input(app: &mut WordApp, inputs: &[EditorInput], pass: &ToolPass, ctx: &egui::Context) {
    if app.keytips_active {
        return;
    }
    // A body or equation replacement interrupted the composition: the framework drops the old
    // composition's late events until the platform answers.
    if std::mem::take(&mut app.canvas.ime_interrupt_now) {
        pass.ime_application_change(ctx);
    }
    let window = app.windows.shown();
    if app.session.view.read_mode {
        read_mode_input(app, ctx, inputs, window);
        return;
    }
    // The erased text's formatting is carried only for a replacement in this list. The undo join
    // belongs to the deletion transaction instead: the engine's scoped `ime_text_join` survives
    // frames until its matching `text.insert` or any other command, key or new request consumes or
    // clears it, so a platform that delivers the commit in a later pass still shares the deletion's
    // Undo checkpoint.
    // The erased text's formatting survives into a later pass only while the engine still promises the join:
    // the framework hands over the input method's edits and the typed text as it receives them, and a text
    // that arrives in the pass after its deletion is still that deletion's replacement.
    if !app.session.ime_text_join_pending() {
        app.canvas.ime_join_text = None;
    }
    let mut batch = Batch {
        drop_equation_replacement: ReplacementDrop::None,
        interrupted_preedit: std::mem::take(&mut app.canvas.ime_interrupted_preedit),
        interrupted_replacement: ReplacementDrop::None,
        committing: false,
    };
    for e in inputs {
        // A key closed the window (Mod+W) or brought another forward: the pass's other inputs
        // were typed in this one and are dropped.
        if app.windows.shown() != window {
            break;
        }
        if stale_composition(app, e, &mut batch) {
            continue;
        }
        if is_key_input(e) {
            app.canvas.ime_join_text = None;
            // A key between a deletion and its text means that text is fresh input, not the
            // replacement the deletion promised (even when the key runs no command).
            app.session.clear_ime_text_join();
        }
        // Escape stops a running Ink Replay before anything else takes it.
        if matches!(e, EditorInput::Cancel) && crate::ink::stop_replay(app) {
            continue;
        }
        // Enter and Escape leave crop mode keeping the crop (`crop` docs). The canvas takes the page's
        // keys as editor input before the crop overlay's own pass reads its Submit and Cancel, so they
        // end it here, and neither types a paragraph over the selected picture.
        if matches!(e, EditorInput::Newline | EditorInput::Cancel) && crate::crop::active(app).is_some() {
            crate::crop::leave(app);
            continue;
        }
        if drop_refused_replacement(app, e, &mut batch) || equation_input(app, ctx, pass, e, &mut batch) {
            continue;
        }
        handle_input(app, ctx, pass, e, &mut batch);
    }
    // A request refused during this pass leaves the platform's surrounding snapshot stale: the page asks the
    // framework for the interrupt itself, in the pass of the commit's receipt, so the commit keeps its input-method
    // cause (the old canvas set the interrupt directly).
    if std::mem::take(&mut app.canvas.ime_interrupt_now) {
        pass.ime_interrupt(ctx);
    }
}

/// Whether the input is a key's, not text or an input method's.
fn is_key_input(e: &EditorInput) -> bool {
    matches!(
        e,
        EditorInput::Move(..)
            | EditorInput::Delete { .. }
            | EditorInput::Edit(_)
            | EditorInput::Newline
            | EditorInput::Tab
            | EditorInput::BackTab
            | EditorInput::Cancel
    )
}

/// Read Mode is for navigation and selection, not typing into the document. The ribbon's Edit
/// Document button or Escape returns to Print Layout before editing; page turns are the reading
/// view's own keys.
fn read_mode_input(app: &mut WordApp, ctx: &egui::Context, inputs: &[EditorInput], window: u64) {
    for e in inputs {
        if app.windows.shown() != window {
            break;
        }
        match e {
            // Not while a dialog or menu that takes Escape is open.
            EditorInput::Cancel if app.escape_free => {
                // Focus or Full Screen first (#1678), then Read Mode.
                let id = crate::chrome::Screen::of(&app.session.view).exit_command().unwrap_or("view.printLayout");
                let _ = app.run(id, json!({"value": false}));
                break;
            }
            EditorInput::Edit(EditCommand::Copy) => {
                let selected = app.session.selected_text();
                if !selected.is_empty() {
                    ClipboardHandle::of(ctx).put(&[ClipItem::Text(selected.into())]);
                }
            }
            _ => {}
        }
    }
}

/// An input that is the queued replacement for an interrupted pre-edit: dropped.
fn stale_composition(app: &mut WordApp, e: &EditorInput, b: &mut Batch) -> bool {
    b.interrupted_preedit |= std::mem::take(&mut app.canvas.ime_interrupted_preedit);
    let stale = if b.interrupted_preedit {
        match e {
            EditorInput::DeleteSurrounding { .. } => {
                b.interrupted_replacement = ReplacementDrop::Awaiting;
                true
            }
            EditorInput::ImeCommit => {
                b.interrupted_replacement = ReplacementDrop::CommitDropped;
                true
            }
            EditorInput::Text(_) => match b.interrupted_replacement {
                ReplacementDrop::Awaiting => {
                    b.interrupted_replacement = ReplacementDrop::TextDropped;
                    true
                }
                ReplacementDrop::CommitDropped => {
                    b.interrupted_preedit = false;
                    b.interrupted_replacement = ReplacementDrop::None;
                    true
                }
                // Text with no commit before it is fresh input.
                _ => {
                    b.interrupted_preedit = false;
                    false
                }
            },
            EditorInput::Preedit { text, .. } if text.is_empty() => true,
            // A new marked composition or any key is not the old target's queued replacement.
            EditorInput::Preedit { .. } => {
                b.interrupted_preedit = false;
                b.interrupted_replacement = ReplacementDrop::None;
                false
            }
            e if is_key_input(e) => {
                b.interrupted_preedit = false;
                b.interrupted_replacement = ReplacementDrop::None;
                false
            }
            _ => false,
        }
    } else {
        false
    };
    if stale {
        app.session.clear_ime_joins();
        app.canvas.ime_join_text = None;
    }
    stale
}

/// The replacement paired with a refused equation deletion is dropped.
fn drop_refused_replacement(app: &mut WordApp, e: &EditorInput, b: &mut Batch) -> bool {
    // The replacement is the input immediately after the refused deletion: an `ImeCommit` and its
    // `Text`, or the adjacent `Text` + `ImeCommit` + `Text` run some platforms use. Any
    // intervening input is unrelated and clears the state before it is handled.
    let (next, drop_event) = match (b.drop_equation_replacement, e) {
        (ReplacementDrop::None, _) => (ReplacementDrop::None, false),
        (ReplacementDrop::Awaiting, EditorInput::Text(_)) => (ReplacementDrop::TextDropped, true),
        (ReplacementDrop::Awaiting | ReplacementDrop::TextDropped, EditorInput::ImeCommit) => (ReplacementDrop::CommitDropped, true),
        (ReplacementDrop::CommitDropped, EditorInput::Text(_)) => (ReplacementDrop::None, true),
        _ => (ReplacementDrop::None, false),
    };
    b.drop_equation_replacement = next;
    if drop_event {
        app.canvas.ime_preedit.clear();
        app.canvas.ime_preedit_range = None;
        app.canvas.ime_interrupt_now = true;
    }
    drop_event
}

/// Editing an equation: text and editing keys go into it.
fn equation_input(app: &mut WordApp, ctx: &egui::Context, pass: &ToolPass, e: &EditorInput, b: &mut Batch) -> bool {
    if app.session.math.is_none() {
        return false;
    }
    match e {
        EditorInput::Text(t) => {
            if std::mem::take(&mut b.committing) {
                equation_commit(app, ctx, pass, t);
            } else if !t.is_empty() && t.chars().all(|c| !c.is_control()) {
                let _ = app.run("equation.type", json!({"text": &**t}));
            } else {
                // Ignored text is not the replacement a deletion promised.
                app.session.clear_ime_joins();
            }
            true
        }
        EditorInput::ImeCommit => {
            app.canvas.ime_preedit.clear();
            app.canvas.ime_preedit_range = None;
            b.committing = true;
            true
        }
        // A phone keyboard's or macOS replacement inside an equation: erase the counted units on
        // each side with one transactional equation command. The tree cannot provide a surrounding
        // snapshot; an engine revision proves a real commit and the platform is never handed
        // fabricated surroundings.
        EditorInput::DeleteSurrounding { before, after, unit: SurroundingUnit::Chars, .. } => {
            delete_in_equation(app, pass, ctx, *before, *after);
            true
        }
        // Byte counts have no reversible mapping into a structured equation, so an exact
        // replacement cannot be performed without guessing at tree units. Refuse the whole request,
        // clear the composition and drop the replacement text that the platform sends for the
        // deletion it believes it made. Inserting it beside the untouched equation would duplicate
        // the old text.
        EditorInput::DeleteSurrounding { before, after, .. } => {
            if *before != 0 || *after != 0 {
                log::debug!("refusing byte delete-surrounding inside an equation");
                app.canvas.ime_preedit.clear();
                app.canvas.ime_preedit_range = None;
                // This refusal is a new request, not the replacement promised by an earlier
                // equation deletion.
                app.session.clear_ime_joins();
                b.drop_equation_replacement = ReplacementDrop::Awaiting;
                app.canvas.ime_interrupt_now = true;
            }
            true
        }
        EditorInput::Edit(EditCommand::Paste) => {
            if let Ok(Some(t)) = ClipboardHandle::of(ctx).read_text()
                && app.run("equation.type", json!({"text": t})).is_err()
            {
                app.canvas.ime_interrupt_now = true;
            }
            true
        }
        EditorInput::Move(mv, m) => {
            // A word, paragraph or document move, or one with the lead modifier, is the body's.
            if m.lead || matches!(m.by, Granularity::Word | Granularity::Paragraph | Granularity::Document) {
                return false;
            }
            let dir = match mv {
                NavMove::Back => "left",
                NavMove::Forward => "right",
                NavMove::Up => "up",
                NavMove::Down => "down",
                NavMove::Start => "home",
                NavMove::End => "end",
                _ => return true,
            };
            let _ = app.run("equation.move", json!({"dir": dir}));
            true
        }
        EditorInput::Tab => {
            let _ = app.run("equation.move", json!({"dir": "next"}));
            true
        }
        EditorInput::BackTab => {
            let _ = app.run("equation.move", json!({"dir": "prev"}));
            true
        }
        EditorInput::Delete { forward, .. } => {
            let _ = app.run(if *forward { "equation.delete" } else { "equation.backspace" }, json!({}));
            true
        }
        EditorInput::Newline => {
            let _ = app.run("equation.enter", json!({}));
            true
        }
        EditorInput::Cancel => {
            let _ = app.run("equation.exit", json!({}));
            true
        }
        // Anything else between the deletion and its replacement (a pre-edit, a selection) means
        // the next equation typing command is a new edit, not the immediate replacement. Drop the
        // promise; a command consumes it in `Session::run` even when it is not a typing command.
        _ => {
            app.session.clear_ime_joins();
            false
        }
    }
}

/// An input method's commit into the equation being edited.
fn equation_commit(app: &mut WordApp, ctx: &egui::Context, pass: &ToolPass, text: &str) {
    if text.is_empty() {
        // An empty commit is not the replacement for a preceding deletion.
        app.session.clear_ime_joins();
        return;
    }
    if text.len() > MAX_IME_COMMIT_BYTES || text.chars().count() > wordcraft_engine::MAX_EQUATION_TYPE_CHARS {
        // Refuse the whole commit (no prefix in either equation mode) and ask the platform to
        // start the composition afresh. It is not the replacement for a preceding deletion.
        log::warn!("ignoring an oversized IME equation commit of {} bytes", text.len());
        app.session.clear_ime_joins();
        app.canvas.ime_interrupt_now = true;
        return;
    }
    let before_stamp = ime_stamp(app);
    match app.run_ime("equation.type", json!({"text": text})) {
        Ok(_) => {
            let outcome = app.session.ime_edit_outcome();
            let after_stamp = ime_stamp(app);
            ime_report(app, pass, ctx, before_stamp, after_stamp, outcome);
        }
        Err(_) => {
            // The equation refused the commit (it may be gone): start the composition afresh
            // instead of leaving one it believes landed.
            app.canvas.ime_interrupt_now = true;
        }
    }
}

/// The input method's cursor as chars of its composition, from the bytes the engine reports.
fn chars_range(text: &str, r: &std::ops::Range<usize>) -> std::ops::Range<usize> {
    let chars = |at: usize| text.get(..at).map_or_else(|| text.chars().count(), |s| s.chars().count());
    chars(r.start)..chars(r.end)
}

/// A committed IME string.
fn ime_commit(app: &mut WordApp, ctx: &egui::Context, pass: &ToolPass, text: &str) {
    app.canvas.ime_preedit.clear();
    app.canvas.ime_preedit_range = None;
    if text.is_empty() {
        // An empty commit is not the replacement a preceding deletion promised.
        app.session.clear_ime_text_join();
        app.canvas.ime_join_text = None;
    } else if text.len() <= MAX_IME_COMMIT_BYTES {
        match crate::dialogs_smart_art::type_into(app, ctx, text) {
            crate::dialogs_smart_art::SmartArtTypeResult::Changed => {
                app.session.clear_ime_text_join();
                app.canvas.ime_join_text = None;
                pass.ime_application_change(ctx);
            }
            crate::dialogs_smart_art::SmartArtTypeResult::ConsumedNoChange => {
                app.session.clear_ime_text_join();
                app.canvas.ime_join_text = None;
            }
            crate::dialogs_smart_art::SmartArtTypeResult::NotHandled => type_text(app, pass, ctx, text, true),
        }
    } else {
        log::warn!("ignoring an IME commit of {} bytes", text.len());
        app.session.clear_ime_text_join();
        app.canvas.ime_join_text = None;
        app.canvas.ime_interrupt_now = true;
    }
}

/// The command of a move of `mv` by `by`; `None` for a move the page does not know.
fn move_command(mv: NavMove, by: Granularity) -> Option<&'static str> {
    Some(match mv {
        NavMove::Back => match by {
            Granularity::Word => "caret.wordLeft",
            Granularity::Line => "caret.home",
            Granularity::Document => "caret.docStart",
            _ => "caret.left",
        },
        NavMove::Forward => match by {
            Granularity::Word => "caret.wordRight",
            Granularity::Line => "caret.end",
            Granularity::Document => "caret.docEnd",
            _ => "caret.right",
        },
        NavMove::Up => {
            if by == Granularity::Paragraph {
                "caret.paraUp"
            } else {
                "caret.up"
            }
        }
        NavMove::Down => {
            if by == Granularity::Paragraph {
                "caret.paraDown"
            } else {
                "caret.down"
            }
        }
        NavMove::PageUp => "caret.pageUp",
        NavMove::PageDown => "caret.pageDown",
        NavMove::Start => {
            if by == Granularity::Document {
                "caret.docStart"
            } else {
                "caret.home"
            }
        }
        NavMove::End => {
            if by == Granularity::Document {
                "caret.docEnd"
            } else {
                "caret.end"
            }
        }
        _ => return None,
    })
}

/// The physical key a move stands for, for a selected object's own key handling.
fn move_key(mv: NavMove) -> Option<Key> {
    Some(match mv {
        NavMove::Back => Key::ArrowLeft,
        NavMove::Forward => Key::ArrowRight,
        NavMove::Up => Key::ArrowUp,
        NavMove::Down => Key::ArrowDown,
        NavMove::PageUp => Key::PageUp,
        NavMove::PageDown => Key::PageDown,
        NavMove::Start => Key::Home,
        NavMove::End => Key::End,
        _ => return None,
    })
}

/// Everything else: typed text, IME, clipboard and keys.
fn handle_input(app: &mut WordApp, ctx: &egui::Context, pass: &ToolPass, e: &EditorInput, b: &mut Batch) {
    match e {
        EditorInput::Text(t) => {
            if std::mem::take(&mut b.committing) {
                ime_commit(app, ctx, pass, t);
            } else if t.chars().all(|c| !c.is_control()) && !t.is_empty() {
                match crate::dialogs_smart_art::type_into(app, ctx, t) {
                    crate::dialogs_smart_art::SmartArtTypeResult::Changed => {
                        pass.ime_application_change(ctx);
                        return;
                    }
                    crate::dialogs_smart_art::SmartArtTypeResult::ConsumedNoChange => return,
                    crate::dialogs_smart_art::SmartArtTypeResult::NotHandled => {}
                }
                // Typing with a text box selected types into it.
                crate::objects::enter_text_box(app);
                type_text(app, pass, ctx, t, false);
            }
        }
        EditorInput::ImeCommit => {
            app.canvas.ime_preedit.clear();
            app.canvas.ime_preedit_range = None;
            b.committing = true;
        }
        EditorInput::DeleteSurrounding { before, after, unit, .. } => {
            if matches!(unit, SurroundingUnit::Bytes) {
                delete_surrounding_bytes(app, pass, ctx, *before, *after);
            } else {
                delete_surrounding(app, pass, ctx, *before, *after);
            }
        }
        EditorInput::Preedit { text, cursor, .. } => {
            if text.len() <= MAX_IME_PREEDIT_BYTES {
                app.canvas.ime_preedit = text.to_string();
                app.canvas.ime_preedit_range = cursor.as_ref().map(|r| chars_range(text, r));
            } else {
                log::warn!("ignoring an IME pre-edit of {} bytes", text.len());
                app.canvas.ime_preedit.clear();
                app.canvas.ime_preedit_range = None;
            }
        }
        EditorInput::Move(mv, m) => move_input(app, *mv, *m),
        // Shift+Delete alone cuts the selection and does nothing without one, as in Word for Windows
        // (#1669); Shift+Backspace erases (#707).
        EditorInput::Delete { forward: true, by: Granularity::Unit, shift: true, .. } => {
            if !app.session.sel.is_collapsed()
                && let Ok(r) = app.run("edit.cut", json!({}))
            {
                crate::system_clipboard::put_result(app, ctx, &r);
            }
        }
        EditorInput::Delete { forward, by, .. } => {
            let key = if *forward { Key::Delete } else { Key::Backspace };
            if crate::objects::key(app, key, Modifiers::NONE) {
                return;
            }
            let id = match (forward, by) {
                (true, Granularity::Word) => "text.deleteWordForward",
                (false, Granularity::Word) => "text.deleteWordBack",
                (true, _) => "text.delete",
                (false, _) => "text.backspace",
            };
            let _ = app.run(id, json!({}));
        }
        EditorInput::Newline => {
            if !crate::objects::key(app, Key::Enter, Modifiers::NONE) {
                let _ = app.run("text.newParagraph", json!({}));
            }
        }
        EditorInput::Tab => {
            if !crate::objects::key(app, Key::Tab, Modifiers::NONE) {
                let _ = app.run("text.tab", json!({}));
            }
        }
        EditorInput::BackTab => {
            if !crate::objects::key(app, Key::Tab, Modifiers { shift: true, ..Modifiers::NONE }) {
                let _ = app.run("text.backTab", json!({}));
            }
        }
        EditorInput::Cancel => cancel(app, ctx),
        EditorInput::Edit(cmd) => edit_command(app, ctx, *cmd),
        _ => {}
    }
}

fn move_input(app: &mut WordApp, mv: NavMove, m: MoveMods) {
    let mods = Modifiers { shift: m.extend, command: m.lead, ..Modifiers::NONE };
    if let Some(key) = move_key(mv)
        && crate::objects::key(app, key, mods)
    {
        return;
    }
    if let Some(id) = move_command(mv, m.by) {
        let _ = app.run(id, json!({"extend": m.extend}));
    }
}

fn edit_command(app: &mut WordApp, ctx: &egui::Context, cmd: EditCommand) {
    match cmd {
        EditCommand::Cut => {
            if let Ok(r) = app.run("edit.cut", json!({})) {
                crate::system_clipboard::put_result(app, ctx, &r);
            }
        }
        EditCommand::Copy => {
            let _ = app.run("edit.copy", json!({}));
        }
        EditCommand::Paste => paste(app, ctx),
        EditCommand::SelectAll => {
            let _ = app.run("select.all", json!({}));
        }
        EditCommand::Undo => {
            let _ = app.run("edit.undo", json!({}));
        }
        EditCommand::Redo => {
            let _ = app.run("edit.redo", json!({}));
        }
        EditCommand::Delete => {
            let _ = app.run("text.delete", json!({}));
        }
        _ => {}
    }
}

/// Paste from the clipboard: copied picture files insert as pictures (#45), text and a clipboard
/// picture go through `edit.paste`.
fn paste(app: &mut WordApp, ctx: &egui::Context) {
    let clip = ClipboardHandle::of(ctx);
    let text = clip.read_text().ok().flatten();
    if text.is_none() && !clip.has(&[RGBA8, URI_LIST]) {
        return;
    }
    if let Some(t) = &text
        && crate::paste_picture::paste_named_files(app, t)
    {
        return;
    }
    let params = text.map_or_else(|| json!({}), |t| json!({"text": t}));
    let _ = app.run("edit.paste", params);
}

/// Escape: the nearest thing that takes it first (a painter, a pen, a table pen, an object, a text
/// box, a header), then Focus or Full Screen.
fn cancel(app: &mut WordApp, ctx: &egui::Context) {
    // The text menu stays open while its bar's commands hand the keyboard back to the page (Bold, then type): its
    // Escape then reaches the page, and dismisses the menu before anything on the page.
    if app.canvas.context_menu_open {
        app.canvas.context_menu_open = false;
        app.canvas.context_menu_at = None;
        return;
    }
    if app.session.painter.is_some() {
        app.session.painter = None;
        return;
    }
    // Escape puts the pen or eraser down (back to selecting).
    if app.session.view.draw.mode != wordcraft_engine::cmd::draw::DrawMode::Select {
        let _ = app.run("draw.select", json!({}));
        return;
    }
    if crate::table_pen::stop(app) {
        return;
    }
    if crate::objects::key(app, Key::Escape, Modifiers::NONE) {
        return;
    }
    // Escape in a text box (or WordArt) selects the box itself, so Delete then removes
    // it (#1538). The anchor position is just after the box's object character.
    if crate::canvas::in_text_box(app)
        && let wordcraft_doc::StoryRef::Part(id) = app.session.sel.focus.story
        && let Some(end) = app.session.doc.text_box_anchor(id)
        && let Some(off) = end.off.checked_sub(wordcraft_doc::para::OBJ.len_utf8())
    {
        let start = wordcraft_doc::Pos { off, ..end.clone() };
        let _ = app.run("select.range", json!({"anchor": start, "focus": end}));
        return;
    }
    if matches!(app.session.sel.focus.story, wordcraft_doc::StoryRef::Part(_)) && app.session.sel.is_collapsed() {
        let _ = app.run("insert.closeHeader", json!({}));
        return;
    }
    if leave_full_screen(app, ctx, Key::Escape, Modifiers::NONE, true) {
        return;
    }
    // Plain Escape reaches the page as the editor's cancel, never as a chord, so the engine's
    // `select.collapse` binding (Escape) is run here: a selection collapses to its start.
    if !app.session.sel.is_collapsed() {
        let _ = app.run("select.collapse", json!({}));
    }
}

/// The Escape no component took, read last in the window's UI: the framework gives plain Escape to the focused
/// component or, with none focused, to the window's intents, never to a command's chord. With the File page open it
/// goes back to the document (`backstagePage.back`'s default key); otherwise it leaves Focus or Full Screen View, as
/// the page's own Escape does when the page has the keyboard.
pub(crate) fn window_cancel(app: &mut WordApp, ctx: &egui::Context) {
    use craft::ui::runtime::keys::{Intent, with_window_intents};
    if !with_window_intents(ctx, false, |window| window.take(Intent::Cancel)) {
        return;
    }
    if app.ui.backstage {
        // A dialog, menu or popup over the page took its own Escape before this read.
        if app.dialog_host.is_empty() {
            let _ = app.run("ui.backstage", json!({"value": false}));
            app.canvas.focus_page(ctx);
        }
        return;
    }
    let _ = leave_full_screen(app, ctx, Key::Escape, Modifiers::NONE, false);
}

/// Escape leaves Focus or Full Screen View (#1678), then Full Screen (#980), once nothing nearer
/// took it: a pen, a text box, a header, a dialog, a menu or a popup, a drag on the page, and in
/// the document (`canvas`) a selection, which Escape collapses first (`select.collapse`), as in a
/// normal window.
pub(super) fn leave_full_screen(app: &mut WordApp, ctx: &egui::Context, key: Key, m: Modifiers, canvas: bool) -> bool {
    let screen = crate::chrome::Screen::of(&app.session.view).exit_command();
    let Some(id) = screen.or(app.session.view.full_screen.then_some("view.fullScreen")) else { return false };
    if key != Key::Escape || !m.is_none() || !app.escape_free {
        return false;
    }
    // Escape with a button held (dragging a task pane's title bar, say) cancels the drag only.
    // egui has already dropped the drag itself, so the held button is what is left of it.
    if ctx.input(|i| i.pointer.any_down()) {
        return false;
    }
    let c = &app.canvas;
    if c.object_gesture().is_some() || c.table_resize.is_some() || crate::table_mouse::dragging(app) || c.link_from.is_some() {
        return false;
    }
    if canvas && !app.session.sel.is_collapsed() {
        return false;
    }
    let _ = app.run(id, json!({"value": false}));
    true
}

/// Typed text (a key's text or an input method's commit). Text replacing what an input method's
/// `DeleteSurrounding` just erased takes the erased text's formatting, never overtypes what
/// follows, and stays in the typing undo group (#1480).
///
/// `ime` says this text came from an accepted IME commit. Only then is a receipt taken, and only
/// around the insertion itself: a transformation the engine performs (AutoCorrect, AutoFormat)
/// makes the frame an application change instead.
fn type_text(app: &mut WordApp, pass: &ToolPass, ctx: &egui::Context, text: &str, ime: bool) {
    let before = ime.then(|| ime_stamp(app));
    // Where the text lands, so a quote can be checked against what the engine actually inserted.
    let start = app.session.sel.ordered().0;
    let pending_before = app.session.pending.clone();
    let typed = if let Some(props) = app.canvas.ime_join_text.take() {
        // The replacement takes the erased text's formatting. A failed `Session::run` puts back
        // what was waiting when it started, which is this formatting, so the earlier one is put
        // back here: a refused replacement must not leave its formatting waiting for the next
        // ordinary key.
        // The undo join itself is the engine's scoped body-replacement promise, consumed by
        // `Session::run` only for this `text.insert` at the promised selection.
        app.session.pending = Some(props);
        let overtype = std::mem::replace(&mut app.session.prefs.overtype, false);
        let typed =
            if ime { app.run_ime("text.insert", json!({"text": text})).is_ok() } else { app.run("text.insert", json!({"text": text})).is_ok() };
        app.session.prefs.overtype = overtype;
        if typed {
            app.session.continue_typing();
        } else {
            app.session.pending = pending_before;
            // The platform already applied its own deletion to the text it believed was there.
            // Its replacement was refused, so the surrounding snapshot is stale and the
            // deletion's join must not reach a later composition.
            app.session.clear_ime_text_join();
            app.canvas.ime_interrupt_now = true;
        }
        typed
    } else if ime {
        app.run_ime("text.insert", json!({"text": text})).is_ok()
    } else {
        app.run("text.insert", json!({"text": text})).is_ok()
    };
    if !typed {
        return;
    }
    let mut outcome = app.session.ime_edit_outcome();
    // The engine's own outcome is the per-command provenance: `text.insert` compares the paragraph
    // with the one it typed and marks AutoCorrect/list/dash rewrites `transformed`. A smart quote
    // is also checked against the actual inserted text, so smart quotes off stays the IME's own
    // edit. Never infer this from an undo label, which may belong to an earlier step. A refused
    // insert has no provenance at all: it must not fabricate an AutoFormat change or receipt.
    let quote = text == "\"" || text == "'";
    outcome.transformed |= quote && !inserted_literal_at(app, text, &start);
    note_automatic_change(app, outcome);
    if let Some(before) = before {
        let after = ime_stamp(app);
        ime_report(app, pass, ctx, before, after, outcome);
    }
}

/// Whether the text at `start` is exactly `text` (the engine inserted it unchanged).
///
/// A smart quote, an AutoCorrect replacement or a relocated insert makes this false: the edit is
/// AutoFormat work, not the input method's own change.
fn inserted_literal_at(app: &WordApp, text: &str, start: &wordcraft_doc::Pos) -> bool {
    let Some(para) = app.session.doc.para_at(start) else { return false };
    let Some(end) = start.off.checked_add(text.len()) else { return false };
    para.text.get(start.off..end) == Some(text)
}
