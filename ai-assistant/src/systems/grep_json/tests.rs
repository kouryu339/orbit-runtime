use super::*;
use crate::systems::grep::{GrepConfig, GrepRoot};
use std::{fs, path::PathBuf};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("grep-json-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn write(&self, text: &str) {
        fs::write(self.0.join("doc.json"), text).unwrap();
    }
    fn service(&self, output: usize) -> GrepService {
        GrepService::new(
            GrepConfig {
                roots: vec![GrepRoot {
                    id: "project".into(),
                    path: self.0.clone(),
                }],
                max_output_bytes: output,
                ..Default::default()
            },
            None,
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn request(action: &str) -> Request {
    Request {
        root_id: "project".into(),
        path: "doc.json".into(),
        action: action.into(),
        limit: 5,
        ..Default::default()
    }
}
fn run(service: &GrepService, req: &Request) -> Result<Value, String> {
    query(service, req, &AtomicBool::new(false))
}

#[test]
fn locates_id_with_parent_identity_and_reads_exact_subtree() {
    let f = Fixture::new();
    f.write(r#"{"sequences":[{"id":"seq-1","name":"Opening","bindings":[{"id":"camera-01","tracks":[{"frame":10,"x":1.5}]}]}]}"#);
    let service = f.service(8192);
    let mut req = request("search");
    req.field = Some("id".into());
    req.value_json = Some(json!("camera-01").to_string());
    let found = run(&service, &req).unwrap();
    let hit = &found["matches"][0];
    assert_eq!(hit["pointer"], "/sequences/0/bindings/0/id");
    assert_eq!(hit["parent_pointer"], "/sequences/0/bindings/0");
    assert_eq!(hit["ancestors"][0]["identity"]["id"]["value"], "camera-01");
    assert_eq!(hit["ancestors"][2]["identity"]["name"]["value"], "Opening");
    req = request("read");
    req.pointer = hit["parent_pointer"].as_str().unwrap().into();
    req.expected_revision = found["revision"].as_str().unwrap().into();
    let result = run(&service, &req).unwrap();
    assert_eq!(result["value_complete"], true);
    assert_eq!(result["value"]["tracks"][0]["frame"], 10);
}

#[test]
fn walks_every_array_page_under_fixed_revision() {
    let f = Fixture::new();
    f.write(
        &json!((0..1037)
            .map(|i| json!({"id":i,"value":format!("item-{i}")}))
            .collect::<Vec<_>>())
        .to_string(),
    );
    let service = f.service(2048);
    let mut req = request("page");
    let mut ids = Vec::new();
    loop {
        let result = run(&service, &req).unwrap();
        assert!(result.to_string().len() <= 2048);
        for item in result["items"].as_array().unwrap() {
            assert_eq!(item["value_complete"], true);
            ids.push(item["value"]["id"].as_u64().unwrap());
        }
        if result["complete"] == true {
            break;
        }
        req.expected_revision = result["revision"].as_str().unwrap().into();
        req.offset = result["next_offset"].as_u64().unwrap() as usize;
    }
    assert_eq!(ids, (0..1037).collect::<Vec<_>>());
    f.write("[]");
    assert!(run(&service, &req)
        .unwrap_err()
        .starts_with("revision_conflict:"));
}

#[test]
fn oversized_subtree_exposes_paths_and_unicode_string_can_be_reassembled() {
    let f = Fixture::new();
    let text = "长🙂\\\"\n".repeat(600);
    f.write(&json!({"payload":text,"answer":42}).to_string());
    let service = f.service(1024);
    let initial = run(&service, &request("read")).unwrap();
    assert_eq!(initial["value_complete"], false);
    let item = initial["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["pointer"] == "/payload")
        .unwrap();
    assert_eq!(item["value_complete"], false);
    let mut req = request("page");
    req.pointer = "/payload".into();
    req.limit = 150;
    req.expected_revision = initial["revision"].as_str().unwrap().into();
    let mut collected = String::new();
    loop {
        let result = run(&service, &req).unwrap();
        assert!(result.to_string().len() <= 1024);
        collected.push_str(result["chunk"].as_str().unwrap());
        if result["complete"] == true {
            break;
        }
        let next = result["next_offset"].as_u64().unwrap() as usize;
        assert!(next > req.offset);
        req.offset = next;
    }
    assert_eq!(collected, text);
}

#[test]
fn preserves_large_integer_and_pointer_escaping() {
    let f = Fixture::new();
    f.write(r#"{"a/b":{"~key":1234567890123456789012345678901234567890},"":null}"#);
    let service = f.service(4096);
    let mut req = request("search");
    req.field = Some("~key".into());
    let result = run(&service, &req).unwrap();
    assert_eq!(result["matches"][0]["pointer"], "/a~1b/~0key");
    req = request("read");
    req.pointer = "/a~1b/~0key".into();
    assert_eq!(
        run(&service, &req).unwrap()["value"].to_string(),
        "1234567890123456789012345678901234567890"
    );
    req.pointer = "/".into();
    assert_eq!(run(&service, &req).unwrap()["value"], Value::Null);
    req.pointer = "/a~2b".into();
    assert!(run(&service, &req).is_err());
}

#[test]
fn rejects_duplicates_invalid_json_missing_pointer_and_unversioned_page() {
    let f = Fixture::new();
    let service = f.service(4096);
    f.write(r#"{"id":1,"id":2}"#);
    assert!(run(&service, &request("read"))
        .unwrap_err()
        .contains("duplicate"));
    f.write(r#"{"id":1} trailing"#);
    assert!(run(&service, &request("read")).is_err());
    f.write(r#"{"id":1}"#);
    let mut req = request("page");
    req.offset = 1;
    assert!(run(&service, &req)
        .unwrap_err()
        .contains("expected_revision"));
    req = request("read");
    req.pointer = "/missing".into();
    assert_eq!(run(&service, &req).unwrap_err(), "pointer_not_found");
}

#[test]
fn search_continues_without_losing_matches() {
    let f = Fixture::new();
    f.write(&json!((0..20).map(|i| json!({"id":i})).collect::<Vec<_>>()).to_string());
    let service = f.service(2048);
    let mut req = request("search");
    req.field = Some("id".into());
    let mut seen = Vec::new();
    loop {
        let result = run(&service, &req).unwrap();
        for item in result["matches"].as_array().unwrap() {
            seen.push(item["value"].as_u64().unwrap());
        }
        if result["complete"] == true {
            break;
        }
        req.offset = result["next_offset"].as_u64().unwrap() as usize;
        req.expected_revision = result["revision"].as_str().unwrap().into();
    }
    assert_eq!(seen, (0..20).collect::<Vec<_>>());
}

#[test]
fn shares_root_exclusions_and_resource_limits() {
    let f = Fixture::new();
    f.write("{}");
    fs::write(f.0.join(".ignore"), "doc.json\n").unwrap();
    let service = f.service(4096);
    assert!(run(&service, &request("read"))
        .unwrap_err()
        .contains("ignored"));
    let mut req = request("read");
    req.path = "../outside.json".into();
    assert!(run(&service, &req).is_err());
    req.path = ".secret.json".into();
    fs::write(f.0.join(".secret.json"), "{}").unwrap();
    assert!(run(&service, &req).is_err());
    req = request("read");
    req.root_id = "unknown".into();
    assert!(run(&service, &req).is_err());
    fs::write(f.0.join(".ignore"), "").unwrap();
    let service = GrepService::new(
        GrepConfig {
            roots: vec![GrepRoot {
                id: "project".into(),
                path: f.0.clone(),
            }],
            max_file_bytes: 8,
            ..Default::default()
        },
        None,
    )
    .unwrap();
    f.write("[123456789]");
    assert!(run(&service, &request("read"))
        .unwrap_err()
        .contains("max_file_bytes"));
}

#[test]
fn rejects_excessive_nodes_and_cancellation() {
    let f = Fixture::new();
    f.write("[1,2,3,4,5]");
    let service = GrepService::new(
        GrepConfig {
            roots: vec![GrepRoot {
                id: "project".into(),
                path: f.0.clone(),
            }],
            max_entries: 3,
            ..Default::default()
        },
        None,
    )
    .unwrap();
    assert!(run(&service, &request("read"))
        .unwrap_err()
        .contains("max_entries"));
    assert!(query(&service, &request("read"), &AtomicBool::new(true))
        .unwrap_err()
        .contains("canceled"));
}

#[test]
fn revision_covers_exact_file_bytes() {
    assert_eq!(
        document_revision(b"{}"),
        "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
    );
    assert_ne!(document_revision(b"{}"), document_revision(b"{ }"));
}

#[tokio::test]
async fn tool_is_readonly_and_shares_grep_concurrency() {
    use corework::execution_unit::{ExecutionUnit, UnitType};
    let factory = inventory::iter::<corework::ai_system::AISystemFactory>
        .into_iter()
        .find(|f| f.metadata.name == "GrepJson")
        .unwrap();
    assert!(factory.metadata.readonly);
    assert!(!factory.metadata.workflow_enabled);
    crate::tool_schema::definitions_for_active_tools(&["GrepJson".into()], true, true).unwrap();
    let f = Fixture::new();
    f.write(r#"{"id":1}"#);
    let service = Arc::new(f.service(4096));
    let unit = Arc::new(ExecutionUnit::new_root_in_scope(
        UnitType::Module,
        corework::world::FrameworkState::initialize().unwrap(),
        format!("json-{}", uuid::Uuid::new_v4()),
    ));
    let ctx = unit.create_context();
    let input = || {
        AIInput::from_args(std::collections::HashMap::from([
            ("root_id".into(), "project".into()),
            ("path".into(), "doc.json".into()),
            ("action".into(), "read".into()),
        ]))
    };
    assert_eq!(
        GrepJsonSystem
            .execute(input(), &ctx)
            .await
            .unwrap()
            .error_code,
        403
    );
    unit.attach_shared_component(service.clone()).unwrap();
    let result = GrepJsonSystem.execute(input(), &ctx).await.unwrap();
    assert_eq!(result.error_code, 0, "{}", result.to_ai);
    assert_eq!(
        serde_json::from_str::<Value>(&result.to_ai).unwrap(),
        result.result
    );
    let _permits = service
        .permits
        .clone()
        .acquire_many_owned(service.config.max_concurrent_searches as u32)
        .await
        .unwrap();
    assert_eq!(
        GrepJsonSystem
            .execute(input(), &ctx)
            .await
            .unwrap()
            .error_code,
        429
    );
}

#[test]
fn paginates_wide_objects_and_all_descriptors_are_readable() {
    let f = Fixture::new();
    let object: serde_json::Map<String, Value> = (0..100)
        .map(|i| (format!("field/{i}"), json!("data".repeat(800))))
        .collect();
    f.write(&Value::Object(object).to_string());
    let service = f.service(2048);
    let mut req = request("outline");
    req.limit = 50;
    let mut pointers = Vec::new();
    loop {
        let result = run(&service, &req).unwrap();
        for item in result["items"].as_array().unwrap() {
            assert_eq!(item["value_complete"], false);
            pointers.push(item["pointer"].as_str().unwrap().to_string());
        }
        req.expected_revision = result["revision"].as_str().unwrap().into();
        if result["complete"] == true {
            break;
        }
        req.offset = result["next_offset"].as_u64().unwrap() as usize;
    }
    assert_eq!(pointers.len(), 100);
    assert!(pointers.iter().all(|p| p.contains("~1")));
    req.action = "page".into();
    req.pointer = pointers[0].clone();
    req.offset = 0;
    assert!(!run(&service, &req).unwrap()["chunk"]
        .as_str()
        .unwrap()
        .is_empty());
}

#[test]
fn scoped_field_search_and_typed_values_are_precise() {
    let f = Fixture::new();
    f.write(r#"{"id":"1","other":{"id":1},"enabled":false}"#);
    let service = f.service(8192);
    let mut req = request("search");
    req.pointer = "/other/id".into();
    req.field = Some("id".into());
    req.value_json = Some(json!(1).to_string());
    let result = run(&service, &req).unwrap();
    assert_eq!(result["total_matches"], 1);
    assert_eq!(result["matches"][0]["pointer"], "/other/id");
    req.pointer = "".into();
    req.value_json = Some(json!("1").to_string());
    assert_eq!(run(&service, &req).unwrap()["matches"][0]["pointer"], "/id");
    req.field = Some("enabled".into());
    req.value_json = Some(json!(false).to_string());
    assert_eq!(run(&service, &req).unwrap()["matches"][0]["value"], false);
}

fn parse_for_test(bytes: &[u8], max_nodes: usize) -> Result<Value, String> {
    parser::parse_document(
        bytes,
        max_nodes,
        &Budget {
            cancel: &AtomicBool::new(false),
            started: Instant::now(),
            timeout: Duration::from_secs(10),
        },
    )
}

#[test]
fn every_object_marker_remains_a_field_alongside_real_numbers() {
    let raw = br#"{"$serde_json::private::Number":"not a number","tail":true,"nested":[{"$serde_json::private::Number":"123","$serde_json::private::RawValue":"unchanged","other":null}],"real":123456789012345678901234567890}"#;
    let value = parse_for_test(raw, 100).unwrap();
    assert_eq!(value["$serde_json::private::Number"], "not a number");
    assert_eq!(value["tail"], true);
    assert_eq!(value["nested"][0]["$serde_json::private::Number"], "123");
    assert_eq!(
        value["nested"][0]["$serde_json::private::RawValue"],
        "unchanged"
    );
    assert_eq!(value["nested"][0]["other"], Value::Null);
    assert_eq!(value["real"].to_string(), "123456789012345678901234567890");
    assert_eq!(
        parse_for_test(value.to_string().as_bytes(), 100).unwrap(),
        value
    );
}

#[test]
fn search_value_json_uses_the_same_lossless_and_duplicate_rejecting_parser() {
    let f = Fixture::new();
    f.write(r#"{"target":{"$serde_json::private::Number":"123","tail":true},"actual":123}"#);
    let service = f.service(8192);
    let mut req = request("search");
    req.value_json = Some(r#"{"$serde_json::private::Number":"123","tail":true}"#.into());
    let found = run(&service, &req).unwrap();
    assert_eq!(found["total_matches"], 1);
    assert_eq!(found["matches"][0]["pointer"], "/target");
    req.value_json = Some(r#"{"same":1,"same":2}"#.into());
    assert!(run(&service, &req).unwrap_err().contains("duplicate"));
}

#[test]
fn large_and_high_precision_numeric_literals_do_not_round() {
    for raw in [
        "1234567890123456789012345678901234567890",
        "-1234567890123456789012345678901234567890",
        "0.123456789012345678901234567890123456789",
        "1.234567890123456789e+1000",
        "1e-1000",
    ] {
        let value = parse_for_test(raw.as_bytes(), 1).unwrap();
        assert!(value.is_number());
        assert_eq!(value.to_string(), raw);
    }
}

#[test]
fn strict_container_parser_rejects_malformed_delimiters_literals_and_strings() {
    for raw in [
        "",
        " ",
        "{}{}",
        "[1,]",
        "{\"a\":1,}",
        "[1 2]",
        "{\"a\" 1}",
        "{a:1}",
        "[true false]",
        "01",
        "-01",
        "1.",
        "1e+",
        "+1",
        ".5",
        "NaN",
        "trueX",
        "false0",
        "nullx",
        "\"\\uD800\"",
        "{\"a\":1,\"\\u0061\":2}",
        "[}",
        "{]",
        "[] trailing",
    ] {
        assert!(
            parse_for_test(raw.as_bytes(), 100).is_err(),
            "accepted {raw}"
        );
    }
    assert!(parse_for_test(b"\"\xff\"", 100).is_err());
}

#[test]
fn generated_nested_documents_round_trip_without_type_or_field_changes() {
    for index in 0..200 {
        let value = json!({
            "index":index, "array":[null,true,false,index,{"x":index - 100}],
            "nested":{"text":format!("中文🙂 \" \\ \n {index}"),"":[]},
            "a/b":{"~field":index % 7},
            "$serde_json::private::Number":format!("marker-{index}")
        });
        assert_eq!(
            parse_for_test(value.to_string().as_bytes(), 100).unwrap(),
            value
        );
    }
}

#[test]
fn node_and_depth_limits_count_real_values_and_reject_excessive_nesting() {
    let raw = br#"{"number":123456789012345678901234567890}"#;
    assert!(parse_for_test(raw, 2).is_ok());
    assert!(parse_for_test(raw, 1).unwrap_err().contains("max_entries"));
    let allowed = format!("{}0{}", "[".repeat(128), "]".repeat(128));
    assert!(parse_for_test(allowed.as_bytes(), 129).is_ok());
    let excessive = format!("{}0{}", "[".repeat(129), "]".repeat(129));
    assert!(parse_for_test(excessive.as_bytes(), 200)
        .unwrap_err()
        .contains("nesting"));
}

#[test]
fn output_limited_pages_keep_small_children_complete() {
    let f = Fixture::new();
    let original = json!((0..32)
        .map(|i| json!({"id":i,"text":"x".repeat(120)}))
        .collect::<Vec<_>>());
    f.write(&original.to_string());
    let service = f.service(1024);
    let mut req = request("page");
    req.limit = 50;
    let mut recovered = Vec::new();
    loop {
        let page = run(&service, &req).unwrap();
        assert!(page.to_string().len() <= 1024);
        for item in page["items"].as_array().unwrap() {
            assert_eq!(item["value_complete"], true);
            recovered.push(item["value"].clone());
        }
        if page["complete"] == true {
            break;
        }
        let next = page["next_offset"].as_u64().unwrap() as usize;
        assert!(next > req.offset);
        req.offset = next;
        req.expected_revision = page["revision"].as_str().unwrap().into();
    }
    assert_eq!(Value::Array(recovered), original);
}
