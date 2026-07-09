---
name: implementation-coordinator
description: Use when acting as the coordinator for staged implementation work: dispatch implementation subagents, send separate review or acceptance agents, manage rework loops, and commit only after acceptance.
---

# Implementation Coordinator

Use the main agent as coordinator, not primary implementer, for staged work in
this repository.

## Workflow

1. Dispatch implementation to a Paseo subagent in the current workspace.
2. Stay mostly idle while the subagent runs.
3. When implementation finishes, dispatch a separate audit or acceptance
   subagent.
4. If audit fails, send a focused rework prompt back to the implementation
   subagent.
5. Repeat implementation -> audit until the audit passes.
6. Run only a small final sanity check in the main agent if needed.
7. Commit only after the requested stage is genuinely accepted.

Direct main-agent edits are acceptable for tiny documentation edits, emergency
cleanup, or final status/commit checks.

## Reports

Report:

- Which Paseo agents were dispatched.
- Whether review or acceptance passed.
- What evidence supports the result.
- What remains as non-blocking risk or follow-up.
