use serde_json::json;

use super::*;

fn sample_request() -> UnifiedDecisionRequest {
    UnifiedDecisionRequest::new("safety-1")
        .with_text("Is this message harmful?")
        .with_asset(DecisionAsset::new("image", "data:image/png;base64,AAAA").with_id("img"))
        .with_question(
            DecisionQuestion::noul("harmful")
                .with_instructions("Decide whether the message is harmful.")
                .with_descriptions("The message is harmful.", "The message is safe."),
        )
        .with_question(DecisionQuestion::choice(
            "category",
            vec![
                DecisionChoiceOption::text("spam").with_description("Bulk unsolicited mail."),
                DecisionChoiceOption::text("abuse"),
                DecisionChoiceOption::text("clean"),
            ],
        ))
        .with_question(DecisionQuestion::score(
            "severity",
            vec![
                DecisionScoreLevel::new("none"),
                DecisionScoreLevel::new("high").with_description("Requires review."),
            ],
        ))
}

fn sample_response() -> UnifiedDecisionResponse {
    UnifiedDecisionResponse {
        model: "safety-1".into(),
        answers: vec![
            DecisionResult {
                name: "harmful".into(),
                answer: DecisionAnswer::Predicate {
                    probability: 0.92,
                    confidence: Some(0.8),
                },
            },
            DecisionResult {
                name: "category".into(),
                answer: DecisionAnswer::Choice {
                    choice: json!("abuse"),
                    probabilities: vec![(json!("abuse"), 0.7), (json!("spam"), 0.3)],
                    confidence: None,
                },
            },
            DecisionResult {
                name: "severity".into(),
                answer: DecisionAnswer::Score {
                    score: 7.5,
                    probabilities: vec![("high".into(), None, 0.6)],
                    legend: None,
                    confidence: None,
                },
            },
        ],
        usage: Some(DecisionUsage::new(30, 4, None)),
    }
}

#[test]
fn question_kind_and_asset_helpers() {
    assert_eq!(
        DecisionQuestionKind::parse("predicate"),
        Some(DecisionQuestionKind::Noul)
    );
    assert_eq!(
        DecisionQuestionKind::parse("NOUL"),
        Some(DecisionQuestionKind::Noul)
    );
    assert_eq!(
        DecisionQuestionKind::parse("box"),
        Some(DecisionQuestionKind::Box)
    );
    assert_eq!(DecisionQuestionKind::parse("wat"), None);
    assert_eq!(DecisionQuestionKind::Score.as_str(), "score");

    let asset = DecisionAsset::new("image", "data:image/webp;base64,ZZZ");
    assert_eq!(asset.media_type.as_deref(), Some("image/webp"));
    assert_eq!(asset.data_url_payload(), Some(("image/webp", "ZZZ")));
    assert!(asset.is_binary_data());
    let remote = DecisionAsset::new("image", "https://example.com/a.png");
    assert_eq!(remote.resolved_media_type(), "image/jpeg");
    assert!(!remote.is_binary_data());
}

#[test]
fn openai_request_round_trips() {
    let req = sample_request();
    let body = to_openai_body(&req);
    assert_eq!(body["model"], "safety-1");
    assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
    assert_eq!(body["input"][0]["content"][1]["type"], "input_image");
    let questions = body["questions"].as_array().unwrap();
    assert_eq!(questions[0]["type"], "predicate");
    assert_eq!(questions[1]["choices"][0]["value"], "spam");
    assert_eq!(questions[2]["levels"][1]["label"], "high");

    let parsed = from_openai_request(&body).unwrap();
    assert_eq!(parsed.input.len(), 2);
    assert_eq!(
        parsed.questions[1].choices[0].description.as_deref(),
        Some("Bulk unsolicited mail.")
    );
    assert_eq!(parsed.questions[2].levels[1].label, "high");

    let plain = UnifiedDecisionRequest::new("m").with_text("hello");
    assert_eq!(to_openai_body(&plain)["input"], json!("hello"));
}

#[test]
fn openai_request_rejects_bad_questions() {
    let dup = json!({
        "model": "m",
        "input": "hi",
        "questions": [{ "type": "box", "name": "a" }, { "type": "box", "name": "a" }]
    });
    assert!(from_openai_request(&dup)
        .unwrap_err()
        .contains("duplicate question name"));
    let empty = json!({
        "model": "m",
        "questions": [{ "type": "choice", "name": "c", "choices": [] }]
    });
    assert!(from_openai_request(&empty)
        .unwrap_err()
        .contains("no options"));
    assert!(from_openai_request(&json!({}))
        .unwrap_err()
        .contains("missing 'model'"));
}

#[test]
fn openai_response_reads_probabilities_boxes_and_usage() {
    let parsed = from_openai_response(
        &json!({
            "model": "safety-1",
            "answers": [
                { "type": "predicate", "name": "harmful", "probability": 0.87, "confidence": 0.9 },
                { "type": "predicate", "name": "legacy", "selected": true },
                { "type": "choice", "name": "category", "choice": "abuse",
                  "probabilities": [{ "value": "abuse", "probability": 0.7 }] },
                { "type": "score", "name": "severity", "score": 7.5 },
                { "type": "box", "name": "where", "boxes": [[0.1, 0.2, 0.3, 0.4]] },
                { "type": "refusal", "name": "declined" }
            ],
            "usage": { "input_tokens": 30, "output_tokens": 4 }
        }),
        "fallback",
    )
    .unwrap();

    let harmful = parsed.answer("harmful").unwrap();
    assert_eq!(harmful.as_probability(), Some(0.87));
    assert_eq!(harmful.confidence(), Some(0.9));
    assert_eq!(
        parsed
            .answer("legacy")
            .and_then(DecisionAnswer::as_probability),
        Some(1.0)
    );
    assert_eq!(
        parsed
            .answer("category")
            .and_then(DecisionAnswer::as_choice),
        Some("abuse")
    );
    assert_eq!(
        parsed.answer("severity").and_then(DecisionAnswer::as_score),
        Some(7.5)
    );
    assert_eq!(
        parsed
            .answer("where")
            .and_then(DecisionAnswer::as_boxes)
            .unwrap()
            .len(),
        1
    );
    assert!(parsed.answer("declined").unwrap().is_refusal());
    assert_eq!(parsed.usage.unwrap().total_tokens, 34);
    assert_eq!(
        from_openai_response(&json!({}), "m").unwrap_err(),
        "missing 'answers' array"
    );
}

#[test]
fn typesafe_body_and_response() {
    let body = to_typesafe_body(&sample_request());
    assert_eq!(body["state"], "Is this message harmful?");
    assert_eq!(body["assets"]["img"]["data_base64"], "AAAA");
    assert_eq!(body["assets"]["img"]["format"], "image/png");
    let questions = body["questions"].as_object().unwrap();
    assert_eq!(
        questions["harmful"]["criteria"]["true"],
        "The message is harmful."
    );
    assert_eq!(
        questions["category"]["criteria"]["spam"],
        "Bulk unsolicited mail."
    );
    assert_eq!(questions["severity"]["criteria"][1], "Requires review.");

    let parsed = from_typesafe_request(&body).unwrap();
    assert_eq!(parsed.input[1].id.as_deref(), Some("img"));
    assert_eq!(parsed.input[1].source, "data:image/png;base64,AAAA");
    let category = parsed
        .questions
        .iter()
        .find(|q| q.name == "category")
        .unwrap();
    assert_eq!(
        category.choices[2].description.as_deref(),
        Some("Bulk unsolicited mail.")
    );
    assert_eq!(category.choices[0].description, None);

    let response = from_typesafe_response(
        &json!({
            "answers": {
                "harmful": { "type": "noul", "noul": 0.75, "confidence": 0.6 },
                "category": { "type": "choice", "choice": "spam", "probabilities": { "spam": 0.9 } },
                "severity": { "type": "score", "score": 3.0, "probabilities": { "high": 0.4 } }
            },
            "usage": { "input_tokens": 12, "output_tokens": 3 }
        }),
        "safety-1",
    )
    .unwrap();
    assert_eq!(
        response
            .answer("harmful")
            .and_then(DecisionAnswer::as_probability),
        Some(0.75)
    );
    assert_eq!(response.usage.unwrap().total_tokens, 15);

    let wire = to_typesafe_response(&sample_response());
    assert_eq!(wire["answers"]["harmful"]["noul"], 0.92);
    assert_eq!(wire["answers"]["category"]["probabilities"]["abuse"], 0.7);
    assert_eq!(wire["usage"].get("total_tokens"), None);
    assert!(from_typesafe_response(&json!({ "error": { "message": "nope" } }), "m").is_err());
}

#[test]
fn gemini_body_and_response() {
    let req = sample_request().with_asset(
        DecisionAsset::new("audio", "data:audio/wav;base64,QQ").with_resolution("high"),
    );
    let body = to_gemini_body(&req);
    assert_eq!(body["input"][0]["type"], "text");
    assert_eq!(body["input"][1]["data"], "AAAA");
    assert_eq!(body["input"][1]["mime_type"], "image/png");
    assert_eq!(body["input"][2]["type"], "audio");
    assert_eq!(body["input"][2]["resolution"], "high");
    let questions = body["questions"].as_object().unwrap();
    assert_eq!(
        questions["harmful"]["criteria"]["true_description"],
        "The message is harmful."
    );
    assert_eq!(
        questions["category"]["criteria"]["spam"],
        "Bulk unsolicited mail."
    );
    assert_eq!(questions["severity"]["criteria"][0], "none");

    let parsed = from_gemini_request(&body).unwrap();
    assert_eq!(parsed.input[1].media_type.as_deref(), Some("image/png"));
    assert_eq!(parsed.input[2].resolution.as_deref(), Some("high"));
    let category = parsed
        .questions
        .iter()
        .find(|q| q.name == "category")
        .unwrap();
    assert_eq!(category.choices.len(), 3);

    let response = from_gemini_response(
        &json!({
            "answers": {
                "harmful": { "type": "noul", "value": true },
                "category": { "type": "choice", "value": "clean" },
                "where": { "type": "box", "boxes": [[0.0, 0.0, 1.0, 1.0]] }
            },
            "usage": { "total_input_tokens": 9, "total_output_tokens": 2, "total_tokens": 11 }
        }),
        "safety-1",
    )
    .unwrap();
    assert_eq!(
        response.answer("harmful").and_then(DecisionAnswer::as_bool),
        Some(true)
    );
    assert_eq!(
        response
            .answer("category")
            .and_then(DecisionAnswer::as_choice),
        Some("clean")
    );
    assert_eq!(
        response
            .answer("where")
            .and_then(DecisionAnswer::as_boxes)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(response.usage.unwrap().total_tokens, 11);

    let wire = to_gemini_response(&sample_response());
    assert_eq!(wire["answers"]["severity"]["probabilities"]["high"], 0.6);
    assert_eq!(wire["usage"]["total_tokens"], 34);
}

#[test]
fn cross_dialect_translation_preserves_answers() {
    let openai = from_openai_response(
        &json!({
            "answers": [
                { "type": "predicate", "name": "ok", "probability": 0.9 },
                { "type": "box", "name": "where", "boxes": [[0.1, 0.2, 0.3, 0.4]] }
            ]
        }),
        "m",
    )
    .unwrap();
    let typesafe = from_typesafe_response(&to_typesafe_response(&openai), "m").unwrap();
    let gemini = from_gemini_response(&to_gemini_response(&typesafe), "m").unwrap();
    let round_trip = from_openai_response(&to_openai_response(&gemini), "m").unwrap();

    assert_eq!(openai.answer_map(), typesafe.answer_map());
    assert_eq!(typesafe.answer_map(), gemini.answer_map());
    assert_eq!(gemini.answer_map(), round_trip.answer_map());
    assert_eq!(
        round_trip
            .answer("ok")
            .and_then(DecisionAnswer::as_probability),
        Some(0.9)
    );
}

#[test]
fn cross_dialect_requests_keep_question_kinds() {
    let req = sample_request();
    let via_typesafe = from_typesafe_request(&to_typesafe_body(&req)).unwrap();
    let via_gemini = from_gemini_request(&to_gemini_body(&req)).unwrap();
    let via_openai = from_openai_request(&to_openai_body(&req)).unwrap();

    for parsed in [&via_typesafe, &via_gemini, &via_openai] {
        assert_eq!(parsed.questions.len(), 3);
        let by_name = |name: &str| parsed.questions.iter().find(|q| q.name == name).unwrap();
        assert_eq!(by_name("harmful").kind, DecisionQuestionKind::Noul);
        assert_eq!(by_name("category").choices.len(), 3);
        assert_eq!(by_name("severity").levels.len(), 2);
    }
}

#[test]
fn answer_views_and_edge_cases() {
    assert_eq!(DecisionAnswer::Refusal.as_bool(), None);
    assert_eq!(DecisionAnswer::Refusal.to_json(), serde_json::Value::Null);
    assert_eq!(
        DecisionAnswer::Choice {
            choice: json!("no"),
            probabilities: Vec::new(),
            confidence: None
        }
        .as_bool(),
        Some(false)
    );

    let mut request = UnifiedDecisionRequest::new("m").with_text("hello");
    assert!(!request.needs_image_processing());
    request = request.with_question(DecisionQuestion::boxed("locate"));
    assert!(request.needs_image_processing());

    let body = to_openai_body(
        &UnifiedDecisionRequest::new("m")
            .with_asset(DecisionAsset::json(json!({ "a": 1 })))
            .with_question(DecisionQuestion::boxed("locate")),
    );
    assert_eq!(body["input"][0]["content"][0]["type"], "input_json");
    assert!(from_gemini_response(&json!({ "answers": {} }), "m")
        .unwrap()
        .answers
        .is_empty());
}
