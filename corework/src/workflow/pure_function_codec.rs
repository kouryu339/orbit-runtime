//! Script-facing aliases for workflow pure nodes.
//!
//! The JSON DAG keeps stable node type names. The AI text view uses concise,
//! function-style aliases and leaves pin wiring to the compiler.

use crate::workflow::chain_ast::InlineExpr;

#[derive(Debug, Clone, Copy)]
pub struct PureFunctionSpec {
    pub name: &'static str,
    pub node_type: &'static str,
    pub input_pins: &'static [&'static str],
    pub default_output_pin: &'static str,
}

const SPECS: &[PureFunctionSpec] = &[
    PureFunctionSpec {
        name: "getvar",
        node_type: "GetVarNode",
        input_pins: &["Name"],
        default_output_pin: "Value",
    },
    PureFunctionSpec {
        name: "add",
        node_type: "AddNode",
        input_pins: &["A", "B"],
        default_output_pin: "Sum",
    },
    PureFunctionSpec {
        name: "mul",
        node_type: "MultiplyNode",
        input_pins: &["A", "B"],
        default_output_pin: "Product",
    },
    PureFunctionSpec {
        name: "neg",
        node_type: "NegNode",
        input_pins: &["Value"],
        default_output_pin: "Negated",
    },
    PureFunctionSpec {
        name: "pow",
        node_type: "PowNode",
        input_pins: &["Base", "Exponent"],
        default_output_pin: "Power",
    },
    PureFunctionSpec {
        name: "eq",
        node_type: "EqualNode",
        input_pins: &["A", "B"],
        default_output_pin: "IsEqual",
    },
    PureFunctionSpec {
        name: "neq",
        node_type: "NotEqualNode",
        input_pins: &["A", "B"],
        default_output_pin: "IsNotEqual",
    },
    PureFunctionSpec {
        name: "gt",
        node_type: "GreaterNode",
        input_pins: &["A", "B"],
        default_output_pin: "IsGreater",
    },
    PureFunctionSpec {
        name: "gte",
        node_type: "GreaterOrEqualNode",
        input_pins: &["A", "B"],
        default_output_pin: "IsGreaterOrEqual",
    },
    PureFunctionSpec {
        name: "lt",
        node_type: "LessNode",
        input_pins: &["A", "B"],
        default_output_pin: "IsLess",
    },
    PureFunctionSpec {
        name: "lte",
        node_type: "LessOrEqualNode",
        input_pins: &["A", "B"],
        default_output_pin: "IsLessOrEqual",
    },
    PureFunctionSpec {
        name: "xor",
        node_type: "XorNode",
        input_pins: &["A", "B"],
        default_output_pin: "IsXor",
    },
    PureFunctionSpec {
        name: "text_concat",
        node_type: "StringAppendNode",
        input_pins: &["A", "B"],
        default_output_pin: "Joined",
    },
    PureFunctionSpec {
        name: "contains",
        node_type: "ContainsNode",
        input_pins: &["Value", "Pattern"],
        default_output_pin: "Found",
    },
    PureFunctionSpec {
        name: "trim",
        node_type: "TrimNode",
        input_pins: &["Value"],
        default_output_pin: "Trimmed",
    },
    PureFunctionSpec {
        name: "regex_match",
        node_type: "RegexMatchNode",
        input_pins: &["Value", "Pattern"],
        default_output_pin: "IsMatch",
    },
    PureFunctionSpec {
        name: "item",
        node_type: "GetArrayElementNode",
        input_pins: &["Array", "Index"],
        default_output_pin: "Element",
    },
];

pub fn by_function_name(name: &str) -> Option<&'static PureFunctionSpec> {
    SPECS.iter().find(|spec| spec.name == name)
}

pub fn by_node_type(node_type: &str) -> Option<&'static PureFunctionSpec> {
    SPECS.iter().find(|spec| spec.node_type == node_type)
}

pub fn inline_expr(
    spec: &PureFunctionSpec,
    args: Vec<crate::workflow::chain_ast::Value>,
    output_pin: Option<String>,
) -> InlineExpr {
    InlineExpr {
        node_type: spec.node_type.to_string(),
        inputs: spec
            .input_pins
            .iter()
            .zip(args)
            .map(|(pin, value)| ((*pin).to_string(), value))
            .collect(),
        output_pin,
    }
}
