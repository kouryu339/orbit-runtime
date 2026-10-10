//! Strict JSON containers with library-parsed primitives. Object keys are never
//! interpreted as serde_json's private arbitrary-precision number marker.
use super::Budget;
use serde::de::DeserializeOwned;
use serde_json::{Map, Number, Value};

const MAX_DEPTH: usize = 128;

pub(super) fn parse_document(
    bytes: &[u8],
    max_nodes: usize,
    budget: &Budget<'_>,
) -> Result<Value, String> {
    let mut reader = Reader {
        bytes,
        at: 0,
        nodes: 0,
        max_nodes,
        budget,
    };
    let value = reader.value(0)?;
    reader.whitespace();
    if reader.at != bytes.len() {
        return Err(reader.error("trailing data"));
    }
    budget.check()?;
    Ok(value)
}

struct Reader<'a, 'b, 'c> {
    bytes: &'a [u8],
    at: usize,
    nodes: usize,
    max_nodes: usize,
    budget: &'b Budget<'c>,
}
impl Reader<'_, '_, '_> {
    fn error(&self, reason: &str) -> String {
        format!("invalid_json: {reason} at byte {}", self.at)
    }
    fn whitespace(&mut self) {
        while self
            .bytes
            .get(self.at)
            .is_some_and(|b| matches!(b, b' ' | b'\n' | b'\r' | b'\t'))
        {
            self.at += 1;
        }
    }
    fn take(&mut self, byte: u8) -> bool {
        self.whitespace();
        if self.bytes.get(self.at) == Some(&byte) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn require(&mut self, byte: u8) -> Result<(), String> {
        if self.take(byte) {
            Ok(())
        } else {
            Err(self.error(&format!("expected {}", byte as char)))
        }
    }
    fn scalar<T: DeserializeOwned>(&mut self) -> Result<T, String> {
        let mut stream =
            serde_json::Deserializer::from_slice(&self.bytes[self.at..]).into_iter::<T>();
        let value = stream
            .next()
            .ok_or_else(|| self.error("missing value"))?
            .map_err(|e| self.error(&e.to_string()))?;
        self.at += stream.byte_offset();
        Ok(value)
    }
    fn value(&mut self, depth: usize) -> Result<Value, String> {
        self.budget.check()?;
        self.nodes += 1;
        if self.nodes > self.max_nodes {
            return Err("max_entries JSON node budget exceeded".into());
        }
        self.whitespace();
        match self.bytes.get(self.at).copied() {
            Some(b'{') => {
                if depth >= MAX_DEPTH {
                    return Err(self.error("JSON nesting limit exceeded"));
                }
                self.at += 1;
                let mut object = Map::new();
                if self.take(b'}') {
                    return Ok(Value::Object(object));
                }
                loop {
                    self.budget.check()?;
                    self.whitespace();
                    if self.bytes.get(self.at) != Some(&b'"') {
                        return Err(self.error("object key must be a string"));
                    }
                    let key: String = self.scalar()?;
                    if object.contains_key(&key) {
                        return Err(self.error("duplicate object key; JSON Pointer is ambiguous"));
                    }
                    self.require(b':')?;
                    let value = self.value(depth + 1)?;
                    object.insert(key, value);
                    if self.take(b'}') {
                        break;
                    }
                    self.require(b',')?;
                }
                Ok(Value::Object(object))
            }
            Some(b'[') => {
                if depth >= MAX_DEPTH {
                    return Err(self.error("JSON nesting limit exceeded"));
                }
                self.at += 1;
                let mut array = Vec::new();
                if self.take(b']') {
                    return Ok(Value::Array(array));
                }
                loop {
                    array.push(self.value(depth + 1)?);
                    if self.take(b']') {
                        break;
                    }
                    self.require(b',')?;
                }
                Ok(Value::Array(array))
            }
            Some(b'"') => self.scalar::<String>().map(Value::String),
            Some(b'-' | b'0'..=b'9') => self.scalar::<Number>().map(Value::Number),
            Some(b't' | b'f') => self.scalar::<bool>().map(Value::Bool),
            Some(b'n') => self.scalar::<()>().map(|_| Value::Null),
            _ => Err(self.error("expected JSON value")),
        }
    }
}
