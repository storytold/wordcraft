//! JSON ↔ Yjs values, and the document's property types as Yjs attributes.
//!
//! Schema 1 stores JSON values as Yjs values (maps, arrays, strings, numbers, booleans, null),
//! not as JSON text. Numbers are JavaScript numbers: integral ones are written as Yjs integers
//! (as JavaScript peers write them) and read back as JSON integers, so a value always converts
//! to the same Yjs value (Yjs compares formatting attributes by value).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map as JsonMap, Value as Json};
use yrs::Any;
use yrs::any::Number;
use yrs::types::Attrs;

/// Largest integer a JavaScript number holds exactly.
const SAFE: f64 = 9_007_199_254_740_991.0;

/// A JSON value as a Yjs value.
pub fn to_any(v: &Json) -> Any {
    match v {
        Json::Null => Any::Null,
        Json::Bool(b) => Any::Bool(*b),
        Json::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) if (i as f64).abs() <= SAFE => Any::Number(Number::Int(i)),
            (_, Some(f)) if f.fract() == 0.0 && f.abs() <= SAFE => Any::Number(Number::Int(f as i64)),
            (_, Some(f)) if f.is_finite() => Any::Number(Number::Float(f)),
            _ => Any::Null,
        },
        Json::String(s) => Any::String(Arc::from(s.as_str())),
        Json::Array(a) => Any::Array(a.iter().map(to_any).collect::<Vec<_>>().into()),
        Json::Object(o) => Any::Map(Arc::new(o.iter().map(|(k, v)| (k.clone(), to_any(v))).collect::<HashMap<_, _>>())),
    }
}

/// A Yjs value as JSON: integral numbers as integers, bytes as an array of numbers.
pub fn to_json(a: &Any) -> Json {
    match a {
        Any::Null | Any::Undefined => Json::Null,
        Any::Bool(b) => Json::Bool(*b),
        Any::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => Json::from(i),
            (_, Some(f)) => serde_json::Number::from_f64(f).map(Json::Number).unwrap_or(Json::Null),
            _ => Json::Null,
        },
        Any::String(s) => Json::String(s.to_string()),
        Any::Buffer(b) => Json::Array(b.iter().map(|x| Json::from(*x)).collect()),
        Any::Array(v) => Json::Array(v.iter().map(to_json).collect()),
        Any::Map(m) => {
            let sorted: BTreeMap<&String, Json> = m.iter().map(|(k, v)| (k, to_json(v))).collect();
            Json::Object(sorted.into_iter().map(|(k, v)| (k.clone(), v)).collect())
        }
    }
}

/// A value's JSON object (empty when it isn't one, or can't be written).
pub fn object_of<T: Serialize>(v: &T) -> JsonMap<String, Json> {
    match serde_json::to_value(v) {
        Ok(Json::Object(o)) => o,
        _ => JsonMap::new(),
    }
}

/// Reads `T` from a JSON object. A key that doesn't fit (another version's, damaged) is dropped
/// rather than losing the rest; `T::default()` when nothing fits.
pub fn from_object<T: DeserializeOwned + Default>(o: JsonMap<String, Json>) -> T {
    if let Ok(v) = serde_json::from_value::<T>(Json::Object(o.clone())) {
        return v;
    }
    let mut kept = JsonMap::new();
    for (k, v) in o {
        let mut trial = kept.clone();
        trial.insert(k.clone(), v.clone());
        if serde_json::from_value::<T>(Json::Object(trial)).is_ok() {
            kept.insert(k, v);
        }
    }
    serde_json::from_value(Json::Object(kept)).unwrap_or_default()
}

/// Reads `T` from any JSON value, `None` when it doesn't fit.
pub fn from_json<T: DeserializeOwned>(v: Json) -> Option<T> {
    serde_json::from_value(v).ok()
}

/// Character properties as Yjs text attributes, every key in `all` that `props` doesn't set
/// cleared (`null`): text inserted with them has exactly these attributes, whatever surrounds it.
pub fn attrs_exact(props: &JsonMap<String, Json>, all: &[String]) -> Attrs {
    let mut a: Attrs = props.iter().map(|(k, v)| (Arc::from(k.as_str()), to_any(v))).collect();
    for k in all {
        if !props.contains_key(k) {
            a.insert(Arc::from(k.as_str()), Any::Null);
        }
    }
    a
}

/// The attributes that turn `old` into `new`: changed and new keys, removed ones as `null`.
pub fn attrs_delta(old: &JsonMap<String, Json>, new: &JsonMap<String, Json>) -> Attrs {
    let mut a = Attrs::new();
    for (k, v) in new {
        if old.get(k) != Some(v) {
            a.insert(Arc::from(k.as_str()), to_any(v));
        }
    }
    for k in old.keys() {
        if !new.contains_key(k) {
            a.insert(Arc::from(k.as_str()), Any::Null);
        }
    }
    a
}

/// Yjs text attributes as a JSON object (`null` ones left out).
pub fn attrs_object(a: Option<&Attrs>) -> JsonMap<String, Json> {
    let mut o = JsonMap::new();
    if let Some(a) = a {
        let sorted: BTreeMap<&Arc<str>, &Any> = a.iter().collect();
        for (k, v) in sorted {
            if !matches!(v, Any::Null | Any::Undefined) {
                o.insert(k.to_string(), to_json(v));
            }
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wordcraft_doc::CharProps;

    #[test]
    fn values_round_trip_and_numbers_are_canonical() {
        for v in [json!(1), json!(-7), json!(1.5), json!("x"), json!([1, {"a": null}]), json!({"b": true, "c": 12}), json!(null)] {
            assert_eq!(to_json(&to_any(&v)), v);
        }
        // 12.0 and 12 are one Yjs value.
        assert_eq!(to_any(&json!(12.0)), to_any(&json!(12)));
        assert_eq!(to_json(&to_any(&json!(12.0))), json!(12));
    }

    #[test]
    fn character_properties_survive_as_attributes() {
        let p = CharProps { bold: Some(true), size: Some(10.5), font: Some("Aptos".into()), spacing: Some(0.1), ..Default::default() };
        let o = object_of(&p);
        let a = attrs_exact(&o, &["italic".to_string()]);
        assert_eq!(a.get("italic"), Some(&Any::Null));
        let back: CharProps = from_object(attrs_object(Some(&a)));
        assert_eq!(back, p);
        let d = attrs_delta(&o, &object_of(&CharProps { bold: Some(true), ..Default::default() }));
        assert_eq!(d.get("size"), Some(&Any::Null));
        assert!(!d.contains_key("bold"));
    }

    #[test]
    fn a_bad_key_drops_only_itself() {
        let mut o = object_of(&CharProps { bold: Some(true), ..Default::default() });
        o.insert("size".into(), json!("huge"));
        let p: CharProps = from_object(o);
        assert_eq!(p.bold, Some(true));
        assert_eq!(p.size, None);
    }
}
