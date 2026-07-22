//! 解释器：BlueprintJson → AI 友好文本视图
//!
//! 沿 exec 连接链遍历蓝图图结构，生成简化的 EXEC 风格文本。

use crate::workflow::blueprint_json::*;
use crate::workflow::chain_id::HierarchicalIdGen;
use crate::workflow::pure_function_codec;
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// 错误
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DecompileError {
    pub message: String,
}

impl std::fmt::Display for DecompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "decompile error: {}", self.message)
    }
}

impl std::error::Error for DecompileError {}

impl DecompileError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ─────────────────────────────────────────────────────────────────────────────

pub struct ChainDecompiler {
    variables: Vec<BlueprintVariable>,
    /// node_id → node
    node_map: HashMap<String, BlueprintNodeJson>,
    /// exec 连接正向查找：(src_node, src_pin) → (tgt_node, tgt_pin)
    exec_out: HashMap<(String, String), (String, String)>,
    /// data 连接反向查找：(tgt_node, tgt_pin) → (src_node, src_pin)
    data_in: HashMap<(String, String), (String, String)>,
    /// ID 生成器
    id_gen: HierarchicalIdGen,
    step_map: HashMap<String, String>,
    indent: usize,
}

impl ChainDecompiler {
    pub fn decompile(bp: &BlueprintJson) -> Result<String, DecompileError> {
        let mut dc = Self::new(bp)?;
        dc.run()
    }

    fn new(bp: &BlueprintJson) -> Result<Self, DecompileError> {
        let mut node_map = HashMap::new();
        let mut exec_out = HashMap::new();
        let mut data_in = HashMap::new();

        // 构建节点查找表
        for node in &bp.nodes {
            node_map.insert(node.id.clone(), node.clone());
        }

        // 构建连接查找表
        for conn in &bp.connections {
            if conn.connection_type == "Exec" {
                exec_out.insert(
                    (conn.source_node.clone(), conn.source_pin.clone()),
                    (conn.target_node.clone(), conn.target_pin.clone()),
                );
            } else {
                // Data 连接：反向查找（目标引脚 → 源引脚）
                data_in.insert(
                    (conn.target_node.clone(), conn.target_pin.clone()),
                    (conn.source_node.clone(), conn.source_pin.clone()),
                );
            }
        }

        Ok(Self {
            variables: bp.variables.clone(),
            node_map,
            exec_out,
            data_in,
            id_gen: HierarchicalIdGen::new(),
            step_map: HashMap::new(),
            indent: 0,
        })
    }

    fn run(&mut self) -> Result<String, DecompileError> {
        let mut lines = Vec::new();

        // 1. 找 StartNode
        let start_node = self
            .find_node_by_type("StartNode")
            .ok_or_else(|| DecompileError::new("未找到 StartNode"))?;
        let start_id = start_node.id.clone();

        // 2. 生成 INPUT 行
        let input_line = self.decompile_start_node(&start_node)?;
        lines.push(input_line);
        for variable in &self.variables {
            let value = variable
                .default_value
                .as_ref()
                .map(|value| self.format_json_value(value))
                .unwrap_or_else(|| "null".to_string());
            lines.push(format!("${} = {}", variable.name, value));
        }

        let mut current = self.follow_exec(&start_id, "Out");

        // 4. 沿 exec 链遍历
        self.decompile_exec_chain(&mut current, &mut lines)?;

        // 5. 找 EndNode，生成 RETURN 行
        let end_node = self
            .find_node_by_type("EndNode")
            .ok_or_else(|| DecompileError::new("未找到 EndNode"))?;
        let return_line = self.decompile_end_node(&end_node)?;
        lines.push(return_line);

        Ok(lines.join("\n"))
    }

    // ── 查找辅助 ─────────────────────────────────────────────────────────

    fn find_node_by_type(&self, node_type: &str) -> Option<BlueprintNodeJson> {
        self.node_map
            .values()
            .find(|n| n.node_type == node_type)
            .cloned()
    }

    fn follow_exec(&self, node_id: &str, pin: &str) -> Option<(String, String)> {
        self.exec_out
            .get(&(node_id.to_string(), pin.to_string()))
            .cloned()
    }

    fn indent_str(&self) -> String {
        "    ".repeat(self.indent)
    }

    // ── StartNode → INPUT ────────────────────────────────────────────────

    fn decompile_start_node(&mut self, node: &BlueprintNodeJson) -> Result<String, DecompileError> {
        let data_outputs: Vec<&NodePin> = node
            .pins
            .iter()
            .filter(|p| p.kind == "DataOutput")
            .collect();

        if data_outputs.is_empty() {
            return Ok("input".to_string());
        }

        let params: Vec<String> = data_outputs
            .iter()
            .map(|p| {
                let public_type = crate::data_type::public_type_name(&p.data_type);
                let dt = (!public_type.is_empty() && public_type != "Any")
                    .then(|| format!(":{}", public_type))
                    .unwrap_or_default();
                // 查找对应 DataInput 引脚的默认值
                let default_val = node
                    .pins
                    .iter()
                    .find(|di| di.name == p.name && di.kind == "DataInput")
                    .and_then(|di| di.default_value.as_ref());
                if let Some(dv) = default_val {
                    format!("{}{}={}", p.name, dt, self.format_json_value(dv))
                } else {
                    format!("{}{}", p.name, dt)
                }
            })
            .collect();

        Ok(format!("input {}", params.join(" ")))
    }

    // ── EndNode → RETURN ─────────────────────────────────────────────────

    fn decompile_end_node(&self, node: &BlueprintNodeJson) -> Result<String, DecompileError> {
        let data_inputs: Vec<&NodePin> =
            node.pins.iter().filter(|p| p.kind == "DataInput").collect();

        if data_inputs.is_empty() {
            return Ok("return".to_string());
        }

        let mut assigns = Vec::new();
        for pin in &data_inputs {
            let value = self.resolve_data_input(&node.id, &pin.name)?;
            assigns.push(format!("{}={}", pin.name, value));
        }

        Ok(format!("return {}", assigns.join(" ")))
    }

    // ── exec 链遍历 ───────────────────────────────────────────────────────

    fn decompile_exec_chain(
        &mut self,
        current: &mut Option<(String, String)>,
        lines: &mut Vec<String>,
    ) -> Result<(), DecompileError> {
        while let Some((node_id, _pin)) = current.take() {
            let node = self
                .node_map
                .get(&node_id)
                .cloned()
                .ok_or_else(|| DecompileError::new(format!("未找到节点 {}", node_id)))?;

            match node.node_type.as_str() {
                "BranchNode" => {
                    self.decompile_branch(&node, lines)?;
                    return Ok(());
                }
                "ForEachNode" => {
                    self.decompile_for_each(&node, lines)?;
                    *current = self.follow_exec(&node.id, "Completed");
                }
                "ForLoopNode" => {
                    self.decompile_for_loop(&node, lines)?;
                    *current = self.follow_exec(&node.id, "Completed");
                }
                "BreakNode" => {
                    let step_id = self.assign_step_id(&node.id);
                    lines.push(format!("{}{}: BREAK", self.indent_str(), step_id));
                    *current = None;
                }
                "SetVarNode" => {
                    let step_id = self.assign_step_id(&node.id);
                    let line = self.decompile_setvar(&node)?;
                    lines.push(format!("{}{}: {}", self.indent_str(), step_id, line));
                    *current = self.follow_exec(&node.id, "Then");
                }
                "EndNode" => {
                    // 到达 EndNode，停止遍历
                    *current = None;
                }
                _ => {
                    // 普通 impure 节点
                    let line = self.decompile_impure_node(&node)?;
                    lines.push(format!("{}{}", self.indent_str(), line));
                    *current = self.follow_exec(&node.id, "Then");
                }
            }
        }
        Ok(())
    }

    // ── 普通 impure 节点 ─────────────────────────────────────────────────

    fn decompile_impure_node(
        &mut self,
        node: &BlueprintNodeJson,
    ) -> Result<String, DecompileError> {
        let step_id = self.assign_step_id(&node.id);
        let args = self.decompile_data_inputs(node)?;
        let node_name = self.strip_node_suffix(&node.node_type);
        if args.is_empty() {
            Ok(format!("{}: EXEC {}", step_id, node_name))
        } else {
            Ok(format!("{}: EXEC {} {}", step_id, node_name, args))
        }
    }

    // ── SetVar（非初始化） ───────────────────────────────────────────────

    fn decompile_setvar(&mut self, node: &BlueprintNodeJson) -> Result<String, DecompileError> {
        let var_name = self.get_pin_value(node, "Name")?;
        let value = self.resolve_data_input(&node.id, "Value")?;
        Ok(format!("setvar {} = {}", var_name, value))
    }

    // ── BranchNode → IF/ELSE ─────────────────────────────────────────────

    fn decompile_branch(
        &mut self,
        node: &BlueprintNodeJson,
        lines: &mut Vec<String>,
    ) -> Result<(), DecompileError> {
        let step_id = self.assign_step_id(&node.id);
        let condition = self.resolve_data_input(&node.id, "Condition")?;
        lines.push(format!(
            "{}{}: IF {}",
            self.indent_str(),
            step_id,
            condition
        ));

        // True 分支
        self.indent += 1;
        self.id_gen.push_scope(&step_id);
        let mut true_next = self.follow_exec(&node.id, "True");
        self.decompile_exec_chain(&mut true_next, lines)?;
        self.id_gen.pop_scope();
        self.indent -= 1;

        // False 分支
        let false_next = self.follow_exec(&node.id, "False");
        if false_next.is_some() {
            lines.push(format!("{}ELSE", self.indent_str()));
            self.indent += 1;
            self.id_gen.push_scope(&step_id);
            let mut false_chain = false_next;
            self.decompile_exec_chain(&mut false_chain, lines)?;
            self.id_gen.pop_scope();
            self.indent -= 1;
        }

        lines.push(format!("{}END", self.indent_str()));
        Ok(())
    }

    // ── ForEachNode → FOR $array: ────────────────────────────────────────

    fn decompile_for_each(
        &mut self,
        node: &BlueprintNodeJson,
        lines: &mut Vec<String>,
    ) -> Result<(), DecompileError> {
        let step_id = self.assign_step_id(&node.id);
        let array = self.resolve_data_input(&node.id, "Array")?;
        lines.push(format!("{}{}: FOR {}", self.indent_str(), step_id, array));

        self.indent += 1;
        self.id_gen.push_scope(&step_id);
        let mut body_next = self.follow_exec(&node.id, "LoopBody");
        self.decompile_exec_chain(&mut body_next, lines)?;
        self.id_gen.pop_scope();
        self.indent -= 1;

        lines.push(format!("{}END", self.indent_str()));
        Ok(())
    }

    // ── ForLoopNode → FOR start TO end: ──────────────────────────────────

    fn decompile_for_loop(
        &mut self,
        node: &BlueprintNodeJson,
        lines: &mut Vec<String>,
    ) -> Result<(), DecompileError> {
        let step_id = self.assign_step_id(&node.id);
        let from = self.resolve_data_input(&node.id, "FirstIndex")?;
        let to = self.resolve_data_input(&node.id, "LastIndex")?;
        lines.push(format!(
            "{}{}: FOR {} TO {}",
            self.indent_str(),
            step_id,
            from,
            to
        ));

        self.indent += 1;
        self.id_gen.push_scope(&step_id);
        let mut body_next = self.follow_exec(&node.id, "LoopBody");
        self.decompile_exec_chain(&mut body_next, lines)?;
        self.id_gen.pop_scope();
        self.indent -= 1;

        lines.push(format!("{}END", self.indent_str()));
        Ok(())
    }

    // ── 数据输入解析 ───────────────────────────────────────────────────────

    /// 解析节点的所有 DataInput 引脚，生成 `--pin value ...` 字符串
    fn decompile_data_inputs(&self, node: &BlueprintNodeJson) -> Result<String, DecompileError> {
        let data_inputs: Vec<&NodePin> =
            node.pins.iter().filter(|p| p.kind == "DataInput").collect();

        let mut parts = Vec::new();
        for pin in &data_inputs {
            let value = self.resolve_data_input(&node.id, &pin.name)?;
            if value != "__unconnected__" {
                parts.push(format!("--{} {}", pin.name, value));
            }
        }

        Ok(parts.join(" "))
    }

    /// 解析单个 DataInput 引脚的值来源
    fn resolve_data_input(&self, node_id: &str, pin_name: &str) -> Result<String, DecompileError> {
        let key = (node_id.to_string(), pin_name.to_string());

        // 有数据连接？
        if let Some((src_node, src_pin)) = self.data_in.get(&key) {
            let src = self
                .node_map
                .get(src_node)
                .ok_or_else(|| DecompileError::new(format!("未找到源节点 {}", src_node)))?;

            return match src.node_type.as_str() {
                "StartNode" => {
                    // 连到 StartNode → input.pinName
                    Ok(format!("input.{}", src_pin))
                }
                "GetVarNode" => {
                    let has_name_connection = self
                        .data_in
                        .contains_key(&(src.id.clone(), "Name".to_string()));
                    if has_name_connection {
                        self.decompile_pure_inline(src, src_pin)
                    } else {
                        let var_name = self.get_pin_value(src, "Name").or_else(|_| {
                            src.properties
                                .get("variable_name")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_string)
                                .ok_or_else(|| {
                                    DecompileError::new(format!(
                                        "GetVarNode {} 的 Name 引脚无默认值",
                                        src.id
                                    ))
                                })
                        })?;
                        Ok(format!("${}", var_name))
                    }
                }
                "ForEachNode" => match src_pin.as_str() {
                    "Item" | "Element" => Ok("$item".to_string()),
                    "Index" => Ok("$index".to_string()),
                    _ => Ok(format!("{}.{}", src_node, src_pin)),
                },
                "ForLoopNode" if src_pin == "Index" => Ok("$index".to_string()),
                _ if self.is_pure_node(src) => {
                    // 连到 pure 节点 → 递归构建内联表达式
                    self.decompile_pure_inline(src, src_pin)
                }
                _ => {
                    // 连到 impure 节点 → step_id.pin
                    if let Some(step_id) = self.step_map.get(src_node) {
                        Ok(format!("{}.{}", step_id, src_pin))
                    } else {
                        // 可能是 GetVar/SetVar
                        if src.node_type == "SetVarNode" || src.node_type == "GetVarNode" {
                            let var_name = self.get_pin_value(src, "Name")?;
                            Ok(format!("${}", var_name))
                        } else {
                            Ok(format!("{}.{}", src_node, src_pin))
                        }
                    }
                }
            };
        }

        // 无连接：检查默认值
        let node = self
            .node_map
            .get(node_id)
            .ok_or_else(|| DecompileError::new(format!("未找到节点 {}", node_id)))?;
        if let Some(pin) = node
            .pins
            .iter()
            .find(|p| p.name == pin_name && p.kind == "DataInput")
        {
            if let Some(ref dv) = pin.default_value {
                return Ok(self.format_json_value(dv));
            }
        }

        Ok("__unconnected__".to_string())
    }

    /// 递归构建 pure 节点的内联表达式
    fn decompile_pure_inline(
        &self,
        node: &BlueprintNodeJson,
        output_pin: &str,
    ) -> Result<String, DecompileError> {
        let spec = pure_function_codec::by_node_type_and_output(&node.node_type, output_pin)
            .ok_or_else(|| {
                DecompileError::new(format!(
                    "pure 节点 {} 的输出引脚 {} 尚未加入脚本函数契约",
                    node.node_type, output_pin
                ))
            })?;
        let args = spec
            .input_pins
            .iter()
            .map(|pin| self.resolve_data_input(&node.id, pin))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(format!("{}({})", spec.name, args.join(", ")))
    }

    // ── 辅助方法 ─────────────────────────────────────────────────────────

    fn assign_step_id(&mut self, node_id: &str) -> String {
        if let Some(existing) = self.step_map.get(node_id) {
            return existing.clone();
        }
        let id = self
            .source_step(node_id)
            .filter(|step| Self::is_step_id(step))
            .or_else(|| {
                if Self::is_step_id(node_id) {
                    Some(node_id.to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| self.id_gen.next_impure_id());
        self.step_map.insert(node_id.to_string(), id.clone());
        id
    }

    fn source_step(&self, node_id: &str) -> Option<String> {
        self.node_map
            .get(node_id)?
            .properties
            .get("source_script")?
            .get("step")?
            .as_str()
            .map(str::to_string)
    }

    fn is_step_id(value: &str) -> bool {
        !value.is_empty()
            && value
                .split('.')
                .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
    }

    /// 判断节点是否为 pure（无 Exec 引脚）
    fn is_pure_node(&self, node: &BlueprintNodeJson) -> bool {
        node.pins
            .iter()
            .all(|p| p.kind != "ExecInput" && p.kind != "ExecOutput")
    }

    /// 获取引脚的字面量值（从 default_value 读取）
    fn get_pin_value(
        &self,
        node: &BlueprintNodeJson,
        pin_name: &str,
    ) -> Result<String, DecompileError> {
        if let Some(pin) = node.pins.iter().find(|p| p.name == pin_name) {
            if let Some(ref dv) = pin.default_value {
                return match dv {
                    serde_json::Value::String(s) => Ok(s.clone()),
                    other => Ok(other.to_string()),
                };
            }
        }
        Err(DecompileError::new(format!(
            "节点 {} 的引脚 {} 无默认值",
            node.id, pin_name
        )))
    }

    /// 格式化 JSON 值为文本表示
    fn format_json_value(&self, val: &serde_json::Value) -> String {
        match val {
            serde_json::Value::String(s) => format!("\"{}\"", s),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Null => "null".to_string(),
            other => other.to_string(),
        }
    }

    fn strip_node_suffix(&self, node_type: &str) -> String {
        if node_type.ends_with("Node") && node_type.len() > 4 {
            node_type[..node_type.len() - 4].to_string()
        } else {
            node_type.to_string()
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 公开接口
// ─────────────────────────────────────────────────────────────────────────────

/// 一步到位：BlueprintJson → 操作链文本
pub fn decompile_chain(bp: &BlueprintJson) -> Result<String, DecompileError> {
    ChainDecompiler::decompile(bp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::chain_compiler_v2::compile_chain_v2;

    #[test]
    fn test_decompile_simple() {
        let bp = compile_chain_v2(
            r#"
input
return result=add(3.0, 4.0)
"#,
        )
        .expect("compile failed");

        let text = decompile_chain(&bp).expect("decompile failed");
        assert!(text.contains("input"), "should contain input");
        assert!(text.contains("return"), "should contain return");
        assert!(
            text.contains("add(3.0, 4.0)"),
            "should use pure function syntax: {text}"
        );
    }

    #[test]
    fn test_decompile_with_inputs() {
        let bp = compile_chain_v2(
            r#"
input x:f64 y:f64
return result=add(input.x, input.y)
"#,
        )
        .expect("compile failed");

        let text = decompile_chain(&bp).expect("decompile failed");
        assert!(text.contains("input"), "should contain input");
        assert!(text.contains("return"), "should contain return");
    }

    #[test]
    fn test_roundtrip_compile_decompile_compile() {
        // Text → JSON₁ → Text' → JSON₂
        // 验证 JSON₁ 和 JSON₂ 的节点类型集合一致
        let original = r#"
input
return result=add(3.0, 4.0)
"#;
        let bp1 = compile_chain_v2(original).expect("first compile failed");
        let text = decompile_chain(&bp1).expect("decompile failed");
        let bp2 = compile_chain_v2(&text).expect("second compile failed");

        // 比较节点类型集合
        let types1: std::collections::HashSet<&str> =
            bp1.nodes.iter().map(|n| n.node_type.as_str()).collect();
        let types2: std::collections::HashSet<&str> =
            bp2.nodes.iter().map(|n| n.node_type.as_str()).collect();
        assert_eq!(types1, types2, "node type sets should match");
    }

    #[test]
    fn test_decompile_distinguishes_division_and_remainder() {
        let bp = compile_chain_v2(
            r#"
input dividend:num divisor:num
return quotient=div(input.dividend, input.divisor) remainder=mod(input.dividend, input.divisor)
"#,
        )
        .expect("compile failed");

        let text = decompile_chain(&bp).expect("decompile failed");
        assert!(
            text.contains("div(input.dividend, input.divisor)"),
            "{text}"
        );
        assert!(
            text.contains("mod(input.dividend, input.divisor)"),
            "{text}"
        );
    }

    #[test]
    fn test_roundtrip_preserves_variables_and_foreach_bindings() {
        let original = r#"
input items:Array[String] separator:String=","
$first = true
$result = ""
1: FOR input.items
    1.1: IF $first
        1.1.1: setvar result = $item
        1.1.2: setvar first = false
    ELSE
        1.2: setvar result = text_concat($result, text_concat(input.separator, $item))
    END
END
return result=$result
"#;
        let bp1 = compile_chain_v2(original).expect("first compile failed");
        let text = decompile_chain(&bp1).expect("decompile failed");

        assert!(
            text.contains("$first = true"),
            "missing first declaration: {text}"
        );
        assert!(
            text.contains("$result = \"\""),
            "missing result declaration: {text}"
        );
        assert!(
            text.contains("$item"),
            "missing foreach item binding: {text}"
        );
        assert!(
            !text.contains(".Item") && !text.contains(".Element"),
            "foreach binding should not decompile as step pin: {text}"
        );

        compile_chain_v2(&text).expect("decompiled text should compile");
    }
}
