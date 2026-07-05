//! 数据操作节点模块
//!
//! 组织节点库的分层结构：
//! - placeholder: 占位节点（框架验证）
//! - variable: 变量节点（基础类型存储）
//! - container: 容器节点（对象操作）
//! - array: 数组节点（数组操作）
//! - convert: 类型转换节点
//! - string: 字符串节点
//! - logic: 逻辑比较节点
//! - math: 数学运算节点
//! - divmod: 除法和取模节点

pub mod array;
pub mod convert;
pub mod divmod;
pub mod logic;
pub mod math;
pub mod placeholder;
pub mod string;
pub mod variable;

// 重新导出常用节点
pub use array::{
    ArrayConcatNode, ArrayLengthNode, ForEachNode, GetArrayElementNode, GetFirstNode, GetLastNode,
    MakeArrayNode, SetArrayElementNode,
};
pub use convert::{ToArrayNode, ToBoolNode, ToFloatNode, ToIntNode, ToStringNode};
pub use divmod::DivModNode;
pub use logic::{
    AndNode, EqualNode, GreaterNode, GreaterOrEqualNode, LessNode, LessOrEqualNode, NotEqualNode,
    NotNode, OrNode, SelectNode, XorNode,
};
pub use math::{AddNode, MultiplyNode, NegNode, PowNode};
pub use placeholder::{DebugPrintNode, NoOpNode};
pub use string::{
    ContainsNode, IndexOfNode, RegexMatchNode, ReplaceNode, StringAppendNode, StringLengthNode,
    SubstringNode, ToLowerNode, ToUpperNode, TrimNode,
};
pub use variable::{GetVarNode, SetVarNode};
