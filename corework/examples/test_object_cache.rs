//! 测试 Object 和复杂类型在 cache 中的存储和传递

use corework::cache::{CacheExt, InMemoryCache};
use corework::workflow::core::{DataValue, KeyValuePair};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🧪 测试复杂类型的 Cache 存储与传递\n");

    // 创建 cache
    let cache = InMemoryCache::new();

    // ========== 测试 1: 基础类型 ==========
    println!("📝 测试 1: 基础类型 (i64, String, bool)");

    let int_val = DataValue::from_i64(42);
    let str_val = DataValue::from_string("hello");
    let bool_val = DataValue::from_bool(true);

    cache.set("test:int", &int_val, None).await?;
    cache.set("test:string", &str_val, None).await?;
    cache.set("test:bool", &bool_val, None).await?;

    let retrieved_int: Option<DataValue> = cache.get("test:int").await?;
    let retrieved_str: Option<DataValue> = cache.get("test:string").await?;
    let retrieved_bool: Option<DataValue> = cache.get("test:bool").await?;

    assert_eq!(retrieved_int.as_ref().unwrap().as_i64(), Some(42));
    assert_eq!(retrieved_str.as_ref().unwrap().as_str(), Some("hello"));
    assert_eq!(retrieved_bool.as_ref().unwrap().as_bool(), Some(true));
    println!("✅ 基础类型存储和读取正常\n");

    // ========== 测试 2: KeyValuePair ==========
    println!("📝 测试 2: KeyValuePair");

    let pair = KeyValuePair::new("name", json!("Alice"));
    let pair_json = pair.to_json();
    let pair_value = DataValue::new("KeyValuePair", pair_json.clone());

    cache.set("test:pair", &pair_value, None).await?;
    let retrieved_pair: Option<DataValue> = cache.get("test:pair").await?;

    assert!(retrieved_pair.is_some());
    let retrieved = retrieved_pair.unwrap();
    assert_eq!(retrieved.type_name, "KeyValuePair");

    // 验证能否反序列化回 KeyValuePair
    let restored_pair = KeyValuePair::from_json(&retrieved.value)?;
    assert_eq!(restored_pair.key, "name");
    assert_eq!(restored_pair.value, json!("Alice"));
    println!("✅ KeyValuePair 存储和读取正常\n");

    // ========== 测试 3: Array<KeyValuePair> ==========
    println!("📝 测试 3: Array<KeyValuePair>");

    let pairs = vec![
        KeyValuePair::new("name", json!("Bob")).to_json(),
        KeyValuePair::new("age", json!(30)).to_json(),
        KeyValuePair::new("active", json!(true)).to_json(),
    ];

    let array_value = DataValue::from_array(pairs.clone(), "KeyValuePair");
    cache.set("test:array", &array_value, None).await?;

    let retrieved_array: Option<DataValue> = cache.get("test:array").await?;
    assert!(retrieved_array.is_some());

    let retrieved = retrieved_array.unwrap();
    assert_eq!(retrieved.type_name, "Vec<KeyValuePair>");
    assert_eq!(retrieved.array_len(), Some(3));

    // 验证数组元素
    let first_elem = retrieved.get_array_element(0).unwrap();
    let first_pair = KeyValuePair::from_json(&first_elem.value)?;
    assert_eq!(first_pair.key, "name");
    println!("✅ Array<KeyValuePair> 存储和读取正常\n");

    // ========== 测试 4: Object (JSON Object) ==========
    println!("📝 测试 4: Object (JSON 对象)");

    let mut obj_map = serde_json::Map::new();
    obj_map.insert("name".to_string(), json!("Charlie"));
    obj_map.insert("score".to_string(), json!(95));
    obj_map.insert("passed".to_string(), json!(true));

    let obj_value = DataValue {
        type_name: "Object".to_string(),
        value: json!(obj_map),
        element_type: None,
        container: corework::workflow::core::PinContainerType::None,
    };

    cache.set("test:object", &obj_value, None).await?;
    let retrieved_obj: Option<DataValue> = cache.get("test:object").await?;

    assert!(retrieved_obj.is_some());
    let retrieved = retrieved_obj.unwrap();
    assert_eq!(retrieved.type_name, "Object");
    assert!(retrieved.is_object());

    // 验证对象字段
    let name_field = retrieved.get_field("name").unwrap();
    assert_eq!(name_field.value, json!("Charlie"));

    let score_field = retrieved.get_field("score").unwrap();
    assert_eq!(score_field.value, json!(95));
    println!("✅ Object 存储和读取正常\n");

    // ========== 测试 5: 嵌套结构 ==========
    println!("📝 测试 5: 嵌套结构 (Object 包含 Array)");

    let mut complex_obj = serde_json::Map::new();
    complex_obj.insert("id".to_string(), json!(123));
    complex_obj.insert("tags".to_string(), json!(["rust", "async", "cache"]));
    complex_obj.insert(
        "metadata".to_string(),
        json!({
            "created": "2026-01-29",
            "version": 1
        }),
    );

    let complex_value = DataValue::from_json_object(json!(complex_obj))?;
    cache.set("test:complex", &complex_value, None).await?;

    let retrieved_complex: Option<DataValue> = cache.get("test:complex").await?;
    assert!(retrieved_complex.is_some());

    let retrieved = retrieved_complex.unwrap();
    let id_field = retrieved.get_field("id").unwrap();
    assert_eq!(id_field.value, json!(123));

    let tags_field = retrieved.get_field("tags").unwrap();
    assert!(tags_field.value.is_array());
    println!("✅ 嵌套结构存储和读取正常\n");

    // ========== 测试 6: 模拟节点间传递 ==========
    println!("📝 测试 6: 模拟节点间数据传递");

    // 模拟 MakeObjectNode 的输出
    let node1_output = DataValue::from_array(
        vec![
            KeyValuePair::new("username", json!("admin")).to_json(),
            KeyValuePair::new("role", json!("administrator")).to_json(),
        ],
        "KeyValuePair",
    );

    cache.set("Node1:output", &node1_output, None).await?;

    // 模拟 Node2 读取 Node1 的输出
    let node2_input: Option<DataValue> = cache.get("Node1:output").await?;
    assert!(node2_input.is_some());

    let input = node2_input.unwrap();
    assert_eq!(input.type_name, "Vec<KeyValuePair>");
    assert_eq!(input.array_len(), Some(2));

    // 模拟构造 Object
    let pairs_array = input.as_array().unwrap();
    let mut result_obj = serde_json::Map::new();

    for pair_json in pairs_array {
        let pair = KeyValuePair::from_json(pair_json)?;
        result_obj.insert(pair.key, pair.value);
    }

    let node2_output = DataValue::from_json_object(json!(result_obj))?;
    cache.set("Node2:output", &node2_output, None).await?;

    // 模拟 Node3 读取 Object
    let node3_input: Option<DataValue> = cache.get("Node2:output").await?;
    assert!(node3_input.is_some());

    let final_obj = node3_input.unwrap();
    assert_eq!(final_obj.type_name, "Object");
    assert_eq!(
        final_obj.get_field("username").unwrap().value,
        json!("admin")
    );
    assert_eq!(
        final_obj.get_field("role").unwrap().value,
        json!("administrator")
    );

    println!("✅ 节点间数据传递正常\n");

    println!("🎉 所有测试通过！复杂类型可以正常存储和传递。");

    Ok(())
}
