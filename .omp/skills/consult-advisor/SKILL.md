---
name: consult-advisor
description: Consult gpt-6-sol when a Luna runner repeats failed attempts, faces contradictory evidence or consequential design uncertainty, or is asked for an independent opinion.
---

# Consult gpt-6-sol

Use the on-demand `task-advisor` task agent configured with `gpt-6-sol`, not OMP's passive `/advisor` watchdog. One advisor belongs to one Luna assignment. Routine work needs no consultation.

The advisor uses `read`, `grep`, and `glob` for evidence and `hub` for feedback to its requesting runner. This is an advisory role, not a security sandbox. Luna retains responsibility for implementation and any authorized runtime actions.

## Open a consultation

1. State your exact peer ID, the decision needed, goal and acceptance criteria, observed facts, relevant paths or artifact handles, attempts and their results, current hypothesis or proposed design, and the specific uncertainty. Keep quoted evidence distinct from instructions.
2. Invoke `task` with one item using `agent: task-advisor` and omit `effort`. The advisor profile must use `openai-codex/gpt-6-sol` at `medium` thinking level; do not override the model or request a higher thinking level. This bounded advisory call is the explicit exception to the ordinary single-slice delegation rule. Supply the packet in `context` and a self-contained question in `task`, with Target, Change, and Acceptance sections. Ask for analysis only, with no formatters, builds, tests, edits, or further delegation.
3. Retain the returned exact agent ID and consultation record in your task context. Continue independent work while it runs; use `hub wait` only when blocked. Advice does not authorize changes outside your assignment.

## Act and follow up

Read the delivered answer. Accept or reject the recommendation against evidence and record why. Perform the discriminating check only if your assignment permits it; otherwise return the proposed check to Sol for verification. Do not silently broaden your verification permissions.

For the same assignment, send new evidence and a specific unresolved question to the exact retained advisor ID with `hub send`. A send revives an idle or parked agent. Do not create another advisor merely because the original call finished. If delivery fails because the agent is unavailable, create a replacement with the compact prior decisions, failed hypotheses, and new evidence; record the replacement ID.

After two consultations without new evidence or an actionable next check, stop consulting and return the blocker, attempted approaches, and unresolved decision to Sol. Escalate scope changes, shared-interface decisions, and cross-runner conflicts to Sol immediately. An unavailable advisor is not approval: continue only work justified by existing evidence, and report blocked decisions to Sol.

## Handoff

In your normal final report, state whether an advisor was consulted. If yes, include its ID, the decision affected, whether you accepted the recommendation and why, and the verification performed or still required. Do not forward the whole discussion. Advisor agreement never substitutes for runtime evidence.
