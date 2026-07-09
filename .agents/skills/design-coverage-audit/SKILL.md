---
name: design-coverage-audit
description: Use when checking whether docs/design.md, docs/codex-protocol.md, SPEC.md, or phase-one requirements have been implemented, including design coverage audits and gap analysis.
---

# Design Coverage Audit

Audit coverage without modifying product code.

## Sources Of Truth

- `docs/codex-protocol.md` controls protocol paths and wire fields.
- `SPEC.md` controls concrete phase-one acceptance.
- `docs/design.md` controls broader product intent and future design.

When documents conflict, use the order above.

## Audit Output

Produce:

- Verdict: pass or fail.
- Coverage matrix grouped by backend, proxy, storage, auth, admin API,
  frontend, tests, and docs.
- Implemented items.
- Partial or skeleton-only items.
- Missing phase-one-required items.
- Explicitly deferred or non-phase-one items.
- Blocking gaps with file/line references when possible.
- Non-blocking gaps and follow-ups.

Do not invent requirements beyond the documents.
