use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use llm_gateway::{ChatMessage, LlmStreamEvent};

fn serve_once(body: String) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock provider");
    let address = listener.local_addr().expect("mock provider address");
    let task = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept provider request");
        let mut request = Vec::new();
        let mut buffer = [0u8; 4096];
        let header_end = loop {
            let read = stream.read(&mut buffer).expect("read provider request");
            assert!(read > 0, "request ended before headers");
            request.extend_from_slice(&buffer[..read]);
            if let Some(index) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or_default();
        while request.len().saturating_sub(header_end) < content_length {
            let read = stream
                .read(&mut buffer)
                .expect("read provider request body");
            assert!(read > 0, "request ended before body");
            request.extend_from_slice(&buffer[..read]);
        }

        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("write provider response");
        stream.flush().expect("flush provider response");
    });
    (format!("http://{address}"), task)
}

#[tokio::test]
async fn openai_chat_streams_tool_identity_and_completes_call() {
    let first = serde_json::json!({
        "choices": [{
            "delta": {"tool_calls": [{
                "index": 0,
                "id": "call-chat",
                "function": {"name": "ReadScene", "arguments": "{\"path\":"}
            }]},
            "finish_reason": null
        }]
    });
    let second = serde_json::json!({
        "choices": [{
            "delta": {"tool_calls": [{
                "index": 0,
                "function": {"arguments": "\"scene.json\"}"}
            }]},
            "finish_reason": "tool_calls"
        }]
    });
    let body = format!("data: {first}\n\ndata: {second}\n\ndata: [DONE]\n\n");
    let (base_url, server) = serve_once(body);
    let mut events = Vec::new();

    let response = llm_gateway::openai_compat::call_inner_streaming(
        &[ChatMessage::user("read scene")],
        &[],
        "mock-chat",
        &base_url,
        "test-key",
        None,
        None,
        None,
        None,
        llm_gateway::providers::ToolChoiceStyle::None,
        false,
        false,
        |event| events.push(event),
    )
    .await
    .expect("OpenAI chat stream");
    server.join().expect("mock provider thread");

    let call = response.tool_calls.expect("tool calls").remove(0);
    assert_eq!(call.id, "call-chat");
    assert_eq!(call.function.name, "ReadScene");
    assert_eq!(call.function.arguments, r#"{"path":"scene.json"}"#);
    assert!(events.iter().any(|event| matches!(
        event,
        LlmStreamEvent::ToolCallDelta { call_id: Some(id), name: Some(name), .. }
            if id == "call-chat" && name == "ReadScene"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        LlmStreamEvent::ToolCallCompleted { call_id: Some(id), .. } if id == "call-chat"
    )));
}

#[tokio::test]
async fn anthropic_streams_tool_identity_and_requires_message_stop() {
    let events = [
        serde_json::json!({"type":"message_start","message":{"usage":{"input_tokens":3}}}),
        serde_json::json!({
            "type":"content_block_start",
            "index":0,
            "content_block":{"type":"tool_use","id":"call-anthropic","name":"ReadScene"}
        }),
        serde_json::json!({
            "type":"content_block_delta",
            "index":0,
            "delta":{"type":"input_json_delta","partial_json":"{\"path\":\"scene.json\"}"}
        }),
        serde_json::json!({"type":"content_block_stop","index":0}),
        serde_json::json!({
            "type":"message_delta",
            "delta":{"stop_reason":"tool_use"},
            "usage":{"output_tokens":5}
        }),
        serde_json::json!({"type":"message_stop"}),
    ];
    let body = events
        .into_iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect::<String>();
    let (base_url, server) = serve_once(body);
    let mut streamed = Vec::new();

    let response = llm_gateway::anthropic_compat::call_inner_streaming(
        &[ChatMessage::user("read scene")],
        &[],
        "mock-anthropic",
        &base_url,
        "test-key",
        None,
        None,
        None,
        None,
        |event| streamed.push(event),
    )
    .await
    .expect("Anthropic stream");
    server.join().expect("mock provider thread");

    let call = response.tool_calls.expect("tool calls").remove(0);
    assert_eq!(call.id, "call-anthropic");
    assert_eq!(call.function.arguments, r#"{"path":"scene.json"}"#);
    assert!(streamed.iter().any(|event| matches!(
        event,
        LlmStreamEvent::ToolCallCompleted { call_id: Some(id), .. } if id == "call-anthropic"
    )));
}

#[tokio::test]
async fn responses_streams_tool_identity_and_commits_completed_response() {
    let call = serde_json::json!({
        "type":"function_call",
        "id":"fc-item",
        "call_id":"call-responses",
        "name":"ReadScene",
        "arguments":"{\"path\":\"scene.json\"}"
    });
    let events = [
        serde_json::json!({"type":"response.output_item.added","output_index":0,"item":call}),
        serde_json::json!({
            "type":"response.function_call_arguments.delta",
            "output_index":0,
            "delta":"{\"path\":\"scene.json\"}"
        }),
        serde_json::json!({"type":"response.output_item.done","output_index":0,"item":call}),
        serde_json::json!({
            "type":"response.completed",
            "response":{
                "status":"completed",
                "output":[call],
                "usage":{"input_tokens":4,"output_tokens":6}
            }
        }),
    ];
    let body = events
        .into_iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect::<String>();
    let (base_url, server) = serve_once(body);
    let mut streamed = Vec::new();

    let response = llm_gateway::openai_responses::call_inner_streaming(
        &[ChatMessage::user("read scene")],
        &[],
        "mock-responses",
        &base_url,
        "test-key",
        None,
        None,
        None,
        None,
        |event| streamed.push(event),
    )
    .await
    .expect("Responses stream");
    server.join().expect("mock provider thread");

    let call = response.tool_calls.expect("tool calls").remove(0);
    assert_eq!(call.id, "call-responses");
    assert_eq!(call.function.name, "ReadScene");
    assert!(streamed.iter().any(|event| matches!(
        event,
        LlmStreamEvent::ToolCallCompleted { call_id: Some(id), .. } if id == "call-responses"
    )));
}

#[tokio::test]
async fn openai_chat_rejects_a_stream_without_terminal_evidence() {
    let event = serde_json::json!({
        "choices": [{"delta": {"content": "partial"}, "finish_reason": null}]
    });
    let (base_url, server) = serve_once(format!("data: {event}\n\n"));

    let error = llm_gateway::openai_compat::call_inner_streaming(
        &[ChatMessage::user("hello")],
        &[],
        "mock-chat",
        &base_url,
        "test-key",
        None,
        None,
        None,
        None,
        llm_gateway::providers::ToolChoiceStyle::None,
        false,
        false,
        |_| {},
    )
    .await
    .expect_err("incomplete OpenAI chat stream must fail");
    server.join().expect("mock provider thread");

    assert!(error.to_string().contains("terminal marker"));
}

#[tokio::test]
async fn anthropic_rejects_a_stream_without_message_stop() {
    let event = serde_json::json!({
        "type":"content_block_delta",
        "index":0,
        "delta":{"type":"text_delta","text":"partial"}
    });
    let (base_url, server) = serve_once(format!("data: {event}\n\n"));

    let error = llm_gateway::anthropic_compat::call_inner_streaming(
        &[ChatMessage::user("hello")],
        &[],
        "mock-anthropic",
        &base_url,
        "test-key",
        None,
        None,
        None,
        None,
        |_| {},
    )
    .await
    .expect_err("incomplete Anthropic stream must fail");
    server.join().expect("mock provider thread");

    assert!(error.to_string().contains("message_stop"));
}

#[tokio::test]
async fn responses_rejects_a_stream_without_response_completed() {
    let event = serde_json::json!({
        "type":"response.output_text.delta",
        "output_index":0,
        "delta":"partial"
    });
    let (base_url, server) = serve_once(format!("data: {event}\n\n"));

    let error = llm_gateway::openai_responses::call_inner_streaming(
        &[ChatMessage::user("hello")],
        &[],
        "mock-responses",
        &base_url,
        "test-key",
        None,
        None,
        None,
        None,
        |_| {},
    )
    .await
    .expect_err("incomplete Responses stream must fail");
    server.join().expect("mock provider thread");

    assert!(error.to_string().contains("response.completed"));
}
