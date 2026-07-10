---
name: acceptance-runner
description: "Use when acting as the real-machine acceptance runner: perform 验收, end-to-end validation, or proof that the gateway works with a real Codex CLI flow rather than only unit tests or mocks."
---

# Acceptance Runner

"验收" means real-machine acceptance, not just code review or unit tests.

## Operating Rules

Prove the requested behavior; do not rediscover the repository.

- Derive the acceptance surface from the task and `git diff --name-only` before
  running commands. Exercise only affected gateway paths, APIs, and panel views.
- Read this skill first. Read only the relevant `README.md` section and
  `docs/codex-mitm-test-env.md`; inspect source or schema only when observed
  behavior disagrees with the documented fast path.
- Run each requested support check once. A passing build, unit test, or review is
  supporting evidence, never real-machine acceptance.
- Reuse a running `codex-gateway-mitm-lab` container. Never restart or stop a
  pre-existing lab unless the task explicitly requires it.
- Bind the temporary gateway to `0.0.0.0:<port>` so the lab can reach it through
  `host.docker.internal`. Probe reachability before seeding or invoking Codex.
- Treat Codex output as client evidence only. A successful `codex exec` is not
  proof of gateway traversal until a new gateway DB row is observed.
- Record all started PIDs, terminal sessions, and temporary paths immediately.
  Clean up only resources created by the current run, including on failure.

## Dedicated Acceptance Upstream

Use a stable acceptance upstream by default instead of whatever relay happens to
be selected in the local Codex config:

- Base URL: `https://ai.input.im`
- API key: the pinned key in
  `~/.config/codex-gateway/acceptance-upstream.env`, loaded only into an
  environment variable at runtime.

Never copy this key into the skill, repository files, shell history, reports, or
SQLite fixtures. Do not fall back to the future contents of `~/.codex/auth.json`;
if the pinned file is missing, fail the acceptance setup and ask the user to
re-pin the acceptance upstream. Only override this upstream when the task
explicitly requires a different relay, and call that out in the report.

## Required Evidence

For gateway/proxy changes, require all of the following:

- Start a real `codex-gateway` process with a temporary SQLite database.
- Use the repository Codex test environment under `infra/codex-mitm-lab`
  whenever practical.
- Run a real Codex CLI request through the gateway.
- Verify a request reached the temporary gateway and created the expected
  `request_logs` and `daily_usage` deltas.
- Confirm sensitive data is not printed or persisted unexpectedly.

Unit tests, `cargo test`, `cargo check`, frontend builds, and code review are
support checks only. They are not sufficient by themselves for acceptance.

For frontend panel changes, require all of the following:

- Build the frontend with the repository script.
- Start a real backend with a temporary SQLite database and seeded admin login.
- Serve the real Vite app, not a mocked component render.
- Use a browser to log in and exercise the changed panel workflow.
- Confirm the panel calls the expected real backend APIs and shows the expected
  state without console errors or broken layout.

Frontend acceptance is required when changes touch `frontend/`, panel-facing API
contracts, auth/session behavior, or data shapes rendered by the panel. It is
not required for backend-only proxy changes unless the changed data is visible
in the panel.

## Fast Gateway Flow

1. Inspect `git status --short` and `git diff --name-only`, then run only the
   required support checks. Do not rerun a passing check unless files changed.
2. Create a mode-`700` temporary directory with
   `DIR="$(mktemp -d /tmp/codex-gateway-accept.XXXXXX)"`. Choose an unused high
   port and record the gateway PID/session used for cleanup.
3. Source `~/.config/codex-gateway/acceptance-upstream.env` with tracing off.
   Require:
   - `CODEX_GATEWAY_ACCEPTANCE_UPSTREAM_BASE_URL=https://ai.input.im`
   - `CODEX_GATEWAY_ACCEPTANCE_UPSTREAM_API_KEY=<pinned secret>`
   Never echo the key.
4. Start `cargo run --quiet` with:
   - `CODEX_GATEWAY_BIND=0.0.0.0:<port>`
   - `CODEX_GATEWAY_DATABASE_URL=sqlite://$DIR/gateway.db`
   - `CODEX_GATEWAY_APP_SECRET=<temporary secret>`
   - `CODEX_GATEWAY_ADMIN_EMAIL=<temporary email>`
   - `CODEX_GATEWAY_ADMIN_PASSWORD=<temporary password>`
   - `CODEX_GATEWAY_BOOTSTRAP_ADMIN_KEY=cgk_live_<prefix>_<secret>`
   The admin email is required for the bootstrap API key to be seeded.
5. Probe `/healthz` from both host and, if using the lab, the container via
   `http://host.docker.internal:<port>/healthz`.
6. Seed only the upstream and model through the real admin API. Pipe JSON into
   `curl --data-binary @-` so the upstream key is not placed in a process
   argument. Use these response shapes instead of probing the source tree:
   - `POST /api/admin/upstreams` returns the upstream object; read `.id`.
   - `POST /api/admin/models` returns the model object; read `.id`. It does not
     embed mappings.
   - Verify mappings once with
     `GET /api/admin/models/<model-id>/mappings`, which returns an array.
   - `GET /api/admin/api-keys` also returns an array. Do not call it merely to
     recover the key; use the configured bootstrap key as the downstream key.
7. Record baseline `request_logs` and `daily_usage` counts. If the lab pcap is
   available, also record the packet count for the chosen port before Codex runs.
8. Run one real Codex CLI request with the exact isolated provider below. Use a
   non-reserved lowercase provider ID, a dedicated environment variable, direct
   `docker exec` without `bash -lc`, no user config, no WebSocket fallback, and
   no retries:

   ```bash
   CODEX_GATEWAY_E2E_KEY="$CODEX_GATEWAY_BOOTSTRAP_ADMIN_KEY" \
   timeout 180 docker exec --workdir /tmp \
     --env CODEX_GATEWAY_E2E_KEY \
     codex-gateway-mitm-lab \
     codex exec \
       --ignore-user-config --strict-config \
       --skip-git-repo-check --ephemeral --json --sandbox read-only \
       --model gpt-5.5 \
       -c 'model_provider="gateway"' \
       -c 'model_providers.gateway.name="Gateway acceptance"' \
       -c "model_providers.gateway.base_url=\"http://host.docker.internal:${PORT}\"" \
       -c 'model_providers.gateway.wire_api="responses"' \
       -c 'model_providers.gateway.env_key="CODEX_GATEWAY_E2E_KEY"' \
       -c 'model_providers.gateway.requires_openai_auth=false' \
       -c 'model_providers.gateway.supports_websockets=false' \
       -c 'model_providers.gateway.request_max_retries=0' \
       -c 'model_providers.gateway.stream_max_retries=0' \
       'Reply with exactly: ok'
   ```

   Do not use `openai`, `OpenAI`, `openai_base_url`, `OPENAI_API_KEY`, or
   unsupported auth flags such as `-a` for this flow.
9. Wait at most five seconds for the DB count to increase, then query the actual
   schema names:
   - `request_logs`: `request_id`, `path`, `method`, `status_code`, `stream`,
     `user_agent`, `usage_source`, token fields, `error_code`,
     `upstream_status`, `started_at`, and `finished_at`.
   - `daily_usage`: `request_count`, `error_count`, `stream_count`, token fields,
     and `latency_ms_sum`.
   Compare post-run deltas, not merely absolute row presence. For a successful
   streaming request, require a new `/responses` row with `stream = 1` and
   consistent request, stream, and token increments in `daily_usage`.
10. Diagnose a missing DB delta once, using this decision tree:
    - No new packets to `<port>`: the CLI did not reach the gateway. Recheck the
      exact command above; do not search CLI package files or binary strings.
    - New packets but no DB row: query `limit_usage_events` and
      `limit_inflight_requests` once. An unfinalized event or lingering inflight
      row proves proxy admission reached the gateway but request finalization
      failed. Classify either result as a gateway persistence/logging failure and
      stop provider debugging.
    - `401`: verify that the temporary admin email was set before startup and
      that `CODEX_GATEWAY_E2E_KEY` contains the downstream bootstrap key, not the
      upstream key.
    - WebSocket attempts: the provider override did not include
      `supports_websockets=false`.

    For a non-80/443 gateway port, use a packet-count delta from the lab's
    `/flows/codex.pcap` with the filter `tcp port <port>`. Do not print packet
    payloads. The MITM flow analyzer normally omits this hop and is supplemental
    evidence only.
11. Add an error-path probe only when it is relevant to the change. Capture
    before/after DB counts so a rejected pre-upstream request can be proven not
    to create usage rows when that is the contract.
12. Scan logs, CLI output, browser artifacts, and DB text for plaintext secrets.
    Exclude the deliberately secret mode-`600` env file itself from this scan;
    otherwise it guarantees a false positive. Report only match/no-match.
13. Stop only the gateway/frontend processes started by this run, remove the
    temporary files, and leave the pre-existing MITM container running.

## Frontend Panel Flow

Use this in addition to the gateway flow when frontend acceptance is required:

1. Run `npm run build` once in `frontend/`.
2. Start a temporary backend with `CODEX_GATEWAY_ADMIN_EMAIL`,
   `CODEX_GATEWAY_ADMIN_PASSWORD`, `CODEX_GATEWAY_BOOTSTRAP_ADMIN_KEY`, a
   temporary SQLite database, and an isolated backend port.
3. Seed representative data needed by the changed view through the real admin
   API. For proxy-visible pages, reuse the dedicated acceptance upstream and run
   the gateway flow first so request and usage rows are real.
4. Account for the checked-in Vite proxy, which targets
   `http://127.0.0.1:8080`. Never point a random-port backend at the normal Vite
   config and assume it is connected. Use port `8080` only when it is free;
   otherwise create a uniquely named, untracked temporary config inside
   `frontend/`, using bare `vite` and `@vitejs/plugin-react` imports and proxying
   `/api`, `/v1`, and `/responses` to the temporary backend. A config under
   `/tmp` or `DIR` cannot reliably resolve the frontend plugins. Pass it through
   `npm run dev -- --config <file>`, then delete it and confirm the worktree
   returned to its pre-run state. Do not stop or reuse an unrelated service
   already bound to `8080`. Do not use `npm run preview` for live API acceptance
   without an explicit reverse proxy.
5. Before browser work, prove one API request through the frontend origin reaches
   the temporary backend. Then log in and visit only the routes relevant to the
   change.
6. Verify the changed workflow directly: create/update/refresh/copy/revoke or
   read-only inspection, depending on the feature. Confirm network requests hit
   the real backend and return expected status codes.
7. Check browser console errors, visible error banners, loading states that never
   resolve, and obvious layout overlap at desktop and mobile widths when the
   changed UI could be responsive.
8. Capture the relevant frontend API status codes and visible state, then clean
   up only processes and files created by this run.

## Report

Report:

- Verdict: real acceptance pass or fail.
- Exact environment, versions, and commands, with secrets redacted.
- Client result, gateway packet delta, routes, DB deltas, and status codes.
- Frontend network, console, and layout evidence when the panel flow applies.
- Cleanup performed.
- Blockers or limitations.
