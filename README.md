# codex-gateway

Phase-one Rust/Axum gateway for aggregating Codex-compatible upstream relays.

The current implementation focuses on the usable core: SQLite migrations, API-key auth, model/upstream routing, Codex Responses proxying, SSE pass-through, request logging, usage extraction, and a lightweight React panel skeleton.

## Docs

- [Follow-up gap spec](SPEC.md): remaining work after the completed phase-one MVP.
- [Design draft](docs/design.md): product, architecture, database, routing, UI, milestones.
- [Codex protocol](docs/codex-protocol.md): wire protocol endpoints, headers, payloads, SSE, usage, errors, and MITM evidence.
- [Codex MITM test environment](docs/codex-mitm-test-env.md): local reproducible lab setup.
- [Agent notes](AGENTS.md): pointer to project-level agent skills.

## Backend

### Configure

```bash
export CODEX_GATEWAY_BIND=127.0.0.1:8080
export CODEX_GATEWAY_DATABASE_URL=sqlite://data/codex-gateway.db
export CODEX_GATEWAY_APP_SECRET='replace-with-a-long-random-secret'
export CODEX_GATEWAY_SECRET_KEY_VERSION=1
export CODEX_GATEWAY_PUBLIC_URL=http://127.0.0.1:8080
export CODEX_GATEWAY_PANEL_ORIGINS=http://localhost:5173
export CODEX_GATEWAY_LOG_LEVEL=info
export CODEX_GATEWAY_REQUEST_LOG_RETENTION_DAYS=90
export CODEX_GATEWAY_DAILY_USAGE_RETENTION_DAYS=730
export CODEX_GATEWAY_RETENTION_RUN_ON_STARTUP=true
```

Outside development (`CODEX_GATEWAY_ENV=production`, `staging`, etc.),
`CODEX_GATEWAY_APP_SECRET` must be set to a non-default value of at least 32
characters. The secret signs panel tokens, hashes downstream API keys, and
derives encryption keys for stored upstream API keys.

`CODEX_GATEWAY_PUBLIC_URL` is allowed by CORS by default. Add any separate
browser panel origins with comma-separated `CODEX_GATEWAY_PANEL_ORIGINS`.

Request-log and daily-usage retention are configurable with the retention
variables above. Set either day count to `0` to disable that deletion class.
When enabled, startup runs the same idempotent retention job exposed at
`POST /api/admin/retention/run`.

Optional bootstrap admin seed:

```bash
export CODEX_GATEWAY_ADMIN_EMAIL=admin@example.com
export CODEX_GATEWAY_ADMIN_PASSWORD='change-me'
export CODEX_GATEWAY_BOOTSTRAP_ADMIN_KEY='cgk_live_adminprefix_replace_with_random_secret'
```

`CODEX_GATEWAY_BOOTSTRAP_ADMIN_KEY` must already use the `cgk_live_{prefix}_{secret}` shape. Startup stores only its keyed hash. Without this variable, the admin user can still be seeded but no initial API key is created.

### Run

```bash
cargo run
```

Startup creates the SQLite parent directory when needed and runs `migrations/`.

### Production Build

Build the Vite panel and embed it into the release binary:

```bash
scripts/build-release.sh
```

The resulting `target/release/codex-gateway` serves the panel and API from the
same listener. Copy `.env.example` to a deployment-specific environment file,
replace every secret placeholder, then load it before starting the binary:

```bash
set -a
source .env.production
set +a
./target/release/codex-gateway
```

Browser routes such as `/overview` use the embedded SPA fallback. Unknown API
routes under `/api`, `/v1`, and `/responses` continue to return `404`.

### Test and Check

```bash
cargo fmt -- --check
cargo test
cargo check
```

## API Shape

Public health:

```text
GET /healthz
```

Authenticated user/admin API routes use either a normal downstream API key or
the scoped panel token returned by `POST /api/login`:

```http
Authorization: Bearer cgk_live_{prefix}_{secret}
Authorization: Bearer cgw_panel_{signed_payload}
```

Codex-compatible proxy routes only accept `cgk_live_*` downstream API keys.
Panel tokens are limited to the web/admin API surface.

Core routes:

```text
POST /api/login
GET  /api/me
GET  /api/overview
GET  /api/api-keys
POST /api/api-keys
GET  /api/requests
GET  /api/usage/daily

GET  /api/admin/users
POST /api/admin/users
GET  /api/admin/api-keys
GET  /api/admin/upstreams
POST /api/admin/upstreams
GET  /api/admin/models
POST /api/admin/models
GET  /api/admin/requests
GET  /api/admin/usage/daily
GET  /api/admin/metrics
POST /api/admin/retention/run
```

All responses include `x-request-id`. If the request supplies a valid
`x-request-id`, the gateway reuses it; otherwise it generates one. Proxy request
logs store the same ID, or an `-N` suffixed attempt ID when a non-streaming
request retries across multiple upstreams.

`GET /api/admin/requests` and `GET /api/requests` accept sanitized filters:
`user_id` (admin only), `key_id`/`api_key_id`, `model_id`, `upstream_id`,
`status`, `from`, `to`, and `limit`. Date filters accept RFC3339 timestamps or
`YYYY-MM-DD`.

`GET /api/admin/metrics` is admin-gated and returns aggregate counts, latency,
token totals, and upstream health/error summaries only. It does not return
prompts, completions, API keys, cookies, or raw client metadata.

Codex-compatible routes:

```text
POST /responses
POST /v1/responses
POST /responses/compact
POST /v1/responses/compact
GET  /v1/models
```

The proxy rewrites downstream model names to configured upstream model names,
replaces downstream auth with the upstream key, strips hop-by-hop/sensitive
headers, preserves Codex/OpenAI tracing headers, and streams SSE responses
without buffering the full body. If a streaming client disconnects, the gateway
cancels the upstream stream and finalizes the request log with
`client_disconnected`.

Unsupported provider endpoints are intentionally not implemented until a real
downstream Codex use case needs them:

```text
POST /v1/chat/completions
POST /v1/images/generations
WebSocket /responses
POST /realtime/calls
POST /memories/trace_summarize
ANY  /codex/{path}
```

Codex CLI should be configured with `wire_api = "responses"` and should not
enable WebSocket provider support for this gateway.

## Secret Storage And Rotation

Upstream API keys are encrypted before storage as `cgwenc_v1` records with a
per-row `api_key_secret_version`. On startup, after migrations and before
serving traffic, the gateway automatically rewrites legacy plaintext upstream
rows with `api_key_secret_version = 0` using the configured
`CODEX_GATEWAY_APP_SECRET` and current `CODEX_GATEWAY_SECRET_KEY_VERSION`.
New creates and updates also always write encrypted values.

Secret-version rotation path after legacy rows have been encrypted:

1. Keep `CODEX_GATEWAY_APP_SECRET` stable.
2. Increase `CODEX_GATEWAY_SECRET_KEY_VERSION`, for example from `1` to `2`.
3. Restart the gateway.
4. Re-save or update each upstream API key through the admin API or panel so it
   is re-encrypted with the new version.
5. Verify `upstreams.api_key_secret_version` has advanced and old raw keys do
   not appear in a database text scan.

Changing `CODEX_GATEWAY_APP_SECRET` is a broader credential rotation: existing
downstream API keys, panel tokens, and encrypted upstream keys will no longer
verify/decrypt. Plan that as a maintenance event with regenerated downstream
keys and re-entered upstream keys.

Admin mutations are recorded in `admin_audit_logs` with actor, action,
resource, status, timestamp, and sanitized metadata. Passwords, API keys,
cookies, prompts, and completions are not stored in audit or request logs.

## SQLite Backup And Restore

For a live SQLite database, prefer SQLite's online backup command so the copy is
consistent while the gateway is running:

```bash
sqlite3 data/codex-gateway.db ".backup 'backups/codex-gateway-$(date +%Y%m%d-%H%M%S).db'"
```

To restore, stop the gateway, copy the selected backup over the configured
database path, and start the gateway again so migrations run normally:

```bash
systemctl stop codex-gateway
cp backups/codex-gateway-YYYYMMDD-HHMMSS.db data/codex-gateway.db
cargo run
```

If the database uses WAL mode in a future deployment, copy `*.db`, `*.db-wal`,
and `*.db-shm` together when using file-level backups. Never back up by dumping
panel tokens, API keys, or upstream keys to logs.

## Minimal Seed Flow

1. Start with `CODEX_GATEWAY_ADMIN_EMAIL`, `CODEX_GATEWAY_ADMIN_PASSWORD`, and `CODEX_GATEWAY_BOOTSTRAP_ADMIN_KEY`.
2. Create an upstream:

```bash
curl -sS http://127.0.0.1:8080/api/admin/upstreams \
  -H "Authorization: Bearer $CODEX_GATEWAY_BOOTSTRAP_ADMIN_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "primary",
    "base_url": "https://upstream.example.com",
    "api_key": "upstream-secret",
    "priority": 1,
    "weight": 1
  }'
```

3. Create a model mapping using the returned upstream id:

```bash
curl -sS http://127.0.0.1:8080/api/admin/models \
  -H "Authorization: Bearer $CODEX_GATEWAY_BOOTSTRAP_ADMIN_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "public_name": "codex-mini",
    "upstream_mappings": [
      {
        "upstream_id": "returned-upstream-id",
        "upstream_model_name": "upstream-codex-mini",
        "priority": 1
      }
    ]
  }'
```

4. Create user keys with `POST /api/api-keys` or admin-created users plus their own authenticated session/key flow.

## Frontend

The frontend skeleton lives in `frontend/` and contains these panel routes:

- Overview
- Requests
- API Keys
- Upstreams
- Models
- Users
- Settings

Run it during development:

```bash
cd frontend
npm install
npm run build
npm run dev
```

Vite proxies `/api`, `/v1`, and `/responses` to `http://127.0.0.1:8080`.

## License

MIT
