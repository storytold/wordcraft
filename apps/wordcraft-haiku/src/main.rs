#![forbid(unsafe_code)]

use egui_software_backend::{App, SoftwareBackend, SoftwareBackendAppConfiguration, run_app_with_software_backend};
use std::io::{BufRead, Read, Write};
use wordcraft_engine::Session;
use wordcraft_ui_egui::{ControlRequest, FileDialogAnswer, FileDialogRequest, Services, WordApp};

struct Editor(WordApp);

impl App for Editor {
    fn ui(&mut self, ui: &mut egui::Ui, _backend: &mut SoftwareBackend) {
        self.0.logic(ui.ctx());
        self.0.ui(ui);
        if self.0.quit_requested && !self.0.printing.busy() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
    fn raw_input_hook(&mut self, input: &mut egui::RawInput) {
        self.0.raw_input_hook(input);
    }
    fn close_requested(&mut self) -> bool {
        self.0.close_requested()
    }
}

fn file_dialog(request: FileDialogRequest, ctx: egui::Context) -> std::sync::mpsc::Receiver<FileDialogAnswer> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let _ = std::thread::Builder::new().name("file dialog".into()).spawn(move || {
        let dialog = rfd::FileDialog::new();
        let path = match request {
            FileDialogRequest::Open { .. } => dialog.pick_file(),
            FileDialogRequest::Save { name } => dialog.set_file_name(name).save_file(),
        };
        let _ = sender.send(Ok(path.map(|path| path.to_string_lossy().into_owned())));
        ctx.request_repaint();
    });
    receiver
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version") {
        println!("wordcraft-haiku {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let control_stdio = args.iter().any(|arg| arg == "--control-stdio");
    let sample = args.iter().any(|arg| arg == "--sample");
    let files: Vec<_> = args.into_iter().filter(|arg| arg != "--sample" && arg != "--control-stdio").collect();
    let configuration = SoftwareBackendAppConfiguration {
        #[cfg(target_os = "haiku")]
        haiku_exit_process_on_close: true,
        viewport_builder: egui::ViewportBuilder::default()
            .with_title("WordCraft")
            .with_app_id("ai.storyteller.wordcraft")
            .with_inner_size([1024.0, 720.0])
            .with_min_inner_size([760.0, 480.0]),
        ..Default::default()
    };
    run_app_with_software_backend(configuration, move |ctx| {
        let doc =
            if sample { wordcraft_engine::sample::sample_document() } else { wordcraft_engine::blank_document(&wordcraft_geom::Region::default()) };
        let dialog_ctx = ctx.clone();
        let services = Services { file_dialog: Some(Box::new(move |request| file_dialog(request, dialog_ctx.clone()))), ..Default::default() };
        let mut app = WordApp::new(Session::new(doc), services);
        if control_stdio {
            app = app.with_control(stdio_control(ctx));
        }
        for path in &files {
            if let Err(error) = app.run("file.open", serde_json::json!({"path": path})) {
                eprintln!("{path}: {error}");
            }
        }
        Editor(app)
    })?;
    Ok(())
}

fn stdio_control(ctx: egui::Context) -> std::sync::mpsc::Receiver<ControlRequest> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let _ = std::thread::Builder::new().name("stdio control".into()).spawn(move || {
        let stdin = std::io::stdin();
        let mut reader = stdin.lock();
        loop {
            let mut line = String::new();
            match reader.by_ref().take(1_048_577).read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) if line.len() > 1_048_576 => break,
                Ok(_) => {}
            }
            let request: serde_json::Value = match serde_json::from_str(&line) {
                Ok(request) => request,
                Err(_) => break,
            };
            let Some(method) = request.get("method").and_then(serde_json::Value::as_str) else { break };
            let params = request.get("params").cloned().unwrap_or_else(|| serde_json::json!({}));
            let (command, reply) = ControlRequest::new(method, params);
            if sender.send(command).is_err() {
                break;
            }
            ctx.request_repaint();
            let Ok(mut response) = reply.recv_timeout(std::time::Duration::from_secs(30)) else { break };
            if let Some(id) = request.get("id") {
                response["id"] = id.clone();
            }
            let stdout = std::io::stdout();
            let mut output = stdout.lock();
            if serde_json::to_writer(&mut output, &response).is_err() || output.write_all(b"\n").is_err() || output.flush().is_err() {
                break;
            }
        }
    });
    receiver
}
