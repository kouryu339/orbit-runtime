use corework::cache::{Cache, InMemoryCache};
use corework::error::Result;
use corework::event::{BaseEvent, EventBus, EventHandler};
use corework::event_line::EventLinePolicy;
use corework::execution_unit::{AccessMode, ExecutionUnit, ResourceRegistry, UnitType};
use corework::scoped_cache::ScopedCache;
use corework::statemachine::{FnState, StateMachine};
use corework::world::FrameworkState;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct CountingHandler {
    name: String,
    count: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl EventHandler for CountingHandler {
    async fn handle(&self, _event: &BaseEvent) -> Result<()> {
        self.count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[tokio::test]
async fn scoped_cache_isolates_keys_and_supports_snapshot_restore() -> Result<()> {
    let base_cache: Arc<dyn Cache> = Arc::new(InMemoryCache::new());
    let scope_a = ScopedCache::new(base_cache.clone(), "scope-a");
    let scope_b = ScopedCache::new(base_cache.clone(), "scope-b");

    scope_a.set_raw("shared", json!("from-a"), None).await?;
    scope_b.set_raw("shared", json!("from-b"), None).await?;
    scope_a.set_raw("only-a", json!({"count": 1}), None).await?;

    assert_eq!(scope_a.get_raw("shared").await?, Some(json!("from-a")));
    assert_eq!(scope_b.get_raw("shared").await?, Some(json!("from-b")));
    assert_eq!(base_cache.get_raw("shared").await?, None);
    assert_eq!(
        base_cache.get_raw("scope-a:shared").await?,
        Some(json!("from-a"))
    );

    let snapshot = scope_a.dump().await;
    assert_eq!(snapshot.get("shared"), Some(&json!("from-a")));
    assert_eq!(snapshot.get("only-a"), Some(&json!({"count": 1})));

    let scope_restored = ScopedCache::new(base_cache.clone(), "scope-restored");
    scope_restored.restore(snapshot).await?;

    assert_eq!(
        scope_restored.get_raw("shared").await?,
        Some(json!("from-a"))
    );
    assert_eq!(
        scope_restored.get_raw("only-a").await?,
        Some(json!({"count": 1}))
    );
    assert_eq!(scope_restored.stats().tracked_keys_count, 2);

    scope_a.cleanup().await?;
    assert_eq!(scope_a.get_raw("shared").await?, None);
    assert_eq!(scope_b.get_raw("shared").await?, Some(json!("from-b")));

    Ok(())
}

#[test]
fn resource_registry_enforces_grant_modes() -> Result<()> {
    let registry = ResourceRegistry::new();

    registry.declare("resource:mode", "owner", AccessMode::ReadWrite)?;
    assert!(registry.check_access("resource:mode", "owner", AccessMode::Read));
    assert!(registry.check_access("resource:mode", "owner", AccessMode::ReadWrite));

    registry.grant_access("resource:mode", "owner", "reader", AccessMode::Read)?;
    assert!(registry.check_access("resource:mode", "reader", AccessMode::Read));
    assert!(!registry.check_access("resource:mode", "reader", AccessMode::ReadWrite));

    registry.grant_access("resource:mode", "owner", "writer", AccessMode::ReadWrite)?;
    assert!(registry.check_access("resource:mode", "writer", AccessMode::Read));
    assert!(registry.check_access("resource:mode", "writer", AccessMode::ReadWrite));

    let owner_grant =
        registry.grant_access("resource:mode", "owner", "next_owner", AccessMode::Owner);
    assert!(owner_grant.is_err());

    Ok(())
}

#[tokio::test]
async fn execution_unit_child_local_cache_is_isolated() -> Result<()> {
    let framework = FrameworkState::initialize()?;
    let parent = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework.clone()));
    let child = ExecutionUnit::new_child(UnitType::StateMachine, &parent)?;
    let sibling = ExecutionUnit::new_root(UnitType::Module, framework);

    assert_ne!(parent.id(), child.id());
    assert_eq!(parent.scope_id(), child.scope_id());
    assert_ne!(parent.scope_id(), sibling.scope_id());

    parent
        .cache()
        .set_raw("turn", json!("parent-value"), None)
        .await?;

    assert_eq!(child.cache().get_raw("turn").await?, None);
    assert_eq!(sibling.cache().get_raw("turn").await?, None);

    child.cache().set_raw("child-key", json!(42), None).await?;
    assert_eq!(parent.cache().get_raw("child-key").await?, None);
    assert_eq!(sibling.cache().get_raw("child-key").await?, None);

    Ok(())
}

#[test]
fn execution_unit_records_real_parent_and_nested_lineage() -> Result<()> {
    let framework = FrameworkState::initialize()?;
    let root = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework));
    let child = Arc::new(ExecutionUnit::new_child(UnitType::Module, &root)?);
    let grandchild = ExecutionUnit::new_child(UnitType::StateMachine, &child)?;

    assert_eq!(child.parent_id(), Some(root.id()));
    assert_eq!(
        child.parent().as_deref().map(ExecutionUnit::id),
        Some(root.id())
    );
    assert_eq!(child.depth(), 1);
    assert!(child.is_descendant_of(&root));

    assert_eq!(
        grandchild.ancestor_ids(),
        &[root.id().to_string(), child.id().to_string()]
    );
    assert_eq!(grandchild.parent_id(), Some(child.id()));
    assert_eq!(grandchild.depth(), 2);
    assert!(grandchild.is_descendant_of(&root));
    assert!(grandchild.is_descendant_of(&child));

    assert!(!root.is_descendant_of(&child));
    assert_eq!(root.parent_id(), None);

    Ok(())
}

#[test]
fn execution_unit_shared_components_follow_real_parent_hierarchy() -> Result<()> {
    #[derive(Debug, PartialEq, Eq)]
    struct ConversationSharedComponent {
        id: String,
    }

    let framework = FrameworkState::initialize()?;
    let root = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework.clone()));
    let child = Arc::new(ExecutionUnit::new_child(UnitType::Module, &root)?);
    let grandchild = Arc::new(ExecutionUnit::new_child(UnitType::StateMachine, &child)?);
    let sibling_root = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework));
    let shared_component = Arc::new(ConversationSharedComponent {
        id: "conversation-a".to_string(),
    });
    let child_shared_component = Arc::new(ConversationSharedComponent {
        id: "conversation-child".to_string(),
    });

    root.attach_shared_component(Arc::clone(&shared_component))?;

    assert!(Arc::ptr_eq(
        &grandchild
            .resolve_shared_component::<ConversationSharedComponent>()
            .unwrap(),
        &shared_component
    ));
    assert!(Arc::ptr_eq(
        &grandchild
            .create_context()
            .resolve_shared_component::<ConversationSharedComponent>()?,
        &shared_component
    ));
    child.attach_shared_component(Arc::clone(&child_shared_component))?;
    assert!(Arc::ptr_eq(
        &grandchild
            .resolve_shared_component::<ConversationSharedComponent>()
            .unwrap(),
        &child_shared_component
    ));
    assert!(Arc::ptr_eq(
        &child
            .create_context()
            .resolve_shared_component::<ConversationSharedComponent>()?,
        &child_shared_component
    ));
    assert!(Arc::ptr_eq(
        &root
            .resolve_shared_component::<ConversationSharedComponent>()
            .unwrap(),
        &shared_component
    ));
    assert!(sibling_root
        .resolve_shared_component::<ConversationSharedComponent>()
        .is_none());
    assert!(root
        .attach_shared_component(Arc::clone(&shared_component))
        .is_err());

    Ok(())
}

#[tokio::test]
async fn hierarchical_cache_is_explicit_and_subtree_isolated() -> Result<()> {
    let framework = FrameworkState::initialize()?;
    let root = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework.clone()));
    let child = Arc::new(ExecutionUnit::new_child(UnitType::Module, &root)?);
    let grandchild = ExecutionUnit::new_child(UnitType::StateMachine, &child)?;
    let sibling_root = ExecutionUnit::new_root(UnitType::Module, framework);

    root.cache()
        .set_raw("local", json!("root-only"), None)
        .await?;
    assert_eq!(
        root.hierarchical_cache().local().get_raw("local").await?,
        Some(json!("root-only"))
    );
    assert_eq!(child.cache().get_raw("local").await?, None);

    root.hierarchical_cache()
        .set_subtree_raw("ledger", json!("root-ledger"), None)
        .await?;
    assert_eq!(
        grandchild
            .hierarchical_cache()
            .get_subtree_raw("ledger")
            .await?,
        Some(json!("root-ledger"))
    );
    assert_eq!(
        sibling_root
            .hierarchical_cache()
            .get_subtree_raw("ledger")
            .await?,
        None
    );

    child
        .hierarchical_cache()
        .set_subtree_raw("ledger", json!("child-ledger"), None)
        .await?;
    assert_eq!(
        grandchild
            .hierarchical_cache()
            .get_subtree_raw("ledger")
            .await?,
        Some(json!("child-ledger"))
    );

    root.hierarchical_cache()
        .set_global_raw("hierarchy-global", json!(42), None)
        .await?;
    assert_eq!(
        sibling_root
            .hierarchical_cache()
            .get_global_raw("hierarchy-global")
            .await?,
        Some(json!(42))
    );

    Ok(())
}

#[tokio::test]
async fn event_line_allows_subtree_publish_with_owner_only_subscription() -> Result<()> {
    let framework = FrameworkState::initialize()?;
    let root = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework));
    let child = Arc::new(ExecutionUnit::new_child(UnitType::StateMachine, &root)?);
    let line = root.create_event_line(
        "conversation",
        EventLinePolicy::subtree_publish_owner_subscribe(),
    )?;
    let child_line = child.event_line("conversation")?;
    let count = Arc::new(AtomicUsize::new(0));

    line.subscribe(
        "agent:event".to_string(),
        Arc::new(CountingHandler {
            name: "conversation-owner".to_string(),
            count: count.clone(),
        }),
    )
    .await?;

    assert!(child_line
        .subscribe(
            "agent:event".to_string(),
            Arc::new(CountingHandler {
                name: "conversation-child".to_string(),
                count: Arc::new(AtomicUsize::new(0)),
            }),
        )
        .await
        .is_err());

    child_line
        .publish(BaseEvent::new("agent:event", json!({"from": "child"})))
        .await?;
    assert_eq!(count.load(Ordering::SeqCst), 1);

    Ok(())
}

#[tokio::test]
async fn default_event_line_is_inherited_as_the_subtree_event_bus() -> Result<()> {
    let framework = FrameworkState::initialize()?;
    let root = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework));
    root.create_event_line(
        "conversation",
        EventLinePolicy::subtree_publish_owner_subscribe(),
    )?;
    root.set_default_event_line("conversation")?;
    let child = Arc::new(ExecutionUnit::new_child(UnitType::StateMachine, &root)?);
    let count = Arc::new(AtomicUsize::new(0));

    root.event_bus()
        .subscribe(
            "default:event".to_string(),
            Arc::new(CountingHandler {
                name: "default-owner".to_string(),
                count: count.clone(),
            }),
        )
        .await?;
    child
        .event_bus()
        .publish(BaseEvent::new("default:event", json!({})))
        .await?;

    assert_eq!(
        child.default_event_line_name().as_deref(),
        Some("conversation")
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(child
        .event_bus()
        .subscribe(
            "default:event".to_string(),
            Arc::new(CountingHandler {
                name: "default-child".to_string(),
                count: Arc::new(AtomicUsize::new(0)),
            }),
        )
        .await
        .is_err());
    Ok(())
}

#[tokio::test]
async fn concurrent_default_event_bus_creation_uses_one_event_line() -> Result<()> {
    let framework = FrameworkState::initialize()?;
    let unit = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework));
    let count = Arc::new(AtomicUsize::new(0));
    let mut tasks = Vec::new();

    for index in 0..32 {
        let unit = unit.clone();
        let count = count.clone();
        tasks.push(tokio::spawn(async move {
            unit.event_bus()
                .subscribe(
                    "concurrent:event".to_string(),
                    Arc::new(CountingHandler {
                        name: format!("concurrent-handler-{index}"),
                        count,
                    }),
                )
                .await
        }));
    }

    for task in tasks {
        task.await.expect("subscription task should not panic")?;
    }

    unit.event_bus()
        .publish(BaseEvent::new("concurrent:event", json!({})))
        .await?;
    assert_eq!(count.load(Ordering::SeqCst), 32);
    Ok(())
}

#[tokio::test]
async fn private_event_line_rejects_child_access_and_siblings_cannot_find_it() -> Result<()> {
    let framework = FrameworkState::initialize()?;
    let root = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework.clone()));
    let child = Arc::new(ExecutionUnit::new_child(UnitType::StateMachine, &root)?);
    let sibling = ExecutionUnit::new_root(UnitType::Module, framework);
    let owner_line = root.create_event_line("private", EventLinePolicy::private())?;

    assert!(child.event_line("private").is_err());
    assert!(sibling.event_line("private").is_err());

    owner_line
        .publish(BaseEvent::new("private:event", json!({})))
        .await?;
    Ok(())
}

#[test]
fn event_line_lookup_uses_nearest_owner_for_same_name() -> Result<()> {
    let framework = FrameworkState::initialize()?;
    let root = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework));
    let child = Arc::new(ExecutionUnit::new_child(UnitType::Module, &root)?);
    let grandchild = ExecutionUnit::new_child(UnitType::StateMachine, &child)?;

    root.create_event_line("shared", EventLinePolicy::subtree())?;
    child.create_event_line("shared", EventLinePolicy::subtree())?;

    let line = grandchild.event_line("shared")?;
    assert_eq!(line.owner_id(), child.id());
    Ok(())
}

#[tokio::test]
async fn event_line_handle_expires_when_owner_is_dropped() -> Result<()> {
    let framework = FrameworkState::initialize()?;
    let line = {
        let owner = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework));
        owner.create_event_line("temporary", EventLinePolicy::subtree())?
    };

    assert!(line
        .publish(BaseEvent::new("temporary:event", json!({})))
        .await
        .is_err());
    Ok(())
}

#[test]
fn execution_unit_rejects_hierarchy_beyond_maximum_depth() -> Result<()> {
    let framework = FrameworkState::initialize()?;
    let mut parent = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework));

    for _ in 0..corework::execution_unit::MAX_EXECUTION_UNIT_DEPTH {
        parent = Arc::new(ExecutionUnit::new_child(UnitType::Module, &parent)?);
    }

    assert!(ExecutionUnit::new_child(UnitType::Module, &parent).is_err());
    Ok(())
}

#[tokio::test]
async fn execution_unit_cached_resource_api_uses_owner_subtree() -> Result<()> {
    let framework = FrameworkState::initialize()?;
    let owner = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework.clone()));
    let child = ExecutionUnit::new_child(UnitType::Module, &owner)?;
    let sibling = ExecutionUnit::new_root(UnitType::Module, framework);
    let resource_key = format!("resource:cached:{}", owner.id());

    owner.declare_resource_access(&resource_key, AccessMode::Owner)?;
    owner.grant_access_to(&resource_key, child.id(), AccessMode::ReadWrite)?;

    owner
        .set_resource_cached(&resource_key, &"cached-value".to_string(), None)
        .await?;

    assert_eq!(
        owner.get_resource_cached::<String>(&resource_key).await?,
        Some("cached-value".to_string())
    );
    assert_eq!(
        child.get_resource_cached::<String>(&resource_key).await?,
        Some("cached-value".to_string())
    );
    child
        .set_resource_cached(&resource_key, &"child-update".to_string(), None)
        .await?;
    assert_eq!(
        owner.get_resource_cached::<String>(&resource_key).await?,
        Some("child-update".to_string())
    );
    assert!(sibling
        .get_resource_cached::<String>(&resource_key)
        .await
        .is_err());

    Ok(())
}

#[tokio::test]
async fn orphaned_child_creates_a_local_default_event_line() -> Result<()> {
    let framework = FrameworkState::initialize()?;
    let child = {
        let root = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework));
        Arc::new(ExecutionUnit::new_child(UnitType::StateMachine, &root)?)
    };
    let count = Arc::new(AtomicUsize::new(0));
    let bus = child.event_bus();

    bus.subscribe(
        "orphan:event".to_string(),
        Arc::new(CountingHandler {
            name: "orphan-handler".to_string(),
            count: count.clone(),
        }),
    )
    .await?;
    bus.publish(BaseEvent::new("orphan:event", json!({})))
        .await?;

    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(
        child.default_event_line_name().as_deref(),
        Some("__corework_default")
    );
    Ok(())
}

#[tokio::test]
async fn nested_state_machine_uses_real_parent_and_keeps_local_cache_isolated() -> Result<()> {
    let framework = FrameworkState::initialize()?;
    let parent = Arc::new(ExecutionUnit::new_root(UnitType::Module, framework.clone()));

    parent
        .cache()
        .set_raw("shared-before-build", json!("visible"), None)
        .await?;

    let sm = StateMachine::builder("nested-contract")
        .with_framework(framework)
        .with_parent_unit(parent.clone())
        .add_state(Box::new(FnState::new("idle")))
        .initial_state("idle")
        .build()
        .await?;

    assert_ne!(parent.id(), sm.unit().id());
    assert_eq!(parent.scope_id(), sm.unit().scope_id());
    assert_eq!(
        sm.unit().cache().get_raw("shared-before-build").await?,
        None
    );

    sm.unit()
        .cache()
        .set_raw("shared-from-sm", json!("state-machine"), None)
        .await?;
    assert_eq!(parent.cache().get_raw("shared-from-sm").await?, None);

    Ok(())
}

#[test]
fn execution_unit_resource_permissions_gate_world_access() -> Result<()> {
    let framework = FrameworkState::initialize()?;
    let owner = ExecutionUnit::new_root(UnitType::Module, framework.clone());
    let reader = ExecutionUnit::new_root(UnitType::Module, framework.clone());
    let writer = ExecutionUnit::new_root(UnitType::Module, framework);
    let resource_key = format!("contract:test:{}", uuid::Uuid::new_v4());

    owner.declare_resource_access(&resource_key, AccessMode::ReadWrite)?;
    owner.set_resource(&resource_key, &"owned-value", None)?;

    assert!(reader.get_resource::<String>(&resource_key).is_err());
    assert!(writer
        .set_resource(&resource_key, &"blocked", None)
        .is_err());

    owner.grant_access_to(&resource_key, reader.id(), AccessMode::Read)?;
    assert_eq!(
        reader.get_resource::<String>(&resource_key)?,
        Some("owned-value".to_string())
    );
    assert!(reader
        .set_resource(&resource_key, &"reader-write", None)
        .is_err());

    owner.grant_access_to(&resource_key, writer.id(), AccessMode::ReadWrite)?;
    writer.set_resource(&resource_key, &"writer-value", None)?;
    assert_eq!(
        owner.get_resource::<String>(&resource_key)?,
        Some("writer-value".to_string())
    );

    Ok(())
}
