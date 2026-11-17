use serde_json::{json, Value};

use super::*;
use crate::providers::{StreamEvent, UpstreamCallError};

const MESSAGE_START: &str = r#"{"type":"message_start","message":{"id":"msg_abc","model":"claude-3-5-haiku-latest","usage":{"input_tokens":12,"output_tokens":1}}}"#;
const TOOL_START: &str = r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_9","name":"get_weather","input":{}}}"#;
const SECOND_TOOL_START: &str = r#"{"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_10","name":"other","input":{}}}"#;

fn stream(frames: &[(&str, &str)]) -> (Vec<StreamEvent>, StreamTranslator) {
    let mut translator = StreamTranslator::new();
    let output = frames
        .iter()
        .filter_map(|(name, data)| translator.translate(name, data))
        .map(|item| item.unwrap())
        .collect();
    (output, translator)
}

fn chunk_data(event: &StreamEvent) -> &Value {
    match event {
        StreamEvent::Chunk { data, .. } => data,
        other => panic!("unexpected: {other:?}"),
    }
}

fn delta(event: &StreamEvent) -> &Value {
    &chunk_data(event)["choices"][0]["delta"]
}

#[test]
fn sse_translation_produces_openai_chunks() {
    let (output, translator) = stream(&[
        ("message_start", MESSAGE_START),
        (
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}"#,
        ),
        (
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{"}}"#,
        ),
        (
            "message_delta",
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":7}}"#,
        ),
        ("message_stop", r#"{"type":"message_stop"}"#),
    ]);
    assert_eq!(output.len(), 5);

    let start = chunk_data(&output[0]);
    assert_eq!(start["object"], "chat.completion.chunk");
    assert_eq!(start["id"], "msg_abc");
    assert_eq!(start["model"], "claude-3-5-haiku-latest");
    assert_eq!(delta(&output[0])["role"], "assistant");

    assert_eq!(delta(&output[1])["content"], "Hi");
    assert_eq!(
        delta(&output[2])["tool_calls"][0]["function"]["arguments"],
        "{"
    );
    assert!(delta(&output[2]).get("content").is_none());

    let end = chunk_data(&output[3]);
    assert_eq!(end["choices"][0]["finish_reason"], "stop");
    assert_eq!(
        end["usage"],
        json!({ "prompt_tokens": 12, "completion_tokens": 7, "total_tokens": 19 })
    );
    match &output[3] {
        StreamEvent::Chunk {
            usage,
            finish_reason,
            ..
        } => {
            assert_eq!(usage.as_ref().map(|u| u.total_tokens), Some(19));
            assert_eq!(finish_reason.as_deref(), Some("stop"));
        }
        other => panic!("unexpected: {other:?}"),
    }

    assert!(matches!(output[4], StreamEvent::Done));
    assert!(translator.is_done());
}

#[test]
fn sse_translation_emits_tool_use_and_thinking_deltas() {
    let (output, _) = stream(&[
        ("message_start", MESSAGE_START),
        ("content_block_start", TOOL_START),
        (
            "content_block_delta",
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"city\":"}}"#,
        ),
        (
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm"}}"#,
        ),
        ("content_block_start", SECOND_TOOL_START),
    ]);

    let call = &delta(&output[1])["tool_calls"][0];
    assert_eq!(call["id"], "toolu_9");
    assert_eq!(
        call["function"],
        json!({ "name": "get_weather", "arguments": "" })
    );
    assert_eq!(
        delta(&output[2])["tool_calls"][0]["function"]["arguments"],
        "{\"city\":"
    );
    assert_eq!(delta(&output[3])["reasoning_content"], "hmm");
    assert_eq!(delta(&output[4])["tool_calls"][0]["index"], 1);
}

#[test]
fn sse_translation_reports_errors_and_ignores_noise() {
    let (ignored, translator) = stream(&[
        ("ping", r#"{"type":"ping"}"#),
        ("content_block_stop", r#"{"type":"content_block_stop"}"#),
        ("message_start", ""),
    ]);
    assert!(ignored.is_empty());
    assert!(!translator.is_done());

    let mut translator = StreamTranslator::new();
    match translator.translate(
        "error",
        r#"{"type":"error","error":{"message":"Overloaded"}}"#,
    ) {
        Some(Err(UpstreamCallError::Protocol(message))) => {
            assert!(message.contains("Overloaded"), "got {message}")
        }
        other => panic!("unexpected: {other:?}"),
    }
    assert!(translator.is_done());

    assert!(matches!(
        translator.translate("message_delta", "{bad json").unwrap(),
        Err(UpstreamCallError::Protocol(_))
    ));
}
