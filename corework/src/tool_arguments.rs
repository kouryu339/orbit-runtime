//! Strict JSON tool arguments shared by text calls, FC and RPC.
use serde::de::{Error, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};
struct Strict(Value);
impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Strict;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("JSON value")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut m: M) -> Result<Strict, M::Error> {
                let mut out = Map::new();
                while let Some(k) = m.next_key::<String>()? {
                    if out.contains_key(&k) {
                        return Err(M::Error::custom(format!("duplicate key '{k}'")));
                    }
                    out.insert(k, m.next_value::<Strict>()?.0);
                }
                Ok(Strict(Value::Object(out)))
            }
            fn visit_seq<S: SeqAccess<'de>>(self, mut s: S) -> Result<Strict, S::Error> {
                let mut out = Vec::new();
                while let Some(v) = s.next_element::<Strict>()? {
                    out.push(v.0)
                }
                Ok(Strict(Value::Array(out)))
            }
            fn visit_str<E: Error>(self, v: &str) -> Result<Strict, E> {
                Ok(Strict(Value::String(v.into())))
            }
            fn visit_bool<E: Error>(self, v: bool) -> Result<Strict, E> {
                Ok(Strict(Value::Bool(v)))
            }
            fn visit_i64<E: Error>(self, v: i64) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_u64<E: Error>(self, v: u64) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_f64<E: Error>(self, v: f64) -> Result<Strict, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| Strict(Value::Number(n)))
                    .ok_or_else(|| E::custom("invalid number"))
            }
            fn visit_unit<E: Error>(self) -> Result<Strict, E> {
                Ok(Strict(Value::Null))
            }
        }
        d.deserialize_any(V)
    }
}
pub fn parse_object(raw: &str) -> Result<Map<String, Value>, String> {
    if raw.len() > 1024 * 1024 {
        return Err("tool arguments exceed 1 MiB".into());
    }
    let mut d = serde_json::Deserializer::from_str(raw);
    let v = Strict::deserialize(&mut d).map_err(|e| e.to_string())?.0;
    d.end().map_err(|e| e.to_string())?;
    match v {
        Value::Object(m) => Ok(m),
        _ => Err("tool arguments must be a JSON object".into()),
    }
}
/// JSON input is strict; CLI compatibility does not use this coercion check.
pub fn matches_type(value: &Value, kind: &str) -> bool {
    let ty = kind.trim().to_ascii_lowercase();
    if matches!(ty.as_str(), "json" | "any" | "value" | "serde_json::value") {
        return true;
    }
    if let Some(inner) = ty
        .strip_prefix("array<")
        .or_else(|| ty.strip_prefix("vec<"))
        .and_then(|s| s.strip_suffix('>'))
    {
        return value
            .as_array()
            .is_some_and(|a| a.iter().all(|v| matches_type(v, inner)));
    }
    if let Some(inner) = ty.strip_suffix("[]") {
        return value
            .as_array()
            .is_some_and(|a| a.iter().all(|v| matches_type(v, inner)));
    }
    match ty.as_str() {
        "object" | "map" => value.is_object(),
        "array" | "anyarray" => value.is_array(),
        "stringarray" => value
            .as_array()
            .is_some_and(|a| a.iter().all(Value::is_string)),
        "numberarray" => value
            .as_array()
            .is_some_and(|a| a.iter().all(Value::is_number)),
        "boolarray" | "booleanarray" => value
            .as_array()
            .is_some_and(|a| a.iter().all(Value::is_boolean)),
        "objectarray" => value
            .as_array()
            .is_some_and(|a| a.iter().all(Value::is_object)),
        "bool" | "boolean" => value.is_boolean(),
        "number" | "num" | "f32" | "f64" => value.is_number(),
        "integer" | "int" | "int64" | "long" | "i64" | "isize" => value.as_i64().is_some(),
        "u64" | "usize" => value.as_u64().is_some(),
        "i8" => value.as_i64().is_some_and(|n| i8::try_from(n).is_ok()),
        "i16" => value.as_i64().is_some_and(|n| i16::try_from(n).is_ok()),
        "i32" => value.as_i64().is_some_and(|n| i32::try_from(n).is_ok()),
        "u8" => value.as_u64().is_some_and(|n| u8::try_from(n).is_ok()),
        "u16" => value.as_u64().is_some_and(|n| u16::try_from(n).is_ok()),
        "u32" => value.as_u64().is_some_and(|n| u32::try_from(n).is_ok()),
        _ => value.is_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_objects_preserve_values_and_reject_ambiguous_input() {
        let a=parse_object(r#"{"text":"123","n":123,"null":null,"bool":true,"nested":{"items":[1,"x",null]},"path":"C:\\files"}"#).unwrap();
        assert!(a["text"].is_string());
        assert!(a["n"].is_number());
        assert!(a["null"].is_null());
        assert!(a["nested"]["items"].is_array());
        for bad in [
            r#"{"a":1,"a":2}"#,
            r#"{"a":{"x":1,"x":2}}"#,
            r#"{"a":1} junk"#,
            "[]",
            "{",
            r#"{'x':1}"#,
        ] {
            assert!(parse_object(bad).is_err(), "{bad}")
        }
    }
    #[test]
    fn json_and_container_types_are_distinct() {
        assert!(matches_type(&Value::Null, "Json"));
        assert!(matches_type(&serde_json::json!([1, null]), "Any"));
        assert!(!matches_type(&serde_json::json!("12"), "Number"));
        assert!(!matches_type(&serde_json::json!([1, "2"]), "Array<Number>"));
        assert!(!matches_type(&Value::Null, "Object"));
    }
}
