//! Tests field-path access for nested workflow data.

use corework::{
    cache::CacheExt, workflow::blueprint_loader::BlueprintLoader, world::FrameworkState,
};
use serde_json::json;

#[tokio::test]
async fn test_field_path_access() -> corework::error::Result<()> {
    let framework = FrameworkState::initialize()?;
    let context = framework.create_context();

    test_data_value_field_path()?;
    test_cache_field_path(&context).await?;
    test_json_blueprint_with_field_path(&context).await?;

    Ok(())
}

fn test_data_value_field_path() -> corework::error::Result<()> {
    use corework::workflow::core::DataValue;

    let sub_question_json = json!({
        "sub_id": "Q1",
        "question_type": "SingleBlank",
        "full_score": 10.0,
        "answer_config": {
            "type": "TextAnswer",
            "keywords": ["expected answer"]
        }
    });

    let data_value = DataValue::new("SubQuestion", sub_question_json);

    let sub_id = data_value
        .get_field_path("sub_id")
        .expect("sub_id should be readable");
    assert_eq!(sub_id.value, json!("Q1"));

    let answer_type = data_value
        .get_field_path("answer_config.type")
        .expect("answer_config.type should be readable");
    assert_eq!(answer_type.value, json!("TextAnswer"));

    let mut mutable_data = DataValue::new("Object", json!({}));
    mutable_data.set_field_path("sub_question.sub_id", DataValue::new("String", json!("Q2")))?;
    mutable_data.set_field_path("sub_question.score", DataValue::new("Float", json!(8.5)))?;

    assert_eq!(
        mutable_data
            .get_field_path("sub_question.sub_id")
            .expect("sub_question.sub_id should be set")
            .value,
        json!("Q2")
    );
    assert_eq!(
        mutable_data
            .get_field_path("sub_question.score")
            .expect("sub_question.score should be set")
            .value,
        json!(8.5)
    );

    Ok(())
}

async fn test_cache_field_path(
    ctx: &corework::orchestration::Context,
) -> corework::error::Result<()> {
    let reference_answer = json!({
        "question_id": "T1",
        "first_question": {
            "sub_id": "Q1",
            "question_type": "SingleBlank",
            "full_score": 10.0
        },
        "second_question": {
            "sub_id": "Q2",
            "question_type": "MultipleBlank",
            "full_score": 15.0
        }
    });

    ctx.cache
        .set_raw("test_reference", reference_answer.clone(), None)
        .await?;

    let sub_id: String = ctx
        .cache
        .get_field("test_reference", "first_question.sub_id")
        .await?
        .expect("first_question.sub_id should exist");
    assert_eq!(sub_id, "Q1");

    let full_score: f64 = ctx
        .cache
        .get_field("test_reference", "second_question.full_score")
        .await?
        .expect("second_question.full_score should exist");
    assert_eq!(full_score, 15.0);

    ctx.cache
        .set_field("test_reference", "first_question.full_score", &12.0)
        .await?;

    let updated_score: f64 = ctx
        .cache
        .get_field("test_reference", "first_question.full_score")
        .await?
        .expect("first_question.full_score should remain readable");
    assert_eq!(updated_score, 12.0);

    Ok(())
}

async fn test_json_blueprint_with_field_path(
    ctx: &corework::orchestration::Context,
) -> corework::error::Result<()> {
    let blueprint_json = json!({
        "version": "1.0",
        "metadata": {
            "name": "FieldPathTest",
            "description": "Field-path access test"
        },
        "nodes": [
            {
                "id": "start_1",
                "node_type": "StartNode",
                "display_name": "Start",
                "position": {"x": 100, "y": 200},
                "pins": [{"name": "Out", "kind": "ExecOutput"}]
            },
            {
                "id": "end_1",
                "node_type": "EndNode",
                "display_name": "End",
                "position": {"x": 400, "y": 200},
                "pins": [{"name": "In", "kind": "ExecInput"}]
            }
        ],
        "connections": [
            {
                "from_node": "start_1",
                "from_pin": "Out",
                "to_node": "end_1",
                "to_pin": "In",
                "connection_type": "Exec"
            }
        ]
    });

    let json_str = serde_json::to_string(&blueprint_json)?;
    let loader = BlueprintLoader::new();
    let workflow = loader.load_workflow_from_json_str(&json_str).await?;
    assert_eq!(workflow.name(), "FieldPathTest");

    let test_data = json!({
        "user": {
            "name": "Zhang San",
            "age": 25,
            "address": {
                "city": "Beijing",
                "street": "Zhongguancun"
            }
        },
        "score": 95.5
    });

    ctx.cache.set_raw("workflow_test", test_data, None).await?;

    let city: String = ctx
        .cache
        .get_field("workflow_test", "user.address.city")
        .await?
        .expect("user.address.city should exist");

    assert_eq!(city, "Beijing");

    Ok(())
}
