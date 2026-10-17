//! The Print page's one Printer list and the Grayscale choice, with in-process fakes: no spooler,
//! no socket, no printer is ever asked.
use std::cell::RefCell;
use std::rc::Rc;

use craft_ui::printing::network as net;
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use wordcraft_engine::Session;
use wordcraft_ui_egui::network_print::NetworkPrint;
use wordcraft_ui_egui::printing::{Printer, Settings};
use wordcraft_ui_egui::{Services, WordApp};

type Jobs = Rc<RefCell<Vec<(Vec<u8>, Settings)>>>;

fn app(jobs: &Jobs) -> WordApp {
    let mut doc = wordcraft_doc::Document::new();
    doc.settings.page_color = Some(wordcraft_doc::Rgb(255, 0, 0));
    let mut app = WordApp::new(Session::new(doc), Services::default());
    app.ui.print_backgrounds = true;
    app.services.printers = Some(Box::new(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(vec![Printer { name: "Office laser".into(), default: false }, Printer { name: "Front desk".into(), default: true }]).unwrap();
        rx
    }));
    let seen = jobs.clone();
    app.services.print_native = Some(Box::new(move |bytes, settings, _collate, _options| {
        seen.borrow_mut().push((bytes, settings));
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(Ok(String::new())).unwrap();
        rx
    }));
    app.services.print_network = Some(NetworkPrint {
        validate: |_| Ok(()),
        query: Box::new(|_| std::sync::mpsc::channel().1),
        send: Box::new(|_, _, _| None::<net::SendJob>),
    });
    app
}

fn harness(app: WordApp) -> egui_kittest::Harness<'static, WordApp> {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(5);
    h
}

fn open_list(h: &mut egui_kittest::Harness<'static, WordApp>) {
    h.get_by_role_and_label(Role::ComboBox, "Printer").click();
    h.run_steps(3);
}

#[test]
fn one_printer_list_holds_the_system_queues_and_the_network_entry() {
    let jobs = Jobs::default();
    let mut a = app(&jobs);
    a.run("file.print", json!({})).unwrap();
    let mut h = harness(a);
    assert!(h.query_by_label("System printer").is_none(), "no System / Network destination switch");
    assert!(h.query_by_label("Query printer").is_none(), "a system queue is chosen: no network section");
    assert_eq!(h.state().printing.settings.printer, "Front desk", "the default queue is selected");

    open_list(&mut h);
    let (front, office, network) =
        (h.get_by_label("Front desk").rect().min.y, h.get_by_label("Office laser").rect().min.y, h.get_by_label("Network printer…").rect().min.y);
    assert!(front < office && office < network, "default first, every queue, then the network entry: {front} {office} {network}");

    h.get_by_label("Network printer…").click();
    h.run_steps(4);
    assert!(h.state().ui.print.network);
    assert!(h.query_by_label("Query printer").is_some(), "the network section is shown");
    assert!(h.query_by_label("Two-sided printing").is_none(), "the local queue's options are not");

    open_list(&mut h);
    h.get_by_label("Office laser").click();
    h.run_steps(4);
    assert!(!h.state().ui.print.network);
    assert_eq!(h.state().printing.settings.printer, "Office laser");
    assert!(h.query_by_label("Query printer").is_none());
    assert!(h.query_by_label("Two-sided printing").is_some());
}

#[test]
fn grayscale_is_the_label_the_preview_choice_and_the_pdf() {
    let labels = wordcraft_ui_egui::network_print::labels();
    assert_eq!(labels.color_monochrome, "Grayscale");
    assert!(labels.unsupported_color.contains("grayscale"), "{}", labels.unsupported_color);

    let jobs = Jobs::default();
    let mut a = app(&jobs);
    a.run("file.print", json!({})).unwrap();
    let mut h = harness(a);
    assert!(h.query_by_label("Black and white").is_none());
    let (colour, gray) = {
        h.state_mut().run("ui.print", json!({})).unwrap();
        h.state_mut().printing.settings.grayscale = true;
        h.state_mut().poll_printing(&egui::Context::default());
        h.state_mut().run("ui.print", json!({})).unwrap();
        let jobs = jobs.borrow();
        assert_eq!(jobs.len(), 2);
        assert!(!jobs[0].1.grayscale && jobs[1].1.grayscale, "the driver is still asked for monochrome");
        (jobs[0].0.clone(), jobs[1].0.clone())
    };
    assert!(colour.starts_with(b"%PDF") && gray.starts_with(b"%PDF"));
    assert!(colour != gray, "the red page is printed gray, not just hinted");
}
