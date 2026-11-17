use serde_json::{json, Value};

use super::*;
use crate::providers::{ChatMessage, StreamEvent};

fn base_request() -> ChatCompletionRequest {
    ChatCompletionRequest {
        model: "default".into(),
        messages: vec![],
        temperature: None,
        top_p: None,
        max_tokens: None,
        max_completion_tokens: None,
        stream: None,
        stream_options: None,
        stop: None,
        presence_penalty: None,
        frequency_penalty: None,
        user: None,
        tools: None,
        tool_choice: None,
        response_format: None,
        seed: None,
        reasoning_effort: None,
        template: None,
        template_version: None,
        template_variables: None,
    }
}

fn msg(role: &str, content: Value) -> ChatMessage {
    ChatMessage {
        role: role.into(),
        content,
        name: None,
        tool_call_id: None,
        tool_calls: None,
    }
}

#[test]
fn urls_strip_version_suffixes() {
    assert_eq!(
        generate_url(
            "https://generativelanguage.googleapis.com",
            "gemini-2.5-flash"
        ),
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent"
    );
    assert_eq!(
        generate_url(
            "https://generativelanguage.googleapis.com/v1beta/",
            "gemini-2.5-flash"
        ),
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent"
    );
    assert_eq!(
        stream_url("https://generativelanguage.googleapis.com/", "gemini-2.5-pro"),
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:streamGenerateContent?alt=sse"
    );
}

#[test]
fn request_translation_maps_roles_and_config() {
    let mut req = base_request();
    req.messages = vec![
        msg("system", json!("Be brief.")),
        msg("user", json!([{ "type": "text", "text": "Hello" }])),
        msg("assistant", json!("Hi there")),
        msg("user", json!("Bye")),
    ];
    req.temperature = Some(0.3);
    req.top_p = Some(0.8);
    req.max_tokens = Some(512);
    req.stop = Some(json!(["###"]));

    let body = build_request_body(&req);

    assert_eq!(body["systemInstruction"]["parts"][0]["text"], "Be brief.");

    let contents = body["contents"].as_array().unwrap();
    assert_eq!(contents.len(), 3);
    assert_eq!(contents[0]["role"], "user");
    assert_eq!(contents[0]["parts"][0]["text"], "Hello");
    assert_eq!(contents[1]["role"], "model");
    assert_eq!(contents[1]["parts"][0]["text"], "Hi there");
    assert_eq!(contents[2]["role"], "user");

    assert_eq!(body["generationConfig"]["temperature"], 0.3);
    assert_eq!(body["generationConfig"]["topP"], 0.8);
    assert_eq!(body["generationConfig"]["maxOutputTokens"], 512);
    assert_eq!(body["generationConfig"]["stopSequences"], json!(["###"]));
}

#[test]
fn request_translation_prefers_completion_tokens() {
    let mut req = base_request();
    req.messages = vec![msg("user", json!("hi"))];
    req.max_tokens = Some(100);
    req.max_completion_tokens = Some(200);

    let body = build_request_body(&req);
    assert_eq!(body["generationConfig"]["maxOutputTokens"], 200);
    assert!(body.get("systemInstruction").is_none());

    req.max_tokens = None;
    req.max_completion_tokens = None;
    let body = build_request_body(&req);
    assert!(body.get("generationConfig").is_none());
}

#[test]
fn response_translation_joins_parts_and_maps_finish() {
    let value = json!({
        "candidates": [{
            "content": {
                "role": "model",
                "parts": [{ "text": "Hello " }, { "text": "world" }]
            },
            "finishReason": "STOP",
            "index": 0
        }],
        "usageMetadata": {
            "promptTokenCount": 9,
            "candidatesTokenCount": 4,
            "totalTokenCount": 13
        }
    });

    let response = translate_response(&value, "gemini-2.5-flash").unwrap();
    assert_eq!(response.object, "chat.completion");
    assert_eq!(response.model, "gemini-2.5-flash");
    assert_eq!(response.choices[0].message.content, json!("Hello world"));
    assert_eq!(response.choices[0].finish_reason.as_deref(), Some("stop"));

    let usage = response.usage.unwrap();
    assert_eq!(usage.prompt_tokens, 9);
    assert_eq!(usage.completion_tokens, 4);
    assert_eq!(usage.total_tokens, 13);
}

#[test]
fn finish_reason_mapping() {
    assert_eq!(map_finish_reason(Some("STOP")).as_deref(), Some("stop"));
    assert_eq!(
        map_finish_reason(Some("MAX_TOKENS")).as_deref(),
        Some("length")
    );
    assert_eq!(
        map_finish_reason(Some("SAFETY")).as_deref(),
        Some("content_filter")
    );
    assert_eq!(map_finish_reason(None), None);
}

#[test]
fn blocked_prompt_reports_protocol_error() {
    let value = json!({
        "promptFeedback": { "blockReason": "SAFETY" }
    });

    match translate_response(&value, "gemini-2.5-flash") {
        Err(UpstreamCallError::Protocol(message)) => {
            assert!(message.contains("SAFETY"), "got {message}")
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn sse_translation_streams_text_then_usage() {
    let mut translator = StreamTranslator::new("gemini-2.5-flash");

    let first = translator
        .translate(r#"{"candidates":[{"content":{"role":"model","parts":[{"text":"Hel"}]}}]}"#)
        .unwrap()
        .unwrap();

    match first {
        StreamEvent::Chunk {
            data,
            delta_text,
            usage,
            finish_reason,
        } => {
            assert_eq!(data["object"], "chat.completion.chunk");
            assert_eq!(data["model"], "gemini-2.5-flash");
            assert_eq!(data["choices"][0]["delta"]["content"], "Hel");
            assert_eq!(delta_text.as_deref(), Some("Hel"));
            assert!(usage.is_none());
            assert!(finish_reason.is_none());
        }
        other => panic!("unexpected: {other:?}"),
    }

    let last = translator
        .translate(
            r#"{"candidates":[{"content":{"role":"model","parts":[{"text":"lo"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":2,"totalTokenCount":7}}"#,
        )
        .unwrap()
        .unwrap();

    match last {
        StreamEvent::Chunk {
            data,
            delta_text,
            usage,
            finish_reason,
        } => {
            assert_eq!(delta_text.as_deref(), Some("lo"));
            assert_eq!(finish_reason.as_deref(), Some("stop"));
            assert_eq!(data["choices"][0]["finish_reason"], "stop");
            assert_eq!(data["usage"]["total_tokens"], 7);
            assert_eq!(usage.unwrap().prompt_tokens, 5);
        }
        other => panic!("unexpected: {other:?}"),
    }

    assert!(translator.translate("").is_none());
    assert!(matches!(
        translator.translate("[DONE]").unwrap().unwrap(),
        StreamEvent::Done
    ));
    assert!(translator.is_done());
}

#[test]
fn sse_translation_reports_errors_and_bad_json() {
    let mut translator = StreamTranslator::new("gemini-2.5-flash");

    match translator
        .translate(r#"{"error":{"code":429,"message":"Quota exceeded"}}"#)
        .unwrap()
    {
        Err(UpstreamCallError::Protocol(message)) => {
            assert!(message.contains("Quota exceeded"), "got {message}")
        }
        other => panic!("unexpected: {other:?}"),
    }
    assert!(translator.is_done());

    let mut translator = StreamTranslator::new("gemini-2.5-flash");
    assert!(matches!(
        translator.translate("{oops").unwrap(),
        Err(UpstreamCallError::Protocol(_))
    ));
}

#[test]
fn parts_translate_inline_and_file_images() {
    let parts = translate_parts(&json!([
        { "type": "text", "text": "what is this?" },
        { "type": "image_url", "image_url": { "url": "data:image/png;base64,QUJD" } },
        { "type": "image_url", "image_url": { "url": "https://example.com/a.webp" } }
    ]));

    assert_eq!(parts[0]["text"], "what is this?");
    assert_eq!(parts[1]["inlineData"]["mimeType"], "image/png");
    assert_eq!(parts[1]["inlineData"]["data"], "QUJD");
    assert_eq!(
        parts[2]["fileData"]["fileUri"],
        "https://example.com/a.webp"
    );
    assert_eq!(parts[2]["fileData"]["mimeType"], "image/webp");
}

#[test]
fn tools_and_tool_history_translate() {
    let mut req = base_request();
    req.messages = vec![
        msg("user", json!("weather in Paris?")),
        ChatMessage {
            role: "assistant".into(),
            content: json!("Checking"),
            name: None,
            tool_call_id: None,
            tool_calls: Some(json!([
                {
                    "id": "call_1",
                    "type": "function",
                    "function": { "name": "get_weather", "arguments": "{\"city\":\"Paris\"}" }
                }
            ])),
        },
        ChatMessage {
            role: "tool".into(),
            content: json!("sunny"),
            name: None,
            tool_call_id: Some("call_1".into()),
            tool_calls: None,
        },
    ];
    req.tools = Some(json!([
        {
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Look up weather",
                "parameters": { "type": "object", "properties": { "city": { "type": "string" } } }
            }
        }
    ]));
    req.tool_choice = Some(json!("required"));

    let body = build_request_body(&req);
    let contents = body["contents"].as_array().unwrap();
    assert_eq!(contents[1]["role"], "model");
    assert_eq!(contents[1]["parts"][0]["text"], "Checking");
    assert_eq!(
        contents[1]["parts"][1]["functionCall"]["name"],
        "get_weather"
    );
    assert_eq!(
        contents[1]["parts"][1]["functionCall"]["args"]["city"],
        "Paris"
    );
    assert_eq!(contents[2]["role"], "user");
    assert_eq!(
        contents[2]["parts"][0]["functionResponse"]["name"],
        "get_weather"
    );
    assert_eq!(
        contents[2]["parts"][0]["functionResponse"]["response"]["result"],
        "sunny"
    );

    let declarations = body["tools"][0]["functionDeclarations"].as_array().unwrap();
    assert_eq!(declarations[0]["name"], "get_weather");
    assert_eq!(declarations[0]["description"], "Look up weather");
    assert_eq!(declarations[0]["parameters"]["type"], "object");
    assert_eq!(body["toolConfig"]["functionCallingConfig"]["mode"], "ANY");
}

#[test]
fn tool_choice_modes_and_allowed_functions() {
    let mut req = base_request();
    req.messages = vec![msg("user", json!("hi"))];
    req.tools = Some(json!([
        { "type": "function", "function": { "name": "f", "parameters": { "type": "object" } } }
    ]));

    req.tool_choice = Some(json!("auto"));
    assert_eq!(
        build_request_body(&req)["toolConfig"]["functionCallingConfig"]["mode"],
        "AUTO"
    );

    req.tool_choice = Some(json!("none"));
    assert_eq!(
        build_request_body(&req)["toolConfig"]["functionCallingConfig"]["mode"],
        "NONE"
    );

    req.tool_choice = Some(json!({ "type": "function", "function": { "name": "f" } }));
    let body = build_request_body(&req);
    assert_eq!(body["toolConfig"]["functionCallingConfig"]["mode"], "ANY");
    assert_eq!(
        body["toolConfig"]["functionCallingConfig"]["allowedFunctionNames"],
        json!(["f"])
    );
}

#[test]
fn response_format_and_thinking_land_in_generation_config() {
    let mut req = base_request();
    req.messages = vec![msg("user", json!("hi"))];

    req.response_format = Some(json!({ "type": "json_object" }));
    let body = build_request_body(&req);
    assert_eq!(
        body["generationConfig"]["responseMimeType"],
        "application/json"
    );
    assert!(body["generationConfig"].get("responseSchema").is_none());

    req.response_format = Some(json!({
        "type": "json_schema",
        "json_schema": { "name": "out", "schema": { "type": "object" } }
    }));
    let body = build_request_body(&req);
    assert_eq!(body["generationConfig"]["responseSchema"]["type"], "object");

    for (effort, budget) in [("none", 0), ("low", 1024), ("medium", 2048), ("high", 4096)] {
        req.reasoning_effort = Some(effort.into());
        let body = build_request_body(&req);
        assert_eq!(
            body["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            budget
        );
    }
}

#[test]
fn function_calls_translate_in_responses_and_streams() {
    let value = json!({
        "candidates": [{
            "content": {
                "role": "model",
                "parts": [{ "functionCall": { "name": "get_weather", "args": { "city": "Paris" } } }]
            },
            "finishReason": "STOP"
        }]
    });

    let response = translate_response(&value, "gemini-2.5-flash").unwrap();
    assert_eq!(
        response.choices[0].finish_reason.as_deref(),
        Some("tool_calls")
    );
    let calls = response.choices[0].message.tool_calls.clone().unwrap();
    assert_eq!(calls[0]["type"], "function");
    assert_eq!(calls[0]["function"]["name"], "get_weather");
    assert_eq!(calls[0]["function"]["arguments"], "{\"city\":\"Paris\"}");
    assert!(calls[0]["id"].as_str().unwrap().starts_with("call_0_"));

    let mut translator = StreamTranslator::new("gemini-2.5-flash");
    let chunk = translator
        .translate(
            r#"{"candidates":[{"content":{"role":"model","parts":[{"functionCall":{"name":"get_weather","args":{"city":"Paris"}}}]},"finishReason":"STOP"}]}"#,
        )
        .unwrap()
        .unwrap();

    match chunk {
        StreamEvent::Chunk {
            data,
            finish_reason,
            ..
        } => {
            assert_eq!(finish_reason.as_deref(), Some("tool_calls"));
            assert_eq!(
                data["choices"][0]["delta"]["tool_calls"][0]["function"]["name"],
                "get_weather"
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn embedding_urls_bodies_and_responses() {
    assert_eq!(
        embedding_url(
            "https://generativelanguage.googleapis.com/v1beta",
            "text-embedding-004",
            false
        ),
        "https://generativelanguage.googleapis.com/v1beta/models/text-embedding-004:embedContent"
    );
    assert_eq!(
        embedding_url(
            "https://generativelanguage.googleapis.com",
            "text-embedding-004",
            true
        ),
        "https://generativelanguage.googleapis.com/v1beta/models/text-embedding-004:batchEmbedContents"
    );
    assert_eq!(
        decisions_url("https://generativelanguage.googleapis.com/v1beta/"),
        "https://generativelanguage.googleapis.com/v1beta/decisions"
    );

    let req = EmbeddingRequest {
        model: "alias".into(),
        input: json!(vec!["a".to_string(), "b".to_string()]),
        encoding_format: None,
        dimensions: Some(8),
        user: None,
    };
    let inputs = embedding_inputs(&req.input).unwrap();
    assert_eq!(inputs.len(), 2);

    let body = build_embedding_body("text-embedding-004", &inputs, &req, true);
    assert_eq!(body["requests"][0]["model"], "models/text-embedding-004");
    assert_eq!(body["requests"][1]["content"]["parts"][0]["text"], "b");
    assert_eq!(body["requests"][0]["outputDimensionality"], 8);

    let single = build_embedding_body("m", &["a".to_string()], &req, false);
    assert_eq!(single["content"]["parts"][0]["text"], "a");

    let response = parse_embedding_response(
        &json!({ "embeddings": [{ "values": [0.1, 0.2] }, { "values": [0.3, 0.4] }] }),
        "text-embedding-004",
    )
    .unwrap();
    assert_eq!(response.object, "list");
    assert_eq!(response.data.len(), 2);
    assert_eq!(response.data[1].index, 1);
    assert_eq!(response.data[1].embedding, json!([0.3, 0.4]));

    let single = parse_embedding_response(
        &json!({ "embedding": { "values": [0.5] } }),
        "text-embedding-004",
    )
    .unwrap();
    assert_eq!(single.data.len(), 1);

    assert!(parse_embedding_response(&json!({}), "m").is_err());
}

#[test]
fn embedding_inputs_reject_token_arrays() {
    match embedding_inputs(&json!([1, 2, 3])) {
        Err(UpstreamCallError::Status { status, .. }) => assert_eq!(status, 400),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn trailing_model_turn_is_dropped() {
    let mut req = base_request();
    req.messages = vec![
        msg("user", json!("Translate hello")),
        msg("assistant", json!("Translation:")),
    ];

    let body = build_request_body(&req);

    let contents = body["contents"].as_array().expect("contents");
    assert_eq!(
        contents.len(),
        1,
        "trailing model turn must go: {contents:?}"
    );
    assert_eq!(contents[0]["role"], json!("user"));
}

#[test]
fn consecutive_user_turns_merge() {
    let mut req = base_request();
    req.messages = vec![msg("user", json!("one")), msg("user", json!("two"))];

    let body = build_request_body(&req);

    let contents = body["contents"].as_array().expect("contents");
    assert_eq!(contents.len(), 1, "roles must alternate: {contents:?}");
    assert_eq!(contents[0]["parts"].as_array().expect("parts").len(), 2);
}

#[test]
fn consecutive_model_turns_merge() {
    let mut req = base_request();
    req.messages = vec![
        msg("user", json!("q")),
        msg("assistant", json!("a")),
        msg("assistant", json!("b")),
        msg("user", json!("next")),
    ];

    let body = build_request_body(&req);

    let roles: Vec<_> = body["contents"]
        .as_array()
        .expect("contents")
        .iter()
        .map(|content| content["role"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(roles, ["user", "model", "user"], "no duplicate model turn");
}
