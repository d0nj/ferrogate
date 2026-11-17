//! End-to-end proxy tests against in-process mock upstreams.
//!
//! Layout:
//! - `support`  — mock provider servers, the FerroGate harness and SSE helpers
//! - `completions`, `streaming`, `failover`, `cache`, `misc` — chat completions
//! - `protocols`, `decisions` — the remaining ingress dialects and headers

mod cache;
mod completions;
mod decisions;
mod failover;
mod misc;
mod prompts;
mod protocols;
mod soak;
mod streaming;
mod support;
