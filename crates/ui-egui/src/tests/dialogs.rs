use super::*;
use crate::dialogs::host::FormHost;
use crate::dialogs::{RecipientDraft, RecipientListDialog};
use craft::host::{AppInfo, Host, Picked, ScriptedFiles};
use std::sync::Arc;

/// One frame of the app in `ctx`, so the dialog host shows, settles and answers what was asked
/// since the last one. A test keeps one context, so what the host keeps in egui memory lasts
/// between frames.
fn pump(a: &mut WordApp, ctx: &egui::Context) {
    crate::test_harness::pass(ctx, a, egui::RawInput::default());
}

/// An app on `document` whose file service is `files` (app.H1).
fn app_over(files: &Arc<ScriptedFiles>, document: wordcraft_doc::Document) -> WordApp {
    let host = Host::builder(AppInfo { id: "wordcraft-test", name: "WordCraft", version: "0" }).files(files.clone()).build();
    WordApp::new(Session::new(document), Services::default()).with_host(Arc::new(host))
}

/// A file dropped on the window, as the platform delivers it.
#[cfg(not(target_arch = "wasm32"))]
fn drop_file(a: &mut WordApp, path: &std::path::Path) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    drop_files(a, vec![craft::platform::open::DroppedFile { path: Some(path.to_path_buf()), name, bytes: None }]);
}

/// The key of the dialog the host has open, if any.
fn open_key(a: &WordApp) -> Option<String> {
    a.dialog_host.open_dialogs().into_iter().find_map(|d| d.key)
}

/// The JSON fields of the open dialog `key`.
fn fields_of(a: &WordApp, key: &str) -> Option<Value> {
    a.dialog_host.open_dialogs().into_iter().find(|d| d.key.as_deref() == Some(key)).and_then(|d| d.fields)
}

/// #4857: with the whole table selected, a row check box the rows differ in shows as mixed and
/// changes nothing left alone; clicked, OK sets it for every selected row.
#[test]
fn table_properties_dialog_sets_every_selected_row() {
    let mut a = app();
    a.run("insert.table", json!({"rows": 3, "cols": 2})).unwrap();
    a.run("table.properties", json!({"allowBreak": false})).unwrap();
    a.run("table.selectTable", json!({})).unwrap();
    a.run("table.properties", json!({})).unwrap();
    let ctx = crate::test_harness::context();
    pump(&mut a, &ctx);
    let Some(host) = a.dialog_host.get::<FormHost<dialogs::TableProperties>>() else { panic!("no table dialog: {:?}", open_key(&a)) };
    let mixed = &host.form().form.mixed;
    assert!(mixed.contains("allowBreak") && !mixed.contains("headerRow"), "{mixed:?}");
    // OK with the mixed box left alone changes no row.
    let rows_breaking = |a: &WordApp| {
        let (tp, _, _) = a.session.sel.focus.path.cell().unwrap();
        a.session.doc.table(a.session.sel.focus.story, &tp).unwrap().rows.iter().map(|r| r.props.cant_split).collect::<Vec<_>>()
    };
    let before = rows_breaking(&a);
    let depth = a.session.undo_depth();
    let id = a.dialog_host.open_dialogs().first().map(|d| d.id).expect("the dialog is open");
    // The held OK files on the first confirm after a shown pass and finishes on a later one (S-L291): the
    // return says nothing, the passes and the document do.
    let _ = a.dialog_host.confirm_by_id(&ctx, id);
    for _ in 0..3 {
        pump(&mut a, &ctx);
    }
    assert!(a.dialog_host.open_dialogs().is_empty(), "OK closes the dialog");
    assert_eq!(rows_breaking(&a), before, "a mixed box left alone changes nothing");
    assert_eq!(a.session.undo_depth(), depth, "and records no edit");
    a.run("table.selectTable", json!({})).unwrap();
    a.run("table.properties", json!({})).unwrap();

    // Drawn: one click on the mixed box turns breaking on for every row.
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| a);
    for _ in 0..4 {
        h.step();
    }
    h.get_by_label("Allow row to break across pages").click();
    for _ in 0..4 {
        h.step();
    }
    h.get_by_label("OK").click();
    for _ in 0..4 {
        h.step();
    }
    let a = h.state();
    assert!(a.dialog_host.open_dialogs().is_empty(), "OK closes the dialog");
    let (tp, _, _) = a.session.sel.focus.path.cell().unwrap();
    let t = a.session.doc.table(a.session.sel.focus.story, &tp).unwrap();
    assert_eq!(t.rows.iter().map(|r| r.props.cant_split).collect::<Vec<_>>(), [false, false, false]);
    let g = dialogs::TableForm::read(a).unwrap();
    assert!(g.mixed.is_empty() && g.allow_break, "reopened, the box is plainly on");
}

/// Table Layout › Cell Margins opens with the table defaults and does not replace imported
/// values until the user changes a value and confirms.
#[test]
fn cell_margins_button_preserves_existing_values_until_confirmed() {
    let mut a = app();
    a.run("insert.table", json!({"rows": 1, "cols": 1})).unwrap();
    let (tp, _, _) = a.session.sel.focus.path.cell().unwrap();
    let table = a.session.doc.table_mut(a.session.sel.focus.story, &tp).unwrap();
    table.props.cell_margins = Some([3.0, 6.0, 9.0, 12.0]);
    table.rows[0].cells[0].props.margins = Some([1.0, 2.0, 3.0, 4.0]);

    let ctx = crate::test_harness::context();
    let frame = |a: &mut WordApp| pump(a, &ctx);
    // A dialog opens once the window's context is known: after its first frame.
    frame(&mut a);
    let id = dialogs::open(&mut a, "tableCellMargins", &json!({})).expect("the dialog opens");
    frame(&mut a);
    let fields = fields_of(&a, "tableCellMargins").expect("the dialog has fields");
    let side = |k: &str| fields.get(k).and_then(Value::as_f64);
    assert_eq!((side("top"), side("left"), side("bottom"), side("right")), (Some(3.0), Some(6.0), Some(9.0), Some(12.0)));
    let depth = a.session.undo_depth();
    // The return of `confirm_by_id` is not asserted (S-L291): the held OK finishes on a later pass.
    let _ = a.dialog_host.confirm_by_id(&ctx, id);
    for _ in 0..3 {
        frame(&mut a);
    }
    let table = a.session.doc.table(a.session.sel.focus.story, &tp).unwrap();
    assert_eq!(table.props.cell_margins, Some([3.0, 6.0, 9.0, 12.0]), "an untouched dialog does not reset margins");
    assert_eq!(a.session.undo_depth(), depth, "an untouched dialog writes no table props");

    let id = dialogs::open(&mut a, "tableCellMargins", &json!({})).expect("the dialog opens");
    frame(&mut a);
    a.dialog_host.set_json_field(&ctx, id, "right", json!(18.0)).unwrap();
    frame(&mut a);
    let _ = a.dialog_host.confirm_by_id(&ctx, id);
    for _ in 0..3 {
        frame(&mut a);
    }
    let table = a.session.doc.table(a.session.sel.focus.story, &tp).unwrap();
    assert_eq!(table.props.cell_margins, Some([3.0, 6.0, 9.0, 18.0]));
    assert_eq!(table.rows[0].cells[0].props.margins, Some([1.0, 2.0, 3.0, 4.0]), "cell-specific margins remain intact");

    // A table with no direct margins stays without them: the style's or fallback margins the
    // dialog shows are not written back.
    let mut b = app();
    b.run("insert.table", json!({"rows": 1, "cols": 1})).unwrap();
    let (tp, _, _) = b.session.sel.focus.path.cell().unwrap();
    assert_eq!(b.session.doc.table(b.session.sel.focus.story, &tp).unwrap().props.cell_margins, None);
    let ctx = crate::test_harness::context();
    pump(&mut b, &ctx);
    let id = dialogs::open(&mut b, "tableCellMargins", &json!({})).expect("the dialog opens");
    pump(&mut b, &ctx);
    let depth = b.session.undo_depth();
    let _ = b.dialog_host.confirm_by_id(&ctx, id);
    for _ in 0..3 {
        pump(&mut b, &ctx);
    }
    assert_eq!(b.session.doc.table(b.session.sel.focus.story, &tp).unwrap().props.cell_margins, None, "untouched, no direct margins appear");
    assert_eq!(b.session.undo_depth(), depth);
}

/// #55: a password-protected document asks for its password (after Save Changes) instead of
/// opening blank; scripts get the error instead of a dialog; Encrypt with Password asks for one.
#[test]
fn password_protected_documents_ask_for_the_password() {
    let dir = scratch("password");
    let path = dir.join("locked.docx");
    let mut s = Session::new(wordcraft_doc::Document::from_text("Locked away"));
    s.run("file.save", &json!({"path": path.to_string_lossy(), "password": "sesame"})).unwrap();

    let ctx = crate::test_harness::context();
    let mut a = typed();
    assert!(a.execute("file.open", json!({"path": path.to_string_lossy()})).is_err(), "a script gets the error");
    assert_eq!(open_key(&a), None);
    a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    pump(&mut a, &ctx);
    assert_eq!(open_key(&a).as_deref(), Some("password"));
    assert!(body_text(&a).contains(UNSAVED), "nothing opens until the password fits");
    let state = fields_of(&a, "password").expect("the prompt has fields").to_string();
    assert!(state.contains("locked.docx") && !state.contains("sesame"));

    let mut b = app();
    b.run("file.encrypt", json!({})).unwrap();
    pump(&mut b, &ctx);
    assert_eq!(open_key(&b).as_deref(), Some("encryptPassword"));
    let mut c = app();
    c.run("file.encrypt", json!({"password": null})).unwrap();
    pump(&mut c, &ctx);
    assert_eq!(open_key(&c), None, "null removes the password without asking");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #778: Edit Recipient List shows every record with the list's filter and sort, and its
/// Filter and Sort section maps to `mailings.filterRecipients`.
#[test]
fn recipient_list_dialog_carries_the_filter() {
    let mut a = app();
    a.run("mailings.recipients", json!({"csv": "Name,City\nAda,London\nAlan,Wilmslow\nGrace,London"})).unwrap();
    a.run(
        "mailings.filterRecipients",
        json!({"rules": [{"field": "City", "op": "equal", "value": "London"}], "sort": [{"field": "Name", "descending": true}]}),
    )
    .unwrap();
    a.run("mailings.editRecipients", json!({})).unwrap();
    pump(&mut a, &crate::test_harness::context());
    let Some(RecipientListDialog { draft: RecipientDraft { rows, filter, fields }, .. }) = a.dialog_host.get::<RecipientListDialog>() else {
        panic!("no recipient dialog")
    };
    assert_eq!(rows.len(), 3, "every record, not only the filtered ones");
    assert!(filter.open);
    let params = filter.params(fields);
    // A rule or sort key on a field renamed or removed in the table drops (#1287).
    assert_eq!(filter.params(&["Name".to_string()]), json!({"rules": [], "sort": [{"field": "Name", "descending": true}]}));
    assert_eq!(params["rules"], json!([{"field": "City", "op": "equal", "value": "London", "join": "and"}]));
    assert_eq!(params["sort"], json!([{"field": "Name", "descending": true}]));
    // Applying the dialog's filter to a reloaded list gives the same records.
    a.run("mailings.recipients", json!({"csv": "Name,City\nAda,London\nAlan,Wilmslow\nGrace,London"})).unwrap();
    a.run("mailings.filterRecipients", params).unwrap();
    let names: Vec<&str> = a.session.merge.rows.iter().map(|r| r[0].as_str()).collect();
    assert_eq!(names, ["Grace", "Ada"]);
    assert!(open_filters("recipients")[0].1.contains(&"xls"));
}

/// #1576: Find in list and "only records that will be merged" hide rows from the grid
/// without touching the list; the shown rows keep their real indices for edit and remove.
#[test]
fn recipient_list_search_and_merged_view() {
    let fields: Vec<String> = vec!["Name".into(), "City".into()];
    let rows: Vec<Vec<String>> = [["Ada", "London"], ["Alan", "Wilmslow"], ["Grace", "LONDON"], ["Émile", "Paris"]]
        .iter()
        .map(|r| r.iter().map(|c| c.to_string()).collect())
        .collect();
    let none = dialogs::visible_recipients(&rows, None, "", false);
    assert_eq!(none, [0, 1, 2, 3], "no search, no filter: every row");
    // Search: any cell, any case, including non-ASCII.
    assert_eq!(dialogs::visible_recipients(&rows, None, "london", false), [0, 2]);
    assert_eq!(dialogs::visible_recipients(&rows, None, "  AL ", false), [1], "trimmed, any case");
    assert_eq!(dialogs::visible_recipients(&rows, None, "émi", false), [3]);
    let greek = vec![vec!["ΟΣ".to_string()], vec!["ος".to_string()], vec!["Αλφα".to_string()]];
    assert_eq!(dialogs::visible_recipients(&greek, None, "ος", false), [0, 1], "final sigma matches capital sigma");
    assert_eq!(dialogs::visible_recipients(&rows, None, "ÉMI", false), [3]);
    assert!(dialogs::visible_recipients(&rows, None, "zzz", false).is_empty());
    // Filter only: City = London keeps Ada and Grace.
    let mut filter = dialogs::RecipientFilter::default();
    filter.rules.push(dialogs::FilterRow {
        field: "City".into(),
        op: wordcraft_engine::cmd::mailings::FilterOp::Equal,
        value: "London".into(),
        or: false,
    });
    let order = dialogs::merge_order(&fields, &rows, &filter).unwrap();
    assert_eq!(order, [Some(1), None, Some(2), None]);
    assert_eq!(dialogs::visible_recipients(&rows, Some(&order), "", false), [0, 1, 2, 3], "unchecked: excluded rows still shown");
    assert_eq!(dialogs::visible_recipients(&rows, Some(&order), "", true), [0, 2]);
    // Both: search within the merged records.
    assert_eq!(dialogs::visible_recipients(&rows, Some(&order), "grace", true), [2]);
    assert!(dialogs::visible_recipients(&rows, Some(&order), "alan", true).is_empty());
    // No filter: only_merged hides nothing.
    assert_eq!(dialogs::visible_recipients(&rows, None, "", true), [0, 1, 2, 3]);
    // Removing the first shown row removes the real record, not row 0.
    let shown = dialogs::visible_recipients(&rows, None, "grace", false);
    let mut edited = rows.clone();
    edited.remove(shown[0]);
    assert_eq!(edited.iter().map(|r| r[0].as_str()).collect::<Vec<_>>(), ["Ada", "Alan", "Émile"]);
    // Hostile order (shorter than the list) never panics; missing entries count as excluded.
    assert_eq!(dialogs::visible_recipients(&rows, Some(&[Some(1)]), "", true), [0]);
    // The view settings don't reach the merge.
    let mut viewed = filter.clone();
    viewed.find = "grace".into();
    viewed.only_merged = true;
    assert_eq!(viewed.params(&fields), filter.params(&fields));
}

/// #1576 review: while a cell is edited the grid keeps its rows, so a record the edit stops
/// matching stays put instead of sliding another record under the cursor; and New Entry
/// shows every row, so the new, empty entry can't be hidden.
#[test]
fn recipient_grid_holds_its_rows_while_a_cell_is_edited() {
    let mut held = None;
    // Not editing: whatever matches now.
    assert_eq!(dialogs::hold_rows(vec![0, 2], &mut held, false, 4), [0, 2]);
    assert_eq!(held, None);
    // Editing starts: the rows shown then are held…
    assert_eq!(dialogs::hold_rows(vec![0, 2], &mut held, true, 4), [0, 2]);
    // …even when the edit makes row 0 stop matching.
    assert_eq!(dialogs::hold_rows(vec![2], &mut held, true, 4), [0, 2]);
    // A held row past the end of a shrunk list is dropped, never indexed.
    assert_eq!(dialogs::hold_rows(vec![], &mut held, true, 1), [0]);
    // Editing ends: the view catches up.
    assert_eq!(dialogs::hold_rows(vec![2], &mut held, false, 4), [2]);
    assert_eq!(held, None);
    assert_eq!(dialogs::hold_rows(vec![1], &mut held, true, 4), [1], "a new edit holds the new rows");

    let mut filter = dialogs::RecipientFilter { find: "grace".into(), only_merged: true, held: Some(vec![1]), ..Default::default() };
    filter.show_all();
    assert!(filter.find.is_empty() && !filter.only_merged && filter.held.is_none());
}

/// #1576: the real Edit Recipient List grid in a harness (the dialog body, without the app
/// around it): Find, merged-only, an edit while the search hides rows, remove, New Entry.
#[test]
fn recipient_grid_edits_the_right_record_through_the_ui() {
    use egui_kittest::kittest::Queryable;
    type State = WordApp;
    let mut a = app();
    a.run("mailings.recipients", json!({"csv": "Name,City\nAda,London\nAlan,Wilmslow\nGrace,London\nÉmile,Paris"})).unwrap();
    a.run("mailings.filterRecipients", json!({"rules": [{"field": "City", "op": "equal", "value": "London"}]})).unwrap();
    dialogs::open(&mut a, "recipientList", &json!({})).expect("the dialog opens");
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| a);
    h.run_steps(4);
    let dlg = |h: &egui_kittest::Harness<'_, State>| {
        let d = h.state().dialog_host.get::<RecipientListDialog>().expect("the recipient dialog is open");
        (d.draft.rows.clone(), d.draft.filter.clone())
    };
    // The records the grid shows: one Remove part per row on screen.
    let cells = |h: &egui_kittest::Harness<'_, State>| h.query_all_by_label("Remove recipient").count();
    // A record's cell field, found by the text it holds.
    // A macro, as the node it finds borrows the harness for as long as the caller keeps it.
    macro_rules! cell_of {
        ($h:expr, $text:expr) => {
            $h.query_all_by_role(egui::accesskit::Role::TextInput).find(|n| n.value().as_deref() == Some($text)).expect("the record's cell is shown")
        };
    }
    let names = |h: &egui_kittest::Harness<'_, State>| dlg(h).0.iter().map(|r| r[0].clone()).collect::<Vec<_>>();
    // Find in list: the first text box. Grace is the only match.
    let find = h.query_all_by_role(egui::accesskit::Role::TextInput).next().unwrap();
    find.click();
    h.run_steps(2);
    h.query_all_by_role(egui::accesskit::Role::TextInput).next().unwrap().type_text("grace");
    h.run_steps(3);
    assert_eq!(dlg(&h).1.find, "grace");
    assert!(cells(&h) == 1, "search hides the other rows");
    // Merged-only on top of it: still Grace (London is kept).
    h.get_by_label("Show only records that will be merged").click();
    h.run_steps(3);
    assert!(dlg(&h).1.only_merged && cells(&h) == 1);
    // Edit Grace's cell to something the search no longer matches: the row stays, and the
    // edit lands on record 2, not on the first visible row's index.
    cell_of!(h, "Grace").click();
    h.run_steps(2);
    h.event(egui::Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers { command: true, ..Default::default() },
    });
    cell_of!(h, "Grace").type_text("Zed");
    h.run_steps(3);
    assert_eq!(names(&h), ["Ada", "Alan", "Zed", "Émile"]);
    assert_eq!(dlg(&h).1.held, Some(vec![2]), "rows are held while the cell has the keyboard");
    assert_eq!(cells(&h), 1);
    // Remove the one shown record, by its Remove part.
    h.get_by_label("Remove recipient").click();
    h.run_steps(3);
    assert_eq!(names(&h), ["Ada", "Alan", "Émile"]);
    assert!(dlg(&h).1.held.is_none());
    // New Entry clears the view and appends an empty record.
    h.get_by_label("New Entry").click();
    h.run_steps(3);
    assert_eq!(dlg(&h).0, vec![vec!["Ada", "London"], vec!["Alan", "Wilmslow"], vec!["Émile", "Paris"], vec!["", ""]]);
    assert!(dlg(&h).1.find.is_empty() && !dlg(&h).1.only_merged);
}

/// Desktop: Use an Existing List… picks a file and loads it as recipients, not as a document.
#[test]
fn an_existing_list_picked_on_the_desktop_loads_recipients() {
    let dir = scratch("recipients");
    let csv = dir.join("people.csv");
    std::fs::write(&csv, "First Name,City\nAda,London\nAlan,Wilmslow\n").unwrap();
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec![csv.clone()])]));
    let mut a = app_over(&files, wordcraft_doc::Document::new());
    a.run("text.insert", json!({"text": UNSAVED})).unwrap();
    a.run("ui.openRecipientList", json!({})).unwrap();
    pump(&mut a, &crate::test_harness::context());
    assert_eq!(files.requests().first().map(|r| r.purpose.as_str()), Some("recipients"), "the host can offer CSV files");
    assert_eq!(a.session.merge.headers, vec!["First Name", "City"]);
    assert_eq!(a.session.merge.rows.len(), 2);
    assert!(body_text(&a).contains(UNSAVED), "the document stays open");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Web: the picked file arrives through the inbox and loads as recipients, even as .txt; a
/// CSV dropped on the page does too. Anything else afterwards opens as before.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn an_existing_list_picked_on_the_web_loads_recipients() {
    // The scripted browser host holds each pick open until the test answers it with the file contents, as the page
    // does when the person has chosen.
    let files = Arc::new(ScriptedFiles::held().in_browser());
    let contents = |name: &str, bytes: &[u8]| Picked::Contents(vec![(name.into(), bytes.to_vec())]);
    let mut a = app_over(&files, wordcraft_doc::Document::new());
    a.run("text.insert", json!({"text": UNSAVED})).unwrap();
    let ctx = crate::test_harness::context();
    assert_eq!(a.run("ui.openRecipientList", json!({})).unwrap(), json!({"pending": true}));
    assert_eq!(files.requests().first().map(|r| r.purpose.as_str()), Some("recipients"));
    files.answer(contents("people.txt", b"Name\tCity\nAda\tLondon\n"));
    pump(&mut a, &ctx);
    assert_eq!(a.session.merge.headers, vec!["Name", "City"]);
    assert!(!a.recipient_list_pending);
    assert!(body_text(&a).contains(UNSAVED), "the document stays open");
    a.run("ui.openRecipientList", json!({})).unwrap();
    files.answer(contents("more.csv", b"Name\nBo\nCy\n"));
    pump(&mut a, &ctx);
    assert_eq!(a.session.merge.rows.len(), 2);
    // A CSV picked through the picker must not turn the next .txt drop into recipients.
    a.run("ui.openRecipientList", json!({})).unwrap();
    files.answer(contents("people.csv", b"Name,City\nAda,London\n"));
    pump(&mut a, &ctx);
    assert!(!a.recipient_list_pending);
    let dir = scratch("recipients-web");
    let notes = dir.join("notes.txt");
    std::fs::write(&notes, "A plain text document.\nAnother line.\n").unwrap();
    drop_file(&mut a, &notes);
    pump(&mut a, &ctx);
    assert_eq!(prompt(&a), None, "the TXT opens like any document, beside the first");
    assert!(body_text(&a).contains("A plain text document."), "the TXT opened as a document");
    let first = a.session.documents().first().map(|d| d.id).unwrap();
    a.run("view.switchWindows", json!({"id": first})).unwrap();
    assert_eq!(a.session.merge.headers, vec!["Name", "City"], "the recipients stay with their document");
    // A cancelled picker leaves nothing pending once the user does something else.
    a.run("ui.openRecipientList", json!({})).unwrap();
    files.answer(Picked::Cancelled);
    a.run("format.bold", json!({})).unwrap();
    assert!(!a.recipient_list_pending);
    let _ = std::fs::remove_dir_all(&dir);
}

/// #712: Find, Replace and Go To open as in Word. Ctrl+F puts the keyboard in the Navigation
/// pane's search box (with the selected text in it), not a dialog.
#[test]
fn find_replace_and_go_to_keys_like_word() {
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| app());
    for _ in 0..6 {
        h.step();
    }
    let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    h.event(egui::Event::Text("Kilns fire clay. Kilns cool.".into()));
    h.step();
    let nav_query = |h: &egui_kittest::Harness<WordApp>| h.ctx.data(|d| d.get_temp::<String>(h.state().window_id("nav_query"))).unwrap_or_default();
    // Ctrl+F: the pane, and typing goes into its search box.
    h.event(key(egui::Key::F, egui::Modifiers::COMMAND));
    h.step();
    h.step();
    assert!(h.state().session.view.nav_pane, "the Navigation pane opened");
    assert_eq!(h.state().ui.nav_tab, "results", "Find exposes its results (#2096)");
    assert!(h.state().dialog_host.open_dialogs().is_empty(), "no dialog: {:?}", open_key(h.state()));
    h.event(key(egui::Key::C, egui::Modifiers::NONE));
    h.event(egui::Event::Text("cool".into()));
    h.step();
    h.step();
    assert_eq!(nav_query(&h), "cool", "typed into the pane's search box");
    assert_eq!(h.state().session.find.results.len(), 1);
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Kilns fire clay. Kilns cool.");
    // Manual navigation remains available until the next search edit.
    h.state_mut().ui.nav_tab = "headings".into();
    h.step();
    assert_eq!(h.state().ui.nav_tab, "headings");
    h.event(egui::Event::Text("missing".into()));
    h.step();
    h.step();
    assert_eq!(h.state().ui.nav_tab, "results", "editing the query reveals even an empty result list");
    assert!(h.state().session.find.results.is_empty());
    // With text selected, Ctrl+F searches for it.
    h.state_mut().ui.nav_tab = "pages".into();
    h.state_mut().run("select.text", json!({"text": "Kilns"})).unwrap();
    let ctx = h.ctx.clone();
    h.state_mut().canvas.focus_page(&ctx);
    h.step();
    h.event(key(egui::Key::F, egui::Modifiers::COMMAND));
    h.step();
    h.step();
    assert_eq!(nav_query(&h), "Kilns");
    assert_eq!(h.state().ui.nav_tab, "results", "Find leaves Pages to show the selected-text matches");
    assert_eq!(h.state().session.find.results.len(), 2);
}
