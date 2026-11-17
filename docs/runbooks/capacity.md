# Capacity

Single-instance deployment until the Redis-backed limiter ships. One gateway owns its in-process rate-limit windows; do not run two writers against the same SQLite file.

SQLite pool is 8 connections; proxy handlers never block on writes (telemetry is async via a 10k-event channel, dropped on backpressure).

Watch:

- Telemetry drops (log volume falls while traffic holds) — channel saturated.
- Circuit-open rate per provider — sustained opens mean the upstream, not the gateway, is the bottleneck.
- Webhook delivery failures — retry queue growing.
- 5xx rate and p95 latency on `/v1/chat/completions`.
- Spend dashboards per virtual key — quota exhaustion looks like errors.

Scale vertically first. Add read-only observability (metrics scrape, log tail) before adding instances.
