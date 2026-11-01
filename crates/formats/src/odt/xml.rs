use super::{BytesStart, Event, XmlVersion};

/// Length → points.
pub(super) fn length(v: &str) -> Option<f32> {
    let v = v.trim().to_ascii_lowercase();
    let num = |s: &str| s.trim().parse::<f32>().ok().filter(|x| x.is_finite());
    if let Some(n) = v.strip_suffix("pt") {
        num(n)
    } else if let Some(n) = v.strip_suffix("in") {
        num(n).map(|x| x * 72.0)
    } else if let Some(n) = v.strip_suffix("cm") {
        num(n).map(|x| x * 72.0 / 2.54)
    } else if let Some(n) = v.strip_suffix("mm") {
        num(n).map(|x| x * 72.0 / 25.4)
    } else if let Some(n) = v.strip_suffix("pc") {
        num(n).map(|x| x * 12.0)
    } else if let Some(n) = v.strip_suffix("px") {
        num(n).map(|x| x * 0.75)
    } else {
        num(&v)
    }
}

pub(super) type Attrs = Vec<(String, String)>;

fn attrs_of(e: &BytesStart<'_>) -> Attrs {
    let mut v = Vec::new();
    for a in e.attributes().with_checks(false).flatten().take(128) {
        let k = String::from_utf8_lossy(a.key.as_ref()).into_owned();
        let val =
            a.normalized_value(XmlVersion::Explicit1_0).map(|c| c.into_owned()).unwrap_or_else(|_| String::from_utf8_lossy(&a.value).into_owned());
        v.push((k, val));
    }
    v
}

pub(super) fn get<'a>(a: &'a Attrs, k: &str) -> Option<&'a str> {
    a.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
}

pub(super) fn local(q: &str) -> &str {
    q.rsplit(':').next().unwrap_or(q)
}

pub(super) enum Ev {
    Start(String, Attrs),
    End(String),
    Text(String),
}

/// Stream XML events (empty elements become start + end). Malformed XML ends the stream.
/// Stream XML events (empty elements become start + end). `false` when the reader stopped on
/// malformed XML before the end of the stream: a truncated or corrupt file, whose partial
/// content the caller must not mistake for a complete read (#1342).
pub(super) fn events(xml: &[u8], mut f: impl FnMut(Ev)) -> bool {
    let mut r = quick_xml::Reader::from_reader(xml);
    let cfg = r.config_mut();
    cfg.trim_text(false);
    cfg.check_end_names = true;
    cfg.allow_unmatched_ends = false;
    let mut buf = Vec::new();
    let mut depth = 0usize;
    // `quick_xml` accepts a second top-level element; one document must have exactly one root.
    let mut root_closed = false;
    loop {
        buf.clear();
        match r.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if depth == 0 && root_closed {
                    return false;
                }
                depth = depth.saturating_add(1);
                f(Ev::Start(String::from_utf8_lossy(e.name().as_ref()).into_owned(), attrs_of(&e)));
            }
            Ok(Event::Empty(e)) => {
                if depth == 0 {
                    if root_closed {
                        return false;
                    }
                    root_closed = true;
                }
                let n = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                f(Ev::Start(n.clone(), attrs_of(&e)));
                f(Ev::End(n));
            }
            Ok(Event::End(e)) => {
                depth = depth.saturating_sub(1);
                f(Ev::End(String::from_utf8_lossy(e.name().as_ref()).into_owned()));
                if depth == 0 {
                    root_closed = true;
                }
            }
            // Outside the document element only XML's four whitespace characters may appear
            // (syntax: S = #x20 | #x9 | #xD | #xA); CDATA sections and entity references are
            // not allowed there at all.
            Ok(Event::Text(t)) if depth == 0 => {
                if t.decode().map(|c| c.chars().any(|ch| !matches!(ch, ' ' | '\t' | '\r' | '\n'))).unwrap_or(true) {
                    return false;
                }
            }
            Ok(Event::CData(_)) if depth == 0 => return false,
            Ok(Event::GeneralRef(_)) if depth == 0 => return false,
            Ok(Event::Text(t)) => f(Ev::Text(t.decode().map(|c| c.into_owned()).unwrap_or_default())),
            Ok(Event::CData(t)) => f(Ev::Text(String::from_utf8_lossy(&t).into_owned())),
            Ok(Event::GeneralRef(r)) => {
                let name = String::from_utf8_lossy(&r).into_owned();
                let s = match name.as_str() {
                    "amp" => "&".to_string(),
                    "lt" => "<".to_string(),
                    "gt" => ">".to_string(),
                    "quot" => "\"".to_string(),
                    "apos" => "'".to_string(),
                    _ => r.resolve_char_ref().ok().flatten().map(|c| c.to_string()).unwrap_or_default(),
                };
                f(Ev::Text(s));
            }
            Ok(Event::Eof) => return root_closed && depth == 0,
            Err(_) => return false,
            Ok(_) => {}
        }
    }
}
