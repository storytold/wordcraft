/// Monotonic milliseconds for deadlines, settling and animation. `web_time::Instant` is a real
/// monotonic clock on native (`Instant::now`) and in the browser (`performance.now()`), so a wall
/// clock or NTP adjustment cannot make a screenshot deadline or autosave fire early or late.
pub fn now_ms() -> f64 {
    use std::sync::OnceLock;
    static START: OnceLock<web_time::Instant> = OnceLock::new();
    START.get_or_init(web_time::Instant::now).elapsed().as_secs_f64() * 1000.0
}
