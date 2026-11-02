//! Comment times in the user's own time zone. A document keeps a comment's time as written, and
//! its UTC time beside it when it knows it (`w16cex:dateUtc`, which WordCraft writes for the
//! comments it stamps). The Comments pane and the balloons show it in local time, as Word does.

/// Minutes to add to UTC for local time at Unix time `secs` (east of Greenwich is positive).
fn offset_minutes(secs: i64) -> i32 {
    #[cfg(not(target_arch = "wasm32"))]
    let minutes = {
        use chrono::{Local, Offset, TimeZone};
        Local.timestamp_opt(secs, 0).earliest().map_or(0, |t| t.offset().fix().local_minus_utc() / 60)
    };
    // The browser knows the zone: `getTimezoneOffset` is UTC minus local time, in minutes.
    #[cfg(target_arch = "wasm32")]
    let minutes = {
        let d = js_sys::Date::new_0();
        let offset = if d.set_time(secs as f64 * 1000.0).is_finite() { d.get_timezone_offset() } else { 0.0 };
        if offset.is_finite() { -(offset as i32) } else { 0 }
    };
    // Real zones are within a day of UTC.
    minutes.clamp(-24 * 60, 24 * 60)
}

/// A comment's time for the pane and the balloons: `YYYY-MM-DD HH:MM` in local time. The UTC
/// time (`date_utc`, Word's `w16cex:dateUtc`) wins, else `date`, converted when it names an
/// offset (`±HH:MM`) and shown as written when it doesn't. A `Z` on `date` alone doesn't count:
/// Word writes local time there with a `Z` it doesn't mean, and files from before `dateUtc` have
/// nothing else. Empty when neither is a date.
pub(crate) fn comment_time(c: &wordcraft_doc::Comment) -> String {
    stamp(&c.date, c.date_utc.as_deref(), offset_minutes)
}

/// Return the user's local calendar date for an ISO-8601 timestamp. Recent-document timestamps
/// are stored in UTC, but grouping is a calendar operation in the user's time zone.
pub(crate) fn local_calendar_date(iso: &str) -> Option<String> {
    let t = parse(iso)?;
    let utc = t.secs - i64::from(t.zone.unwrap_or(0)) * 60;
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(time) = chrono::DateTime::<chrono::Utc>::from_timestamp(utc, 0) {
        let date = if t.timed { time.with_timezone(&chrono::Local).date_naive() } else { time.date_naive() };
        return Some(date.format("%Y-%m-%d").to_string());
    }
    // Keep the browser implementation, and provide a fallback for timestamps outside chrono's
    // representable range on native targets.
    let local = if t.timed { utc + i64::from(offset_minutes(utc)) * 60 } else { utc };
    let (y, m, d) = wordcraft_engine::cmd::civil_from_days(local.div_euclid(86_400));
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

/// Format a stored UTC timestamp as a short date in the device's regional convention.
pub(crate) fn local_display_date(iso: &str) -> Option<String> {
    let date = local_calendar_date(iso)?;
    let mut parts = date.split('-');
    let year = parts.next()?;
    let month = parts.next()?;
    let day = parts.next()?;
    let locale = sys_locale::get_locale().unwrap_or_default();
    let region = locale_region(&locale);
    Some(match region.as_str() {
        "us" | "ph" => format!("{month}/{day}/{year}"),
        "cn" | "jp" | "tw" => format!("{year}/{month}/{day}"),
        "kr" => format!("{year}. {month}. {day}."),
        _ => format!("{day}/{month}/{year}"),
    })
}

/// Format a stored UTC timestamp as a regional date and time. Date-only legacy values remain
/// date-only, while timestamps include hours and minutes in the device's usual clock convention.
pub(crate) fn local_display_datetime(iso: &str) -> Option<String> {
    let t = parse(iso)?;
    if !t.timed {
        return local_display_date(iso);
    }
    let utc = t.secs - i64::from(t.zone.unwrap_or(0)) * 60;
    let local = utc + i64::from(offset_minutes(utc)) * 60;
    let seconds = local.rem_euclid(86_400);
    let hour = seconds / 3_600;
    let minute = seconds / 60 % 60;
    let locale = sys_locale::get_locale().unwrap_or_default();
    let region = locale_region(&locale);
    let time = if matches!(region.as_str(), "us" | "ph") {
        let suffix = if hour < 12 { "AM" } else { "PM" };
        let display_hour = match hour % 12 {
            0 => 12,
            h => h,
        };
        format!("{display_hour}:{minute:02} {suffix}")
    } else {
        format!("{hour:02}:{minute:02}")
    };
    Some(format!("{} {time}", local_display_date(iso)?))
}

/// Find the locale's region, skipping the language and optional script subtags. A singleton
/// starts extensions and cannot be a region (for example, `en-US-u-hc-h12`).
fn locale_region(locale: &str) -> String {
    let locale = locale.replace('_', "-").to_ascii_lowercase();
    let mut parts = locale.split('-');
    let _language = parts.next();
    for part in parts {
        if part.len() == 1 {
            break;
        }
        let is_alpha_region = part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_alphabetic());
        let is_numeric_region = part.len() == 3 && part.bytes().all(|byte| byte.is_ascii_digit());
        if is_alpha_region || is_numeric_region {
            return part.to_string();
        }
    }
    String::new()
}

fn stamp(date: &str, date_utc: Option<&str>, offset: impl Fn(i64) -> i32) -> String {
    let utc = date_utc.and_then(parse).filter(|t| t.timed).map(|t| Time { zone: Some(t.zone.unwrap_or(0)), ..t });
    let local_date = || parse(date).map(|t| if date.trim_end().ends_with(['Z', 'z']) { Time { zone: None, ..t } } else { t });
    let Some(t) = utc.or_else(local_date) else { return String::new() };
    // A date without a time of day stays the date it says.
    let local = match t.zone.filter(|_| t.timed) {
        Some(zone) => {
            let utc = t.secs - i64::from(zone) * 60;
            utc + i64::from(offset(utc)) * 60
        }
        None => t.secs,
    };
    let (y, m, d) = wordcraft_engine::cmd::civil_from_days(local.div_euclid(86_400));
    if !t.timed {
        return format!("{y:04}-{m:02}-{d:02}");
    }
    let s = local.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", s / 3600, s / 60 % 60)
}

/// A date and time as a comment stores it.
#[derive(Clone, Copy)]
struct Time {
    /// The wall time as if it were UTC, in seconds since the Unix epoch.
    secs: i64,
    /// The zone's offset from UTC in minutes, when the text names one.
    zone: Option<i32>,
    /// It has a time of day, not just a date.
    timed: bool,
}

/// `YYYY-MM-DD[THH:MM[:SS[.fff]]][Z|±HH:MM]`, else None.
fn parse(iso: &str) -> Option<Time> {
    let iso = iso.trim();
    let num = |a: usize, b: usize| -> Option<u32> {
        let s = iso.get(a..b)?;
        if s.bytes().all(|c| c.is_ascii_digit()) { s.parse().ok() } else { None }
    };
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    if iso.get(4..5) != Some("-") || iso.get(7..8) != Some("-") || !(1..=12).contains(&mo) || d == 0 || d > days_in_month(y, mo) || y == 0 {
        return None;
    }
    let mut secs = days_from_civil(i64::from(y), mo, d) * 86_400;
    let mut rest = iso.get(10..)?;
    let mut zone = None;
    let mut timed = false;
    if let Some(time) = rest.strip_prefix('T').or_else(|| rest.strip_prefix(' ')) {
        timed = true;
        let part = |a: usize, b: usize| time.get(a..b).filter(|s| s.bytes().all(|c| c.is_ascii_digit())).and_then(|s| s.parse::<u32>().ok());
        let (h, mi) = (part(0, 2)?, part(3, 5)?);
        if time.get(2..3) != Some(":") || h > 23 || mi > 59 {
            return None;
        }
        let mut end = 5;
        let mut s = 0;
        if time.get(5..6) == Some(":") {
            s = part(6, 8).filter(|s| *s <= 59)?;
            end = 8;
            // Fractions of a second.
            if time.get(8..9) == Some(".") {
                end = 9 + time.get(9..)?.bytes().take_while(u8::is_ascii_digit).count();
            }
        }
        secs += i64::from(h * 3600 + mi * 60 + s);
        rest = time.get(end..)?;
    }
    match rest {
        "" => {}
        "Z" | "z" => zone = Some(0),
        _ => {
            let sign = match rest.get(0..1) {
                Some("+") => 1,
                Some("-") => -1,
                _ => return None,
            };
            let digits: String = rest.get(1..)?.chars().filter(|c| *c != ':').collect();
            if digits.len() != 4 || !digits.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let (h, m) = (digits.get(0..2)?.parse::<i32>().ok()?, digits.get(2..4)?.parse::<i32>().ok()?);
            if h > 18 || m > 59 {
                return None;
            }
            zone = Some(sign * (h * 60 + m));
        }
    }
    Some(Time { secs, zone, timed })
}

/// Days in month `m` (1–12) of year `y`, proleptic Gregorian.
fn days_in_month(y: u32, m: u32) -> u32 {
    match m {
        2 if y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400)) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (the inverse of
/// [`wordcraft_engine::cmd::civil_from_days`]).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_round_trip() {
        for days in [-800_000, -1, 0, 1, 19_000, 20_376, 2_932_896] {
            let (y, m, d) = wordcraft_engine::cmd::civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(days_from_civil(2026, 10, 12), 20_738);
    }

    #[test]
    fn locale_region_skips_scripts_and_extensions() {
        assert_eq!(locale_region("zh-Hant-TW"), "tw");
        assert_eq!(locale_region("sr_Latn_RS"), "rs");
        assert_eq!(locale_region("en-419"), "419");
        assert_eq!(locale_region("en-US-u-hc-h12"), "us");
        assert_eq!(locale_region("en"), "");
    }

    /// WordCraft's own comments carry their UTC time: shown in the user's zone, across a date
    /// line too.
    #[test]
    fn utc_times_show_in_local_time() {
        let toronto = |_: i64| -4 * 60;
        let tokyo = |_: i64| 9 * 60;
        let ours = |d: &str, offset: fn(i64) -> i32| stamp(d, Some(d), offset);
        assert_eq!(ours("2026-10-12T04:44:31Z", toronto), "2026-10-12 00:44");
        assert_eq!(ours("2026-10-12T02:05:00Z", toronto), "2026-10-11 22:05");
        assert_eq!(ours("2026-10-12T20:30:00Z", tokyo), "2026-10-13 05:30");
        assert_eq!(ours("2026-10-12T04:44:31Z", |_| 0), "2026-10-12 04:44");
        // A zone in the text is honoured, and fractions of a second are fine.
        assert_eq!(stamp("2026-10-12T09:00:00+05:30", None, |_| 0), "2026-10-12 03:30");
        assert_eq!(stamp("2026-10-12T09:00:00.125-0200", None, |_| 0), "2026-10-12 11:00");
        assert_eq!(ours("2026-10-12T04:44Z", toronto), "2026-10-12 00:44");
    }

    /// A Word file's `date` is local time (often with a Z it doesn't mean) and its `dateUtc` the real
    /// time: the real time wins. A time without a zone shows as written.
    #[test]
    fn word_times_prefer_the_utc_one() {
        let berlin = |_: i64| 2 * 60;
        assert_eq!(stamp("2026-10-12T10:00:00Z", Some("2026-10-12T08:00:00Z"), berlin), "2026-10-12 10:00");
        // Without `dateUtc` (older files), the local time shows as written, `Z` or not.
        assert_eq!(stamp("2026-10-12T10:00:00Z", None, berlin), "2026-10-12 10:00");
        assert_eq!(stamp("2026-10-12T10:00:00", None, berlin), "2026-10-12 10:00");
        assert_eq!(stamp("2026-10-12T10:00:00", Some("not a date"), berlin), "2026-10-12 10:00");
        // A date alone stays a date.
        assert_eq!(stamp("2026-10-12", None, berlin), "2026-10-12");
        assert_eq!(stamp("2026-10-12Z", Some("2026-10-12"), |_| -4 * 60), "2026-10-12");
    }

    /// Whatever a file holds, a stamp is a date or nothing, never a panic.
    #[test]
    fn hostile_times_show_nothing() {
        for bad in [
            "",
            "yesterday",
            "2026-13-01T00:00:00Z",
            "2026-10-12T25:00:00Z",
            "2026-10-12T10:61Z",
            "2026-10-12T10:00:00+99:00",
            "2026-10-12T10:00:00+1",
            "2026-1O-12",
            "0000-01-01",
            "２０２６-10-12",
            "2026-10-12T10:00:00ZZ",
            "2026-10-12T1",
            "+2026-10-12",
            "2026-02-30T10:00:00Z",
            "2026-02-29",
            "2026-10-12T10:00:99Z",
        ] {
            assert_eq!(stamp(bad, None, |_| 0), "", "{bad:?}");
        }
        assert_eq!(stamp("2024-02-29", None, |_| 0), "2024-02-29");
        assert_eq!(stamp("9999-12-31T23:59:59Z", Some("9999-12-31T23:59:59Z"), |_| 24 * 60), "10000-01-01 23:59");
        assert_eq!(stamp("0001-01-01T00:00:00Z", Some("0001-01-01T00:00:00Z"), |_| -24 * 60), "0000-12-31 00:00");
    }

    #[test]
    fn the_local_offset_is_a_real_zone() {
        let m = offset_minutes(1_760_000_000);
        assert!((-24 * 60..=24 * 60).contains(&m), "{m}");
    }
}
