//! 层级 ID 生成器
//!
//!
//! - **StartNode** ID = `start`
//! - **EndNode** ID = `end`

use std::collections::HashMap;

/// 层级 ID 生成器
///
/// 顶层生成 `1`, `2`, `3`...
/// push_scope("1") 后生成 `1.1`, `1.2`...
/// push_scope("1.1") 后生成 `1.1.1`, `1.1.2`...
pub struct HierarchicalIdGen {
    /// scope 栈：每层是 (prefix, next_counter)
    /// 顶层 prefix 为空字符串
    scope_stack: Vec<(String, usize)>,
    /// Pure 节点计数器：(consumer_id, pin) → seq
    pure_counters: HashMap<(String, String), usize>,
}

impl Default for HierarchicalIdGen {
    fn default() -> Self {
        Self::new()
    }
}

impl HierarchicalIdGen {
    pub fn new() -> Self {
        Self {
            scope_stack: vec![("".to_string(), 0)],
            pure_counters: HashMap::new(),
        }
    }

    /// 生成下一个 impure 节点 ID
    ///
    /// 顶层返回 `"1"`, `"2"`, ...
    /// scope "1" 内返回 `"1.1"`, `"1.2"`, ...
    pub fn next_impure_id(&mut self) -> String {
        let (prefix, counter) = self.scope_stack.last_mut().expect("scope stack empty");
        *counter += 1;
        if prefix.is_empty() {
            format!("{}", counter)
        } else {
            format!("{}.{}", prefix, counter)
        }
    }

    /// 进入控制流嵌套 scope
    ///
    /// `parent_id` 是刚分配的 impure ID（如 `"1"`），
    pub fn push_scope(&mut self, parent_id: &str) {
        self.scope_stack.push((parent_id.to_string(), 0));
    }

    /// 离开控制流嵌套 scope
    pub fn pop_scope(&mut self) {
        if self.scope_stack.len() > 1 {
            self.scope_stack.pop();
        }
    }

    /// 生成 pure 节点 ID
    ///
    /// 格式：`{consumer_id}.{consumer_pin}~{seq}`
    /// 例如：`"2.value~1"`, `"2.value~2"`
    pub fn next_pure_id(&mut self, consumer_id: &str, consumer_pin: &str) -> String {
        let key = (consumer_id.to_string(), consumer_pin.to_string());
        let seq = self.pure_counters.entry(key).or_insert(0);
        *seq += 1;
        format!("{}.{}~{}", consumer_id, consumer_pin, seq)
    }

    /// StartNode 固定 ID
    pub fn start_id(&self) -> String {
        "start".to_string()
    }

    /// EndNode 固定 ID
    pub fn end_id(&self) -> String {
        "end".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_top_level_impure_ids() {
        let mut gen = HierarchicalIdGen::new();
        assert_eq!(gen.next_impure_id(), "1");
        assert_eq!(gen.next_impure_id(), "2");
        assert_eq!(gen.next_impure_id(), "3");
    }

    #[test]
    fn test_nested_scope() {
        let mut gen = HierarchicalIdGen::new();
        let id1 = gen.next_impure_id(); // "1"
        assert_eq!(id1, "1");

        gen.push_scope(&id1);
        assert_eq!(gen.next_impure_id(), "1.1");
        assert_eq!(gen.next_impure_id(), "1.2");
        gen.pop_scope();

        assert_eq!(gen.next_impure_id(), "2");
    }

    #[test]
    fn test_deeply_nested_scope() {
        let mut gen = HierarchicalIdGen::new();
        let id1 = gen.next_impure_id(); // "1"
        gen.push_scope(&id1);
        let id11 = gen.next_impure_id(); // "1.1"
        assert_eq!(id11, "1.1");

        gen.push_scope(&id11);
        assert_eq!(gen.next_impure_id(), "1.1.1");
        assert_eq!(gen.next_impure_id(), "1.1.2");
        gen.pop_scope();

        assert_eq!(gen.next_impure_id(), "1.2");
        gen.pop_scope();

        assert_eq!(gen.next_impure_id(), "2");
    }

    #[test]
    fn test_pure_id_generation() {
        let mut gen = HierarchicalIdGen::new();
        assert_eq!(gen.next_pure_id("2", "value"), "2.value~1");
        assert_eq!(gen.next_pure_id("2", "value"), "2.value~2");
        assert_eq!(gen.next_pure_id("2", "other"), "2.other~1");
        assert_eq!(gen.next_pure_id("3", "value"), "3.value~1");
    }

    #[test]
    fn test_start_end_ids() {
        let gen = HierarchicalIdGen::new();
        assert_eq!(gen.start_id(), "start");
        assert_eq!(gen.end_id(), "end");
    }
}
