//! Surface library warnings and errors on stderr in headless CLI runs.

struct Warnings;

impl log::Log for Warnings {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            let level = if record.level() == log::Level::Warn { "warning" } else { "error" };
            eprintln!("wordcraft-cli: {level}: {}", record.args());
        }
    }

    fn flush(&self) {}
}

pub(crate) fn init() {
    static LOGGER: Warnings = Warnings;
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Warn);
    }
}
