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
export CODEX_GATEWAY_LOG_LEVEL=info
```

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

Authenticated user/admin API routes use:

```http
Authorization: Bearer cgk_live_{prefix}_{secret}
```

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
```

Codex-compatible routes:

```text
POST /responses
POST /v1/responses
POST /responses/compact
POST /v1/responses/compact
GET  /v1/models
```

The proxy rewrites downstream model names to configured upstream model names, replaces downstream auth with the upstream key, strips hop-by-hop/sensitive headers, and streams SSE responses without buffering the full body.

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
