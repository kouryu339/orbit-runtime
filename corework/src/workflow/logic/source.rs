//! JSON-shaped source syntax. Node args and return values permit bare expressions.
use super::{error, pointer, Diagnostic, LogicDocument};
use serde_json::{json, Map, Value};

#[derive(Clone, Copy)]
enum Context {
    Document,
    Json,
    Nodes,
    Node,
    Cases,
    Case,
    Args,
    Bindings,
    Returns,
    Return,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn prompt_minimal_examples_parse_compile_and_return_declared_values() {
        use crate::workflow::{execution::ExecutionContext, BlueprintLoader, DataValue};
        use crate::world::FrameworkState;
        use std::collections::HashMap;
        for prompt in [
            include_str!("../../../../ai-assistant/prompts/zh/workflow_tool_rules.md"),
            include_str!("../../../../ai-assistant/prompts/en/workflow_tool_rules.md"),
            include_str!("../../../../ai-assistant/skills/system/thinking-pro/SKILL.md"),
        ] {
            let source = prompt
                .split("```text")
                .nth(1)
                .unwrap()
                .split("```")
                .next()
                .unwrap()
                .trim();
            let resource = normalize_source(source).unwrap();
            assert_eq!(resource["schema"], "orbit.workflow.resource/v1");
            let logic: LogicDocument = serde_json::from_value(resource["logic"].clone()).unwrap();
            let blueprint = crate::workflow::logic::compile(&logic, &[]).unwrap();
            let ctx = FrameworkState::initialize().unwrap().create_context();
            let loaded = BlueprintLoader::new()
                .load_from_blueprint_json(blueprint, &ctx)
                .unwrap();
            let mut execution = ExecutionContext::from_context(ctx);
            loaded
                .compiled
                .initialize_defaults(&mut execution)
                .await
                .unwrap();
            let output = loaded
                .compiled
                .executor()
                .execute_with_params(
                    &mut execution,
                    HashMap::from([("text".into(), DataValue::from_string("hello"))]),
                )
                .await
                .unwrap();
            assert_eq!(output["text"].json_value(), &json!("hello"));
            assert_eq!(output["done"].json_value(), &json!(true));
        }
    }

    #[test]
    fn lifecycle_stamping_preserves_authored_source() {
        let source = r#" { "metadata":{"说明":"中文", "state":"draft", "revision":1}, "revision":1, "state":"draft", "logic":{"returns":{"x":{"value":add(1, 2)},"text":{"value":"revision:1, state:draft"}}}} "#;
        let stamped = stamp_resource_source(source, "registered", 23).unwrap();
        assert_eq!(
            stamped,
            source.replace(
                "\"revision\":1, \"state\":\"draft\", \"logic\"",
                "\"revision\":23, \"state\":\"registered\", \"logic\""
            )
        );
        assert!(stamp_resource_source(
            r#"{"state":"draft","state":"draft","revision":1}"#,
            "registered",
            2
        )
        .is_err());
    }

    fn assert_same(actual: &Value, expected: &Value, path: &str) {
        match (actual, expected) {
            (Value::Object(a), Value::Object(b)) => {
                assert_eq!(
                    a.keys().collect::<Vec<_>>(),
                    b.keys().collect::<Vec<_>>(),
                    "keys at {path}"
                );
                for (key, value) in a {
                    assert_same(value, &b[key], &format!("{path}/{key}"));
                }
            }
            (Value::Array(a), Value::Array(b)) => {
                assert_eq!(a.len(), b.len(), "array size at {path}");
                for (i, (a, b)) in a.iter().zip(b).enumerate() {
                    assert_same(a, b, &format!("{path}/{i}"));
                }
            }
            _ => assert_eq!(actual, expected, "value at {path}"),
        }
    }

    #[test]
    fn published_examples_match_their_normalized_json() {
        let basic = include_str!("../../../../examples/workflows/args-and-returns.logic.workflow");
        let document = parse_source(basic).unwrap();
        crate::workflow::logic::compile(&document, &[]).unwrap();
        let source = include_str!("../../../../examples/workflows/excel-read-rows.logic.workflow");
        let expected: Value = serde_json::from_str(include_str!(
            "../../../../examples/workflows/excel-read-rows.logic.json"
        ))
        .unwrap();
        assert_same(&normalize_source(source).unwrap(), &expected, "");
        parse_source(source).unwrap();
        let resource =
            include_str!("../../../../examples/workflows/excel-read-rows.resource.workflow");
        let expected: Value = serde_json::from_str(include_str!(
            "../../../../examples/workflows/excel-read-rows.resource.json"
        ))
        .unwrap();
        assert_same(&normalize_source(resource).unwrap(), &expected, "");
    }

    #[tokio::test]
    async fn nested_arguments_and_returns_preserve_literals_and_execute() {
        use crate::{
            workflow::{execution::ExecutionContext, BlueprintLoader, DataValue},
            world::FrameworkState,
        };
        use std::collections::HashMap;
        let document = parse_source(r#"{
          "schema":"orbit.workflow.logic/v1",
          "inputs":{"path":{"type":"String"}},
          "variables":{"count":{"type":"i64","initial":2}},
          "steps":[{"id":5,"call":"StringAppendNode","args":{"A":input.path,"B":""}},
            {"id":6,"call":"DebugPrintNode","args":{"Value":{"input":"literal key", "nested":[5.Joined, add(var.count, 1), "var.count", null, true, 6, 7]}}}],
          "returns":{
            "result":{"value":{"source":5.Joined,"values":[add(var.count, 1), "add(var.count, 1)", {"expression":"input.path"}],"empty":{},"static_array":["a",true,123],"empty_array":[],"null":null},"disablename":"结果","description":"返回组合结果。"},
            "quote":{"value":"input.path"},
            "unicode":{"value":text_concat("\u4e2d", "文")}
          }
        }"#).unwrap();
        let blueprint = crate::workflow::logic::compile(&document, &[]).unwrap();
        let ctx = FrameworkState::initialize().unwrap().create_context();
        let loaded = BlueprintLoader::new()
            .load_from_blueprint_json(blueprint, &ctx)
            .unwrap();
        let mut execution = ExecutionContext::from_context(ctx);
        loaded
            .compiled
            .initialize_defaults(&mut execution)
            .await
            .unwrap();
        let result = loaded
            .compiled
            .executor()
            .execute_with_params(
                &mut execution,
                HashMap::from([("path".into(), DataValue::from_string("demo.xlsx"))]),
            )
            .await
            .unwrap();
        assert_eq!(result["quote"].as_str(), Some("input.path"));
        assert_eq!(result["unicode"].as_str(), Some("中文"));
        let object = result["result"].json_value();
        assert_eq!(object["source"], "demo.xlsx");
        assert_eq!(object["values"][0].as_f64(), Some(3.0));
        assert_eq!(object["values"][1], "add(var.count, 1)");
        assert_eq!(object["values"][2], json!({"expression":"input.path"}));
        assert_eq!(object["empty"], json!({}));
        assert_eq!(object["static_array"], json!(["a", true, 123]));
        assert_eq!(object["empty_array"], json!([]));
        assert!(object["null"].is_null());
    }

    #[test]
    fn source_is_strict_outside_values_and_reports_locations() {
        for source in [
            r#"{"schema":"orbit.workflow.logic/v1","inputs":{"x":{"type":input.x}},"steps":[]}"#,
            r#"{"schema":"orbit.workflow.logic/v1","steps":[{"id":1,"for":input.x,"body":[]}]}"#,
            r#"{"schema":"orbit.workflow.logic/v1","steps":[],"returns":{"x":{"value":1,"description":input.x}}}"#,
            r#"{"schema":"orbit.workflow.logic/v1","steps":[],"steps":[]}"#,
            r#"{"schema":"orbit.workflow.logic/v1","steps":[{"id":1,"call":"DebugPrintNode","args":{"Value":add(1,,2)}}]}"#,
            r#"{"schema":"orbit.workflow.logic/v1","steps":[{"id":1,"call":"DebugPrintNode","args":{"Value":split("x", [","])}}]} garbage"#,
        ] {
            assert!(parse_source(source).is_err(), "accepted {source}");
        }
        let error = parse_source(r#"{"schema":"orbit.workflow.logic/v1","steps":[{"id":1,"call":"DebugPrintNode","args":{"Value":unknown(1)}}]}"#).unwrap_err();
        assert_eq!(error.pointer, "/steps/0/args/Value");
        assert!(error.message.contains("byte"));
    }
}

pub fn parse_source(text: &str) -> Result<LogicDocument, Diagnostic> {
    let value = normalize_source(text)?;
    serde_json::from_value(value).map_err(|e| error(None, "", "source_structure", e.to_string()))
}

/// Parse logic or resource source into ordinary JSON for structural validation.
/// Resource lifecycle fields are data here; this function does not import/publish.
pub fn normalize_source(text: &str) -> Result<Value, Diagnostic> {
    let mut parser = Parser { text, offset: 0 };
    let value = parser.value(Context::Document, "", 0)?;
    parser.space();
    if parser.offset != text.len() {
        return Err(parser.fail("", "Unexpected trailing content"));
    }
    Ok(value)
}

/// Update only lifecycle fields; keep authored expressions and literal strings exact.
pub fn stamp_resource_source(text: &str, state: &str, revision: u64) -> Result<String, Diagnostic> {
    normalize_source(text)?;
    let mut parser = Parser { text, offset: 0 };
    parser.expect(b'{', "")?;
    let mut changes = Vec::new();
    loop {
        parser.space();
        if parser.text.as_bytes().get(parser.offset) == Some(&b'}') {
            break;
        }
        let key = parser.string("")?;
        parser.expect(b':', "")?;
        parser.space();
        let start = parser.offset;
        parser.value(Parser::child(Context::Document, &key), "", 1)?;
        let end = parser.offset;
        match key.as_str() {
            "state" => changes.push((start, end, serde_json::to_string(state).unwrap())),
            "revision" => changes.push((start, end, revision.to_string())),
            _ => {}
        }
        parser.space();
        if parser.text.as_bytes().get(parser.offset) != Some(&b',') {
            break;
        }
        parser.offset += 1;
    }
    if changes.len() != 2 {
        return Err(parser.fail("", "Resource state and revision are required"));
    }
    let mut result = text.to_string();
    for (start, end, replacement) in changes.into_iter().rev() {
        result.replace_range(start..end, &replacement);
    }
    Ok(result)
}

struct Parser<'a> {
    text: &'a str,
    offset: usize,
}
impl Parser<'_> {
    fn space(&mut self) {
        while self
            .text
            .as_bytes()
            .get(self.offset)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.offset += 1;
        }
    }
    fn fail(&self, path: &str, message: &str) -> Diagnostic {
        error(
            None,
            path,
            "source_syntax",
            format!("{message} (byte {})", self.offset),
        )
    }
    fn string(&mut self, path: &str) -> Result<String, Diagnostic> {
        let start = self.offset;
        self.offset += 1;
        let mut escaped = false;
        while let Some(&byte) = self.text.as_bytes().get(self.offset) {
            self.offset += 1;
            if escaped {
                escaped = false;
                continue;
            }
            if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                return serde_json::from_str(&self.text[start..self.offset])
                    .map_err(|e| self.fail(path, &e.to_string()));
            }
        }
        Err(self.fail(path, "Unterminated JSON string"))
    }
    fn expect(&mut self, byte: u8, path: &str) -> Result<(), Diagnostic> {
        self.space();
        if self.text.as_bytes().get(self.offset) != Some(&byte) {
            return Err(self.fail(path, "Unexpected token"));
        }
        self.offset += 1;
        Ok(())
    }
    fn child(context: Context, key: &str) -> Context {
        match (context, key) {
            (Context::Document, "logic") => Context::Document,
            (Context::Document, "steps") => Context::Nodes,
            (Context::Document, "returns") => Context::Returns,
            (Context::Returns, _) => Context::Return,
            (Context::Return, "value") => Context::Args,
            (Context::Node, "args") => Context::Bindings,
            (Context::Bindings, _) => Context::Args,
            (Context::Node, "body" | "then" | "otherwise" | "default")
            | (Context::Case, "body") => Context::Nodes,
            (Context::Node, "cases") => Context::Cases,
            (Context::Args, _) => Context::Args,
            _ => Context::Json,
        }
    }
    fn value(&mut self, context: Context, path: &str, depth: usize) -> Result<Value, Diagnostic> {
        if depth > 128 {
            return Err(self.fail(path, "Maximum nesting depth exceeded"));
        }
        self.space();
        let args = matches!(context, Context::Args);
        match self.text.as_bytes().get(self.offset).copied() {
            Some(b'"') => {
                let value = Value::String(self.string(path)?);
                Ok(if args {
                    json!({"literal":value})
                } else {
                    value
                })
            }
            Some(b'{') => {
                self.offset += 1;
                let mut object = Map::new();
                self.space();
                if self.text.as_bytes().get(self.offset) != Some(&b'}') {
                    loop {
                        self.space();
                        if self.text.as_bytes().get(self.offset) != Some(&b'"') {
                            return Err(self.fail(path, "Object keys must be quoted"));
                        }
                        let key = self.string(path)?;
                        let child_path = format!("{path}/{}", pointer(&key));
                        if object.contains_key(&key) {
                            return Err(self.fail(&child_path, "Duplicate object key"));
                        }
                        self.expect(b':', &child_path)?;
                        let value =
                            self.value(Self::child(context, &key), &child_path, depth + 1)?;
                        object.insert(key, value);
                        self.space();
                        if self.text.as_bytes().get(self.offset) != Some(&b',') {
                            break;
                        }
                        self.offset += 1;
                    }
                }
                self.expect(b'}', path)?;
                if args {
                    Ok(json!({"object": object}))
                } else {
                    Ok(Value::Object(object))
                }
            }
            Some(b'[') => {
                self.offset += 1;
                let mut array = Vec::new();
                let child = match context {
                    Context::Nodes => Context::Node,
                    Context::Cases => Context::Case,
                    other => other,
                };
                self.space();
                if self.text.as_bytes().get(self.offset) != Some(&b']') {
                    loop {
                        array.push(self.value(
                            child,
                            &format!("{path}/{}", array.len()),
                            depth + 1,
                        )?);
                        self.space();
                        if self.text.as_bytes().get(self.offset) != Some(&b',') {
                            break;
                        }
                        self.offset += 1;
                    }
                }
                self.expect(b']', path)?;
                Ok(if args {
                    json!({"array":array})
                } else {
                    Value::Array(array)
                })
            }
            Some(_) => {
                let start = self.offset;
                let mut stack = Vec::new();
                let mut quoted = false;
                let mut escaped = false;
                while let Some(&byte) = self.text.as_bytes().get(self.offset) {
                    if quoted {
                        if escaped {
                            escaped = false;
                        } else if byte == b'\\' {
                            escaped = true;
                        } else if byte == b'"' {
                            quoted = false;
                        }
                    } else {
                        if stack.is_empty() && matches!(byte, b',' | b']' | b'}') {
                            break;
                        }
                        match byte {
                            b'"' => quoted = true,
                            b'(' => stack.push(b')'),
                            b'[' => stack.push(b']'),
                            b'{' => stack.push(b'}'),
                            b')' | b']' | b'}' => {
                                if stack.pop() != Some(byte) {
                                    return Err(self.fail(path, "Mismatched expression delimiter"));
                                }
                            }
                            _ => (),
                        }
                        if stack.len() > 128 {
                            return Err(self.fail(path, "Maximum expression nesting exceeded"));
                        }
                    }
                    self.offset += 1;
                }
                if quoted || !stack.is_empty() {
                    return Err(self.fail(path, "Unterminated expression"));
                }
                let token = self.text[start..self.offset].trim();
                if let Ok(value) = serde_json::from_str::<Value>(token) {
                    return Ok(if args {
                        json!({"literal":value})
                    } else {
                        value
                    });
                }
                if !args {
                    return Err(self.fail(
                        path,
                        "Bare expressions are allowed only in node args and returns.*.value",
                    ));
                }
                super::super::chain_compiler_v2::parse_logic_expression(token)
                    .map_err(|e| self.fail(path, &e.message))?;
                Ok(json!({"expression": token}))
            }
            None => Err(self.fail(path, "Unexpected end of source")),
        }
    }
}
