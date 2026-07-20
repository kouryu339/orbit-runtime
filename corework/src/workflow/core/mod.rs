//! 核心类型定义模块
//!
//! 包含 Pin、DataValue、Connection 等基础类型

pub mod builtin_types;
pub mod connection;
pub mod data_value;
pub mod node_output;
pub mod pin;
pub mod pin_cache_mapping;

// 重新导出
pub use builtin_types::{is_builtin_type, types_compatible, KeyValuePair, BUILTIN_TYPES};
pub use connection::Connection;
pub use data_value::DataValue;
pub use node_output::{LoopIteration, NodeOutput};
pub use pin::{Pin, PinContainerType, PinDirection, PinType};
pub use pin_cache_mapping::{
    build_output_object, read_input_map, write_output_map, PinCacheMapping,
};
