# codex-gateway Phase-One Acceptance Spec

This spec turns `docs/design.md` and `docs/codex-protocol.md` into a concrete phase-one checklist. Protocol paths and wire fields follow `docs/codex-protocol.md`; product behavior follows `docs/design.md`.

## Scope

Phase one delivers a single-node Codex relay gateway:

- Rust/Axum backend with SQLite storage, migrations, tracing, env config, and health checks.
- Downstream API key auth using `Authorization: Bearer cgk_*` keys stored only as keyed hashes.
- Admin/user JSON API skeleton for users, API keys, upstreams, models, request logs, and daily usage.
- Codex-compatible proxy routes:
  - `POST /responses`
  - `POST /v1/responses`
  - `POST /responses/compact`
  - `POST /v1/responses/compact`
  - `GET /v1/models`
- Priority-first routing over enabled, non-down upstream/model mappings.
- Safe header forwarding, upstream `Authorization` rewrite, hop-by-hop header stripping, and SSE streaming without full-body buffering.
- Best-effort request logging and usage extraction without storing prompt, completion, downstream secret, or upstream secret material.
- A compileable React/TypeScript/Vite frontend skeleton for the planned panel routes.

Not in phase one:

- WebSocket Responses proxying.
- Image generation API support.
- OpenAI OAuth/login reverse engineering.
- Billing, organizations, fine-grained RBAC, distributed deployment, or Prometheus metrics.

## Configuration Acceptance

The backend must read these environment variables:

- `CODEX_GATEWAY_BIND`, default `127.0.0.1:8080`.
- `CODEX_GATEWAY_DATABASE_URL`, default `sqlite://data/codex-gateway.db`.
- `CODEX_GATEWAY_APP_SECRET`, required for production and used for keyed API-key hashing.
- `CODEX_GATEWAY_PUBLIC_URL`, default `http://localhost:8080`.
- `CODEX_GATEWAY_LOG_LEVEL`, default `info`.
- Optional bootstrap values:
  - `CODEX_GATEWAY_ADMIN_EMAIL`
  - `CODEX_GATEWAY_ADMIN_PASSWORD`
  - `CODEX_GATEWAY_BOOTSTRAP_ADMIN_KEY`

Startup must create the SQLite file parent directory when possible, run migrations, and seed the bootstrap admin user/key when configured.

## Storage Acceptance

Migrations must create:

- `users`
- `api_keys`
- `upstreams`
- `models`
- `upstream_models`
- `request_logs`
- `daily_usage`

Acceptance checks:

- Downstream API key plaintext is never persisted.
- `api_keys.key_prefix` and `api_keys.key_hash` are indexed.
- Enabled/visible model and upstream mappings have indexes suitable for routing and `/v1/models`.
- Request logs include request id, user id, key id, model/upstream ids, method/path/status, stream flag, usage counters, usage source, latency, started/finished timestamps, and sanitized client metadata.
- `daily_usage` has a uniqueness constraint over date/user/key/model/upstream for upserts.

## Auth Acceptance

- Valid downstream auth format is `Authorization: Bearer cgk_live_{prefix}_{secret}`.
- The gateway locates by prefix and verifies a keyed hash in constant time.
- Disabled, revoked, expired, or unknown keys are rejected with OpenAI-compatible error JSON.
- Disabled users are rejected.
- Admin API routes require an authenticated user with role `admin`.
- User API routes require an authenticated active user and must scope records to that user unless the route is explicitly admin-only.

## Proxy Acceptance

For `POST /responses` and `POST /v1/responses`:

- Parse JSON only enough to read `model` and `stream`.
- Preserve unknown request fields.
- Rewrite `model` to the selected upstream model name when a mapping exists.
- Forward to the selected upstream using the canonical `/responses` path appended to the configured upstream base URL.
- If `stream: true` or the upstream returns `text/event-stream`, stream bytes to the downstream without buffering the full response.

For compact routes:

- Use canonical `/responses/compact`.
- Treat as unary JSON by default.
- Preserve request and response JSON fields.

For `/v1/models`:

- Return enabled, visible gateway model names from local configuration in an OpenAI/Codex-compatible object with a `data` array.

Header behavior:

- Strip hop-by-hop headers, `host`, and downstream `authorization`.
- Set upstream `Authorization: Bearer <upstream_key>`.
- Preserve safe Codex/OpenAI tracing headers including `x-codex-*`, `x-openai-*`, and `openai-*`.
- Strip sensitive upstream response headers before returning to the downstream.

Error behavior:

- Gateway-authored errors use `{ "error": { "message", "type", "code" } }`.
- Upstream responses with bodies are returned as-is where practical.
- No fallback is attempted after a streaming response has started.

## Routing Acceptance

- Default strategy is `priority`.
- A route is eligible when the gateway model is enabled, the upstream mapping is enabled, and the upstream is enabled and not marked `down`.
- Priority order is:
  1. `upstream_models.priority`
  2. `upstreams.priority`
  3. deterministic id order
- Weighted and sticky strategies may be skeletal in phase one, but selection must be deterministic enough to test and must not choose disabled/down routes.

## Usage Acceptance

- Request logs are written for proxied attempts.
- Non-streaming JSON responses are scanned for `usage.input_tokens`, `usage.output_tokens`, and `usage.total_tokens`.
- SSE streams are scanned opportunistically for `response.completed.response.usage` while bytes pass through.
- If upstream usage is unavailable, use `usage_source = "unknown"` or an explicitly marked estimate.
- Prompt, completion, tool definitions, encrypted content, API keys, cookies, and `prompt_cache_key` are not logged.

## Frontend Acceptance

The frontend skeleton must include routes/pages for:

- Overview
- Requests
- API Keys
- Upstreams
- Models
- Users
- Settings

It may use placeholder API calls and light styling in phase one, but it must be clearly separated under `frontend/` and be ready to wire to backend JSON APIs.

## Test Acceptance

Phase one should include focused Rust tests for:

- API key generation, prefix parsing, hashing, and verification.
- Config defaults and env parsing.
- Routing selection/model mapping.
- Request/response header filtering.
- Health endpoint and at least one proxy path using a mock upstream when practical.

Before handoff, run:

```bash
cargo fmt
cargo test
cargo check
```

If the frontend dependency toolchain is installed, also run the frontend build/check.
