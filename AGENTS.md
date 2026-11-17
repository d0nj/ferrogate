# Repository Guidelines

## Project Overview

FerroGate is a single-binary, local-first AI API gateway and control plane. A Rust (Axum) server (`crates/ferrogate`) exposes an OpenAI-compatible proxy (`/v1/chat/completions`, `/v1/models`), a session-guarded admin REST API (`/admin/api/*`), and an embedded React + TypeScript SPA (`apps/web`) served via `rust-embed` from `apps/web/dist`.

---

## Architecture & Data Flow

### Proxy Hot Path (`crates/ferrogate/src/`)
1. **Auth & Quota (`auth/middleware.rs`, `auth/mod.rs`, `auth/limiter.rs`):** `require_virtual_key` hashes `Bearer <key>` with SHA-256 and looks up `CachedKey` in `moka` (5s TTL; misses memoized). `/v1/models` skips budget and RPM/TPM enforcement; `/v1/chat/completions` checks `live_spend` (`DashMap<String, Arc<AtomicI64>>` in micro-USD) and GCRA `governor` RPM/TPM limiters before dispatch.
2. **Cache & Routing (`cache.rs`, `routing.rs`):** Deterministic (`temperature == 0`, `stream == false`) requests without `Cache-Control: no-cache` or `X-FerroGate-Cache: false` check `ResponseCache` (`moka`, 60s TTL). `RoutingService::resolve_targets` queries `moka` (5s TTL) or joins `routing_rules` + active `providers`, filters open `CircuitBreakerRegistry` providers (`3` consecutive failures -> `15s` open), and orders each priority tier via weighted exponential-race shuffle.
3. **Adapter Translation & Zero-Buffer Streaming (`providers/`, `proxy/`):** `UpstreamProvider` adapters (`openai.rs`, `anthropic.rs`, `gemini.rs`) translate requests and responses to OpenAI format over a pooled HTTP/2 `reqwest::Client`. For `stream: true`, `open_stream` (`proxy/streaming.rs`) peeks only the first upstream `StreamEvent`; retryable `429`/`5xx`/transport failures fail over before sending HTTP `200`, then `ManagedStream` pipes subsequent SSE chunks directly without buffering. `StreamTelemetry::Drop` guarantees telemetry and live-spend accounting even on client disconnect.
4. **Decoupled Async Telemetry (`telemetry.rs`):** Request handlers never write to SQLite. They emit `TelemetryEvent` via `mpsc::Sender::try_send` (`10_000` capacity, dropping on backpressure rather than stalling traffic). A background task batches up to `100` events or flushes every `500ms` inside one SQLite transaction (`request_logs` insert + `virtual_keys.spent_usd` increment), then reconciles `live_spend`.
5. **SPA Asset Embedding (`assets.rs`):** `FrontendAssets` embeds `../../apps/web/dist` (`#[allow_missing = true]`). `serve_spa` serves `/assets/*` as `public, max-age=31536000, immutable`, returns JSON `404` for unknown `/v1/*`, `/admin/*`, `/health`, or missing dotted files, and falls back to `index.html` (`no-cache`) for client routes.

### Control Plane UI (`apps/web/src/`)
- Code-based `@tanstack/react-router` routes (`/`, `/login`, `/keys`, `/routing`, `/routing/$providerId`, `/guardrails`, `/webhooks`, `/logs`, `/settings`, `/prompts`, `/mcp-servers`, `/playground`, `/users`) render inside `Theme` (`mode="dark"`) -> `LayerProvider` -> `AppShell` + collapsible `SideNav` (`App.tsx`).
- Data fetching uses TanStack Query (`api/client.ts` + `api/queryKeys.ts`) against `/admin/api/*`; the Playground uses the same API client to stream `/v1/chat/completions`, with `AbortController` cancellation and live TTFT/tok-s calculation.

---

## Key Directories

- `crates/ferrogate/src/` — Rust gateway (`admin/`, `auth/`, `entities/`, `migration/`, `providers/`, `proxy/`, plus root state/config/cache/routing/telemetry modules).
- `crates/ferrogate/tests/` — Integration test binaries (`admin_integration.rs`, `assets_integration.rs`, `auth_integration.rs`, `proxy_integration/`).
- `apps/web/src/` — React SPA (`api/`, `components/{common,guardrails,keys,logs,mcp,overview,playground,prompts,routing,settings,users,webhooks}/`, `hooks/`, `pages/`, `utils/`).
- `.github/workflows/` — CI pipeline (`ci.yml`); enforces `frontend` build before `backend` test/release build.

---

## Development Commands

Build order matters: `apps/web/dist` is embedded by `rust-embed` at Rust compile time. Run frontend builds before Rust release builds or asset integration tests.

```bash
# Install JS workspace dependencies (root)
npm install

# Run gateway (:8080) + Vite dev server concurrently (Vite proxies /v1, /admin, /health)
make dev

# Build SPA (apps/web/dist) then optimized release binary (target/release/ferrogate)
make build

# Run Rust workspace tests, oxlint, and frontend production build
make test

# Check formatting, clippy (-D warnings), and oxlint
make lint

# Format Rust workspace
make fmt

# Build multi-stage Docker image / remove apps/*/dist and target/
make docker
make clean
```

Workspace equivalents: `npm run build` (`tsc -b && vite build`), `npm run lint` (`oxlint`), `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --locked`.

---

## Code Conventions & Common Patterns

### Rust (`crates/ferrogate`)
- **Routing & State DI:** Axum `0.8` requires brace parameter syntax (`/{id}`, `/{model}`)—never `:id`. Handlers receive `State(state): State<AppState>` (`Arc`-backed caches, `DatabaseConnection`, `reqwest::Client`, `TelemetrySender`).
- **Errors:** Use `AppResult<T>` (`error.rs`), emitting OpenAI-shaped `{"error":{"message","type","code","param"}}`. Return `not_found` helpers for missing entities (`<resource> '<id>' not found`); keep `UpstreamCallError` distinct and map explicitly.
- **Database & Migrations:** SeaORM `2.0` over SQLite `WAL` (`db.rs` applies `WAL`, `NORMAL`, `foreign_keys=ON`, `busy_timeout=5000` on `after_connect`). Booleans in entities are `i64` (`!= 0` / `i64::from(...)`); timestamps are RFC 3339 `String`; list/JSON fields use `*_json` text columns. Name migrations `mYYYYMMDD_NNNNNN_<snake_case>.rs` using `sea_orm_migration::schema` helpers, idempotent seeds (`OnConflict::do_nothing()`), and register in `migration/mod.rs`.
- **Caches & Concurrency:** Never hold `Mutex` or run SQLite writes in proxy handlers. When admin endpoints mutate `providers`/`routing_rules`, `model_pricing`, `virtual_keys`, or `app_settings`, invalidate the corresponding cache **and** await `run_pending_tasks().await` (or update `AtomicBool`/`AuthService`) immediately.

### Frontend (`apps/web`)
- **UI & Styling (`apps/web/AGENTS.md` + `src/index.css`):** Import Astryx components by deep path (`@astryxdesign/core/Button`, `/Stack`, `/Table`, `/Dialog`, etc.) wrapped under `<Theme>` and `<LayerProvider>`. The app uses the generated `neutral` theme via `src/astryx-themes.ts`; never hand-edit that generated file. `src/index.css` imports Astryx reset/base styles. Prefer `HStack`/`VStack`/`Grid` and component props; raw `<div>` / inline styles are only permitted as minimal DOM hosts for third-party renderers (e.g., Recharts in `UsageCharts.tsx`, table scroll container in `LogsTable.tsx`).
- **Router & Tables:** Code-based routes in `src/router.tsx` (`SideNavItem` uses `RouterLink` adapter in `App.tsx` mapping `href` -> `to`). `LogsTable.tsx` uses `@tanstack/react-table/legacy` (`useLegacyTable`, `getCoreRowModel`); other tables use Astryx `<Table>` with `T & Record<string, unknown>`, `pixel()`, and `proportional()`.
- **TypeScript Strictness (`tsconfig.app.json`):** `strict`, `verbatimModuleSyntax` (`import type` required), `erasableSyntaxOnly` (no TS `enum`/parameter properties), `noUnusedLocals`, `noUnusedParameters`. Use `@/*` alias for `src/*`; keep files focused and modular.

---

## Important Files

- **Workspace & Infra:** `Cargo.toml`, `crates/ferrogate/Cargo.toml`, `package.json`, `apps/web/package.json`, `Makefile`, `Dockerfile`, `compose.yaml`, `.github/workflows/ci.yml`, `.env.example`, `.editorconfig`.
- **Rust Entry & Core:** `crates/ferrogate/src/main.rs` (graceful shutdown + 10s telemetry drain), `lib.rs` (`AppState`, `build_app`, layer stack), `config.rs`, `db.rs`, `assets.rs`, `error.rs`, `routing/mod.rs`, `telemetry.rs`, `cache.rs`, `tokenizer.rs`, `admin/mod.rs`, `proxy/{mod,chat/finish.rs,streaming.rs}`, `providers/{mod,openai,anthropic,gemini}.rs`.
- **Frontend Entry & Core:** `apps/web/src/main.tsx`, `App.tsx`, `router.tsx`, `index.css`, `astryx-themes.ts`, `api/{client,types}.ts`, `apps/web/vite.config.ts`.

---

## Runtime & Tooling Preferences

- **Toolchains:** Rust `stable` + `clippy` + `rustfmt` (`rust-toolchain.toml`); Node `22` (`.nvmrc`, `engines.node >= 22`) with **npm workspaces** (not pnpm/yarn/bun). `Makefile` requires a POSIX `bash` shell.
- **Linter & Formatter:** `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, and `oxlint` (`apps/web/.oxlintrc.json`: `react`, `typescript`, `oxc`; `react/rules-of-hooks: error`, `react/only-export-components: warn`). No Prettier or ESLint.
- **Environment & Defaults:** `HOST=0.0.0.0`, `PORT=8080`, `DATABASE_URL=sqlite://ferrogate.db?mode=rwc` (`sqlite:///data/ferrogate.db?mode=rwc` in `Dockerfile`), `RUST_LOG=ferrogate=debug,tower_http=debug,info`. Seed migration creates `key-demo-local` (`sk-fg-demo-ferrogate-local-key-01`, $50 cap, 120 RPM, 200k TPM) and inactive sample providers. `/admin/api/*` requires a session cookie from `POST /admin/api/session/login` (bootstrap owner `admin@ferrogate.local` / `ferrogate`), a matching `sec-fetch-site: same-origin` proof on state-changing requests, and a capability check per path (`admin/policy.rs`).

---

## Testing & QA

- **Backend Only (No JS Test Runner):** Frontend QA relies on `npm run lint` (`oxlint`) and `npm run build` (`tsc -b && vite build`). Backend tests use `#[tokio::test]` with one `[dev-dependencies]` entry (`tower`) and zero external network calls.
- **Isolation Pattern:** Each test spins up an ephemeral SQLite file under `std::env::temp_dir()` (converting `\` to `/` for Windows SQLx compatibility) via `ferrogate::db::init_db`, neutralizes seeded rows as needed, binds `127.0.0.1:0`, and uses in-process Axum mock upstreams (`tests/proxy_integration/support/mock.rs`).
- **Async Telemetry Tests:** Because log inserts are batched asynchronously, poll DB state (e.g., `Harness::await_logs`) rather than sleeping fixed intervals.

```bash
cargo test --workspace                       # Full unit + integration suite (requires apps/web/dist built)
cargo test -p ferrogate --lib                # Unit tests (providers, tokenizer, admin helpers)
cargo test -p ferrogate --test proxy_integration
cargo test -p ferrogate --test auth_integration
cargo test -p ferrogate --test admin_integration
cargo test -p ferrogate --test assets_integration
```

<!-- ASTRYX:START -->
Astryx v0.7.15 · 174 components
CLI: run every command as `npx astryx <cmd>` (shown below as `astryx ...`).

SETUP (once, in app entry e.g. main.tsx) — without these, components render unstyled:
  import "@astryxdesign/core/reset.css";
  import "@astryxdesign/core/astryx.css";

WORKFLOW — start every page from a template, never from scratch:
1. `astryx build "<idea>"` — names the template to scaffold from and the parts it lacks.
2. `astryx template <name> <path>` — scaffold it; keep its frame, gap and padding; replace the content.
3. `astryx template <Block>` for parts it lacks; `astryx component <Name>` to read props before using one.
Changing an existing page? Keep it: skip step 2 and add blocks/components inside its sections.
Before you deliver a build, run `astryx doctor --built <dir>`; don't deliver it if it fails.

RULES:
- No <div> — components handle all layout and spacing.
- Read `astryx docs layout` before changing a template's frame.
- Dense data = rows (Table, List/Item), not Card-wrapped lists. Badge = counts only.
- Style with component props first, then tokens (var(--color-*|--spacing-*|--radius-*)). No raw hex/px. No StyleX/Tailwind compiler here — don't use xstyle/utility classes directly. Some templates use StyleX and need a compiler plugin; see `astryx docs styling-overview`.
- Palettes and custom colors go through the theme, never :root overrides. Run `astryx theme add <slug>` to import; `theme eject <slug>` only forks source. See `astryx docs theme`.
- SELF-CHECK: re-read the file; replace any raw <div>/<span>, imported .css/@apply, or hardcoded #hex/px with the component or a token.

`astryx help` lists every command. Key ones beyond the workflow:
  search "<query>"   find any component / hook / doc / template
  discover <words>   integrations you could add
  docs <topic>       getting-started, principles, tokens, theme … (`astryx docs` lists all)
  docs cli           commands and API reference
  docs authoring     build an integration package, and every field it holds
  upgrade --from <old version> --apply   run after a dependency bump
<!-- ASTRYX:END -->
