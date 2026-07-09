---
name: implementation-coordinator
description: Use when acting as the coordinator for staged implementation work: dispatch implementation-agent, code-reviewer, requirements-reviewer, and acceptance-runner subagents, manage rework loops, and commit only after acceptance.
---

# Implementation Coordinator

Use the main agent as coordinator, not primary implementer, for staged work in
this repository.

## Workflow

1. Dispatch an `implementation-agent` Paseo subagent in the current workspace
   for scoped code changes. Tell it not to commit.
2. Stay mostly idle while the implementation agent runs.
3. When implementation finishes, dispatch `code-reviewer` and
   `requirements-reviewer` subagents in parallel.
4. Wait for both review agents to return before deciding whether to rework or
   advance.
5. If either review fails, merge the blocking findings into a focused rework
   prompt and send it back to the implementation agent.
6. Repeat implementation -> parallel code/requirements review until both
   reviewers pass.
7. When the user asks for 验收 or the work needs end-to-end proof, dispatch an
   `acceptance-runner` subagent for real-machine acceptance.
8. If acceptance fails, send focused rework back to the implementation agent and
   rerun review/acceptance as needed.
9. Run only a small final sanity check in the main agent if needed.
10. Commit only after the requested stage is genuinely accepted.

Direct main-agent edits are acceptable for tiny documentation edits, emergency
cleanup, or final status/commit checks.

## Subagent Prompts

Do not rely on implicit skill loading inside Paseo subagents. Every subagent
prompt must explicitly state the agent identity and require the subagent to read
its project skill before acting:

- `implementation-agent`: read
  `.agents/skills/implementation-agent/SKILL.md`.
- `code-reviewer`: read `.agents/skills/code-reviewer/SKILL.md`.
- `requirements-reviewer`: read
  `.agents/skills/requirements-reviewer/SKILL.md`.
- `acceptance-runner`: read `.agents/skills/acceptance-runner/SKILL.md`.

Keep the task prompt specific: include the current objective, allowed scope,
whether edits are allowed, required checks, and the expected report format.

## Reports

Report:

- Which Paseo agents were dispatched.
- Whether review or acceptance passed.
- What evidence supports the result.
- What remains as non-blocking risk or follow-up.
