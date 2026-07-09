# Agent Workflow

This repository uses a delegated Paseo workflow. When the user asks for
implementation, review, acceptance, or staged project work, follow this process
instead of doing all work directly in the main agent context.

## Main Agent Role

The main agent should act as coordinator and monitor:

1. Dispatch implementation work to a Paseo subagent.
2. Stay mostly idle while the subagent runs.
3. When implementation finishes, dispatch a separate audit/acceptance subagent.
4. If audit fails, send a focused rework prompt back to the implementation
   subagent.
5. Repeat implementation -> audit until the audit passes.
6. Only after passing audit, run a small final local sanity check if needed.
7. Commit only after the requested stage is genuinely accepted.

Do not prematurely replace this with direct main-agent implementation just
because the task is tempting or easy. Direct work is acceptable for tiny
documentation edits, emergency cleanup, or final commit/status checks.

## Acceptance Standard

"验收" means real-machine acceptance, not just code review or unit tests.

For gateway/proxy features, acceptance should include:

- Starting a real `codex-gateway` process with a temporary database.
- Using the repository Codex test environment under `infra/codex-mitm-lab`
  whenever practical.
- Running a real Codex CLI request through the gateway.
- Verifying gateway logs and SQLite rows such as `request_logs` and
  `daily_usage`.
- Confirming sensitive data is not printed or persisted unexpectedly.

Unit tests, `cargo test`, `cargo check`, frontend builds, and code review are
necessary support checks, but they are not sufficient by themselves when the
user asks for acceptance.

## Real Codex Test Preference

For end-to-end gateway validation:

1. Prefer `scripts/codex-mitm-lab.sh` and the existing lab container.
2. Point the lab Codex provider `base_url` at the local gateway.
3. Configure the gateway to forward to the real relay from local Codex config.
4. Use a minimal non-sensitive prompt.
5. Redact all auth tokens, cookies, API keys, prompts, and completions in
   reports.
6. Clean up temporary gateway processes, databases, logs, and lab artifacts
   started by the acceptance run.

If the MITM analyzer cannot capture a non-80/443 gateway hop, gateway logs and
SQLite evidence are acceptable as long as a real Codex CLI request traversed
the gateway.

## Reporting

Reports should say clearly:

- Which Paseo agents were dispatched.
- Whether review/acceptance passed or failed.
- What commands or real-machine flows were run.
- What evidence proves the result.
- What remains as non-blocking risk or follow-up.
