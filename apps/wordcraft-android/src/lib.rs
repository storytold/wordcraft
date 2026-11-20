#![cfg(target_os = "android")]

use eframe::{App, Frame, NativeOptions, egui};
use winit::platform::android::activity::AndroidApp;
use wordcraft_engine::Session;
use wordcraft_ui_egui::{Services, WordApp};

struct AndroidWordCraft(WordApp);

impl App for AndroidWordCraft {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        self.0.logic(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut Frame) {
        self.0.ui(ui);
    }
}

#[unsafe(no_mangle)]
fn android_main(android_app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));

    let options =
        NativeOptions { android_app: Some(android_app), viewport: egui::ViewportBuilder::default().with_title("WordCraft"), ..Default::default() };

    let result = eframe::run_native(
        "WordCraft",
        options,
        Box::new(|_creation_context| {
            let region = wordcraft_ui_egui::i18n::system_region();
            let mut session = Session::new(wordcraft_engine::blank_document(&region));
            session.system_region = region;
            let mut app = WordApp::new(session, Services::default());
            app.start_screen();
            Ok(Box::new(AndroidWordCraft(app)))
        }),
    );

    if let Err(error) = result {
        log::error!("WordCraft Android startup failed: {error}");
    }
}
