use super::*;

/// #4857: with the whole table selected, a row check box the rows differ in shows as mixed and
/// changes nothing left alone; clicked, OK sets it for every selected row.
#[test]
fn table_properties_dialog_sets_every_selected_row() {
    let mut a = app();
    a.run("insert.table", json!({"rows": 3, "cols": 2})).unwrap();
    a.run("table.properties", json!({"allowBreak": false})).unwrap();
    a.run("table.selectTable", json!({})).unwrap();
    a.run("table.properties", json!({})).unwrap();
    let Some(dialogs::Dialog::TableProperties { form, basis }) = a.dialog.clone() else { panic!("no dialog: {:?}", a.dialog) };
    assert!(form.mixed.contains("allowBreak") && !form.mixed.contains("headerRow"), "{:?}", form.mixed);
    assert_eq!(form.changes(&basis), json!({}), "a mixed box left alone changes nothing");
    let mut f = (*form).clone();
    f.mixed.remove("allowBreak");
    f.allow_break = false;
    assert_eq!(f.changes(&basis), json!({"allowBreak": false}), "clicked, it sets every row");

    // Drawn: one click on the mixed box turns breaking on for every row.
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
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
    assert!(a.dialog.is_none(), "OK closes the dialog");
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

    a.run("table.cellMargins", json!({})).unwrap();
    let Some(dialogs::Dialog::CellMargins { form, basis }) = a.dialog.clone() else { panic!("no dialog: {:?}", a.dialog) };
    assert_eq!((form.top, form.left, form.bottom, form.right), (3.0 / 72.0, 6.0 / 72.0, 9.0 / 72.0, 12.0 / 72.0));
    assert_eq!(form.changes(&basis), json!({}), "an untouched dialog does not reset margins");

    let mut changed = (*form).clone();
    changed.right = 18.0 / 72.0;
    let params = changed.changes(&basis);
    assert_eq!(params, json!({"top": 3.0, "left": 6.0, "bottom": 9.0, "right": 18.0}));
    a.dialog = None;
    a.run("table.cellMargins", params).unwrap();
    let table = a.session.doc.table(a.session.sel.focus.story, &tp).unwrap();
    assert_eq!(table.props.cell_margins, Some([3.0, 6.0, 9.0, 18.0]));
    assert_eq!(table.rows[0].cells[0].props.margins, Some([1.0, 2.0, 3.0, 4.0]), "cell-specific margins remain intact");
}

/// #55: a password-protected document asks for its password (after Save Changes) instead of
/// opening blank; scripts get the error instead of a dialog; Encrypt with Password asks for one.
#[test]
fn password_protected_documents_ask_for_the_password() {
    let dir = scratch("password");
    let path = dir.join("locked.docx");
    let mut s = Session::new(wordcraft_doc::Document::from_text("Locked away"));
    s.run("file.save", &json!({"path": path.to_string_lossy(), "password": "sesame"})).unwrap();

    let mut a = typed();
    assert!(a.execute("file.open", json!({"path": path.to_string_lossy()})).is_err(), "a script gets the error");
    assert_eq!(prompt(&a), None);
    a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!(prompt(&a), Some("password"));
    assert!(body_text(&a).contains(UNSAVED), "nothing opens until the password fits");
    let state = serde_json::to_string(&a.dialog).unwrap();
    assert!(state.contains("locked.docx") && !state.contains("sesame"));

    let mut b = app();
    b.run("file.encrypt", json!({})).unwrap();
    assert_eq!(prompt(&b), Some("encryptPassword"));
    b.dialog = None;
    b.run("file.encrypt", json!({"password": null})).unwrap();
    assert_eq!(prompt(&b), None, "null removes the password without asking");
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
    let Some(dialogs::Dialog::RecipientList { rows, filter, fields, .. }) = &a.dialog else { panic!() };
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
    type State = (WordApp, Vec<String>, Vec<Vec<String>>, String, dialogs::RecipientFilter);
    let rows = [["Ada", "London"], ["Alan", "Wilmslow"], ["Grace", "London"], ["Émile", "Paris"]];
    let mut filter = dialogs::RecipientFilter::default();
    let rule = dialogs::FilterRow { field: "City".into(), op: wordcraft_engine::cmd::mailings::FilterOp::Equal, value: "London".into(), or: false };
    filter.rules.push(rule);
    let state: State =
        (app(), vec!["Name".into(), "City".into()], rows.iter().map(|r| r.iter().map(|c| c.to_string()).collect()).collect(), String::new(), filter);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 600.0)).build_ui_state(
        |ui, s: &mut State| {
            let (app, fields, rows, message, filter) = s;
            dialogs::recipient_list(app, ui, fields, rows, message, filter);
        },
        state,
    );
    h.run_steps(4);
    // Text boxes in order: Find, two field names, then the visible records' cells.
    let cells = |h: &egui_kittest::Harness<'_, State>| h.query_all_by_role(egui::accesskit::Role::TextInput).count().saturating_sub(3) / 2;
    let names = |h: &egui_kittest::Harness<'_, State>| h.state().2.iter().map(|r| r[0].clone()).collect::<Vec<_>>();
    // Find in list: the first text box. Grace is the only match.
    let find = h.query_all_by_role(egui::accesskit::Role::TextInput).next().unwrap();
    find.click();
    h.run_steps(2);
    h.query_all_by_role(egui::accesskit::Role::TextInput).next().unwrap().type_text("grace");
    h.run_steps(3);
    assert_eq!(h.state().4.find, "grace");
    assert!(cells(&h) == 1, "search hides the other rows");
    // Merged-only on top of it: still Grace (London is kept).
    h.get_by_label("Show only records that will be merged").click();
    h.run_steps(3);
    assert!(h.state().4.only_merged && cells(&h) == 1);
    // Edit Grace's cell to something the search no longer matches: the row stays, and the
    // edit lands on record 2, not on the first visible row's index.
    h.query_all_by_role(egui::accesskit::Role::TextInput).nth(3).unwrap().click();
    h.run_steps(2);
    h.event(egui::Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers { command: true, ..Default::default() },
    });
    h.query_all_by_role(egui::accesskit::Role::TextInput).nth(3).unwrap().type_text("Zed");
    h.run_steps(3);
    assert_eq!(names(&h), ["Ada", "Alan", "Zed", "Émile"]);
    assert_eq!(h.state().4.held, Some(vec![2]), "rows are held while the cell has the keyboard");
    assert_eq!(cells(&h), 1);
    // Remove the one shown record: the third "×" (two are field headers).
    let xs: Vec<_> = h.get_all_by_label("×").collect();
    xs[2].click();
    h.run_steps(3);
    assert_eq!(names(&h), ["Ada", "Alan", "Émile"]);
    assert!(h.state().4.held.is_none());
    // New Entry clears the view and appends an empty record.
    h.get_by_label("New Entry").click();
    h.run_steps(3);
    assert_eq!(h.state().2, vec![vec!["Ada", "London"], vec!["Alan", "Wilmslow"], vec!["Émile", "Paris"], vec!["", ""]]);
    assert!(h.state().4.find.is_empty() && !h.state().4.only_merged);
}

/// Desktop: Use an Existing List… picks a file and loads it as recipients, not as a document.
#[test]
fn an_existing_list_picked_on_the_desktop_loads_recipients() {
    let dir = scratch("recipients");
    let csv = dir.join("people.csv");
    std::fs::write(&csv, "First Name,City\nAda,London\nAlan,Wilmslow\n").unwrap();
    let mut a = typed();
    let asked = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    let (seen, path) = (asked.clone(), csv.to_string_lossy().to_string());
    a.services.pick_open = Some(Box::new(move |purpose| {
        *seen.borrow_mut() = purpose.to_string();
        Some(path.clone())
    }));
    a.run("ui.openRecipientList", json!({})).unwrap();
    assert_eq!(*asked.borrow(), "recipients", "the host can offer CSV files");
    assert_eq!(a.session.merge.headers, vec!["First Name", "City"]);
    assert_eq!(a.session.merge.rows.len(), 2);
    assert!(body_text(&a).contains(UNSAVED), "the document stays open");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Web: the picked file arrives through the inbox and loads as recipients, even as .txt; a
/// CSV dropped on the page does too. Anything else afterwards opens as before.
#[test]
fn an_existing_list_picked_on_the_web_loads_recipients() {
    let mut a = typed();
    let asked = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    let seen = asked.clone();
    a.services.open_async = Some(Box::new(move |purpose| *seen.borrow_mut() = purpose.to_string()));
    let inbox: Inbox = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    a.services.inbox = Some(inbox.clone());
    assert_eq!(a.run("ui.openRecipientList", json!({})).unwrap(), json!({"pending": true}));
    assert_eq!(*asked.borrow(), "recipients");
    inbox.lock().unwrap().push(("people.txt".into(), b"Name\tCity\nAda\tLondon\n".to_vec()));
    a.drain_inbox();
    assert_eq!(a.session.merge.headers, vec!["Name", "City"]);
    assert!(!a.recipient_list_pending);
    assert!(body_text(&a).contains(UNSAVED), "the document stays open");
    inbox.lock().unwrap().push(("more.csv".into(), b"Name\nBo\nCy\n".to_vec()));
    a.drain_inbox();
    assert_eq!(a.session.merge.rows.len(), 2);
    // A CSV picked through the picker must not turn the next .txt drop into recipients.
    a.run("ui.openRecipientList", json!({})).unwrap();
    inbox.lock().unwrap().push(("people.csv".into(), b"Name,City\nAda,London\n".to_vec()));
    a.drain_inbox();
    assert!(!a.recipient_list_pending);
    inbox.lock().unwrap().push(("notes.txt".into(), b"A plain text document.\nAnother line.\n".to_vec()));
    a.drain_inbox();
    assert_eq!(prompt(&a), None, "the TXT opens like any document, beside the first");
    assert!(body_text(&a).contains("A plain text document."), "the TXT opened as a document");
    let first = a.session.documents().first().map(|d| d.id).unwrap();
    a.run("view.switchWindows", json!({"id": first})).unwrap();
    assert_eq!(a.session.merge.headers, vec!["Name", "City"], "the recipients stay with their document");
    // A cancelled picker leaves nothing pending once the user does something else.
    a.run("ui.openRecipientList", json!({})).unwrap();
    a.run("format.bold", json!({})).unwrap();
    assert!(!a.recipient_list_pending);
}

/// #712: Find, Replace and Go To open as in Word. Ctrl+F puts the keyboard in the Navigation
/// pane's search box (with the selected text in it), not a dialog; Ctrl+H and Ctrl+G / F5 open
/// the one Find and Replace dialog on its Replace and Go To tabs.
#[test]
fn find_replace_and_go_to_keys_like_word() {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app(),
    );
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
    assert!(h.state().dialog.is_none(), "no dialog: {:?}", h.state().dialog.as_ref().map(|d| d.name()));
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
    h.state_mut().canvas.want_focus = true;
    h.step();
    h.event(key(egui::Key::F, egui::Modifiers::COMMAND));
    h.step();
    h.step();
    assert_eq!(nav_query(&h), "Kilns");
    assert_eq!(h.state().ui.nav_tab, "results", "Find leaves Pages to show the selected-text matches");
    assert_eq!(h.state().session.find.results.len(), 2);
    // Ctrl+H: the Replace tab; Go To: the Go To tab of the same dialog.
    h.state_mut().canvas.want_focus = true;
    h.event(key(egui::Key::Escape, egui::Modifiers::NONE));
    h.step();
    h.step();
    h.event(key(egui::Key::H, egui::Modifiers::COMMAND));
    h.step();
    h.step();
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("replace"));
    h.state_mut().run("edit.goto", json!({})).unwrap();
    h.step();
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("goto"));
    h.state_mut().run("edit.advancedFind", json!({})).unwrap();
    h.step();
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("find"));
}
