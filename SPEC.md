# codex-gateway Follow-Up Gap Spec

This spec replaces the completed phase-one acceptance checklist. Phase one has
already proven that a real Codex CLI request can traverse the gateway. This file
tracks the remaining work needed to turn the MVP into a practical operator/user
product.

## Baseline

Already working and treated as regression-sensitive:

- Rust/Axum backend startup with SQLite migrations.
- Bootstrap admin/key flow.
- `cgk_live_*` API-key authentication with hashed downstream keys.
- Codex Responses proxy routes:
  - `POST /responses`
  - `POST /v1/responses`
  - `POST /responses/compact`
  - `POST /v1/responses/compact`
  - `GET /v1/models`
- SSE pass-through to a real upstream relay.
- Priority fallback over enabled, non-down upstream/model mappings.
- Request logging, daily usage aggregation, and sanitized metadata storage.
- React/Vite frontend skeleton.
- Project agent workflow skills under `.agents/skills/`.

Do not regress these behaviors while closing the gaps below.

## Gap 1: Complete Operator CRUD

Admin APIs should support day-to-day operation without direct database edits.

Required backend work:

- Users:
  - list
  - create
  - update role/status/display name
  - reset password
- API keys:
  - list all
  - create for a user
  - disable/revoke
  - show prefix/status/last used/expires, never plaintext after creation
- Upstreams:
  - list
  - create
  - update base URL, key, enabled flag, priority, weight, timeout, retries,
    health-check path
  - disable
  - manual health check
- Models:
  - list
  - create
  - update description/enabled/visibility
  - manage upstream mappings
  - disable mappings

Acceptance:

- Admin routes require admin auth.
- User-scoped routes cannot access other users' records.
- Mutations validate inputs and return structured error JSON.
- Tests cover update/disable/revoke paths and authorization failures.

## Gap 2: Wire The Frontend To Real APIs

The frontend should become a usable panel, not only route placeholders.

Required frontend work:

- Login page using `POST /api/login`.
- Auth token persistence with logout.
- Overview page with real request/usage data.
- API Keys page:
  - create key
  - show plaintext once
  - list/revoke keys
- Requests page with status, model, upstream, latency, usage, and error code.
- Upstreams page with CRUD and manual health check.
- Models page with mappings editor.
- Users page for admins.
- Settings page for server/config summary.

Acceptance:

- `npm run build` passes.
- Frontend handles loading, empty, error, and unauthorized states.
- No in-app mock data remains on pages with implemented APIs.
- Basic responsive layout works on desktop and mobile widths.

## Gap 3: Secret Storage And Production Safety

The MVP avoids logging secrets, but upstream secrets are not encrypted at rest.

Required work:

- Encrypt upstream API keys before storing them.
- Add key versioning or a clear rotation path.
- Require a strong `CODEX_GATEWAY_APP_SECRET` outside development.
- Tighten CORS to configured panel/public origins by default.
- Replace login-created API keys as panel sessions with a safer session/JWT
  mechanism or clearly scoped panel token.
- Add audit logs for admin mutations without storing secrets.

Acceptance:

- Database scans do not reveal downstream keys, upstream keys, passwords,
  cookies, prompts, or completions.
- Secret rotation is documented and tested.
- Production-like config refuses default secrets.
- CORS tests prove untrusted origins are rejected when configured.

## Gap 4: Routing, Retry, And Health Management

Phase one has priority fallback, but routing is not yet production-grade.

Required work:

- Implement `weighted` routing.
- Implement `sticky_by_key` routing.
- Honor per-upstream `max_retries`.
- Add a background health-check worker.
- Track degraded/down transitions with timestamps and recent error samples.
- Avoid retrying after any streaming response has begun.
- Expose route decisions in request logs without leaking secrets.

Acceptance:

- Tests prove disabled/down upstreams are skipped.
- Tests prove weighted/sticky strategies are deterministic enough to validate.
- Tests prove retryable statuses and connect/timeout errors use the next
  eligible upstream.
- Health worker can be enabled/disabled in config.

## Gap 5: Observability And Operations

Operators need enough visibility to debug live traffic.

Required work:

- Structured request IDs in logs and responses.
- Metrics endpoint or export path for:
  - request count
  - error count
  - latency
  - upstream health
  - token usage
- Request-log filters by user, key, model, upstream, status, date range.
- Retention policy for request logs and daily usage.
- Document backup/restore for SQLite.
- Optional Dockerfile/compose for the gateway itself.

Acceptance:

- Operators can identify failing upstreams from the panel/API.
- Metrics do not expose prompt, completion, or secret material.
- Retention job is safe to run repeatedly.

## Gap 6: Compatibility Hardening

The gateway should stay compatible with Codex Responses traffic as upstreams
evolve.

Required work:

- Add integration coverage for `/responses/compact`.
- Add end-to-end SSE disconnect/cancel handling.
- Preserve and test important Codex/OpenAI tracing headers.
- Document unsupported endpoints explicitly.
- Keep `/v1/chat/completions` and image generation out of scope unless a real
  downstream use case appears.

Acceptance:

- Real Codex CLI acceptance still passes after changes.
- Compact route has mock-upstream and real-machine validation where practical.
- Disconnect tests prove logs are finalized with a clear status.

## Required Review Flow

Each implementation stage should follow the project agent workflow:

1. `implementation-agent` makes scoped changes and does not commit.
2. `code-reviewer` and `requirements-reviewer` run in parallel.
3. Both reviewers must pass before the stage advances.
4. `acceptance-runner` performs real-machine acceptance when the change touches
   gateway/proxy behavior or when the user asks for 验收.
5. The coordinator commits only after the required checks pass.

## Out Of Scope Until Requested

- Billing, payment, invoices.
- Organizations and multi-tenant hierarchy.
- Fine-grained RBAC beyond admin/user.
- Distributed deployment.
- Plugin strategy engine.
- OpenAI OAuth reverse engineering.
