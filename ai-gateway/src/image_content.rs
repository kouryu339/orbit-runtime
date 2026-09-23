use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::types::{ChatMessage, MessagePart};
use crate::{ApiError, Result};

fn data_url(part: &MessagePart) -> Result<String> {
    let MessagePart::Image {
        image_id,
        mime_type,
        path,
        sha256,
    } = part
    else {
        return Err(ApiError::LlmFailed("expected image part".into()));
    };
    if std::fs::metadata(path)
        .map(|meta| meta.len())
        .unwrap_or(u64::MAX)
        > 20 * 1024 * 1024
    {
        return Err(ApiError::LlmFailed(format!(
            "image {image_id} is unavailable or exceeds 20 MiB"
        )));
    }
    let bytes = std::fs::read(path)
        .map_err(|e| ApiError::LlmFailed(format!("image {image_id} is unavailable: {e}")))?;
    if bytes.len() > 20 * 1024 * 1024 {
        return Err(ApiError::LlmFailed(format!(
            "image {image_id} exceeds 20 MiB"
        )));
    }
    let digest = format!("{:x}", Sha256::digest(&bytes));
    if &digest != sha256 {
        return Err(ApiError::LlmFailed(format!(
            "image {image_id} checksum mismatch"
        )));
    }
    Ok(format!(
        "data:{mime_type};base64,{}",
        STANDARD.encode(bytes)
    ))
}

pub(crate) fn openai_chat_content(message: &ChatMessage) -> Result<Value> {
    if message.parts.is_empty() {
        return Ok(json!(message.content));
    }
    let mut blocks = Vec::with_capacity(message.parts.len());
    for part in &message.parts {
        blocks.push(match part {
            MessagePart::Text { text } => json!({"type":"text", "text":text}),
            MessagePart::Image { .. } => {
                json!({"type":"image_url", "image_url":{"url":data_url(part)?}})
            }
        });
    }
    Ok(Value::Array(blocks))
}

pub(crate) fn responses_content(message: &ChatMessage) -> Result<Value> {
    if message.parts.is_empty() {
        return Ok(json!(message.content));
    }
    let mut blocks = Vec::with_capacity(message.parts.len());
    for part in &message.parts {
        blocks.push(match part {
            MessagePart::Text { text } => json!({"type":"input_text", "text":text}),
            MessagePart::Image { .. } => json!({"type":"input_image", "image_url":data_url(part)?}),
        });
    }
    Ok(Value::Array(blocks))
}

pub(crate) fn anthropic_blocks(message: &ChatMessage) -> Result<Vec<Value>> {
    let mut blocks = Vec::with_capacity(message.parts.len());
    for part in &message.parts {
        blocks.push(match part {
            MessagePart::Text { text } => json!({"type":"text", "text":text}),
            MessagePart::Image { mime_type, .. } => {
                let url = data_url(part)?;
                let encoded = url.split_once(',').map(|(_, value)| value).unwrap_or_default();
                json!({"type":"image", "source":{"type":"base64", "media_type":mime_type, "data":encoded}})
            }
        });
    }
    Ok(blocks)
}

pub(crate) fn validate_model(model: &str, messages: &[ChatMessage]) -> Result<()> {
    if !messages.iter().any(|message| {
        message
            .parts
            .iter()
            .any(|part| matches!(part, MessagePart::Image { .. }))
    }) {
        return Ok(());
    }
    if let Some(definition) = crate::config::find_model(model) {
        if !definition
            .input_modal
            .contains(&crate::config::Modal::Image)
        {
            return Err(ApiError::LlmFailed(format!(
                "model {model} does not accept image input"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_projects_to_each_provider_and_detects_missing_media() {
        let path = std::env::temp_dir().join(format!(
            "orbit-image-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"image bytes").unwrap();
        let digest = image_digest_for_test(b"image bytes");
        let mut message = ChatMessage::user("what is this?");
        message.parts = vec![
            MessagePart::Text {
                text: "what is this?".into(),
            },
            MessagePart::Image {
                image_id: digest.clone(),
                mime_type: "image/png".into(),
                path: path.to_string_lossy().into_owned(),
                sha256: digest,
            },
        ];
        let chat = openai_chat_content(&message).unwrap();
        assert_eq!(chat[1]["type"], "image_url");
        assert!(chat[1]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
        let responses = responses_content(&message).unwrap();
        assert_eq!(responses[1]["type"], "input_image");
        let anthropic = anthropic_blocks(&message).unwrap();
        assert_eq!(anthropic[1]["source"]["type"], "base64");
        assert!(validate_model("deepseek-v4-pro", &[message.clone()]).is_err());
        assert!(validate_model("gpt-5.5", &[message.clone()]).is_ok());
        std::fs::remove_file(&path).unwrap();
        assert!(openai_chat_content(&message).is_err());
    }

    fn image_digest_for_test(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }
}
