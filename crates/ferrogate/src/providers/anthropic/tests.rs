use serde_json::{json, Value};

use super::*;
use crate::providers::{ChatCompletionRequest, ChatMessage};

fn msg(role: &str, content: Value, tool_calls: Option<Value>) -> ChatMessage {
    ChatMessage {
        role: role.into(),
        content,
        name: None,
        tool_call_id: None,
        tool_calls,
    }
}

fn reply(id: &str, content: Value) -> ChatMessage {
    ChatMessage {
        tool_call_id: Some(id.into()),
        ..msg("tool", content, None)
    }
}

fn base_request(messages: Vec<ChatMessage>) -> ChatCompletionRequest {
    ChatCompletionRequest {
        model: "smart".into(),
        messages,
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

fn translated(messages: Vec<ChatMessage>) -> Value {
    build_request_body(&base_request(messages), "claude-sonnet-4-20250514")["messages"].clone()
}

fn openai_call(id: &str, name: &str) -> Value {
    json!({ "id": id, "type": "function", "function": { "name": name, "arguments": "{}" } })
}

fn text(value: &str) -> Value {
    json!({ "type": "text", "text": value })
}

fn user(content: &str) -> ChatMessage {
    msg("user", json!(content), None)
}

#[test]
fn messages_url_does_not_duplicate_v1() {
    for base in ["https://api.anthropic.com", "https://api.anthropic.com/v1"] {
        assert_eq!(messages_url(base), "https://api.anthropic.com/v1/messages");
    }
    assert_eq!(
        messages_url("https://api.anthropic.com/"),
        "https://api.anthropic.com/v1/messages"
    );
}

#[test]
fn request_translation_lifts_system_and_maps_fields() {
    let mut req = base_request(vec![
        msg("system", json!("You are terse."), None),
        msg("system", json!([text("Answer in French.")]), None),
        user("Bonjour"),
        msg("assistant", json!([text("Salut!")]), None),
        user("one"),
        user("two"),
    ]);
    req.temperature = Some(0.4);
    req.top_p = Some(0.9);
    req.max_tokens = Some(256);
    req.stop = Some(json!(["STOP", "END"]));
    req.stream = Some(true);

    assert_eq!(
        build_request_body(&req, "claude-3-5-haiku-latest"),
        json!({
            "model": "claude-3-5-haiku-latest",
            "max_tokens": 256,
            "temperature": 0.4,
            "top_p": 0.9,
            "stream": true,
            "stop_sequences": ["STOP", "END"],
            "system": "You are terse.\n\nAnswer in French.",
            "messages": [
                { "role": "user", "content": "Bonjour" },
                { "role": "assistant", "content": [text("Salut!")] },
                { "role": "user", "content": "one\n\ntwo" }
            ]
        })
    );

    req.max_tokens = None;
    req.max_completion_tokens = Some(2_000);
    assert_eq!(build_request_body(&req, "m")["max_tokens"], 2_000);
    req.max_completion_tokens = None;
    assert_eq!(build_request_body(&req, "m")["max_tokens"], 4096);
}

#[test]
fn response_translation_maps_content_stop_reason_and_usage() {
    let response = translate_response(&json!({
        "id": "msg_123",
        "model": "claude-3-5-haiku-latest",
        "content": [text("Hello"), text(" world")],
        "stop_reason": "end_turn",
        "usage": { "input_tokens": 11, "output_tokens": 5 }
    }))
    .unwrap();

    assert_eq!(
        (response.id.as_str(), response.object.as_str()),
        ("msg_123", "chat.completion")
    );
    assert_eq!(response.model, "claude-3-5-haiku-latest");
    assert_eq!(response.choices[0].message.role, "assistant");
    assert_eq!(response.choices[0].message.content, json!("Hello world"));
    assert!(response.choices[0].message.tool_calls.is_none());
    assert_eq!(response.choices[0].finish_reason.as_deref(), Some("stop"));

    let usage = response.usage.unwrap();
    assert_eq!(
        (
            usage.prompt_tokens,
            usage.completion_tokens,
            usage.total_tokens
        ),
        (11, 5, 16)
    );
}

#[test]
fn response_translation_maps_tool_use() {
    let response = translate_response(&json!({
        "id": "msg_tool",
        "model": "claude-sonnet-4-20250514",
        "content": [text("Checking"), {
            "type": "tool_use", "id": "toolu_1", "name": "get_weather", "input": { "city": "Paris" }
        }],
        "stop_reason": "tool_use",
        "usage": { "input_tokens": 3, "output_tokens": 9 }
    }))
    .unwrap();

    assert_eq!(
        response.choices[0].finish_reason.as_deref(),
        Some("tool_calls")
    );
    assert_eq!(
        response.choices[0].message.tool_calls.clone().unwrap(),
        json!([{ "id": "toolu_1", "type": "function", "function": {
            "name": "get_weather", "arguments": "{\"city\":\"Paris\"}"
        } }])
    );
}

#[test]
fn stop_reason_mapping_covers_all_variants() {
    for (reason, expected) in [
        ("end_turn", "stop"),
        ("max_tokens", "length"),
        ("stop_sequence", "stop"),
        ("tool_use", "tool_calls"),
        ("pause_turn", "content_filter"),
        ("refusal", "content_filter"),
        ("model_context_window_exceeded", "length"),
        ("custom", "stop"),
    ] {
        assert_eq!(map_stop_reason(Some(reason)).as_deref(), Some(expected));
    }
    assert_eq!(map_stop_reason(None), None);
}

#[test]
fn content_translation_keeps_text_cache_control_and_maps_images() {
    let cached = json!([
        { "type": "text", "text": "cached", "cache_control": { "type": "ephemeral" } },
        text("plain")
    ]);
    assert_eq!(translate_content(&cached), cached);
    assert_eq!(translate_content(&json!("plain")), json!("plain"));
    assert_eq!(translate_content(&Value::Null), json!(""));

    assert_eq!(
        translate_content(&json!([
            { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAAB" } },
            { "type": "image_url", "image_url": { "url": "https://example.com/cat.jpg" } }
        ])),
        json!([
            { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "AAAB" } },
            { "type": "image", "source": { "type": "url", "url": "https://example.com/cat.jpg" } }
        ])
    );
}

#[test]
fn assistant_tool_calls_become_tool_use_blocks() {
    let call = json!([{ "id": "call_1", "type": "function", "function": {
        "name": "get_weather", "arguments": "{\"city\":\"Paris\"}"
    } }]);
    let messages = translated(vec![
        user("weather?"),
        msg("assistant", json!("Checking"), Some(call)),
    ]);

    assert_eq!(
        messages[1]["content"],
        json!([text("Checking"), {
            "type": "tool_use",
            "id": "call_1",
            "name": "get_weather",
            "input": { "city": "Paris" }
        }])
    );
}

#[test]
fn tool_messages_become_merged_tool_results() {
    let messages = translated(vec![
        msg(
            "assistant",
            json!(""),
            Some(json!([openai_call("call_1", "a")])),
        ),
        reply("call_1", json!("sunny")),
        reply("call_2", json!("20C")),
    ]);
    let messages = messages.as_array().unwrap();

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1]["role"], "user");
    assert_eq!(
        messages[1]["content"],
        json!([
            { "type": "tool_result", "tool_use_id": "call_1", "content": "sunny" },
            { "type": "tool_result", "tool_use_id": "call_2", "content": "20C" }
        ])
    );

    assert_eq!(
        translated(vec![reply("call_1", json!("sunny"))]),
        json!([{ "role": "user", "content": [
            { "type": "tool_result", "tool_use_id": "call_1", "content": "sunny" }
        ] }])
    );
}

#[test]
fn tools_tool_choice_and_response_format_are_translated() {
    let mut req = base_request(vec![user("hi")]);
    req.tools = Some(json!([{ "type": "function", "function": {
        "name": "get_weather",
        "description": "Look up weather",
        "parameters": { "type": "object", "properties": { "city": { "type": "string" } } }
    } }]));
    req.tool_choice = Some(json!({ "type": "function", "function": { "name": "get_weather" } }));
    req.response_format = Some(json!({
        "type": "json_schema",
        "json_schema": { "schema": { "type": "object", "properties": {} } }
    }));

    let body = build_request_body(&req, "claude-sonnet-4-20250514");
    assert_eq!(body["tools"][0]["name"], "get_weather");
    assert_eq!(body["tools"][0]["description"], "Look up weather");
    assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
    assert_eq!(
        body["tool_choice"],
        json!({ "type": "tool", "name": "get_weather" })
    );
    assert_eq!(body["output_config"]["format"]["type"], "json_schema");
    assert_eq!(body["output_config"]["format"]["schema"]["type"], "object");

    for (mode, expected) in [("auto", "auto"), ("required", "any")] {
        req.tool_choice = Some(json!(mode));
        assert_eq!(
            build_request_body(&req, "m")["tool_choice"]["type"],
            expected
        );
    }

    req.tool_choice = Some(json!("none"));
    assert!(build_request_body(&req, "m").get("tool_choice").is_none());

    req.tool_choice = None;
    req.tools = None;
    req.response_format = Some(json!({ "type": "json_object" }));
    assert_eq!(
        build_request_body(&req, "m")["output_config"]["format"]["type"],
        "json_schema"
    );
}

#[test]
fn reasoning_effort_maps_to_thinking_budget() {
    let mut req = base_request(vec![user("hi")]);
    req.max_tokens = Some(1024);
    req.temperature = Some(0.5);

    for (effort, budget) in [("low", 1024), ("medium", 4096), ("high", 8192)] {
        req.reasoning_effort = Some(effort.into());
        let body = build_request_body(&req, "m");
        assert_eq!(
            body["thinking"],
            json!({ "type": "enabled", "budget_tokens": budget })
        );
        assert!(body.get("temperature").is_none());
        assert!(body["max_tokens"].as_u64().unwrap() > budget);
    }

    req.reasoning_effort = Some("none".into());
    let body = build_request_body(&req, "m");
    assert!(body.get("thinking").is_none());
    assert_eq!(body["temperature"], 0.5);
}

#[test]
fn consecutive_user_turns_merge_into_one_message() {
    let body = translated(vec![user("first"), user("second")]);

    let messages = body.as_array().expect("messages");
    assert_eq!(messages.len(), 1, "one turn, not two: {messages:?}");
    let content = messages[0]["content"].as_str().expect("string content");
    assert!(
        content.contains("first") && content.contains("second"),
        "{content}"
    );
}

#[test]
fn consecutive_assistant_turns_merge() {
    let body = translated(vec![
        user("q"),
        msg("assistant", json!("one"), None),
        msg("assistant", json!("two"), None),
    ]);

    let messages = body.as_array().expect("messages");
    let roles: Vec<_> = messages
        .iter()
        .map(|m| m["role"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(roles, ["user", "assistant"], "no duplicate assistant turn");
}

#[test]
fn tool_replies_merge_into_the_preceding_user_turn() {
    let body = translated(vec![
        user("do it"),
        msg(
            "assistant",
            Value::Null,
            Some(json!([openai_call("call_1", "go")])),
        ),
        msg("tool", json!("result one"), None),
        msg("tool", json!("result two"), None),
    ]);

    let messages = body.as_array().expect("messages");
    let roles: Vec<_> = messages
        .iter()
        .map(|m| m["role"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(roles, ["user", "assistant", "user"]);
    let tool_results = messages[2]["content"]
        .as_array()
        .expect("block array")
        .iter()
        .filter(|block| block["type"] == json!("tool_result"))
        .count();
    assert_eq!(tool_results, 2, "both tool replies preserved");
}

#[test]
fn assistant_with_null_content_still_carries_its_tool_calls() {
    let body = translated(vec![
        user("go"),
        msg(
            "assistant",
            Value::Null,
            Some(json!([openai_call("call_9", "act")])),
        ),
    ]);

    let messages = body.as_array().expect("messages");
    let blocks = messages[1]["content"].as_array().expect("block array");
    assert!(
        blocks
            .iter()
            .any(|block| block["type"] == json!("tool_use")),
        "tool_use lost when content is null: {blocks:?}"
    );
}
