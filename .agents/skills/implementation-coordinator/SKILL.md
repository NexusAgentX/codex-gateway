---
name: implementation-coordinator
description: Use when acting as the coordinator for staged implementation work: dispatch implementation-agent, requirements-reviewer, and acceptance-runner subagents, manage rework loops, and commit only after acceptance.
---

# Implementation Coordinator

Use the main agent as coordinator, not primary implementer, for staged work in
this repository.

## Workflow

1. Dispatch an `implementation-agent` Paseo subagent in the current workspace
   for scoped code changes. Tell it not to commit.
2. Stay mostly idle while the implementation agent runs.
3. When implementation finishes, dispatch a separate `requirements-reviewer`
   subagent to check the current task and available requirements.
4. If requirements review fails, send a focused rework prompt back to the
   implementation agent.
5. Repeat implementation -> requirements review until the review passes.
6. When the user asks for 验收 or the work needs end-to-end proof, dispatch an
   `acceptance-runner` subagent for real-machine acceptance.
7. If acceptance fails, send focused rework back to the implementation agent and
   rerun review/acceptance as needed.
8. Run only a small final sanity check in the main agent if needed.
9. Commit only after the requested stage is genuinely accepted.

Direct main-agent edits are acceptable for tiny documentation edits, emergency
cleanup, or final status/commit checks.

## Reports

Report:

- Which Paseo agents were dispatched.
- Whether review or acceptance passed.
- What evidence supports the result.
- What remains as non-blocking risk or follow-up.
