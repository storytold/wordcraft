use super::options::PdfOptions;
use super::{DateTime, Document, Metadata};

pub(crate) fn metadata(doc: &Document, opts: &PdfOptions) -> Metadata {
    let c = &doc.core;
    let mut m = Metadata { creator: "WordCraft".into(), producer: "WordCraft".into(), ..Default::default() };
    let title = opts.title.clone().unwrap_or_else(|| c.title.clone());
    if !title.trim().is_empty() {
        m.title = Some(title);
    }
    let author = opts.author.clone().unwrap_or_else(|| c.creator.clone());
    if !author.trim().is_empty() {
        m.authors = vec![author];
    }
    if !c.subject.trim().is_empty() {
        m.description = Some(c.subject.clone());
    } else if !c.description.trim().is_empty() {
        m.description = Some(c.description.clone());
    }
    let kw: Vec<String> = c.keywords.split([',', ';']).map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect();
    if !kw.is_empty() {
        m.keywords = kw;
    }
    let date = parse_iso(&c.created).or_else(|| now_unix().map(civil));
    if let Some((y, mo, d, h, mi, s)) = date {
        m.created =
            Some(DateTime::new(y.clamp(0, 9999) as u16).month(mo).day(d).hour(h).minute(mi).second(s).utc_offset_hour(0).utc_offset_minute(0));
    }
    m
}

type Civil = (i64, u8, u8, u8, u8, u8);

/// `YYYY-MM-DD[THH:MM[:SS]]…` → civil time.
pub(crate) fn parse_iso(s: &str) -> Option<Civil> {
    let s = s.trim();
    let n = |a: usize, b: usize| s.get(a..b).and_then(|x| x.parse::<i64>().ok());
    let (y, mo, d) = (n(0, 4)?, n(5, 7)?, n(8, 10)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let h = n(11, 13).unwrap_or(0).clamp(0, 23);
    let mi = n(14, 16).unwrap_or(0).clamp(0, 59);
    let sec = n(17, 19).unwrap_or(0).clamp(0, 59);
    Some((y, mo as u8, d as u8, h as u8, mi as u8, sec as u8))
}

/// The system clock, or the browser's on the web.
fn now_unix() -> Option<i64> {
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
        i64::try_from(secs).ok()
    }
    // `SystemTime::now()` panics on wasm32-unknown-unknown, so ask the browser's clock.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let ms = js_sys::Date::now();
        (ms.is_finite() && ms > 0.0).then(|| (ms / 1000.0) as i64)
    }
}

/// Unix seconds (UTC) → civil time (proleptic Gregorian).
///
/// The year is returned unclamped (the caller clamps it to the PDF `DateTime` range 0..=9999);
/// month, day and time of day always come through correctly. Past chrono's ±262143-year range the
/// Gregorian 400-year cycle (146_097 days) keeps the result exact — chrono converts the day within
/// one cycle and whole cycles go back into the year (an `i128` so the extremes can't overflow) —
/// rather than falling back to the epoch, so the date never changes for a value the old
/// hand-rolled converter accepted. 1970-01-01 is `num_days_from_ce` 719_163.
pub(crate) fn civil(t: i64) -> Civil {
    use chrono::Datelike;
    let (days, secs) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    let (cycles, rest) = (days.div_euclid(146_097), days.rem_euclid(146_097));
    // `rest + 719_163` is 719_163..865_259, always a valid CE day.
    let (y, m, d) = match i32::try_from(rest + 719_163).ok().and_then(chrono::NaiveDate::from_num_days_from_ce_opt) {
        Some(date) => {
            let y = i128::from(date.year()) + i128::from(cycles) * 400;
            (i64::try_from(y).unwrap_or(if y < 0 { i64::MIN } else { i64::MAX }), date.month() as u8, date.day() as u8)
        }
        None => (1970, 1, 1), // unreachable
    };
    (y, m, d, (secs / 3600) as u8, (secs / 60 % 60) as u8, (secs % 60) as u8)
}
