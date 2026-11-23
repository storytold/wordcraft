use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

fn temp_dir(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!("wordcraft-logging-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("read log")
}

#[test]
fn a_poisoned_sink_still_logs() {
    use log::Log;
    let dir = temp_dir("poison");
    let logger = std::sync::Arc::new(AppLogger::new(Filter::parse("info"), false));
    let l = std::sync::Arc::clone(&logger);
    let _ = std::thread::spawn(move || {
        let _guard = l.sink.lock().expect("lock");
        panic!("poison the sink");
    })
    .join();
    assert!(logger.sink.is_poisoned());
    let path = logger.attach_dir(&dir).expect("attach");
    logger.log(&log::Record::builder().level(log::Level::Warn).target("wordcraft").args(format_args!("after the poison")).build());
    assert!(read(&path).contains("wordcraft: after the poison"));
    let _ = std::fs::remove_dir_all(&dir);
}
