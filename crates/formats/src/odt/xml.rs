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
        let k = a.key.as_ref().to_owned();
        let val = a.normalized_value(XmlVersion::Explicit1_0).map(|c| c.into_owned()).unwrap_or_else(|_| a.value.to_string());
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
pub(super) fn events(xml: &[u8], mut f: impl FnMut(Ev)) -> Result<(), String> {
    let mut r = quick_xml::Reader::from_reader(xml);
    let cfg = r.config_mut();
    cfg.trim_text(false);
    cfg.check_end_names = false;
    cfg.allow_unmatched_ends = true;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match r.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => f(Ev::Start(e.name().as_ref().to_owned(), attrs_of(&e))),
            Ok(Event::Empty(e)) => {
                let n = e.name().as_ref().to_owned();
                f(Ev::Start(n.clone(), attrs_of(&e)));
                f(Ev::End(n));
            }
            Ok(Event::End(e)) => f(Ev::End(e.name().as_ref().to_owned())),
            Ok(Event::Text(t)) => f(Ev::Text((*t).to_owned())),
            Ok(Event::CData(t)) => f(Ev::Text((*t).to_owned())),
            Ok(Event::GeneralRef(r)) => {
                let name = (*r).to_owned();
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
            Ok(Event::Eof) => return Ok(()),
            Err(e) => return Err(format!("ODT XML: {e}")),
            Ok(_) => {}
        }
    }
}
