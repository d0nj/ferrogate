# FerroGate

Local-first AI gateway for LLM providers. A single Rust (Axum) binary proxies
OpenAI-compatible chat traffic across OpenAI, Anthropic and Gemini upstreams with
virtual-key auth, per-key budgets and rate limits, routing rules, request logging,
and an embedded React admin console.

## Repository layout

```
crates/ferrogate/   Rust gateway crate (Axum, SeaORM/SQLite, embedded SPA)
apps/web/           React + TypeScript + Vite admin console (Astryx, Tailwind)
Cargo.toml          Cargo workspace root
package.json        npm workspaces root
Dockerfile          Multi-stage single-binary container build
compose.yaml        Local Docker deployment with persistent SQLite volume
.github/workflows/  CI checks for frontend and Rust workspace
apps/web/dist/      Built frontend, embedded into the binary at compile time
```

## Quick start

Requires a stable Rust toolchain (see `rust-toolchain.toml`) and Node.js 22
(see `.nvmrc`).

```bash
npm install                     # install npm workspace dependencies
make dev                        # gateway on :8080 + Vite dev server
make build                      # release binary with the SPA embedded
make test                       # cargo test, oxlint, frontend build
make lint                       # rustfmt, clippy (-D warnings), oxlint
```

`make dev` runs the API and the Vite dev server together; Vite proxies `/v1`,
`/admin` and `/health` to the gateway. `make build` produces
`target/release/ferrogate` with `apps/web/dist` embedded.

### Docker

```bash
docker compose up --build
```

## Configuration

Configuration comes from the environment; copy `.env.example` to `.env` to override
defaults.

| Variable       | Default                              | Purpose                              |
| -------------- | ------------------------------------ | ------------------------------------ |
| `HOST`         | `0.0.0.0`                            | Bind address                         |
| `PORT`         | `8080`                               | Bind port                            |
| `DATABASE_URL` | `sqlite://ferrogate.db?mode=rwc`     | SQLite connection string             |
| `RUST_LOG`     | `ferrogate=debug,tower_http=debug,info` | Tracing filter                    |
| `FERROGATE_ENCRYPTION_KEY` | *(required)*             | Seals provider credentials at rest; the gateway refuses to start without it. `openssl rand -hex 32`. Changing it makes stored credentials unreadable. |

## API surface

| Endpoint                    | Method          | Description                                       |
| --------------------------- | --------------- | ------------------------------------------------- |
| `/v1/chat/completions`      | `POST`          | OpenAI-compatible chat completions (streaming supported) |
| `/v1/models`                | `GET`           | Models available through configured providers     |
| `/admin/api/*`              | `GET`/`POST`/`PUT`/`PATCH`/`DELETE` | Admin console API: providers, routing, virtual keys, logs, pricing, settings |
| `/health`                   | `GET`           | Liveness plus database reachability                |

`/v1/*` endpoints require `Authorization: Bearer <virtual-key>`. Admin endpoints are
served by the console; all other paths serve the embedded SPA.
