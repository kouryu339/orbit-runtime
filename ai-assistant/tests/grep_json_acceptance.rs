use ai_assistant::systems::{
    grep::{GrepConfig, GrepRoot, GrepService},
    grep_json::{document_revision, GrepJsonSystem},
};
use corework::{
    ai_system::{AIInput, AIOutput},
    execution_unit::{ExecutionUnit, UnitType},
    orchestration::Context,
    system::SystemOperation,
    world::FrameworkState,
};
use serde_json::{json, Value};
use std::{collections::HashMap, fs, path::PathBuf, sync::Arc};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let dir =
            std::env::temp_dir().join(format!("grep-json-acceptance-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn write(&self, bytes: &[u8]) {
        fs::write(self.0.join("scene.json"), bytes).unwrap();
    }
    fn context(&self, max_file_bytes: usize) -> (Arc<ExecutionUnit>, Context) {
        let service = Arc::new(
            GrepService::new(
                GrepConfig {
                    roots: vec![GrepRoot {
                        id: "project".into(),
                        path: self.0.clone(),
                    }],
                    max_file_bytes,
                    max_output_bytes: 8192,
                    max_entries: 500_000,
                    ..Default::default()
                },
                None,
            )
            .unwrap(),
        );
        let unit = Arc::new(ExecutionUnit::new_root_in_scope(
            UnitType::Module,
            FrameworkState::initialize().unwrap(),
            format!("json-acceptance-{}", uuid::Uuid::new_v4()),
        ));
        unit.attach_shared_component(service).unwrap();
        let ctx = unit.create_context();
        (unit, ctx)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
async fn call(ctx: &Context, fields: &[(&str, &str)]) -> AIOutput {
    let mut args: HashMap<String, String> = fields
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    args.insert("root_id".into(), "project".into());
    args.insert("path".into(), "scene.json".into());
    GrepJsonSystem
        .execute(AIInput::from_args(args), ctx)
        .await
        .unwrap()
}
fn success(out: AIOutput) -> Value {
    assert_eq!(out.error_code, 0, "{}", out.to_ai);
    assert!(out.to_ai.len() <= 8192);
    assert_eq!(
        serde_json::from_str::<Value>(&out.to_ai).unwrap(),
        out.result
    );
    out.result
}

#[tokio::test]
async fn large_scene_locate_recover_and_reject_stale_pointer() {
    let fixture = Fixture::new();
    let mut scene = json!({"sequences":[{"id":"scene-1","name":"Opening","bindings":
        (0..150).map(|camera| json!({
            "id":format!("camera-{camera:03}"),
            "name":format!("Camera {camera}"),
            "tracks":(0..4).map(|track| json!({
                "id":format!("track-{track}"),
                "frames":(0..64).map(|frame| json!({
                    "frame":frame, "x":frame as f64 / 10.0,
                    "y":-2.5, "z":track, "interpolation":"linear"
                })).collect::<Vec<_>>()
            })).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    }]});
    let bytes = serde_json::to_vec(&scene).unwrap();
    assert!(
        bytes.len() > 2 * 1024 * 1024,
        "fixture must exceed default file limit"
    );
    fixture.write(&bytes);
    let (_default_unit, default_ctx) = fixture.context(2 * 1024 * 1024);
    let denied = call(&default_ctx, &[("action", "read")]).await;
    assert_eq!(denied.error_code, 400);
    assert!(denied.to_ai.contains("max_file_bytes"));
    let (_unit, ctx) = fixture.context(8 * 1024 * 1024);
    let found = success(
        call(
            &ctx,
            &[
                ("action", "search"),
                ("field", "id"),
                ("value_json", "\"camera-149\""),
            ],
        )
        .await,
    );
    let revision = found["revision"].as_str().unwrap();
    assert_eq!(revision, document_revision(&bytes));
    let hit = &found["matches"][0];
    let parent = hit["parent_pointer"].as_str().unwrap();
    assert_eq!(parent, "/sequences/0/bindings/149");
    assert_eq!(hit["ancestors"][0]["identity"]["id"]["value"], "camera-149");
    let camera = success(
        call(
            &ctx,
            &[
                ("action", "read"),
                ("pointer", parent),
                ("expected_revision", revision),
            ],
        )
        .await,
    );
    assert_eq!(camera["value_complete"], false);
    let mut recovered_frames = 0usize;
    let mut calls = 0usize;
    for track in 0..4 {
        let pointer = format!("{parent}/tracks/{track}/frames");
        let mut offset = 0usize;
        let mut recovered = Vec::new();
        loop {
            let offset_arg = offset.to_string();
            let page = success(
                call(
                    &ctx,
                    &[
                        ("action", "page"),
                        ("pointer", &pointer),
                        ("expected_revision", revision),
                        ("offset", &offset_arg),
                        ("limit", "50"),
                    ],
                )
                .await,
            );
            calls += 1;
            for item in page["items"].as_array().unwrap() {
                let item_pointer = item["pointer"].as_str().unwrap();
                assert_eq!(item_pointer, format!("{pointer}/{}", recovered.len()));
                if item["value_complete"] == true {
                    recovered.push(item["value"].clone());
                } else {
                    let full = success(
                        call(
                            &ctx,
                            &[
                                ("action", "read"),
                                ("pointer", item_pointer),
                                ("expected_revision", revision),
                            ],
                        )
                        .await,
                    );
                    calls += 1;
                    assert_eq!(full["value_complete"], true);
                    recovered.push(full["value"].clone());
                }
            }
            if page["complete"] == true {
                break;
            }
            let next = page["next_offset"].as_u64().unwrap() as usize;
            assert!(next > offset);
            offset = next;
        }
        assert_eq!(
            Value::Array(recovered.clone()),
            scene["sequences"][0]["bindings"][149]["tracks"][track]["frames"]
        );
        recovered_frames += recovered.len();
    }
    scene["sequences"][0]["bindings"]
        .as_array_mut()
        .unwrap()
        .swap(0, 149);
    fixture.write(&serde_json::to_vec(&scene).unwrap());
    let stale = call(
        &ctx,
        &[
            ("action", "read"),
            ("pointer", parent),
            ("expected_revision", revision),
        ],
    )
    .await;
    assert_eq!(stale.error_code, 409);
    let refreshed = success(
        call(
            &ctx,
            &[
                ("action", "search"),
                ("field", "id"),
                ("value_json", "\"camera-149\""),
            ],
        )
        .await,
    );
    assert_eq!(
        refreshed["matches"][0]["parent_pointer"],
        "/sequences/0/bindings/0"
    );
    println!(
        "ACCEPTANCE bytes={} recovered_frames={} page_calls={} stale_revision=409",
        bytes.len(),
        recovered_frames,
        calls
    );
}

#[tokio::test]
async fn every_legal_object_key_keeps_its_structure() {
    let fixture = Fixture::new();
    // This is an ordinary legal JSON field, not a numeric literal.
    fixture.write(br#"{"payload":{"$serde_json::private::Number":"123"}}"#);
    let (_unit, ctx) = fixture.context(8 * 1024 * 1024);
    let out = call(&ctx, &[("action", "read")]).await;
    assert_eq!(out.error_code, 0, "{}", out.to_ai);
    assert!(
        out.result["value"]["payload"].is_object(),
        "legal object changed type: {}",
        out.to_ai
    );
    assert_eq!(
        out.result["value"]["payload"]["$serde_json::private::Number"],
        "123"
    );
    assert_eq!(out.result["value_complete"], true);
    // Validate the wire JSON with a typed map so the library's generic Value
    // marker interpretation is not used by the acceptance oracle itself.
    #[derive(serde::Deserialize)]
    struct Envelope {
        value: Payload,
    }
    #[derive(serde::Deserialize)]
    struct Payload {
        payload: HashMap<String, String>,
    }
    let wire: Envelope = serde_json::from_str(&out.to_ai).unwrap();
    assert_eq!(wire.value.payload.len(), 1);
    assert_eq!(wire.value.payload["$serde_json::private::Number"], "123");
}
