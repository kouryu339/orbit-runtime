//! Opt-in paid API acceptance. Credentials are read in memory from a host file.
//! Run only with explicit user authorization; never commit keys or raw thinking.
use llm_gateway::config::{build_index_and_resolver, ApiParadigm, LlmConfig};
use llm_gateway::{ChatMessage, FunctionDefinition, LlmResponse, ToolDefinition};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

async fn request(
    messages: &[ChatMessage],
    tools: &[ToolDefinition],
    stream: bool,
) -> Result<LlmResponse, String> {
    let result = tokio::time::timeout(Duration::from_secs(150), async {
        if stream {
            llm_gateway::dispatch::call_llm_with_tools_stream_events_cancellable(
                800001,
                messages,
                tools,
                Some(0.3),
                Some(0.9),
                Some(4096),
                tokio_util::sync::CancellationToken::new(),
                |_| {},
            )
            .await
        } else {
            llm_gateway::call_llm_with_tools(
                800001,
                messages,
                tools,
                Some(0.3),
                Some(0.9),
                Some(4096),
            )
            .await
        }
    })
    .await
    .map_err(|_| "live request timed out".to_string())?;
    result.map_err(|error| error.to_string())
}
fn configure(key: &str, base: &str, model: &str, paradigm: ApiParadigm, reasoning: Option<Value>) {
    let config: LlmConfig = serde_json::from_value(json!({"providers":[{
        "id":800,"name":"Authorized live verification","type":"deepseek","api_key":key,
        "base_url":base,"api_paradigm":paradigm,
        "enabled_models":[{"uid":800001,"model_id":model,"reasoning":reasoning}]
    }]}))
    .unwrap();
    build_index_and_resolver(config);
}
fn facts(response: &LlmResponse) -> Value {
    json!({"finish_reason":response.finish_reason,"tokens":response.tokens,
        "visible_chars":response.content.chars().count(),
        "reasoning_chars":response.reasoning_content.as_ref().map(|v|v.chars().count()).unwrap_or(0),
        "native_items":response.provider_items.as_ref().map(|v|v.len()).unwrap_or(0),
        "signed_thinking_blocks":response.provider_items.as_ref().map(|items|items.iter().filter(|v|v["type"]=="thinking").count()).unwrap_or(0)})
}
fn record(
    rows: &mut Vec<Value>,
    key: &str,
    case: &str,
    paradigm: ApiParadigm,
    stream: bool,
    started: Instant,
    outcome: Result<Value, String>,
) {
    let mut row = json!({"case":case,"paradigm":paradigm,"stream":stream,"elapsed_ms":started.elapsed().as_millis()});
    match outcome {
        Ok(value) => {
            row["pass"] = json!(true);
            row["response"] = value;
        }
        Err(error) => {
            row["pass"] = json!(false);
            row["error"] = json!(error
                .replace(key, "[REDACTED]")
                .chars()
                .take(400)
                .collect::<String>());
        }
    }
    println!("LIVE_RESULT {row}");
    rows.push(row);
}

#[tokio::test]
#[ignore = "requires explicit authorization and host API credentials"]
async fn live_reasoning_compatibility() {
    let path = std::env::var("RUNTIME_REASONING_LIVE_CONFIG").expect("set host config path");
    let output = std::env::var("RUNTIME_REASONING_LIVE_REPORT").expect("set sanitized report path");
    let host: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let provider = host["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["type"] == "deepseek")
        .expect("authorized DeepSeek provider");
    let key = provider["api_key"]
        .as_str()
        .or_else(|| provider["apiKey"].as_str())
        .unwrap();
    let base = provider["base_url"]
        .as_str()
        .or_else(|| provider["baseUrl"].as_str())
        .unwrap_or("https://api.deepseek.com")
        .trim_end_matches('/');
    assert_eq!(
        base, "https://api.deepseek.com",
        "fixture only calls authorized official endpoints"
    );
    let model = provider["enabled_models"].as_array().unwrap()[0]["model_id"]
        .as_str()
        .unwrap();
    let mut rows = Vec::new();
    let phase = std::env::var("RUNTIME_REASONING_LIVE_PHASE").unwrap_or_default();
    if phase != "extended" {
        for paradigm in [
            ApiParadigm::OpenAiChatCompletions,
            ApiParadigm::OpenAiResponses,
            ApiParadigm::AnthropicMessages,
        ] {
            let endpoint = if paradigm == ApiParadigm::AnthropicMessages {
                format!("{base}/anthropic")
            } else {
                base.to_string()
            };
            for (case, reasoning) in [
                ("default", None),
                ("low", Some(json!({"effort":"low"}))),
                ("high", Some(json!({"effort":"high"}))),
            ] {
                let reasoning = reasoning.map(|mut r| {
                    if paradigm == ApiParadigm::AnthropicMessages {
                        r["thinking"] = json!("adaptive");
                    }
                    r
                });
                configure(key, &endpoint, model, paradigm, reasoning);
                for stream in [false, true] {
                    let started = Instant::now();
                    let outcome = request(
                        &[ChatMessage::user(
                            "Compute 17*19. Reply with exactly the number 323 and nothing else.",
                        )],
                        &[],
                        stream,
                    )
                    .await
                    .and_then(|r| {
                        if r.content.trim() != "323" {
                            Err(format!(
                                "incomplete or wrong answer; finish={:?}, chars={}",
                                r.finish_reason,
                                r.content.chars().count()
                            ))
                        } else {
                            Ok(facts(&r))
                        }
                    });
                    record(&mut rows, key, case, paradigm, stream, started, outcome);
                }
            }
            // Exercise the actual complete assistant(tool_calls) -> tool(result)
            // -> assistant pair. The tool returns a private-to-this-fixture marker.
            let reasoning = if paradigm == ApiParadigm::AnthropicMessages {
                json!({"effort":"low","thinking":"adaptive"})
            } else {
                json!({"effort":"low"})
            };
            configure(key, &endpoint, model, paradigm, Some(reasoning));
            let tool = ToolDefinition {
                tool_type: "function".into(),
                function: FunctionDefinition {
                    name: "LiveProbe".into(),
                    description:
                        "Returns the verification code. You must call it to learn the code.".into(),
                    parameters: json!({"type":"object","properties":{},"additionalProperties":false}),
                    strict: None,
                },
            };
            let started = Instant::now();
            let outcome=async {
            let mut messages=vec![ChatMessage::user("Call LiveProbe once, then reply with only the verification code it returns. Do not guess the code.")];
            let first=request(&messages,std::slice::from_ref(&tool),true).await?;
            let calls=first.tool_calls.clone().ok_or("model did not call LiveProbe")?;
            if calls.len()!=1 || calls[0].function.name!="LiveProbe" {return Err("unexpected tool selection".into());}
            serde_json::from_str::<Value>(&calls[0].function.arguments).map_err(|_|"invalid tool argument JSON")?;
            let mut assistant=ChatMessage::assistant(first.content.clone());
            assistant.tool_calls=first.tool_calls.clone();assistant.reasoning_content=first.reasoning_content.clone();assistant.provider_items=first.provider_items.clone();
            messages.push(assistant);
            messages.push(ChatMessage::tool_with_id("ORBIT_LIVE_7281",&calls[0].id,"LiveProbe"));
            let second=request(&messages,std::slice::from_ref(&tool),true).await?;
            if second.content.trim()!="ORBIT_LIVE_7281" || second.tool_calls.is_some() {return Err("tool continuation did not return the verification code".into());}
            Ok(json!({"first":facts(&first),"second":facts(&second),"tool_roundtrip":true}))
        }.await;
            record(
                &mut rows,
                key,
                "tool-roundtrip",
                paradigm,
                true,
                started,
                outcome,
            );
            let extra = match paradigm {
                ApiParadigm::OpenAiChatCompletions => json!({"effort":"none"}),
                ApiParadigm::OpenAiResponses => json!({"effort":"low","summary":"auto"}),
                ApiParadigm::AnthropicMessages => {
                    json!({"thinking":"enabled","budget_tokens":2048})
                }
            };
            configure(key, &endpoint, model, paradigm, Some(extra));
            let started = Instant::now();
            let outcome = request(&[ChatMessage::user("Reply with exactly OK.")], &[], false)
                .await
                .and_then(|r| {
                    if r.content.trim() == "OK" {
                        Ok(facts(&r))
                    } else {
                        Err("supplemental setting returned wrong/incomplete answer".into())
                    }
                });
            record(
                &mut rows,
                key,
                "supplemental-fields",
                paradigm,
                false,
                started,
                outcome,
            );
        }
    }

    // Extended acceptance checks enum values advertised by our generic UI.
    if phase == "extended" {
        for paradigm in [
            ApiParadigm::OpenAiChatCompletions,
            ApiParadigm::OpenAiResponses,
            ApiParadigm::AnthropicMessages,
        ] {
            let endpoint = if paradigm == ApiParadigm::AnthropicMessages {
                format!("{base}/anthropic")
            } else {
                base.to_string()
            };
            for effort in ["minimal", "medium", "xhigh", "max"] {
                if paradigm == ApiParadigm::AnthropicMessages && effort == "minimal" {
                    continue;
                }
                let mut reasoning = json!({"effort":effort});
                if paradigm == ApiParadigm::AnthropicMessages {
                    reasoning["thinking"] = json!("adaptive");
                }
                configure(key, &endpoint, model, paradigm, Some(reasoning));
                let started = Instant::now();
                let outcome = request(
                    &[ChatMessage::user(
                        "Compute 17*19. Reply with exactly 323 and nothing else.",
                    )],
                    &[],
                    true,
                )
                .await
                .and_then(|r| {
                    if r.content.trim() == "323" {
                        Ok(facts(&r))
                    } else {
                        Err("wrong or incomplete enum test answer".into())
                    }
                });
                record(&mut rows, key, effort, paradigm, true, started, outcome);
            }
            let reasoning = if paradigm == ApiParadigm::AnthropicMessages {
                json!({"thinking":"disabled"})
            } else {
                json!({"effort":"none"})
            };
            configure(key, &endpoint, model, paradigm, Some(reasoning));
            let started = Instant::now();
            let outcome = request(&[ChatMessage::user("Reply with exactly OK.")], &[], true)
                .await
                .and_then(|r| {
                    let has_thinking = r.reasoning_content.as_ref().is_some_and(|t| !t.is_empty())
                        || r.provider_items.as_ref().is_some_and(|items| {
                            items.iter().any(|v| {
                                matches!(v["type"].as_str(), Some("thinking" | "reasoning"))
                            })
                        });
                    if r.content.trim() == "OK" && !has_thinking {
                        Ok(facts(&r))
                    } else {
                        Err("disabled thinking returned reasoning or an incomplete answer".into())
                    }
                });
            record(
                &mut rows,
                key,
                "thinking-off",
                paradigm,
                true,
                started,
                outcome,
            );
        }
    }

    let failures = rows.iter().filter(|row| row["pass"] != true).count();
    std::fs::write(output,serde_json::to_string_pretty(&json!({"provider":"DeepSeek official compatible endpoints","model":model,"requests":rows,"failed_cases":failures})).unwrap()).unwrap();
    assert_eq!(
        failures, 0,
        "live cases failed; consult the sanitized report"
    );
}
