//! 类型推导引擎
//!
//! 参考 UE 的 FEdGraphSchema::TryCreateConnection 和类型传播机制
//! 实现 Wildcard 引脚的类型推导和约束验证

use crate::error::Result;
use crate::workflow::blueprint::BlueprintWorkflow;
use crate::workflow::core::{Pin, PinContainerType, PinDirection, PinType};
use std::collections::HashMap;

/// 类型约束 - 管理通配符类型的解析状态
#[derive(Debug, Clone)]
pub struct TypeConstraint {
    pub wildcard_id: String,
    pub resolved_type: Option<String>,
    pub container: PinContainerType,
    /// 关联的引脚 (node_name, pin_name, direction)
    pub related_pins: Vec<(String, String, PinDirection)>,
}

impl TypeConstraint {
    pub fn new(wildcard_id: String, container: PinContainerType) -> Self {
        Self {
            wildcard_id,
            resolved_type: None,
            container,
            related_pins: Vec::new(),
        }
    }

    /// 添加关联引脚
    pub fn add_pin(&mut self, node: String, pin: String, direction: PinDirection) {
        self.related_pins.push((node, pin, direction));
    }

    /// 解析类型
    pub fn resolve(&mut self, type_name: String) -> Result<()> {
        if let Some(existing) = &self.resolved_type {
            if existing != &type_name {
                return Err(anyhow::anyhow!(
                    "Type mismatch: wildcard {} already resolved to {}, cannot resolve to {}",
                    self.wildcard_id,
                    existing,
                    type_name
                )
                .into());
            }
        } else {
            self.resolved_type = Some(type_name);
        }
        Ok(())
    }

    /// 是否已解析
    pub fn is_resolved(&self) -> bool {
        self.resolved_type.is_some()
    }

    /// 获取完整类型名（含容器）
    pub fn full_type_name(&self) -> Option<String> {
        self.resolved_type
            .as_ref()
            .map(|base| match self.container {
                PinContainerType::None => base.clone(),
                PinContainerType::Array => format!("Array<{}>", base),
                PinContainerType::Option => format!("Option<{}>", base),
            })
    }
}

/// 蓝图类型推导器
///
/// 参考 UE 的 UEdGraphSchema::TryCreateConnection 实现
pub struct BlueprintTypeInference {
    /// Wildcard ID → 类型约束
    constraints: HashMap<String, TypeConstraint>,

    pin_cache: HashMap<(String, String), Pin>,
}

impl Default for BlueprintTypeInference {
    fn default() -> Self {
        Self::new()
    }
}

impl BlueprintTypeInference {
    pub fn new() -> Self {
        Self {
            constraints: HashMap::new(),
            pin_cache: HashMap::new(),
        }
    }

    /// 从蓝图工作流初始化类型推导器
    pub fn from_workflow(_workflow: &BlueprintWorkflow) -> Result<Self> {
        let inference = Self::new();

        // Future: 需要访问workflow.nodes字段
        // 暂时返回空的推导器

        Ok(inference)
    }

    pub fn register_pin(&mut self, node_name: String, pin: Pin) {
        let key = (node_name.clone(), pin.name.clone());

        // 处理通配符引脚
        if let PinType::Wildcard { id, container, .. } = &pin.pin_type {
            let constraint = self
                .constraints
                .entry(id.clone())
                .or_insert_with(|| TypeConstraint::new(id.clone(), *container));

            constraint.add_pin(node_name.clone(), pin.name.clone(), pin.direction);
        }

        self.pin_cache.insert(key, pin);
    }

    /// 获取引脚
    pub fn get_pin(&self, node: &str, pin_name: &str) -> Option<&Pin> {
        self.pin_cache
            .get(&(node.to_string(), pin_name.to_string()))
    }

    /// 尝试连接两个引脚（触发类型推导）
    ///
    /// 参考 UE: UEdGraphSchema::TryCreateConnection
    pub fn try_connect(
        &mut self,
        from_node: &str,
        from_pin: &str,
        to_node: &str,
        to_pin: &str,
    ) -> Result<()> {
        let from = self
            .get_pin(from_node, from_pin)
            .ok_or_else(|| anyhow::anyhow!("Source pin not found"))?
            .clone();
        let to = self
            .get_pin(to_node, to_pin)
            .ok_or_else(|| anyhow::anyhow!("Target pin not found"))?
            .clone();

        // 方向检查
        if from.direction == to.direction {
            return Err(anyhow::anyhow!("Cannot connect pins with same direction").into());
        }

        // 执行引脚只能连接执行引脚
        if from.pin_type.is_exec() != to.pin_type.is_exec() {
            return Err(anyhow::anyhow!("Cannot connect exec pin to data pin").into());
        }

        // 执行引脚不需要类型推导
        if from.pin_type.is_exec() {
            return Ok(());
        }

        // 数据引脚类型推导
        self.propagate_type(&from, &to)?;

        Ok(())
    }

    /// 从容器类型中提取元素类型
    ///
    /// 例如："Array<String>" -> "String", "Option<i32>" -> "i32"
    fn extract_element_type(container_type: &str, container: &PinContainerType) -> Option<String> {
        match container {
            PinContainerType::Array => {
                // Array<T> -> T
                if let Some(inner) = container_type
                    .strip_prefix("Array<")
                    .and_then(|s| s.strip_suffix('>'))
                {
                    return Some(inner.to_string());
                }
            }
            PinContainerType::Option => {
                // Option<T> -> T
                if let Some(inner) = container_type
                    .strip_prefix("Option<")
                    .and_then(|s| s.strip_suffix('>'))
                {
                    return Some(inner.to_string());
                }
            }
            PinContainerType::None => {
                return Some(container_type.to_string());
            }
        }
        None
    }

    /// 类型传播核心逻辑
    ///
    /// 参考 UE 的类型传播规则：
    /// 1. 具体类型 → Wildcard: 解析 Wildcard
    /// 2. Wildcard → 具体类型: 解析 Wildcard
    /// 3. Wildcard → Wildcard: 必须同 ID，共享约束
    /// 4. 具体类型 → 具体类型: 类型必须兼容
    fn propagate_type(&mut self, from: &Pin, to: &Pin) -> Result<()> {
        match (&from.pin_type, &to.pin_type) {
            // 1. 具体类型 → Wildcard
            (PinType::Data(concrete), PinType::Wildcard { id, container, .. }) => {
                // 从具体类型中提取元素类型
                if let Some(element_type) = Self::extract_element_type(concrete, container) {
                    self.resolve_wildcard(id, &element_type, container)?;
                } else {
                    return Err(anyhow::anyhow!(
                        "Cannot extract element type from {} for container {:?}",
                        concrete,
                        container
                    )
                    .into());
                }
            }

            // 2. Wildcard → 具体类型
            (PinType::Wildcard { id, container, .. }, PinType::Data(concrete)) => {
                // 从具体类型中提取元素类型
                if let Some(element_type) = Self::extract_element_type(concrete, container) {
                    self.resolve_wildcard(id, &element_type, container)?;
                } else {
                    return Err(anyhow::anyhow!(
                        "Cannot extract element type from {} for container {:?}",
                        concrete,
                        container
                    )
                    .into());
                }
            }

            // 3. Wildcard → Wildcard（同ID）
            (
                PinType::Wildcard {
                    id: id1,
                    container: c1,
                    ..
                },
                PinType::Wildcard {
                    id: id2,
                    container: c2,
                    ..
                },
            ) => {
                if id1 != id2 {
                    return Err(anyhow::anyhow!("Cannot connect different wildcard types").into());
                }
                if c1 != c2 {
                    return Err(anyhow::anyhow!("Container type mismatch").into());
                }
                // 同ID的通配符会共享约束，不需要额外处理
            }

            // 4. 具体类型 → 具体类型
            (PinType::Data(type1), PinType::Data(type2)) => {
                if type1 != type2 {
                    // Future: 支持类型转换/兼容性检查
                    return Err(anyhow::anyhow!("Type mismatch: {} vs {}", type1, type2).into());
                }
            }

            _ => {}
        }

        Ok(())
    }

    /// 解析通配符类型
    ///
    /// 参考 UE: PropagateTypeToRelatedPins
    fn resolve_wildcard(
        &mut self,
        wildcard_id: &str,
        concrete_type: &str,
        container: &PinContainerType,
    ) -> Result<()> {
        // 获取或创建约束
        let constraint = self
            .constraints
            .entry(wildcard_id.to_string())
            .or_insert_with(|| TypeConstraint::new(wildcard_id.to_string(), *container));

        // 容器类型必须匹配
        if &constraint.container != container {
            return Err(anyhow::anyhow!(
                "Container mismatch for wildcard {}: expected {:?}, got {:?}",
                wildcard_id,
                constraint.container,
                container
            )
            .into());
        }

        // 解析类型（会自动检查冲突）
        constraint.resolve(concrete_type.to_string())?;

        // 传播到所有相关引脚
        self.update_related_pins(wildcard_id, concrete_type)?;

        Ok(())
    }

    /// 更新同一通配符ID的所有引脚
    fn update_related_pins(&mut self, wildcard_id: &str, concrete_type: &str) -> Result<()> {
        let constraint = self
            .constraints
            .get(wildcard_id)
            .ok_or_else(|| anyhow::anyhow!("Wildcard constraint not found"))?;

        // 复制引脚列表（避免借用冲突）
        let pins_to_update = constraint.related_pins.clone();

        for (node, pin_name, _direction) in pins_to_update {
            let key = (node.clone(), pin_name.clone());

            if let Some(pin) = self.pin_cache.get_mut(&key) {
                if let PinType::Wildcard { resolved_type, .. } = &mut pin.pin_type {
                    *resolved_type = Some(concrete_type.to_string());
                }
            }
        }

        Ok(())
    }

    /// 验证所有通配符都已解析
    ///
    /// 参考 UE: FKismetCompiler::ValidateBlueprint
    pub fn validate(&self) -> Result<()> {
        let mut unresolved = Vec::new();

        for (id, constraint) in &self.constraints {
            if !constraint.is_resolved() {
                unresolved.push(id.clone());
            }
        }

        if !unresolved.is_empty() {
            return Err(
                anyhow::anyhow!("Unresolved wildcard types: {}", unresolved.join(", ")).into(),
            );
        }

        Ok(())
    }

    /// 获取约束信息（用于调试）
    pub fn get_constraint(&self, wildcard_id: &str) -> Option<&TypeConstraint> {
        self.constraints.get(wildcard_id)
    }

    /// 获取所有约束
    pub fn all_constraints(&self) -> &HashMap<String, TypeConstraint> {
        &self.constraints
    }

    /// 清除所有类型推导结果（重置）
    pub fn reset(&mut self) {
        for constraint in self.constraints.values_mut() {
            constraint.resolved_type = None;
        }

        for pin in self.pin_cache.values_mut() {
            if let PinType::Wildcard { resolved_type, .. } = &mut pin.pin_type {
                *resolved_type = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wildcard_resolution() {
        let mut inference = BlueprintTypeInference::new();

        // 注册通配符引脚
        let wildcard_in = Pin::wildcard_in("Input", "T", PinContainerType::None);
        let wildcard_out = Pin::wildcard_out("Output", "T", PinContainerType::None);

        inference.register_pin("Node1".to_string(), wildcard_in);
        inference.register_pin("Node1".to_string(), wildcard_out);

        // 注册具体类型引脚
        let concrete = Pin::data_out("Value", "String");
        inference.register_pin("Node2".to_string(), concrete);

        // 连接：具体类型 → 通配符
        inference
            .try_connect("Node2", "Value", "Node1", "Input")
            .unwrap();

        // 验证通配符已解析
        let constraint = inference.get_constraint("T").unwrap();
        assert_eq!(constraint.resolved_type, Some("String".to_string()));

        // 验证两个引脚都已解析
        let input = inference.get_pin("Node1", "Input").unwrap();
        let output = inference.get_pin("Node1", "Output").unwrap();

        assert_eq!(input.pin_type.type_name(), Some("String"));
        assert_eq!(output.pin_type.type_name(), Some("String"));
    }

    #[test]
    fn test_array_wildcard() {
        let mut inference = BlueprintTypeInference::new();

        // Array<T> 引脚
        let array_in = Pin::wildcard_in("Array", "T", PinContainerType::Array);
        inference.register_pin("ForEach".to_string(), array_in);

        // 具体数组类型
        let concrete_array = Pin {
            name: "Items".to_string(),
            direction: PinDirection::Output,
            pin_type: PinType::Data("Array<String>".to_string()),
        };
        inference.register_pin("Source".to_string(), concrete_array);

        // 连接后推导元素类型
        // Future: 需要容器层级推导
    }
}
