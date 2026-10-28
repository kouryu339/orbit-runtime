//! Structured editing frontend. Execution still uses the existing Chain compiler.
pub mod source;
use super::{blueprint_json::BlueprintJson, chain_ast as ast, pure_function_codec};
use crate::rpc_tool::RuntimeToolMetadata;
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub use source::parse_source;
use std::collections::{BTreeMap, HashSet};

pub const SCHEMA: &str = "orbit.workflow.logic/v1";
pub const COMPILER_VERSION: &str = "7";
pub const JSON_SCHEMA: &str = include_str!("logic.schema.json");
pub const RESOURCE_JSON_SCHEMA: &str = include_str!("resource.schema.json");
const MAX_ID: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogicDocument {
    pub schema: String,
    #[serde(default)]
    pub inputs: BTreeMap<String, Input>,
    #[serde(default)]
    pub variables: BTreeMap<String, Variable>,
    pub steps: Vec<LogicStep>,
    #[serde(default)]
    pub returns: BTreeMap<String, ReturnBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    #[serde(rename = "type")]
    pub data_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disablename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReturnDefinition {
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disablename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Existing expression-only returns remain readable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ReturnBinding {
    Definition(ReturnDefinition),
    Expression(Value),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogicStep {
    pub id: u64,
    #[serde(flatten)]
    pub operation: Operation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variable {
    #[serde(rename = "type")]
    pub data_type: String,
    pub initial: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disablename: Option<String>,
}

/// Each operation has one distinctive field. Unknown or mixed operation fields
/// are rejected instead of guessing which control structure the author intended.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Operation {
    Set {
        #[serde(rename = "set")]
        variable: String,
        value: Value,
    },
    Call {
        #[serde(rename = "call")]
        tool: String,
        args: BTreeMap<String, Value>,
    },
    If {
        #[serde(rename = "if")]
        condition: Value,
        then: Vec<LogicStep>,
        otherwise: Vec<LogicStep>,
    },
    ForEach {
        #[serde(rename = "for")]
        array: Value,
        body: Vec<LogicStep>,
    },
    Switch {
        #[serde(rename = "switch")]
        value: Value,
        cases: Vec<Case>,
        default: Vec<LogicStep>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub value: Value,
    pub body: Vec<LogicStep>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Diagnostic {
    pub step_id: Option<u64>,
    pub pointer: String,
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} at {} (step {:?}): {}",
            self.code, self.pointer, self.step_id, self.message
        )
    }
}
impl std::error::Error for Diagnostic {}

fn error(id: Option<u64>, path: &str, code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        step_id: id,
        pointer: path.into(),
        code: code.into(),
        message: message.into(),
    }
}
fn pointer(name: &str) -> String {
    name.replace('~', "~0").replace('/', "~1")
}

#[derive(Clone, Default)]
struct Scope {
    available: HashSet<u64>,
    in_loop: bool,
}

struct Lower<'a> {
    document: &'a LogicDocument,
    ids: HashSet<u64>,
    /// Backend line diagnostics map back to JSON locations; these are not IDs.
    locations: Vec<(Option<u64>, String)>,
}

impl Lower<'_> {
    fn validate_expression(
        &self,
        value: &ast::Value,
        scope: &Scope,
        id: Option<u64>,
        path: &str,
    ) -> Result<(), Diagnostic> {
        let fail = |message: &str| error(id, path, "invalid_expression", message);
        match value {
            ast::Value::InputRef(name) if !self.document.inputs.contains_key(name) => {
                Err(fail("Input does not exist"))
            }
            ast::Value::VarRef(name)
                if !self.document.variables.contains_key(name)
                    && !(scope.in_loop && matches!(name.as_str(), "item" | "index")) =>
            {
                Err(fail(
                    "Variable is not declared or loop local is outside its scope",
                ))
            }
            ast::Value::StepRef { step_id, .. }
                if !step_id
                    .parse::<u64>()
                    .is_ok_and(|n| scope.available.contains(&n)) =>
            {
                Err(fail(
                    "Node ID must precede this expression in the same or an ancestor scope",
                ))
            }
            ast::Value::Inline(expression) => {
                for (_, input) in &expression.inputs {
                    self.validate_expression(input, scope, id, path)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn expr(
        &self,
        value: &Value,
        scope: &Scope,
        id: Option<u64>,
        path: &str,
    ) -> Result<ast::Value, Diagnostic> {
        let fail = |message: &str| error(id, path, "invalid_expression", message);
        if let Some(text) = value
            .as_object()
            .filter(|o| o.len() == 1)
            .and_then(|o| o.get("expression"))
            .and_then(Value::as_str)
        {
            let expression = super::chain_compiler_v2::parse_logic_expression(text)
                .map_err(|e| error(id, path, "invalid_expression", e.message))?;
            self.validate_expression(&expression, scope, id, path)?;
            return Ok(expression);
        }
        let Some(object) = value.as_object() else {
            // Arrays are fixed JSON data; computed arrays require a pure function.
            return Ok(ast::Value::Literal(value.clone()));
        };
        if object.len() == 1 {
            if let Some(values) = object.get("array").and_then(Value::as_array) {
                let values = values
                    .iter()
                    .enumerate()
                    .map(|(i, value)| self.expr(value, scope, id, &format!("{path}/{i}")))
                    .collect::<Result<Vec<_>, _>>()?;
                return Ok(make_array(values));
            }
            if let Some(fields) = object.get("object").and_then(Value::as_object) {
                let keys = fields.keys().cloned().collect::<Vec<_>>();
                let values = fields
                    .iter()
                    .map(|(key, value)| {
                        self.expr(value, scope, id, &format!("{path}/{}", pointer(key)))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if values.iter().all(|v| matches!(v, ast::Value::Literal(_))) {
                    let object = keys
                        .into_iter()
                        .zip(values)
                        .map(|(key, value)| match value {
                            ast::Value::Literal(value) => (key, value),
                            _ => unreachable!(),
                        })
                        .collect::<serde_json::Map<_, _>>();
                    return Ok(json_literal(Value::Object(object)));
                }
                return Ok(ast::Value::Inline(Box::new(ast::InlineExpr {
                    node_type: "MakeObjectNode".into(),
                    inputs: vec![
                        ("Keys".into(), ast::Value::Literal(serde_json::json!(keys))),
                        ("Values".into(), make_array(values)),
                    ],
                    output_pin: Some("Object".into()),
                })));
            }
            if let Some(v) = object.get("literal") {
                if v.is_object() || v.is_null() {
                    return Ok(json_literal(v.clone()));
                }
                return Ok(ast::Value::Literal(v.clone()));
            }
            if let Some(name) = object.get("var").and_then(Value::as_str) {
                if !self.document.variables.contains_key(name) {
                    return Err(fail("Workflow variable is not declared"));
                }
                return Ok(ast::Value::VarRef(name.into()));
            }
            if let Some(name) = object.get("input").and_then(Value::as_str) {
                if !self.document.inputs.contains_key(name) {
                    return Err(fail("Input does not exist"));
                }
                return Ok(ast::Value::InputRef(name.into()));
            }
            if let Some(name) = object.get("local").and_then(Value::as_str) {
                if !scope.in_loop || !matches!(name, "item" | "index") {
                    return Err(fail(
                        "Only item and index are available inside the current loop",
                    ));
                }
                return Ok(ast::Value::VarRef(name.into()));
            }
        }
        if object.len() == 2 {
            if let (Some(step), Some(output)) = (
                object.get("step").and_then(Value::as_u64),
                object.get("output").and_then(Value::as_str),
            ) {
                if !scope.available.contains(&step) {
                    return Err(fail(
                        "Step must precede this expression in the same or an ancestor scope",
                    ));
                }
                return Ok(ast::Value::StepRef {
                    step_id: step.to_string(),
                    pin_name: output.into(),
                });
            }
            if let (Some(function), Some(args)) = (
                object.get("fn").and_then(Value::as_str),
                object.get("args").and_then(Value::as_array),
            ) {
                let spec = pure_function_codec::by_function_name(function)
                    .ok_or_else(|| fail("Unknown pure function"))?;
                if args.len() != spec.input_pins.len() {
                    return Err(fail(
                        "Pure function argument count does not match its contract",
                    ));
                }
                let args = args
                    .iter()
                    .enumerate()
                    .map(|(i, v)| self.expr(v, scope, id, &format!("{path}/args/{i}")))
                    .collect::<Result<Vec<_>, _>>()?;
                return Ok(ast::Value::Inline(Box::new(
                    pure_function_codec::inline_expr(spec, args),
                )));
            }
        }
        Err(fail(
            "Expected input, var, local, step/output, fn/args, or literal object",
        ))
    }

    fn block(
        &mut self,
        steps: &[LogicStep],
        scope: &mut Scope,
        path: &str,
    ) -> Result<Vec<ast::Step>, Diagnostic> {
        let mut result = Vec::new();
        for (index, step) in steps.iter().enumerate() {
            let path = format!("{path}/{index}");
            let id = Some(step.id);
            if step.id == 0 || step.id > MAX_ID || !self.ids.insert(step.id) {
                return Err(error(
                    id,
                    &format!("{path}/id"),
                    "invalid_id",
                    "IDs must be unique positive safe integers",
                ));
            }
            self.locations.push((id, path.clone()));
            let line = self.locations.len();
            let sid = Some(step.id.to_string());
            let lowered = match &step.operation {
                Operation::Set { variable, value } => {
                    let declaration = self.document.variables.get(variable).ok_or_else(|| {
                        error(
                            id,
                            &format!("{path}/set"),
                            "unknown_variable",
                            "Declare the workflow variable before assigning it",
                        )
                    })?;
                    let expression = self.expr(value, scope, id, &format!("{path}/value"))?;
                    if let ast::Value::Literal(literal) = &expression {
                        if !matches_type(literal, &declaration.data_type) {
                            return Err(error(
                                id,
                                &format!("{path}/value"),
                                "variable_type",
                                "Literal does not match the declared variable type",
                            ));
                        }
                    }
                    ast::Step::Node {
                        line,
                        step_id: sid,
                        node_type: "SetVarNode".into(),
                        inputs: vec![
                            (
                                "Name".into(),
                                ast::Value::Literal(Value::String(variable.clone())),
                            ),
                            ("Value".into(), expression),
                        ],
                    }
                }
                Operation::Call { tool, args } => {
                    let inputs = args
                        .iter()
                        .map(|(name, value)| {
                            Ok((
                                name.clone(),
                                self.expr(
                                    value,
                                    scope,
                                    id,
                                    &format!("{path}/args/{}", pointer(name)),
                                )?,
                            ))
                        })
                        .collect::<Result<Vec<_>, Diagnostic>>()?;
                    ast::Step::Node {
                        line,
                        step_id: sid,
                        node_type: tool.clone(),
                        inputs,
                    }
                }
                Operation::If {
                    condition,
                    then,
                    otherwise,
                } => {
                    let condition = self.expr(condition, scope, id, &format!("{path}/if"))?;
                    let true_block =
                        self.block(then, &mut scope.clone(), &format!("{path}/then"))?;
                    let false_block =
                        self.block(otherwise, &mut scope.clone(), &format!("{path}/otherwise"))?;
                    ast::Step::If {
                        line,
                        step_id: sid,
                        condition,
                        true_block,
                        false_block,
                    }
                }
                Operation::ForEach { array, body } => {
                    let array = self.expr(array, scope, id, &format!("{path}/for"))?;
                    let mut child = scope.clone();
                    child.in_loop = true;
                    let body = self.block(body, &mut child, &format!("{path}/body"))?;
                    ast::Step::ForEach {
                        line,
                        step_id: sid,
                        array,
                        body,
                    }
                }
                Operation::Switch {
                    value,
                    cases,
                    default,
                } => {
                    let value = self.expr(value, scope, id, &format!("{path}/switch"))?;
                    let mut seen = HashSet::new();
                    let mut arms = Vec::new();
                    for (i, case) in cases.iter().enumerate() {
                        if !case.value.is_null()
                            && !case.value.is_boolean()
                            && !case.value.is_number()
                            && !case.value.is_string()
                        {
                            return Err(error(
                                id,
                                &format!("{path}/cases/{i}/value"),
                                "invalid_case",
                                "Case value must be a scalar literal",
                            ));
                        }
                        if !seen.insert(case.value.to_string()) {
                            return Err(error(
                                id,
                                &path,
                                "duplicate_case",
                                "Switch case values must be unique",
                            ));
                        }
                        let body = self.block(
                            &case.body,
                            &mut scope.clone(),
                            &format!("{path}/cases/{i}/body"),
                        )?;
                        arms.push((case.value.clone(), body));
                    }
                    let mut tail =
                        self.block(default, &mut scope.clone(), &format!("{path}/default"))?;
                    for (i, (case, body)) in arms.into_iter().enumerate().rev() {
                        let condition =
                            ast::Value::Inline(Box::new(pure_function_codec::inline_expr(
                                pure_function_codec::by_function_name("eq").unwrap(),
                                vec![value.clone(), ast::Value::Literal(case)],
                            )));
                        tail = vec![ast::Step::If {
                            line,
                            step_id: if i == 0 {
                                sid.clone()
                            } else {
                                Some(format!("logic_switch_{}_{}", step.id, i))
                            },
                            condition,
                            true_block: body,
                            false_block: tail,
                        }];
                    }
                    ast::Step::Block(tail)
                }
            };
            result.push(lowered);
            // Only calls expose outputs. Structured children never leak to a parent.
            if matches!(step.operation, Operation::Call { .. }) {
                scope.available.insert(step.id);
            }
        }
        Ok(result)
    }
}

fn make_array(values: Vec<ast::Value>) -> ast::Value {
    if values.iter().all(|v| matches!(v, ast::Value::Literal(_))) {
        return json_literal(Value::Array(
            values
                .into_iter()
                .map(|v| match v {
                    ast::Value::Literal(v) => v,
                    _ => unreachable!(),
                })
                .collect(),
        ));
    }
    let mut chunks = values.chunks(5).map(|values| {
        ast::Value::Inline(Box::new(ast::InlineExpr {
            node_type: "MakeArrayNode".into(),
            inputs: values
                .iter()
                .enumerate()
                .map(|(i, v)| (format!("Element{i}"), v.clone()))
                .collect(),
            output_pin: Some("Array".into()),
        }))
    });
    let mut result = chunks.next().expect("nonliteral array is nonempty");
    for next in chunks {
        result = ast::Value::Inline(Box::new(ast::InlineExpr {
            node_type: "ArrayConcatNode".into(),
            inputs: vec![("Array1".into(), result), ("Array2".into(), next)],
            output_pin: Some("Result".into()),
        }));
    }
    result
}

fn json_literal(value: Value) -> ast::Value {
    ast::Value::Inline(Box::new(ast::InlineExpr {
        node_type: "JsonLiteralNode".into(),
        inputs: vec![(
            "Json".into(),
            ast::Value::Literal(Value::String(value.to_string())),
        )],
        output_pin: Some("Value".into()),
    }))
}

fn matches_type(value: &Value, data_type: &str) -> bool {
    match data_type {
        "Any" => true,
        "String" => value.is_string(),
        "bool" | "Boolean" => value.is_boolean(),
        "i64" => value.as_i64().is_some(),
        "f64" => value.as_f64().is_some(),
        "Object" => value.is_object(),
        _ => data_type
            .strip_prefix("Array<")
            .and_then(|v| v.strip_suffix('>'))
            .is_some_and(|inner| {
                value
                    .as_array()
                    .is_some_and(|items| items.iter().all(|item| matches_type(item, inner)))
            }),
    }
}

pub fn compile(
    document: &LogicDocument,
    tools: &[RuntimeToolMetadata],
) -> Result<BlueprintJson, Diagnostic> {
    if document.schema != SCHEMA {
        return Err(error(None, "/schema", "unsupported_schema", SCHEMA));
    }
    let mut lower = Lower {
        document,
        ids: HashSet::new(),
        locations: Vec::new(),
    };
    let mut steps = Vec::new();
    for (name, input) in &document.inputs {
        steps.push(ast::Step::Input {
            line: 0,
            param_name: name.clone(),
            var_name: name.clone(),
            param_type: Some(input.data_type.clone()),
            default: input.default.clone().map(ast::Value::Literal),
        });
    }
    // Empty inputs still need a StartNode.
    if steps.is_empty() {
        steps.push(ast::Step::Block(Vec::new()));
    }
    for (name, variable) in &document.variables {
        let path = format!("/variables/{}", pointer(name));
        if name.is_empty()
            || name == "item"
            || name == "index"
            || document.inputs.contains_key(name)
        {
            return Err(error(None, &path, "invalid_variable", "Variable names must be nonempty, distinct from inputs, and cannot be item or index"));
        }
        if !matches_type(&variable.initial, &variable.data_type) {
            return Err(error(
                None,
                &format!("{path}/initial"),
                "variable_type",
                "Initial value must be a static JSON literal matching the declared type",
            ));
        }
        if variable.initial.as_array().is_some_and(Vec::is_empty) {
            return Err(error(
                None,
                &format!("{path}/initial"),
                "unsupported_initializer",
                "The existing compiler requires a nonempty array initializer",
            ));
        }
        steps.push(ast::Step::VarInit {
            line: 0,
            name: name.clone(),
            initial: ast::Value::Literal(variable.initial.clone()),
        });
    }
    let mut scope = Scope::default();
    steps.extend(lower.block(&document.steps, &mut scope, "/steps")?);
    let assigns = document
        .returns
        .iter()
        .map(|(name, binding)| {
            let (value, path) = match binding {
                ReturnBinding::Definition(definition) => (
                    &definition.value,
                    format!("/returns/{}/value", pointer(name)),
                ),
                ReturnBinding::Expression(value) => (value, format!("/returns/{}", pointer(name))),
            };
            Ok((name.clone(), lower.expr(value, &scope, None, &path)?))
        })
        .collect::<Result<Vec<_>, Diagnostic>>()?;
    steps.push(ast::Step::Return { line: 0, assigns });
    let mut blueprint = super::chain_compiler::compile_chain_from_ast_with_runtime_tools(
        &ast::Chain { steps },
        tools,
    )
    .map_err(|e| {
        let (id, path) = e
            .line
            .checked_sub(1)
            .and_then(|i| lower.locations.get(i))
            .cloned()
            .unwrap_or((None, String::new()));
        error(id, &path, "compile_contract", e.message)
    })?;
    // Preserve explicit declarations; the legacy frontend infers broad types.
    for variable in &mut blueprint.variables {
        if let Some(declaration) = document.variables.get(&variable.name) {
            variable.data_type = declaration.data_type.clone();
            variable.display_name = declaration.disablename.clone();
        }
    }
    for (node_type, pin_kind, is_input) in [
        ("StartNode", "DataOutput", true),
        ("EndNode", "DataInput", false),
    ] {
        let mut metadata = Vec::new();
        for node in blueprint
            .nodes
            .iter_mut()
            .filter(|n| n.node_type == node_type)
        {
            for pin in node.pins.iter_mut().filter(|p| p.kind == pin_kind) {
                let (display_name, description) = if is_input {
                    document
                        .inputs
                        .get(&pin.name)
                        .map(|i| (i.disablename.clone(), i.description.clone()))
                        .unwrap_or_default()
                } else {
                    match document.returns.get(&pin.name) {
                        Some(ReturnBinding::Definition(r)) => {
                            (r.disablename.clone(), r.description.clone())
                        }
                        _ => (None, None),
                    }
                };
                if let Some(description) = &description {
                    pin.description = description.clone();
                }
                metadata.push(super::blueprint_json::PinMetadata {
                    name: pin.name.clone(),
                    data_type: pin.data_type.clone(),
                    default_value: pin.default_value.clone(),
                    display_name,
                    description: description.unwrap_or_else(|| pin.description.clone()),
                    ..Default::default()
                });
            }
        }
        if is_input {
            blueprint.metadata.inputs = metadata;
        } else {
            blueprint.metadata.outputs = metadata;
        }
    }
    Ok(blueprint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    #[tokio::test]
    async fn dotted_references_and_nested_pure_calls_execute_with_scope_checks() {
        use crate::{
            workflow::{execution::ExecutionContext, BlueprintLoader},
            world::FrameworkState,
        };
        let mut document = parse_source(r#"{
            "schema":"orbit.workflow.logic/v1",
            "inputs":{"names":{"type":"String"}},
            "variables":{"count":{"type":"i64","initial":0}},
            "steps":[
                {"id":1,"call":"StringAppendNode","args":{"A":input.names,"B":""}},
                {"id":2,"for":{"fn":"split","args":[{"step":1,"output":"Joined"},[",","，"]]},"body":[{"id":3,"set":"count","value":{"fn":"add","args":[{"var":"count"},1]}}]}
            ],
            "returns":{
                "count":{"value":var.count},
                "second":{"value":item(split(input.names, [",", "，"]), 1)},
                "literal":{"value":"add(var.missing, 1)"}
            }
        }"#).unwrap();
        let blueprint = compile(&document, &[]).unwrap();
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
        let outputs = loaded
            .compiled
            .executor()
            .execute_with_params(
                &mut execution,
                HashMap::from([(
                    "names".into(),
                    crate::workflow::DataValue::from_string("a,b，c"),
                )]),
            )
            .await
            .unwrap();
        assert_eq!(outputs["count"].as_f64(), Some(3.0));
        assert_eq!(outputs["second"].as_str(), Some("b"));
        assert_eq!(outputs["literal"].as_str(), Some("add(var.missing, 1)"));
        for text in [
            "add(var.missing, 1)",
            "add(var.item, 1)",
            "add(99.Value, 1)",
            "split(input.missing, [\",\"])",
            "split(input.names)",
            "add(1,,2)",
            "unknown(input.names)",
        ] {
            document.returns.insert(
                "bad".into(),
                ReturnBinding::Expression(json!({"expression":text})),
            );
            assert_eq!(compile(&document, &[]).unwrap_err().pointer, "/returns/bad");
        }
    }

    #[test]
    fn rejects_mixed_operation_fields_and_keeps_literals_opaque() {
        for value in [
            json!({"id":1,"for":[],"as":"row","body":[]}),
            json!({"id":1,"call":"DebugPrintNode","args":{},"set":"count","value":1}),
            json!({"id":1,"for":[],"body":[],"if":true,"then":[],"otherwise":[]}),
        ] {
            assert!(serde_json::from_value::<LogicStep>(value).is_err());
        }
        let step: LogicStep = serde_json::from_value(json!({"id":1,"call":"DebugPrintNode","args":{"Value":{"literal":{"op":"user data","for":"user data"}}}})).unwrap();
        let saved = serde_json::to_value(step).unwrap();
        assert!(saved.get("op").is_none());
        assert_eq!(saved["call"], "DebugPrintNode");
        assert_eq!(saved["args"]["Value"]["literal"]["op"], "user data");
    }

    #[test]
    fn labels_and_descriptions_reach_original_blueprint_definitions() {
        let document: LogicDocument = serde_json::from_value(json!({
            "schema":SCHEMA,
            "inputs":{"path":{"type":"String","disablename":"Excel 文件","description":"文件路径；文件必须存在。"}},
            "variables":{"count":{"type":"i64","initial":0,"disablename":"处理数量"}},
            "steps":[],
            "returns":{"result":{"value":{"input":"path"},"disablename":"文件路径","description":"原样返回输入路径。"}}
        })).unwrap();
        let blueprint = compile(&document, &[]).unwrap();
        assert_eq!(
            blueprint.metadata.inputs[0].display_name.as_deref(),
            Some("Excel 文件")
        );
        assert_eq!(
            blueprint.metadata.inputs[0].description,
            "文件路径；文件必须存在。"
        );
        assert_eq!(
            blueprint.metadata.outputs[0].display_name.as_deref(),
            Some("文件路径")
        );
        assert_eq!(
            blueprint.metadata.outputs[0].description,
            "原样返回输入路径。"
        );
        assert_eq!(
            blueprint.variables[0].display_name.as_deref(),
            Some("处理数量")
        );
        let saved = serde_json::to_value(&blueprint).unwrap();
        let restored: BlueprintJson = serde_json::from_value(saved).unwrap();
        assert_eq!(
            restored.variables[0].display_name,
            blueprint.variables[0].display_name
        );
        let old: super::super::blueprint_json::BlueprintVariable =
            serde_json::from_value(json!({"name":"count","data_type":"i64","default_value":0}))
                .unwrap();
        assert!(old.display_name.is_none());
        let saved_logic = serde_json::to_value(&document).unwrap();
        assert_eq!(saved_logic["inputs"]["path"]["disablename"], "Excel 文件");
        assert!(saved_logic["variables"]["count"]
            .get("description")
            .is_none());
    }

    #[tokio::test]
    async fn variables_accumulate_per_run_and_reject_invalid_declarations() {
        use crate::{
            workflow::{execution::ExecutionContext, BlueprintLoader},
            world::FrameworkState,
        };
        let mut document: LogicDocument = serde_json::from_value(json!({
            "schema": SCHEMA,
            "variables": {"count":{"type":"i64","initial":0}},
            "steps":[{"id":1,"for":[10,20,30],"body":[{"id":2,"set":"count","value":{"fn":"add","args":[{"var":"count"},{"local":"index"}]}}]}],
            "returns":{"count":{"var":"count"}}
        })).unwrap();
        let blueprint = compile(&document, &[]).unwrap();
        assert_eq!(blueprint.variables[0].data_type, "i64");
        for _ in 0..2 {
            let ctx = FrameworkState::initialize().unwrap().create_context();
            let loaded = BlueprintLoader::new()
                .load_from_blueprint_json(blueprint.clone(), &ctx)
                .unwrap();
            let mut execution = ExecutionContext::from_context(ctx);
            loaded
                .compiled
                .initialize_defaults(&mut execution)
                .await
                .unwrap();
            let outputs = loaded
                .compiled
                .executor()
                .execute_with_params(&mut execution, HashMap::new())
                .await
                .unwrap();
            assert_eq!(outputs["count"].as_f64(), Some(3.0));
        }
        document.variables.get_mut("count").unwrap().initial = json!("wrong");
        assert_eq!(
            compile(&document, &[]).unwrap_err().pointer,
            "/variables/count/initial"
        );
        document.variables.clear();
        assert_eq!(
            compile(&document, &[]).unwrap_err().code,
            "unknown_variable"
        );
    }
    fn doc(steps: Value, returns: Value) -> LogicDocument {
        serde_json::from_value(json!({"schema": SCHEMA, "inputs":{"x":{"type":"String"}}, "steps": steps, "returns":returns})).unwrap()
    }
    #[test]
    fn compiles_explicit_input_and_literal() {
        let bp = compile(
            &doc(
                json!([]),
                json!({"value":{"input":"x"}, "fixed":{"literal":{"step":99}}}),
            ),
            &[],
        )
        .unwrap();
        bp.validate_graph().unwrap();
    }
    #[test]
    fn switch_has_dynamic_case_count_and_no_new_executor_node() {
        let cases = (0..5)
            .map(|i| json!({"value":i.to_string(),"body":[]}))
            .collect::<Vec<_>>();
        let bp = compile(
            &doc(
                json!([{"id":50,"switch":{"input":"x"},"cases":cases,"default":[]}]),
                json!({}),
            ),
            &[],
        )
        .unwrap();
        assert_eq!(
            bp.nodes
                .iter()
                .filter(|n| n.node_type == "BranchNode")
                .count(),
            5
        );
    }
    #[test]
    fn rejects_duplicate_ids_and_scope_escape() {
        let step = json!({"id":4,"if":true,"then":[],"otherwise":[]});
        assert_eq!(
            compile(&doc(json!([step, step]), json!({})), &[])
                .unwrap_err()
                .code,
            "invalid_id"
        );
        let err = compile(&doc(json!([]), json!({"out":{"local":"item"}})), &[]).unwrap_err();
        assert_eq!(err.pointer, "/returns/out");
        assert!(compile(
            &doc(json!([]), json!({"out":{"step":9,"output":"Value"}})),
            &[]
        )
        .is_err());
    }

    #[test]
    fn loop_local_binds_to_existing_backend_item() {
        let bp = compile(&doc(json!([{"id":8,"for":["a"],"body":[{"id":90,"call":"DebugPrintNode","args":{"Value":{"local":"item"}}}]}]), json!({})), &[]).unwrap();
        assert!(bp
            .connections
            .iter()
            .any(|e| e.source_node == "8" && e.source_pin == "Item" && e.target_node == "90"));
    }

    #[tokio::test]
    async fn switch_executes_only_selected_arm_and_default() {
        use crate::{
            workflow::{execution::ExecutionContext, BlueprintLoader},
            world::FrameworkState,
        };
        for (selector, expected) in [("a", "11"), ("b", "12"), ("other", "13")] {
            let call = |id| json!({"id":id,"call":"DebugPrintNode","args":{"Value":"visited"}});
            let bp = compile(&doc(json!([{"id":50,"switch":{"input":"x"},"cases":[{"value":"a","body":[call(11)]},{"value":"b","body":[call(12)]}],"default":[call(13)]}]), json!({"result":{"input":"x"}})), &[]).unwrap();
            let ctx = FrameworkState::initialize().unwrap().create_context();
            let loaded = BlueprintLoader::new()
                .load_from_blueprint_json(bp, &ctx)
                .unwrap();
            let mut execution = ExecutionContext::from_context(ctx);
            execution.enable_trace("logic-switch", loaded.compiled.source_map.clone());
            loaded
                .compiled
                .initialize_defaults(&mut execution)
                .await
                .unwrap();
            let outputs = loaded
                .compiled
                .executor()
                .execute_with_params(
                    &mut execution,
                    HashMap::from([(
                        "x".into(),
                        crate::workflow::DataValue::from_string(selector),
                    )]),
                )
                .await
                .unwrap();
            assert_eq!(
                outputs.get("result").and_then(|v| v.as_str()),
                Some(selector)
            );
            let trace = execution.take_trace().unwrap();
            let visited = trace
                .nodes
                .iter()
                .filter_map(|n| n.source.as_ref().and_then(|s| s.step.as_deref()))
                .filter(|id| ["11", "12", "13"].contains(id))
                .collect::<Vec<_>>();
            assert_eq!(visited, vec![expected]);
        }
    }
}
