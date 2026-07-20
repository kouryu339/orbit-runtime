//! WorkflowsModule 集成测试（单函数顺序执行，避免全局 World 资源冲突）

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use corework::workflow::blueprint_json::{
        BlueprintJson, BlueprintNodeJson, ConnectionJson, NodePin, NodePosition, NodeSize,
    };

    use crate::workflow::workflows::executor::WorkflowsModule;

    // -------------------------------------------------------------------------
    // 辅助：构造一个最简节点（exec in + then out）
    // -------------------------------------------------------------------------
    fn simple_node(id: &str, node_type: &str) -> BlueprintNodeJson {
        BlueprintNodeJson {
            id: id.to_string(),
            node_type: node_type.to_string(),
            position: NodePosition { x: 0.0, y: 0.0 },
            size: NodeSize::from_pins(&[]),
            pins: vec![
                NodePin {
                    name: "exec".to_string(),
                    kind: "ExecInput".to_string(),
                    data_type: String::new(),
                    description: String::new(),
                    default_value: None,
                    resolved_type: None,
                    split_config: None,
                },
                NodePin {
                    name: "then".to_string(),
                    kind: "ExecOutput".to_string(),
                    data_type: String::new(),
                    description: String::new(),
                    default_value: None,
                    resolved_type: None,
                    split_config: None,
                },
            ],
            properties: HashMap::new(),
            display_name: None,
            comment: None,
        }
    }

    fn exec_connection(id: &str, from: &str, to: &str) -> ConnectionJson {
        ConnectionJson {
            id: id.to_string(),
            source_node: from.to_string(),
            source_pin: "then".to_string(),
            target_node: to.to_string(),
            target_pin: "exec".to_string(),
            connection_type: "Exec".to_string(),
        }
    }

    // =========================================================================
    // 全场景大测试 — 一次性顺序执行，避免全局 World 并发冲突
    // =========================================================================

    #[tokio::test]
    async fn test_workflows_module_all() {
        let m = WorkflowsModule::new().expect("WorkflowsModule::new() 应该成功");

        // ── 1. 初始状态 ──────────────────────────────────────────────────────
        assert!(m.list().unwrap().is_empty(), "初始蓝图列表应为空");
        assert!(m.draft_get().unwrap().is_none(), "未 draft_new 时草稿应为 None");

        // ── 2. draft_new 创建命名草稿 ────────────────────────────────────────
        m.draft_new("我的工作流").unwrap();
        let draft = m.draft_get().unwrap().expect("draft_new 后应有草稿");
        assert_eq!(draft.metadata.name, "我的工作流");
        assert!(draft.nodes.is_empty());
        assert!(draft.connections.is_empty());

        // ── 3. 逐个追加节点（模拟 WfAppendNode JSON 操作）──────────────────
        for (i, t) in ["NodeA", "NodeB", "NodeC"].iter().enumerate() {
            let mut d = m.draft_get().unwrap().unwrap();
            d.add_node(simple_node(&format!("n{}", i + 1), t));
            m.draft_put(&d).unwrap();
        }
        let draft = m.draft_get().unwrap().unwrap();
        assert_eq!(draft.nodes.len(), 3);
        assert_eq!(draft.nodes[0].node_type, "NodeA");
        assert_eq!(draft.nodes[2].node_type, "NodeC");

        // ── 4. 添加连线（纯 JSON，无 Workflow 实例化）───────────────────────
        let mut draft = m.draft_get().unwrap().unwrap();
        draft.add_connection(exec_connection("c1", "n1", "n2"));
        draft.add_connection(exec_connection("c2", "n2", "n3"));
        m.draft_put(&draft).unwrap();

        let draft = m.draft_get().unwrap().unwrap();
        assert_eq!(draft.connections.len(), 2);
        assert_eq!(draft.connections[0].source_node, "n1");
        assert_eq!(draft.connections[1].target_node, "n3");

        // ── 5. 节点属性写入 ──────────────────────────────────────────────────
        let mut draft = m.draft_get().unwrap().unwrap();
        if let Some(n) = draft.find_node_mut("n2") {
            n.properties
                .insert("format".to_string(), serde_json::json!("mp3"));
        }
        m.draft_put(&draft).unwrap();

        let saved = m.draft_get().unwrap().unwrap();
        assert_eq!(
            saved
                .find_node("n2")
                .unwrap()
                .properties
                .get("format")
                .and_then(|v| v.as_str()),
            Some("mp3")
        );

        // ── 6. 删除节点及其连线 ──────────────────────────────────────────────
        let mut draft = m.draft_get().unwrap().unwrap();
        draft.remove_node("n1");
        draft.remove_connections_for_node("n1");
        m.draft_put(&draft).unwrap();

        let draft = m.draft_get().unwrap().unwrap();
        assert_eq!(draft.nodes.len(), 2, "删除 n1 后剩 n2/n3");
        assert_eq!(draft.connections.len(), 1, "c1(n1→n2) 被删，c2(n2→n3) 保留");
        assert_eq!(draft.connections[0].id, "c2");

        // ── 7. JSON 往返序列化 ───────────────────────────────────────────────
        let json = serde_json::to_string(&m.draft_get().unwrap().unwrap()).unwrap();
        let restored: BlueprintJson = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.nodes.len(), 2);
        assert_eq!(restored.connections.len(), 1);

        // ── 8. draft_new 重置所有状态 ────────────────────────────────────────
        m.draft_new("新草稿").unwrap();
        let fresh = m.draft_get().unwrap().unwrap();
        assert_eq!(fresh.metadata.name, "新草稿");
        assert!(fresh.nodes.is_empty(), "重置后节点列表应为空");
        assert!(fresh.connections.is_empty(), "重置后连接列表应为空");
    }
}
