---
name: phase-start
description: Open a Monitra roadmap phase. Reads that phase's DESIGN.md sections, restates the contract and acceptance gate, surfaces unresolved choices as an explicit question, and produces a brief for approval BEFORE any code is written. Use whenever starting a numbered phase or any non-trivial change.
---

# Opening a phase

The user requires a brief before implementation and a decision finalised before code.
This skill produces that brief. **Write no implementation code while running it.**

## 1. Load the contract

Read `docs/DESIGN.md` for the phase in question. Map:

| Phase | Sections that govern it |
|---|---|
| 1 Project setup | §3.2, §4, ADR-001, ADR-005, ADR-007, Appendix |
| 2 CLI base | §3.4, §4 `cli`, P3, §11.7, §11.8 |
| 3 Provider layer | §4 `provider`, §4.1, ADR-007, §11.7, §11.8 |
| 4 Storage | §4 `storage`, §5, §5.4, ADR-002, §11.1 |
| 5 Backend API | §4 `backend`, §3.3, P6 |
| 6 Engine | §6 (all), §5.3, §7.2, §11.2, §11.3, §11.5 |
| 7 Events + notify | §4.1, §7.2, §7.3, §1.3 alerting boundary |
| 8 TUI | §4 `tui`, §5.2, §11.4 |
| 9 Web dashboard | ADR-004, §8, P3 |
| 10 Bundling | §1.5, §8, §11.6, §11.9 |

Also read the phase's row in the §10 roadmap table — the **Gate** column is the definition
of done, and it is not negotiable downward.

## 2. Check for unresolved questions

Scan §11. If any open question governs this phase, it must be **decided before coding**, not
discovered mid-implementation. Present each as a choice with real trade-offs — including which
design principle each option serves or violates. Use `AskUserQuestion`.

Never resolve an open question silently. §11 exists so an unknown is not mistaken for a
settled decision; quietly picking an answer defeats the whole mechanism.

## 3. Produce the brief

Keep it short enough to actually read:

- **Scope** — what will exist when this phase is done, and what will not.
- **Files** — every file to be created or modified, one line each on its purpose.
- **Flow** — any control or data flow introduced (a small diagram if it helps).
- **Decision points** — what is being decided in code, and the reasoning.
- **Gate** — the exact commands and assertions that will prove the phase is done.
- **Risks** — anything that could invalidate a DESIGN.md claim.

## 4. Wait

Stop. Get explicit approval. Then implement.

## 5. After implementing

Hand back, in this order:

1. **Per-file summary** — what each touched file now contains and why.
2. **Flow walkthrough** — how the pieces interact at runtime, if this phase added any.
3. **Decision points implemented** — including any judgement call made during the work
   that was not in the brief. These especially.
4. **`phase-verify` output** — the real output, including any failures.
5. **A proposed commit message.** Do not commit. The user commits.
6. **DESIGN.md deltas** — the roadmap status cell, any ADR to add, any §11 item now resolved
   or newly discovered.
