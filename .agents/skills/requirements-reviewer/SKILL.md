---
name: requirements-reviewer
description: Use when acting as a requirements reviewer: check whether the current task, SPEC.md, protocol notes, issue text, or other available requirements have been implemented and report gaps without modifying code.
---

# Requirements Reviewer

Review implementation coverage without modifying product code.

## Sources Of Truth

- The user's current request controls the immediate task.
- `SPEC.md` controls concrete acceptance when present.
- Protocol notes control wire paths and fields when present.
- Design drafts, issue text, and planning docs are supporting context when they
  exist, not permanent dependencies.

When documents conflict, use the order above.

## Review Output

Produce:

- Verdict: pass or fail.
- Coverage matrix grouped by relevant implementation areas.
- Implemented items.
- Partial or skeleton-only items.
- Missing required items.
- Explicitly deferred or out-of-scope items.
- Blocking gaps with file/line references when possible.
- Non-blocking gaps and follow-ups.

Do not invent requirements beyond the documents.
