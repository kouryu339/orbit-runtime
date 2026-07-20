//!
//! 从 blueprint.rs 提取的 Pin 相关类型

use serde::{Deserialize, Serialize};

/// Pin Direction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PinDirection {
    Input,
    Output,
}

/// 容器类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PinContainerType {
    None,   // 单个值
    Array,  // Vec<T>
    Option, // Option<T>
}

impl Default for PinContainerType {
    fn default() -> Self {
        Self::None
    }
}

/// Pin Type (运行时使用)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PinType {
    Exec,         // Execution flow control
    Data(String), // Data type (registered in DataTypeRegistry)
    /// 通配符类型 - 类似 UE 的 Wildcard Pin
    /// 连接时自动推导类型，同一 id 的 wildcard 必须类型一致
    Wildcard {
        id: String,                    // 关联 ID，相同 ID 的引脚类型必须一致
        container: PinContainerType,   // 容器类型
        resolved_type: Option<String>, // 连接后解析的具体类型
    },
}

impl PinType {
    pub fn is_exec(&self) -> bool {
        matches!(self, PinType::Exec)
    }

    pub fn is_data(&self) -> bool {
        matches!(self, PinType::Data(_))
    }

    pub fn is_wildcard(&self) -> bool {
        matches!(self, PinType::Wildcard { .. })
    }

    pub fn type_name(&self) -> Option<&str> {
        match self {
            PinType::Data(name) => Some(name.as_str()),
            PinType::Wildcard {
                resolved_type: Some(name),
                ..
            } => Some(name.as_str()),
            _ => None,
        }
    }

    /// 获取完整类型描述（包含容器）
    pub fn full_type_name(&self) -> Option<String> {
        match self {
            PinType::Data(name) => Some(name.clone()),
            PinType::Wildcard {
                resolved_type,
                container,
                ..
            } => resolved_type.as_ref().map(|base| match container {
                PinContainerType::None => base.clone(),
                PinContainerType::Array => format!("Vec<{}>", base),
                PinContainerType::Option => format!("Option<{}>", base),
            }),
            _ => None,
        }
    }
}

/// Pin definition (运行时使用)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pin {
    pub name: String,
    pub direction: PinDirection,
    pub pin_type: PinType,
}

impl Pin {
    pub fn exec_in(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            direction: PinDirection::Input,
            pin_type: PinType::Exec,
        }
    }

    pub fn exec_out(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            direction: PinDirection::Output,
            pin_type: PinType::Exec,
        }
    }

    pub fn data_in(name: impl Into<String>, type_name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            direction: PinDirection::Input,
            pin_type: PinType::Data(type_name.into()),
        }
    }

    pub fn data_out(name: impl Into<String>, type_name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            direction: PinDirection::Output,
            pin_type: PinType::Data(type_name.into()),
        }
    }

    pub fn wildcard_in(
        name: impl Into<String>,
        id: impl Into<String>,
        container: PinContainerType,
    ) -> Self {
        Self {
            name: name.into(),
            direction: PinDirection::Input,
            pin_type: PinType::Wildcard {
                id: id.into(),
                container,
                resolved_type: None,
            },
        }
    }

    pub fn wildcard_out(
        name: impl Into<String>,
        id: impl Into<String>,
        container: PinContainerType,
    ) -> Self {
        Self {
            name: name.into(),
            direction: PinDirection::Output,
            pin_type: PinType::Wildcard {
                id: id.into(),
                container,
                resolved_type: None,
            },
        }
    }

    pub fn key(&self) -> String {
        format!("{}_{:?}", self.name, self.direction)
    }
}
