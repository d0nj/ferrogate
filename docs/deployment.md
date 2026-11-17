# Deployment

## Fresh install (compose)

```bash
export FERROGATE_ENCRYPTION_KEY="$(openssl rand -hex 32)"
docker compose up -d --build
curl -fsS http://127.0.0.1:8080/health
```

Compose wires `FERROGATE_ENCRYPTION_KEY` (required, no default), persists
SQLite at the `ferrogate_data` volume (`/data/ferrogate.db`), restarts
unless stopped, and healthchecks `GET /health` every 30s. The key seals
upstream provider credentials: back it up, and never change it without
re-entering every provider credential afterwards.

From source instead:

```bash
make build   # frontend first: apps/web/dist is embedded at compile time
PORT=8080 DATABASE_URL='sqlite://ferrogate.db?mode=rwc' ./target/release/ferrogate
```

The gateway refuses to start without `FERROGATE_ENCRYPTION_KEY`.

## First boot

Watch stderr for a one-time line:

```text
bootstrap owner admin@ferrogate.local initial password: fg-<32 hex> (change it immediately)
```

Sign in at the console with it. The account is flagged
`must_change_password` and can only change its password, view the session,
or log out until rotation. The seeded `key-demo-local` virtual key ships
inactive.

## Behind a reverse proxy

| Variable | Set to |
|---|---|
| `TRUSTED_PROXIES` | LB/proxy CIDRs, e.g. `10.0.0.0/8,172.16.0.0/12,192.168.0.0/16` |
| `COOKIE_SECURE` | `1` when terminating TLS at the proxy |
| `CORS_ALLOWED_ORIGINS` | Calling origins, e.g. `https://gateway.example.com` |

Without `TRUSTED_PROXIES` the sign-in throttle keys on the proxy address and
throttles all clients together, and forwarded `X-Forwarded-Proto` is ignored
when deciding the session cookie's `Secure` flag. An empty
`CORS_ALLOWED_ORIGINS` disables cross-origin reads; the same-origin console
needs no entry.

## Upgrades

Migrations run forward on boot. Rollback is restore, never down-migration;
see `runbooks/backup-restore.md`. Stop the gateway before restoring.

Notes for existing databases: the hardening migration deactivates
`key-demo-local`, forces rotation on an unchanged bootstrap owner, seals
stored webhook secrets (needs the real encryption key set first), clears
semantic cache rows for rebuild, and drops all console sessions (every
operator signs in again).

## Environment reference

| Variable | Default | Notes |
|---|---|---|
| `HOST` / `PORT` | `0.0.0.0` / `8080` | Bind address and port |
| `DATABASE_URL` | `sqlite://ferrogate.db?mode=rwc` | Docker image uses `sqlite:///data/ferrogate.db?mode=rwc` |
| `FERROGATE_ENCRYPTION_KEY` | required | 32-byte hex; boot fails without it |
| `RUST_LOG` | `ferrogate=debug,tower_http=debug,info` | Tracing filter |
| `REDIS_URL` | unset (single-node) | Shares sessions and rate-limit windows; outage degrades to local-only |
| `QDRANT_URL` | unset (SQLite semantic cache) | Enables vector-backed semantic search when reachable |
| `SEMANTIC_EMBEDDING_MODEL` | unset (built-in n-gram) | Routed like any `/v1/embeddings` call; needs an embeddings provider |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | unset (no trace export) | OTLP collector endpoint |
| `CORS_ALLOWED_ORIGINS` | empty (no cross-origin reads) | Comma-separated origins |
| `LOG_RETENTION_DAYS` | `90` | `0` disables the hourly cleanup |
| `ALLOW_PRIVATE_UPSTREAMS` | off | `1` permits loopback/RFC 1918 upstream and infra URLs |
| `COOKIE_SECURE` | auto from request scheme | `1` always, `0` never |
| `HEALTH_DETAILED` | off (status only) | `1` adds database/redis detail to `/health` |
