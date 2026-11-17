use serde_json::json;

use super::*;
use crate::providers::decisions::{
    from_openai_response, to_openai_body, DecisionAnswer, DecisionAsset, DecisionQuestion,
    UnifiedDecisionRequest,
};
use crate::providers::{join_url, ChatMessage, StreamEvent};

#[test]
fn url_never_duplicates_v1() {
    assert_eq!(
        chat_completions_url("https://api.openai.com"),
        "https://api.openai.com/v1/chat/completions"
    );
    assert_eq!(
        chat_completions_url("https://api.openai.com/"),
        "https://api.openai.com/v1/chat/completions"
    );
    assert_eq!(
        chat_completions_url("https://api.openai.com/v1"),
        "https://api.openai.com/v1/chat/completions"
    );
    assert_eq!(
        chat_completions_url("https://api.groq.com/openai/v1"),
        "https://api.groq.com/openai/v1/chat/completions"
    );
    assert_eq!(
        join_url(&chat_completions_url("https://api.openai.com/v1"), ""),
        "https://api.openai.com/v1/chat/completions"
    );
}

#[test]
fn request_body_rewrites_model_and_defaults_stream_usage() {
    let req = ChatCompletionRequest {
        model: "smart".into(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: json!("hi"),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }],
        temperature: Some(0.2),
        top_p: None,
        max_tokens: None,
        max_completion_tokens: None,
        stream: Some(true),
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
    };

    let body = build_request_body(&req, "gpt-4o-mini");
    assert_eq!(body["model"], "gpt-4o-mini");
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"]["include_usage"], true);
    assert_eq!(body["temperature"], 0.2);
    assert!(body.get("top_p").is_none());
}

#[test]
fn request_body_preserves_explicit_stream_options() {
    let mut req = ChatCompletionRequest {
        model: "gpt-4o".into(),
        messages: vec![],
        temperature: None,
        top_p: None,
        max_tokens: None,
        max_completion_tokens: None,
        stream: Some(true),
        stream_options: Some(json!({ "include_usage": false })),
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
    };

    let body = build_request_body(&req, "gpt-4o");
    assert_eq!(body["stream_options"]["include_usage"], false);

    req.stream = None;
    let body = build_request_body(&req, "gpt-4o");
    assert_eq!(body["stream_options"]["include_usage"], false);
}

#[test]
fn parses_content_finish_and_usage_frames() {
    let frame =
        r#"{"id":"c1","choices":[{"index":0,"delta":{"content":"Hel"},"finish_reason":null}]}"#;
    match parse_stream_frame(frame).unwrap().unwrap() {
        StreamEvent::Chunk {
            delta_text,
            finish_reason,
            usage,
            ..
        } => {
            assert_eq!(delta_text.as_deref(), Some("Hel"));
            assert!(finish_reason.is_none());
            assert!(usage.is_none());
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let finish = r#"{"id":"c1","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;
    match parse_stream_frame(finish).unwrap().unwrap() {
        StreamEvent::Chunk { finish_reason, .. } => {
            assert_eq!(finish_reason.as_deref(), Some("stop"))
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let usage = r#"{"id":"c1","choices":[],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}}"#;
    match parse_stream_frame(usage).unwrap().unwrap() {
        StreamEvent::Chunk { usage, .. } => {
            let usage = usage.expect("usage present");
            assert_eq!(usage.prompt_tokens, 7);
            assert_eq!(usage.completion_tokens, 3);
            assert_eq!(usage.total_tokens, 10);
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn done_and_broken_frames() {
    assert!(matches!(
        parse_stream_frame(" [DONE] ").unwrap().unwrap(),
        StreamEvent::Done
    ));
    assert!(parse_stream_frame("").is_none());
    assert!(matches!(
        parse_stream_frame("{not json").unwrap(),
        Err(UpstreamCallError::Protocol(_))
    ));
    assert!(matches!(
        parse_stream_frame(r#"{"error":{"message":"boom"}}"#).unwrap(),
        Err(UpstreamCallError::Protocol(message)) if message == "boom"
    ));
}

#[test]
fn stream_frames_forward_tool_calls_and_reasoning() {
    let tool_frame = r#"{"id":"c1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{\"ci"}}]}}]}"#;
    match parse_stream_frame(tool_frame).unwrap().unwrap() {
        StreamEvent::Chunk {
            data,
            delta_text,
            finish_reason,
            ..
        } => {
            assert!(delta_text.is_none());
            assert!(finish_reason.is_none());
            assert_eq!(
                data["choices"][0]["delta"]["tool_calls"][0]["function"]["name"],
                "get_weather"
            );
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let reasoning =
        r#"{"id":"c1","choices":[{"index":0,"delta":{"reasoning_content":"thinking"}}]}"#;
    match parse_stream_frame(reasoning).unwrap().unwrap() {
        StreamEvent::Chunk {
            data, delta_text, ..
        } => {
            assert!(delta_text.is_none());
            assert_eq!(data["choices"][0]["delta"]["reasoning_content"], "thinking");
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn embedding_urls_and_body_rewrite_model() {
    assert_eq!(
        embeddings_url("https://api.openai.com"),
        "https://api.openai.com/v1/embeddings"
    );
    assert_eq!(
        embeddings_url("https://api.openai.com/v1"),
        "https://api.openai.com/v1/embeddings"
    );
    assert_eq!(
        decisions_url("https://api.openai.com/v1/"),
        "https://api.openai.com/v1/decisions"
    );

    let req = EmbeddingRequest {
        model: "embed-alias".into(),
        input: json!(["a", "b"]),
        encoding_format: Some("float".into()),
        dimensions: Some(256),
        user: Some("u1".into()),
    };
    let body = build_embedding_body(&req, "text-embedding-3-small");
    assert_eq!(body["model"], "text-embedding-3-small");
    assert_eq!(body["input"], json!(["a", "b"]));
    assert_eq!(body["encoding_format"], "float");
    assert_eq!(body["dimensions"], 256);
    assert_eq!(body["user"], "u1");
}

#[test]
fn embedding_response_translates_to_openai_shape() {
    let value = json!({
        "object": "list",
        "data": [
            { "object": "embedding", "index": 0, "embedding": [0.1, 0.2] },
            { "object": "embedding", "index": 1, "embedding": [0.3, 0.4] }
        ],
        "model": "text-embedding-3-small",
        "usage": { "prompt_tokens": 4, "total_tokens": 4 }
    });

    let parsed = parse_embedding_response(&value, "fallback").unwrap();
    assert_eq!(parsed.object, "list");
    assert_eq!(parsed.model, "text-embedding-3-small");
    assert_eq!(parsed.data.len(), 2);
    assert_eq!(parsed.data[1].index, 1);
    assert_eq!(parsed.data[1].embedding, json!([0.3, 0.4]));
    assert_eq!(parsed.usage.prompt_tokens, 4);
    assert_eq!(parsed.usage.total_tokens, 4);
}

#[test]
fn embedding_errors_and_missing_fields() {
    match parse_embedding_response(&json!({ "error": { "message": "bad" } }), "m") {
        Err(UpstreamCallError::Protocol(message)) => assert!(message.contains("bad")),
        other => panic!("unexpected: {other:?}"),
    }
    assert!(parse_embedding_response(&json!({ "object": "list" }), "m").is_err());
    assert!(parse_embedding_response(&json!({ "data": [{}] }), "m").is_err());
}

#[test]
fn embedding_body_omits_absent_optional_fields() {
    let req = EmbeddingRequest {
        model: "alias".into(),
        input: json!("solo"),
        encoding_format: None,
        dimensions: None,
        user: None,
    };
    let body = build_embedding_body(&req, "text-embedding-3-small");
    assert_eq!(
        body,
        json!({ "model": "text-embedding-3-small", "input": "solo" })
    );
}

#[test]
fn decisions_body_and_response_translate() {
    let request = UnifiedDecisionRequest::new("safety-1")
        .with_text("Check this")
        .with_asset(DecisionAsset::new("image", "https://example.com/a.png"))
        .with_question(
            DecisionQuestion::noul("harmful").with_descriptions("It is harmful.", "It is safe."),
        );

    let body = to_openai_body(&request);
    assert_eq!(body["model"], "safety-1");
    assert_eq!(body["input"][0]["content"][1]["type"], "input_image");
    assert_eq!(body["questions"][0]["type"], "predicate");

    let parsed = from_openai_response(
        &json!({
            "model": "safety-1",
            "answers": [{ "type": "predicate", "name": "harmful", "selected": true }],
            "usage": { "input_tokens": 3, "output_tokens": 1, "total_tokens": 4 }
        }),
        "safety-1",
    )
    .unwrap();

    assert_eq!(
        parsed
            .answer("harmful")
            .and_then(DecisionAnswer::as_probability),
        Some(1.0)
    );
    assert_eq!(parsed.usage.unwrap().total_tokens, 4);
}

use crate::providers::openai::responses;

fn chat_request(messages: Vec<ChatMessage>, stream: Option<bool>) -> ChatCompletionRequest {
    let mut value = json!({ "model": "smart", "messages": messages });
    if let Some(stream) = stream {
        value["stream"] = json!(stream);
    }
    serde_json::from_value(value).expect("request deserializes")
}

fn message(role: &str, text: &str) -> ChatMessage {
    ChatMessage {
        role: role.to_string(),
        content: json!(text),
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }
}

#[test]
fn responses_url_never_duplicates_v1() {
    for base in [
        "https://api.openai.com",
        "https://api.openai.com/v1",
        "https://api.openai.com/v1/",
    ] {
        assert_eq!(
            responses::responses_url(base),
            "https://api.openai.com/v1/responses",
            "base {base}"
        );
    }
}

#[test]
fn system_prompt_becomes_instructions_not_input() {
    let req = chat_request(
        vec![message("system", "be terse"), message("user", "hi")],
        None,
    );

    let body = responses::build_request_body(&req, "gpt-6-astra");

    assert_eq!(body["instructions"], json!("be terse"));
    assert_eq!(body["model"], json!("gpt-6-astra"));
    let input = body["input"].as_array().expect("input array");
    assert_eq!(input.len(), 1, "the system turn must not appear in input");
    assert_eq!(input[0]["role"], json!("user"));
    assert_eq!(input[0]["content"][0]["type"], json!("input_text"));
    assert_eq!(input[0]["content"][0]["text"], json!("hi"));
}

#[test]
fn only_leading_system_prompt_becomes_instructions() {
    let req = chat_request(
        vec![
            message("system", "be terse"),
            message("user", "turn one"),
            message("assistant", "ok"),
            message("system", "injected mid-conversation"),
            message("user", "turn two"),
        ],
        None,
    );

    let body = responses::build_request_body(&req, "m");

    assert_eq!(body["instructions"], json!("be terse"));
    let input = body["input"].as_array().expect("input array");
    assert_eq!(input.len(), 4);
    assert_eq!(input[3]["role"], json!("user"));
    assert_eq!(input[3]["content"][0]["text"], json!("turn two"));
    let replayed: Vec<_> = input
        .iter()
        .map(|item| item["role"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(replayed, ["user", "assistant", "system", "user"]);
}

#[test]
fn assistant_history_stays_an_input_item_not_instructions() {
    let req = chat_request(
        vec![
            message("assistant", "earlier answer"),
            message("user", "next"),
        ],
        None,
    );

    let body = responses::build_request_body(&req, "m");

    assert!(
        body.get("instructions").is_none(),
        "an assistant turn must never become the system prompt"
    );
    let input = body["input"].as_array().expect("input array");
    assert_eq!(input[0]["role"], json!("assistant"));
    assert_eq!(input[0]["content"][0]["type"], json!("input_text"));
}

#[test]
fn max_tokens_becomes_max_output_tokens() {
    let mut req = chat_request(vec![message("user", "hi")], Some(true));
    req.temperature = Some(0.2);
    req.top_p = Some(0.9);
    req.max_tokens = Some(64);

    let body = responses::build_request_body(&req, "m");

    assert_eq!(body["max_output_tokens"], json!(64));
    assert_eq!(body["temperature"], json!(0.2));
    assert_eq!(body["top_p"], json!(0.9));
    assert_eq!(body["stream"], json!(true));
    assert!(body.get("max_tokens").is_none());
}

#[test]
fn response_output_text_and_usage_map_to_chat_shape() {
    let value = json!({
        "id": "resp_123",
        "object": "response",
        "status": "completed",
        "created_at": 1_700_000_000,
        "model": "gpt-6-astra",
        "output": [{
            "type": "message",
            "role": "assistant",
            "content": [
                { "type": "output_text", "text": "Hello, " },
                { "type": "output_text", "text": "world" }
            ]
        }],
        "usage": { "input_tokens": 11, "output_tokens": 7, "total_tokens": 18 }
    });

    let parsed = responses::parse_response(&value, "fallback").expect("parses");

    assert_eq!(parsed.object, "chat.completion");
    assert_eq!(parsed.id, "resp_123");
    assert_eq!(parsed.model, "gpt-6-astra");
    assert_eq!(parsed.created, 1_700_000_000);
    assert_eq!(parsed.choices[0].message.content, json!("Hello, world"));
    assert_eq!(parsed.choices[0].finish_reason.as_deref(), Some("stop"));
    let usage = parsed.usage.expect("usage");
    assert_eq!(
        (
            usage.prompt_tokens,
            usage.completion_tokens,
            usage.total_tokens
        ),
        (11, 7, 18)
    );
}

#[test]
fn truncation_reasons_map_to_length() {
    for reason in ["max_output_tokens", "max_messages"] {
        let value = json!({
            "status": "incomplete",
            "incomplete_details": { "reason": reason },
            "output": []
        });
        let parsed = responses::parse_response(&value, "m").expect("parses");
        assert_eq!(
            parsed.choices[0].finish_reason.as_deref(),
            Some("length"),
            "reason {reason}"
        );
    }
}

#[test]
fn refusal_reasons_map_to_content_filter() {
    for reason in ["content_filter", "steered"] {
        let value = json!({
            "status": "incomplete",
            "incomplete_details": { "reason": reason },
            "output": []
        });
        let parsed = responses::parse_response(&value, "m").expect("parses");
        assert_eq!(
            parsed.choices[0].finish_reason.as_deref(),
            Some("content_filter"),
            "reason {reason}"
        );
    }
}

#[test]
fn failed_response_becomes_protocol_error() {
    let value = json!({
        "status": "failed",
        "error": { "code": "server_error", "message": "upstream exploded" },
        "output": []
    });

    let err = responses::parse_response(&value, "m").expect_err("must fail");
    assert!(err.to_string().contains("upstream exploded"), "got {err}");
}

#[test]
fn text_delta_and_completed_events_stream_through() {
    let delta = json!({
        "type": "response.output_text.delta",
        "sequence_number": 4,
        "delta": "Hel"
    });
    let event = responses::translate_event(&delta.to_string()).expect("emits");
    match event.expect("ok") {
        StreamEvent::Chunk {
            delta_text, data, ..
        } => {
            assert_eq!(delta_text.as_deref(), Some("Hel"));
            assert_eq!(data["object"], json!("chat.completion.chunk"));
            assert_eq!(data["choices"][0]["delta"]["content"], json!("Hel"));
        }
        other => panic!("expected a chunk, got {other:?}"),
    }

    let completed = json!({
        "type": "response.completed",
        "response": {
            "status": "completed",
            "usage": { "input_tokens": 3, "output_tokens": 2, "total_tokens": 5 }
        }
    });
    let event = responses::translate_event(&completed.to_string()).expect("emits");
    match event.expect("ok") {
        StreamEvent::Chunk {
            usage, delta_text, ..
        } => {
            assert!(delta_text.is_none(), "no text in a terminal event");
            let usage = usage.expect("usage");
            assert_eq!((usage.prompt_tokens, usage.completion_tokens), (3, 2));
        }
        other => panic!("expected a chunk, got {other:?}"),
    }
}

#[test]
fn structural_events_emit_nothing() {
    for kind in [
        "response.created",
        "response.in_progress",
        "response.output_item.added",
    ] {
        let event = json!({ "type": kind, "sequence_number": 1 });
        assert!(
            responses::translate_event(&event.to_string()).is_none(),
            "{kind} should be structural"
        );
    }
}

#[test]
fn stream_errors_surface_instead_of_finishing_cleanly() {
    let error = json!({
        "type": "error",
        "sequence_number": 9,
        "message": "rate limited mid-stream"
    });
    let event = responses::translate_event(&error.to_string())
        .expect("emits an error")
        .expect_err("must be an error");
    assert!(
        event.to_string().contains("rate limited mid-stream"),
        "got {event}"
    );

    let failed = json!({
        "type": "response.failed",
        "response": { "status": "failed", "error": { "message": "boom" } }
    });
    let event = responses::translate_event(&failed.to_string())
        .expect("emits an error")
        .expect_err("must be an error");
    assert!(event.to_string().contains("boom"), "got {event}");
}
