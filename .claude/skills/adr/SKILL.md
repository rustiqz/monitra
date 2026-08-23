---
name: adr
description: Write an Architecture Decision Record into DESIGN.md section 9 in the established format, or supersede an existing one. Use whenever a design decision is made, reversed, or revised — especially when it changes a design principle, the dependency graph, or the roadmap.
---

# Writing an ADR

ADRs live in `docs/DESIGN.md` §9, numbered sequentially. Check the highest existing number
first.

## Format — match the existing entries exactly

```markdown
### ADR-0NN — <decision as a short statement, not a question>

**Status:** Accepted (Phase N)

**Context:** What forced a decision. The constraint or conflict, not background.

**Decision:** What was chosen. Present tense, specific.

**Alternatives:**
- *Option* — what it offered, why it lost. Name the principle it violated where one applies.

**Consequences:** ✅ what we gain. ❌ what we pay. Both are mandatory.
```

## Rules

- **Supersede, never delete.** The closing line of DESIGN.md is explicit: the reasoning behind
  an abandoned choice is often more valuable than the choice itself. Mark the old ADR
  `**Superseded by ADR-0NN**`, keep its body, and say in the new one what specifically
  is reversed — often it is one clause, not the whole decision. ADR-002 is the worked example:
  superseded, but its reasoning about *why SQLite is the right default* still stands.
- **Every alternative must be real.** An alternatives list of strawmen is worse than none —
  it manufactures false confidence. If there was genuinely only one option, say so and explain
  why the space was that narrow.
- **❌ consequences are not optional.** An ADR with only upsides was not a decision.
- **Cascade the change.** An ADR that alters the crate graph must also update §3.2, the
  Appendix layout, `scripts/dep-check.py`, and `CLAUDE.md`. One that alters scope must update
  §1.3 and §10. Leaving DESIGN.md internally inconsistent is the failure mode to avoid.
- **Record the date and phase**, so a future reader knows what was and was not known.
