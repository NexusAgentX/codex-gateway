# Stage 0 Compatibility Baseline

Captured on 2026-07-12 from revision `20beb66`, before the planned structural
stages. This file records externally visible shapes and persistent objects that
later stages must preserve. Route behavior remains defined by the implementation
and tests; samples below use placeholders instead of real credentials or IDs.

## HTTP routes

There are 46 distinct paths and 55 method/path combinations. `Panel` marks the
user-facing routes; their current authentication accepts either a panel bearer
session or a gateway API key. `Admin` routes additionally require the admin
role, and `API key` routes use a gateway API key.

| Method | Path | Access |
| --- | --- | --- |
| GET | `/healthz` | Public |
| POST | `/api/login` | Public |
| GET | `/api/me` | Panel |
| GET | `/api/models` | Panel |
| GET | `/api/overview` | Panel |
| GET, POST | `/api/api-keys` | Panel |
| GET | `/api/api-keys/{id}/usage` | Panel, own key |
| POST | `/api/api-keys/{id}/disable` | Panel, own key |
| POST | `/api/api-keys/{id}/revoke` | Panel, own key |
| GET | `/api/requests` | Panel |
| GET | `/api/analytics` | Panel |
| GET | `/api/usage/daily` | Panel |
| GET | `/api/usage/summary` | Panel |
| GET | `/api/limits` | Panel |
| GET, POST | `/api/admin/users` | Admin |
| PATCH | `/api/admin/users/{id}` | Admin |
| POST | `/api/admin/users/{id}/password` | Admin |
| GET, PATCH | `/api/admin/users/{id}/limits` | Admin |
| GET, POST | `/api/admin/api-keys` | Admin |
| GET | `/api/admin/api-keys/{id}/usage` | Admin |
| GET, PATCH | `/api/admin/api-keys/{id}/limits` | Admin |
| POST | `/api/admin/api-keys/{id}/disable` | Admin |
| POST | `/api/admin/api-keys/{id}/revoke` | Admin |
| GET, POST | `/api/admin/upstreams` | Admin |
| PATCH | `/api/admin/upstreams/{id}` | Admin |
| POST | `/api/admin/upstreams/{id}/disable` | Admin |
| POST | `/api/admin/upstreams/{id}/health` | Admin |
| GET, POST | `/api/admin/models` | Admin |
| PATCH | `/api/admin/models/{id}` | Admin |
| GET, POST | `/api/admin/models/{id}/mappings` | Admin |
| PATCH | `/api/admin/model-mappings/{id}` | Admin |
| POST | `/api/admin/model-mappings/{id}/disable` | Admin |
| GET | `/api/admin/requests` | Admin |
| GET | `/api/admin/analytics` | Admin |
| GET | `/api/admin/usage/daily` | Admin |
| GET | `/api/admin/usage/summary` | Admin |
| GET | `/api/admin/metrics` | Admin |
| GET | `/api/admin/limits` | Admin |
| PATCH | `/api/admin/limits/system` | Admin |
| POST | `/api/admin/retention/run` | Admin |
| GET, PATCH | `/api/admin/settings` | Admin |
| POST | `/responses` | API key |
| POST | `/v1/responses` | API key |
| POST | `/responses/compact` | API key |
| POST | `/v1/responses/compact` | API key |
| GET | `/v1/models` | API key |

## Representative responses

Successful health check, `GET /healthz`, status `200`:

```json
{"status":"ok","service":"codex-gateway"}
```

Successful login, `POST /api/login`, status `200`:

```json
{
  "user": {"id":"<user-id>","email":"user@example.com","role":"user"},
  "token":"<panel-token>",
  "token_type":"panel"
}
```

Authenticated identity, `GET /api/me`, status `200`:

```json
{
  "user_id":"<user-id>",
  "api_key_id":"panel:<session-id>",
  "key_prefix":"panel",
  "email":"user@example.com",
  "role":"user"
}
```

Gateway model list, `GET /v1/models`, status `200`:

```json
{
  "object":"list",
  "data":[{
    "id":"codex-mini",
    "display_name":"codex-mini",
    "object":"model",
    "type":"model",
    "created_at":"<timestamp>"
  }]
}
```

Proxy response bodies for the four response routes are owned by the selected
upstream and are forwarded without a gateway response DTO. The integration test
fixture's representative non-streaming response is:

```json
{
  "model_seen":"upstream-codex-mini",
  "auth_seen":"Bearer <upstream-key>",
  "unknown_seen":{"preserve":true},
  "usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3}
}
```

Authentication error, status `401`:

```json
{
  "error":{
    "message":"invalid API key",
    "type":"gateway_error",
    "code":"invalid_api_key",
    "details":null
  }
}
```

Concurrency rejection, status `429`:

```json
{
  "error":{
    "message":"concurrent request limit exceeded",
    "type":"limit_error",
    "code":"concurrency_limited",
    "details":{
      "scope":"system",
      "subject_id":"",
      "limit_name":"concurrency",
      "limit":1,
      "used":1,
      "reset_at":null
    }
  }
}
```

## Database tables

The eight migrations through `20260710000600_gap4_runtime_config.sql` produce
13 application tables:

| Table | Primary identity and compatibility note |
| --- | --- |
| `users` | Text `id`; unique `email`; role and status checks. |
| `api_keys` | Text `id`; unique prefix and hash; belongs to `users`. |
| `upstreams` | Text `id`; unique name; encrypted API key fields and runtime timeout mode. |
| `models` | Text `id`; unique public name. |
| `upstream_models` | Text `id`; unique model/upstream/upstream-name mapping. |
| `request_logs` | Text `id`; `request_id` is indexed but intentionally not unique. |
| `daily_usage` | Text `id`; unique date/user/key/model/upstream dimensions. |
| `admin_audit_logs` | Text `id`; records actor, action, resource, status, and metadata. |
| `limit_policies` | Composite primary key `(scope, subject_id)`. |
| `limit_usage_events` | Text `id`; tracks request/token usage finalization. |
| `limit_rate_counters` | Composite key `(scope, subject_id, window_started_at)`. |
| `limit_inflight_requests` | Text `id`; tracks expiring concurrent admissions. |
| `system_config` | Singleton integer key constrained to `id = 1`. |

## Test inventory

`cargo test -- --list` reports 81 Rust tests:

| Target | Tests | Benchmarks |
| --- | ---: | ---: |
| `src/lib.rs` unit tests | 20 | 0 |
| `src/main.rs` unit tests | 0 | 0 |
| `tests/phase_one.rs` integration tests | 61 | 0 |
| Doc tests | 0 | 0 |
| **Total** | **81** | **0** |

The frontend package currently has a build script but no configured test script.
