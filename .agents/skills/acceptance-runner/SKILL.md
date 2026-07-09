---
name: acceptance-runner
description: Use when acting as the real-machine acceptance runner: perform 验收, end-to-end validation, or proof that the gateway works with a real Codex CLI flow rather than only unit tests or mocks.
---

# Acceptance Runner

"验收" means real-machine acceptance, not just code review or unit tests.

## Required Evidence

For gateway/proxy features, acceptance should include:

- Start a real `codex-gateway` process with a temporary SQLite database.
- Use the repository Codex test environment under `infra/codex-mitm-lab`
  whenever practical.
- Run a real Codex CLI request through the gateway.
- Verify gateway logs and SQLite rows such as `request_logs` and `daily_usage`.
- Confirm sensitive data is not printed or persisted unexpectedly.

Unit tests, `cargo test`, `cargo check`, frontend builds, and code review are
support checks only. They are not sufficient by themselves for acceptance.

## Preferred End-To-End Flow

1. Use `scripts/codex-mitm-lab.sh` and the existing lab container when possible.
2. Point the lab Codex provider `base_url` at the local gateway.
3. Configure the gateway to forward to the real relay from local Codex config.
4. Use a minimal non-sensitive prompt.
5. Redact all auth tokens, cookies, API keys, prompts, and completions in
   reports.
6. Clean up temporary gateway processes, databases, logs, and lab artifacts
   started by the run.

If the MITM analyzer cannot capture a non-80/443 gateway hop, gateway logs and
SQLite evidence are acceptable only when a real Codex CLI request traversed the
gateway.

## Report

Report:

- Verdict: real acceptance pass or fail.
- Exact environment and commands, with secrets redacted.
- Routes observed, database rows, log evidence, and status codes.
- Cleanup performed.
- Blockers or limitations.
