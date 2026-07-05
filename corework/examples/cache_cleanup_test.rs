// 缓存系统自动清理测试

use corework::prelude::*;
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    println!("=== 缓存系统自动清理测试 ===\n");

    // ========== 测试 1: TTL 过期自动清理 ==========
    println!("测试 1: TTL 过期自动清理（惰性删除）");
    println!("------------------------------------------------------------");

    let cache = Arc::new(InMemoryCache::default());

    // 设置短期缓存（100ms 过期）
    println!("\n设置缓存:");
    cache
        .set(
            "short_lived",
            &"临时数据".to_string(),
            Some(Duration::from_millis(100)),
        )
        .await?;
    cache
        .set(
            "long_lived",
            &"长期数据".to_string(),
            Some(Duration::from_secs(60)),
        )
        .await?;

    // 立即读取
    let value: Option<String> = cache.get("short_lived").await?;
    println!("  ✓ 立即读取 'short_lived': {:?}", value);
    assert_eq!(value, Some("临时数据".to_string()));

    let value: Option<String> = cache.get("long_lived").await?;
    println!("  ✓ 立即读取 'long_lived': {:?}", value);
    assert_eq!(value, Some("长期数据".to_string()));

    // 等待短期缓存过期
    println!("\n等待 150ms...");
    tokio::time::sleep(Duration::from_millis(150)).await;

    // 尝试读取过期缓存（应触发自动清理）
    let value: Option<String> = cache.get("short_lived").await?;
    println!("  ✓ 过期后读取 'short_lived': {:?} (已自动清理)", value);
    assert_eq!(value, None);

    // 长期缓存应该仍然存在
    let value: Option<String> = cache.get("long_lived").await?;
    println!("  ✓ 长期缓存 'long_lived' 仍然有效: {:?}", value);
    assert_eq!(value, Some("长期数据".to_string()));

    println!("\n✓ 惰性删除测试通过");
    println!("------------------------------------------------------------");

    // ========== 测试 2: exists 方法的过期检查 ==========
    println!("\n测试 2: exists 方法的过期检查");
    println!("------------------------------------------------------------");

    let cache = Arc::new(InMemoryCache::default());

    // 设置短期缓存
    cache
        .set("temp_key", &42, Some(Duration::from_millis(100)))
        .await?;

    // 立即检查存在性
    let exists = cache.exists("temp_key").await?;
    println!("\n  ✓ 立即检查: exists('temp_key') = {}", exists);
    assert!(exists);

    // 等待过期
    println!("  等待 150ms...");
    tokio::time::sleep(Duration::from_millis(150)).await;

    // 检查过期后的存在性（应触发清理）
    let exists = cache.exists("temp_key").await?;
    println!(
        "  ✓ 过期后检查: exists('temp_key') = {} (已自动清理)",
        exists
    );
    assert!(!exists);

    println!("\n✓ exists 过期检查测试通过");
    println!("------------------------------------------------------------");

    // ========== 测试 3: 批量操作的 TTL ==========
    println!("\n测试 3: 批量操作的 TTL");
    println!("------------------------------------------------------------");

    let cache = Arc::new(InMemoryCache::default());

    // 批量设置带 TTL 的缓存
    let items = vec![
        ("batch_1".to_string(), "值1".to_string()),
        ("batch_2".to_string(), "值2".to_string()),
        ("batch_3".to_string(), "值3".to_string()),
    ];

    cache.mset(&items, Some(Duration::from_millis(100))).await?;
    println!("\n  ✓ 批量设置 3 个缓存（TTL=100ms）");

    // 立即批量获取
    let keys = vec![
        "batch_1".to_string(),
        "batch_2".to_string(),
        "batch_3".to_string(),
    ];
    let values: Vec<Option<String>> = cache.mget(&keys).await?;
    println!("  ✓ 立即批量获取: {:?}", values);
    assert_eq!(values.len(), 3);
    assert!(values.iter().all(|v| v.is_some()));

    // 等待过期
    println!("  等待 150ms...");
    tokio::time::sleep(Duration::from_millis(150)).await;

    // 过期后批量获取
    let values: Vec<Option<String>> = cache.mget(&keys).await?;
    println!("  ✓ 过期后批量获取: {:?} (全部已清理)", values);
    assert!(values.iter().all(|v| v.is_none()));

    println!("\n✓ 批量 TTL 测试通过");
    println!("------------------------------------------------------------");

    // ========== 测试 4: expire 方法动态设置过期时间 ==========
    println!("\n测试 4: 动态设置过期时间");
    println!("------------------------------------------------------------");

    let cache = Arc::new(InMemoryCache::default());

    // 设置永久缓存
    cache.set("dynamic", &"初始数据".to_string(), None).await?;
    println!("\n  ✓ 设置永久缓存 'dynamic'");

    // 动态设置过期时间为 100ms
    cache.expire("dynamic", Duration::from_millis(100)).await?;
    println!("  ✓ 动态设置过期时间为 100ms");

    // 立即读取
    let value: Option<String> = cache.get("dynamic").await?;
    println!("  ✓ 立即读取: {:?}", value);
    assert_eq!(value, Some("初始数据".to_string()));

    // 等待过期
    println!("  等待 150ms...");
    tokio::time::sleep(Duration::from_millis(150)).await;

    // 过期后读取
    let value: Option<String> = cache.get("dynamic").await?;
    println!("  ✓ 过期后读取: {:?} (已过期)", value);
    assert_eq!(value, None);

    println!("\n✓ 动态过期时间测试通过");
    println!("------------------------------------------------------------");

    // ========== 测试 5: 计数器的 TTL ==========
    println!("\n测试 5: 计数器的 TTL");
    println!("------------------------------------------------------------");

    let cache = Arc::new(InMemoryCache::default());

    // 首次增加计数器（会使用默认 TTL）
    let count = cache.incr("counter_with_ttl", 1).await?;
    println!("\n  ✓ 增加计数器: {}", count);
    assert_eq!(count, 1);

    // 手动设置过期时间
    cache
        .expire("counter_with_ttl", Duration::from_millis(100))
        .await?;
    println!("  ✓ 设置计数器过期时间为 100ms");

    // 再次增加
    let count = cache.incr("counter_with_ttl", 5).await?;
    println!("  ✓ 再次增加: {}", count);
    assert_eq!(count, 6);

    // 等待过期
    println!("  等待 150ms...");
    tokio::time::sleep(Duration::from_millis(150)).await;

    // 检查是否已清理
    let exists = cache.exists("counter_with_ttl").await?;
    println!("  ✓ 检查存在性: {} (已过期清理)", exists);
    assert!(!exists);

    // 过期后重新计数应该从 1 开始
    let count = cache.incr("counter_with_ttl", 1).await?;
    println!("  ✓ 过期后重新计数: {} (从头开始)", count);
    assert_eq!(count, 1);

    println!("\n✓ 计数器 TTL 测试通过");
    println!("------------------------------------------------------------");

    // ========== 测试 6: flush 清空所有缓存 ==========
    println!("\n测试 6: flush 清空所有缓存");
    println!("------------------------------------------------------------");

    let cache = Arc::new(InMemoryCache::default());

    // 设置多个缓存
    cache.set("key1", &"value1".to_string(), None).await?;
    cache.set("key2", &"value2".to_string(), None).await?;
    cache.set("key3", &"value3".to_string(), None).await?;
    println!("\n  ✓ 设置 3 个缓存");

    // 验证存在
    assert!(cache.exists("key1").await?);
    assert!(cache.exists("key2").await?);
    assert!(cache.exists("key3").await?);
    println!("  ✓ 全部存在");

    // 清空所有缓存
    cache.flush().await?;
    println!("  ✓ 执行 flush()");

    // 验证已清空
    assert!(!cache.exists("key1").await?);
    assert!(!cache.exists("key2").await?);
    assert!(!cache.exists("key3").await?);
    println!("  ✓ 全部已清空");

    println!("\n✓ flush 测试通过");
    println!("------------------------------------------------------------");

    // ========== 测试 7: 配置自定义默认 TTL ==========
    println!("\n测试 7: 自定义默认 TTL");
    println!("------------------------------------------------------------");

    let config = CacheConfig {
        default_ttl: Some(Duration::from_millis(100)),
        key_prefix: "test".to_string(),
        enable_compression: false,
        max_value_size: 1024,
    };

    let cache = Arc::new(InMemoryCache::with_config(config));

    // 不指定 TTL，应使用默认值（100ms）
    cache
        .set("auto_expire", &"自动过期".to_string(), None)
        .await?;
    println!("\n  ✓ 使用默认 TTL 设置缓存");

    // 立即读取
    let value: Option<String> = cache.get("auto_expire").await?;
    println!("  ✓ 立即读取: {:?}", value);
    assert_eq!(value, Some("自动过期".to_string()));

    // 等待过期
    println!("  等待 150ms...");
    tokio::time::sleep(Duration::from_millis(150)).await;

    // 应该已过期
    let value: Option<String> = cache.get("auto_expire").await?;
    println!("  ✓ 过期后读取: {:?} (使用默认 TTL)", value);
    assert_eq!(value, None);

    println!("\n✓ 自定义默认 TTL 测试通过");
    println!("------------------------------------------------------------");

    // 总结
    println!("\n=== 测试总结 ===");
    println!("✓ TTL 过期自动清理（惰性删除）");
    println!("✓ exists 方法的过期检查");
    println!("✓ 批量操作的 TTL");
    println!("✓ 动态设置过期时间");
    println!("✓ 计数器的 TTL");
    println!("✓ flush 清空所有缓存");
    println!("✓ 自定义默认 TTL");
    println!("\n缓存系统的自动清理功能正常！");
    println!("\n说明:");
    println!("  - 采用惰性删除策略：在 get/exists 时检查并清理过期项");
    println!("  - 支持单个缓存和批量缓存的 TTL");
    println!("  - 支持动态设置过期时间（expire 方法）");
    println!("  - 支持全局清空（flush 方法）");
    println!("  - 支持自定义默认 TTL 配置\n");

    Ok(())
}
