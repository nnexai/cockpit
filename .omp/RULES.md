# Cockpit delivery rules

- Work in vertical increments: define one observable outcome, delegate independent slices, integrate once, verify the real surface, then commit.
- A Sol orchestrator delegates research, implementation, and review to Luna workers. It directly edits only the shared integration boundary or a tiny repair.
- Record live user observations as queued acceptance items. Interrupt one affected worker only when continuing is unsafe or its contract became invalid; send factual neutral steering.
- Exercise Herdr mutations and terminal ownership only in uniquely named disposable sessions, never the user's active session.
- Claim an increment complete only after its acceptance checks pass and its owned changes have a commit hash.

## Luna advisory consultations

- Sol may assign the existing Luna profiles unchanged; each may consult `task-advisor` for a bounded second opinion without requesting approval. This is the explicit exception to single-question and single-slice delegation limits, not permission to delegate implementation.
- Luna follows `skill://consult-advisor` when its runner instructions trigger consultation. One advisor is reused per assignment. Sol retains scope, shared contracts, integration, and verification ownership; an advisor cannot expand a runner's assignment.
- The gpt-6-sol advisor uses medium thinking. Launch `task-advisor` without an `effort` argument. Never override the model or request a higher thinking level. This is an instruction policy, not a runtime-enforced ceiling; Luna's own effort remains unrestricted by it.
- Keep at least two task levels available for Sol → Luna → task-advisor. Do not launch consulting runners at the recursion ceiling or disable their `task`/`hub` tools. The task-advisor profile is read-only and cannot spawn workers.
- Sol's handoff review includes consequential advisory decisions and actual verification, not advisor approval. The passive `/advisor` watchdog is separate and is not required for this workflow.
