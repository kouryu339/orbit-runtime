use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use llm_gateway::{ChatMessage, FunctionDefinition, LlmStreamEvent, ToolDefinition};

fn serve_once(
    body: String,
) -> (
    String,
    std::sync::mpsc::Receiver<serde_json::Value>,
    thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock provider");
    let address = listener.local_addr().expect("mock provider address");
    let (request_tx, request_rx) = std::sync::mpsc::channel();
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
        let request_body =
            serde_json::from_slice(&request[header_end..header_end + content_length])
                .expect("parse provider request body");
        request_tx
            .send(request_body)
            .expect("capture provider request");

        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("write provider response");
        stream.flush().expect("flush provider response");
    });
    (format!("http://{address}"), request_rx, task)
}

fn test_tool() -> ToolDefinition {
    ToolDefinition {
        tool_type: "function".to_string(),
        function: FunctionDefinition {
            name: "ReadScene".to_string(),
            description: "Read the current scene".to_string(),
            parameters: serde_json::json!({"type":"object","properties":{}}),
            strict: None,
        },
    }
}

#[tokio::test]
async fn required_tool_call_rejects_an_empty_tool_set_before_dispatch() {
    let error = llm_gateway::call_llm_with_tools_stream_events_with_requirement_cancellable(
        999_999,
        &[ChatMessage::user("work")],
        &[],
        None,
        None,
        None,
        true,
        tokio_util::sync::CancellationToken::new(),
        |_| {},
    )
    .await
    .expect_err("empty child tool set must fail");
    assert!(error
        .to_string()
        .contains("requires at least one available tool"));
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
                "id": "",
                "function": {"name": "", "arguments": "\"scene.json\"}"}
            }]},
            "finish_reason": "tool_calls"
        }]
    });
    let body = format!("data: {first}\n\ndata: {second}\n\ndata: [DONE]\n\n");
    let (base_url, request, server) = serve_once(body);
    let mut events = Vec::new();
    let tools = [test_tool()];

    let response = llm_gateway::openai_compat::call_inner_streaming(
        &[ChatMessage::user("read scene")],
        &tools,
        "mock-chat",
        &base_url,
        "test-key",
        None,
        None,
        None,
        None,
        llm_gateway::providers::ToolChoiceStyle::ForceName,
        true,
        false,
        false,
        |event| events.push(event),
    )
    .await
    .expect("OpenAI chat stream");
    assert_eq!(
        request.recv().expect("OpenAI request")["tool_choice"],
        "required"
    );
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
async fn openai_chat_rejects_conflicting_non_empty_tool_call_ids() {
    let first = serde_json::json!({
        "choices": [{
            "delta": {"tool_calls": [{
                "index": 0,
                "id": "call-first",
                "function": {"name": "ReadScene", "arguments": "{"}
            }]},
            "finish_reason": null
        }]
    });
    let second = serde_json::json!({
        "choices": [{
            "delta": {"tool_calls": [{
                "index": 0,
                "id": "call-second",
                "function": {"arguments": "}"}
            }]},
            "finish_reason": "tool_calls"
        }]
    });
    let body = format!("data: {first}\n\ndata: {second}\n\ndata: [DONE]\n\n");
    let (base_url, _request, server) = serve_once(body);

    let error = llm_gateway::openai_compat::call_inner_streaming(
        &[ChatMessage::user("read scene")],
        &[test_tool()],
        "mock-chat",
        &base_url,
        "test-key",
        None,
        None,
        None,
        None,
        llm_gateway::providers::ToolChoiceStyle::ForceName,
        true,
        false,
        false,
        |_| {},
    )
    .await
    .expect_err("conflicting tool call ids must fail");
    server.join().expect("mock provider thread");

    let message = error.to_string();
    assert!(message.contains("changed tool call id at index 0"));
    assert!(message.contains("call-first"));
    assert!(message.contains("call-second"));
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
    let (base_url, request, server) = serve_once(body);
    let mut streamed = Vec::new();
    let tools = [test_tool()];

    let response = llm_gateway::anthropic_compat::call_inner_streaming(
        &[ChatMessage::user("read scene")],
        &tools,
        "mock-anthropic",
        &base_url,
        "test-key",
        None,
        None,
        None,
        None,
        true,
        |event| streamed.push(event),
    )
    .await
    .expect("Anthropic stream");
    assert_eq!(
        request.recv().expect("Anthropic request")["tool_choice"],
        serde_json::json!({"type":"any"})
    );
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
    let (base_url, request, server) = serve_once(body);
    let mut streamed = Vec::new();
    let tools = [test_tool()];

    let response = llm_gateway::openai_responses::call_inner_streaming(
        &[ChatMessage::user("read scene")],
        &tools,
        "mock-responses",
        &base_url,
        "test-key",
        None,
        None,
        None,
        None,
        true,
        |event| streamed.push(event),
    )
    .await
    .expect("Responses stream");
    assert_eq!(
        request.recv().expect("Responses request")["tool_choice"],
        "required"
    );
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
    let (base_url, _request, server) = serve_once(format!("data: {event}\n\n"));

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
    let (base_url, _request, server) = serve_once(format!("data: {event}\n\n"));

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
        false,
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
    let (base_url, _request, server) = serve_once(format!("data: {event}\n\n"));

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
        false,
        |_| {},
    )
    .await
    .expect_err("incomplete Responses stream must fail");
    server.join().expect("mock provider thread");

    assert!(error.to_string().contains("response.completed"));
}

#[tokio::test]
async fn model_reasoning_reaches_all_three_http_paradigms_in_both_modes() {
    use llm_gateway::config::{build_index_and_resolver, LlmConfig};
    for (index, paradigm) in [
        "openai_chat_completions",
        "openai_responses",
        "anthropic_messages",
    ]
    .iter()
    .enumerate()
    {
        for streaming in [false, true] {
            let expected = if *paradigm == "anthropic_messages" {
                serde_json::json!({"effort":"high","thinking":"adaptive"})
            } else if *paradigm == "openai_responses" {
                serde_json::json!({"effort":"high","summary":"auto"})
            } else {
                serde_json::json!({"effort":"high"})
            };
            let signed = serde_json::json!({"type":"thinking","thinking":"private analysis","signature":"original-signature"});
            let response = match (*paradigm, streaming) {
                ("openai_chat_completions", false) => serde_json::json!({"choices":[{"message":{"content":"OK"},"finish_reason":"stop"}]}).to_string(),
                ("openai_responses", false) => serde_json::json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"OK"}]}]}).to_string(),
                ("anthropic_messages", false) => serde_json::json!({"content":[signed,{"type":"text","text":"OK"}],"stop_reason":"end_turn"}).to_string(),
                ("openai_chat_completions", true) => "data: {\"choices\":[{\"delta\":{\"content\":\"OK\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".into(),
                ("openai_responses", true) => "data: {\"type\":\"response.output_text.delta\",\"delta\":\"OK\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"OK\"}]}]}}\n\n".into(),
                _ => [
                    serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
                    serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"private analysis"}}),
                    serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"original-signature"}}),
                    serde_json::json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"OK"}}),
                    serde_json::json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}}),
                    serde_json::json!({"type":"message_stop"}),
                ].iter().map(|event| format!("data: {event}\n\n")).collect(),
            };
            let (url, request, server) = serve_once(response);
            let uid = 900_000 + index as u32;
            let config: LlmConfig = serde_json::from_value(serde_json::json!({
                "providers":[{"id":900,"name":"test","type":"openai-gpt","api_key":"test-key",
                    "base_url":url,"api_paradigm":paradigm,
                    "enabled_models":[{"uid":uid,"model_id":"same-custom-alias","reasoning":expected}]}]
            })).unwrap();
            build_index_and_resolver(config);
            let messages = [ChatMessage::user("Hello")];
            let result = if streaming {
                llm_gateway::dispatch::call_llm_with_tools_stream_events_cancellable(
                    uid,
                    &messages,
                    &[],
                    Some(0.7),
                    Some(0.8),
                    Some(4096),
                    tokio_util::sync::CancellationToken::new(),
                    |_| {},
                )
                .await
            } else {
                llm_gateway::call_llm(uid, &messages, Some(0.7), Some(0.8), Some(4096)).await
            }
            .unwrap();
            assert_eq!(result.content, "OK");
            let body = request.recv().unwrap();
            server.join().unwrap();
            assert!(body.get("temperature").is_none() && body.get("top_p").is_none());
            match *paradigm {
                "openai_chat_completions" => {
                    assert_eq!(body["reasoning_effort"], "high");
                    assert!(body.get("reasoning").is_none());
                }
                "openai_responses" => {
                    assert_eq!(
                        body["reasoning"],
                        serde_json::json!({"effort":"high","summary":"auto"})
                    );
                    assert!(body.get("reasoning_effort").is_none());
                }
                _ => {
                    assert_eq!(body["output_config"]["effort"], "high");
                    assert_eq!(body["thinking"]["type"], "adaptive");
                    assert_eq!(result.provider_items.unwrap(), vec![signed]);
                }
            }
        }
    }
    // Same model alias, separate model UIDs: concurrent requests retain their
    // own settings rather than reverse-resolving by name or sharing a task value.
    let answer =
        serde_json::json!({"choices":[{"message":{"content":"OK"},"finish_reason":"stop"}]})
            .to_string();
    let (url_a, request_a, server_a) = serve_once(answer.clone());
    let (url_b, request_b, server_b) = serve_once(answer);
    let config: LlmConfig = serde_json::from_value(serde_json::json!({"providers":[
        {"id":901,"name":"a","type":"openai-gpt","api_key":"test","base_url":url_a,"api_paradigm":"openai_chat_completions",
            "enabled_models":[{"uid":901001,"model_id":"shared-alias","reasoning":{"effort":"low"}}]},
        {"id":902,"name":"b","type":"openai-gpt","api_key":"test","base_url":url_b,"api_paradigm":"openai_chat_completions",
            "enabled_models":[{"uid":902001,"model_id":"shared-alias","reasoning":{"effort":"high"}}]}
    ]})).unwrap();
    build_index_and_resolver(config);
    let messages = [ChatMessage::user("Hello")];
    let (a, b) = tokio::join!(
        llm_gateway::call_llm(901001, &messages, None, None, Some(4096)),
        llm_gateway::call_llm(902001, &messages, None, None, Some(4096))
    );
    assert_eq!(a.unwrap().content, "OK");
    assert_eq!(b.unwrap().content, "OK");
    assert_eq!(request_a.recv().unwrap()["reasoning_effort"], "low");
    assert_eq!(request_b.recv().unwrap()["reasoning_effort"], "high");
    server_a.join().unwrap();
    server_b.join().unwrap();
}
