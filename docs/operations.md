# Operations

## Surfaces

Proxy (Bearer virtual key, 600s timeout, 512 concurrency, 8 MiB body cap):
`POST /v1/chat/completions`, `/v1/responses`, `/v1/messages`,
`/v1/embeddings`, `/v1/decisions` (aliases `/decisions`,
`/openai/v1/decisions`), `/v1/systemone` (aliases `/systemone`,
`/typesafe/v1/systemone`), `/v1beta/decisions` (alias
`/genai/v1beta/decisions`), `GET /v1/models`, `/typesafe/v1/models`,
`POST /mcp`.

Control plane (30s timeout, 256 concurrency): `GET /health` (status only
unless `HEALTH_DETAILED=1`; 503 when the database is down), `GET /metrics`
(requires an operator session, Prometheus exposition), `/admin/api/*`
(session cookie plus per-path capability), everything else serves the
embedded console SPA.

## Day-to-day signals

- `/health` in the load-balancer check; `HEALTH_DETAILED=1` only for
  internal probes since it names database/redis state.
- Scrape `/metrics` with an operator session. Rising telemetry-drop and
  audit-failure counters mean the async writer or audit path is failing;
  sustained circuit-open counts point at the upstream, not the gateway.
- `LOG_RETENTION_DAYS` (default 90) bounds `request_logs`; `0` disables the
  hourly sweep. Enabling prompt redaction purges the exact and semantic
  caches; per-row deletes go through `DELETE /admin/api/logs/{id}`.

## Degraded modes

All three integrations fail soft by design:

- **Redis down or unset:** sessions stay in SQLite, rate limits stay
  per-instance. Do not run two writers against one SQLite file; the shared
  limiter is what makes multi-instance safe.
- **Qdrant unreachable or URL blocked:** semantic search falls back to the
  local SQLite backend.
- **OTLP endpoint blocked or unset:** trace export is disabled.

Verify after enabling one: point it at an unreachable address and confirm
the gateway boots, serves proxy traffic, and logs a warning naming the
disabled feature.

## When traffic errors

1. Identify the plane: 5xx with a gateway error shape on `/v1/*` versus
   upstream-mapped errors. Provider credential failures surface per target;
   repeated `ClientError` outcomes are never retried fleet-wide.
2. Check provider health and circuit state from the console overview, then
   request logs filtered by key and model for spend and status codes.
3. Quota-shaped errors are spend or RPM/TPM limits on the virtual key;
   confirm on the key's spend dashboard before touching providers.

## Runbooks

- `runbooks/backup-restore.md` — consistent SQLite snapshots and restore
- `runbooks/key-rotation.md` — provider overlap rotation, virtual-key
  revoke, encryption-key re-entry
- `runbooks/incident-leak.md` — leaked virtual key or session response
- `runbooks/capacity.md` — single-instance constraint, pool and channel
  budgets, what to watch before scaling
