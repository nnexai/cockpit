# Cockpit delivery rules

- Work toward one observable outcome and assess progress by verified improvement or new diagnostic evidence. Do not turn illustrative durations into deadlines or impose a default time limit; honor only limits explicitly set by the user.
- Record each meaningful attempt as hypothesis, result, what it rules out, and next check. After two failed attempts on the same criterion, stop editing until new evidence supports a repair. Allow at most one review/repair wave.
- Sol owns scope, consequential design decisions, and integration; Luna remains the default implementation worker. Assign one owner per coherent repair. Delegate additional work only where it creates useful concurrency or addresses a named review risk; the parent may complete bounded work directly when another handoff adds no value. Focused checks are allowed under exclusive or isolated ownership; no shared validation gates while concurrent writers mutate the integration surface.
- Reuse real-surface acceptance scenarios, avoid duplicate unchanged gates, and classify failures as product, automation, stale runtime, or missing prerequisite before product edits. Explicit pauses and workflow corrections are immediate control instructions, not queued observations; pause remains sticky until explicit resume.
- Exercise Herdr mutations and terminal ownership only in uniquely named disposable sessions, never the user's active session. Claim completion only with truthful acceptance evidence and any required owned-change commit.

## Luna advisory consultations

- Sol may assign the existing Luna profiles unchanged; each may consult `task-advisor` for a bounded second opinion without requesting approval. This permits advisory consultation, not delegation of implementation.
- Luna follows `skill://consult-advisor` when its runner instructions trigger consultation. One advisor is reused per assignment. Sol retains scope, shared contracts, integration, and verification ownership; an advisor cannot expand a runner's assignment.
- The gpt-6-sol advisor uses medium thinking. Launch `task-advisor` without an `effort` argument. Never override the model or request a higher thinking level. This is an instruction policy, not a runtime-enforced ceiling; Luna's own effort remains unrestricted by it.
- Keep at least two task levels available for Sol → Luna → task-advisor. Do not launch consulting runners at the recursion ceiling or disable their `task`/`hub` tools. The task-advisor profile is read-only and cannot spawn workers.
- Sol's handoff review includes consequential advisory decisions and actual verification, not advisor approval. The passive `/advisor` watchdog is separate and is not required for this workflow.
