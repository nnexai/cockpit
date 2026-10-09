# WS-25 Docs restructure

Wave 4 · Size M · Depends on: all other workstreams merged · Blocks: –

## Goal
A newcomer (human or agent) can get the mental model in one sitting. Each document has one job, says each thing once, and describes only the current product.

## Owns
- `CONTEXT.md`, `DECISIONS.md`, `CODE_GUIDE.md`
- `docs/*`, including `docs/supervisor-surfaces.md` and a new `docs/verification-log.md`
- `integrations/agent-skills/*/SKILL.md` (wording only)
- `.omp/AGENTS.md`: pointer lines only

## Evidence
- The three docs total ~30,000 words, with 101 paragraphs over 700 characters.
- Supervisor invariants are restated in CODE_GUIDE (~53-85), CONTEXT (~328-358), DECISIONS (~52-73) and `docs/supervisor-surfaces.md`.
- Dated live-verification logs are embedded, e.g. CONTEXT ~451 and CODE_GUIDE ~248 and ~283.
- Migration clauses are listed in `../migration-inventory.md`, "Docs".
- Each worker's handoff carries doc notes.

## Change
1. **CONTEXT.md:** product, domains, runtime architecture and ownership boundaries. Short paragraphs, no verification history, no per-flag detail. Target: under 250 lines.
2. **DECISIONS.md:** one rule per bullet, each with a link to the module that enforces it. The rationale is one sentence; history goes to `archive/`.
3. **CODE_GUIDE.md:** the where-to-change table, updated for every moved or split file in this plan, plus short call flows. Configuration examples move to `docs/configuration.md`.
4. **`docs/verification-log.md`:** every dated live-verification paragraph, moved verbatim.
5. Remove the migration and adoption clauses per the inventory and the WS-03/05/06 notes. Update the provider-token entry list (WS-11).
6. Trim the historical sections in `docs/supervisor-surfaces.md`. It stays the surface inventory.
7. Replace decision IDs (`D17`, `S6`, …) with plain words wherever they remain.

## Keep
- Every rule that is still in force.
- `docs/keyboard-shortcuts.md`, which stays generated.

## Acceptance
- Every backticked repository path in the docs exists. Check this with a small script and include its output in the handoff.
- No paragraph over 700 characters in the three root docs.
- Interpretation smoke: a fresh agent given only the docs answers these correctly, with file references:
  1. Who owns tab membership vs placement?
  2. Where do I change a Library follow refresh?
  3. How does a supervisor worker get Execute authority?
  4. How are provider tokens stored?
  5. How do I run a disposable UI smoke?
