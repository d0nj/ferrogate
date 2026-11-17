use std::time::{Duration, Instant};

use axum::http::StatusCode;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
use serde_json::json;

use ferrogate::entities::request_log;

use crate::support::{FailureMode, Harness, MockServer};

const TOTAL: usize = 240;

fn body(idx: usize) -> serde_json::Value {
    json!({
        "model": "soak",
        "messages": [{ "role": "user", "content": format!("soak {idx}") }],
        "stream": false,
        "temperature": 0.9,
    })
}

fn p99(mut elapsed: Vec<Duration>) -> Duration {
    elapsed.sort_unstable();
    elapsed[(TOTAL * 99).div_ceil(100) - 1]
}

#[tokio::test]
#[ignore]
async fn soak_two_x_rpm_without_telemetry_loss() {
    let mock = MockServer::start(FailureMode::None).await;
    let harness = Harness::start(&[("prov-openai", "openai", &mock.base_url())]).await;
    harness
        .add_rule("soak", "prov-openai", "gpt-4o-mini", 0)
        .await;

    let mut elapsed = Vec::with_capacity(TOTAL);
    for idx in 0..TOTAL {
        let start = Instant::now();
        let resp = harness.post_chat(body(idx)).await;
        elapsed.push(start.elapsed());
        assert_eq!(resp.status(), StatusCode::OK, "request {idx} must succeed");
    }

    let tail = p99(elapsed);
    println!("soak: n={TOTAL} p99={tail:?}");
    assert!(tail < Duration::from_secs(5), "p99 {tail:?} exceeds 5s");

    for _ in 0..300 {
        let count = request_log::Entity::find()
            .filter(request_log::Column::RequestedModel.eq("soak"))
            .count(&harness.db)
            .await
            .unwrap();
        if count as usize == TOTAL {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    panic!("telemetry lost rows for `soak` within 30s");
}
