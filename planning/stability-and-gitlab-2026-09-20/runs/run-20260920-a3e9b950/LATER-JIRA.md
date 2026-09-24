# LATER-JIRA — Jira work items: refresh, conflicts and failures (2026-09-24)

## Identity and scope

The user activated this task on 2026-09-24 (OBS-049). `e6709202b0b3f0eb5541913665d7b34e62510566` delivered the provider and link-first setup; `SETUP-link-first.md` proves a real import during setup. This run covers the remaining acceptance: refresh, local-edit conflict and failures, through the real browser UI.

**Environment.** Herdr 0.9.1, jira-cli 1.7.0, headless Chrome via `playwright-cli` (private in-memory session `cjira`), 1440×900.

**Fixture.** Root `/tmp/csetup-cvlyzamz`, Herdr session `csetup76960d32` (PID 553535), gateway `127.0.0.1:53573` (PID 553566), built from the working tree at `3497787` plus the `SourceImport.tsx` change below. Herdr and Cockpit state were private to the root. jira-cli used the user's existing configuration through `JIRA_CONFIG_FILE` and a linked `.netrc`.

**Remote fixture mutation.** Two comments were added to the user's test item `SCRUM-5` ("Cockpit fixture refresh check 1/2 (csetup76960d32)"). Before: 1 comment, updated `2026-09-24T18:44:53.898+0200`. After: 3 comments, updated `2026-09-24T19:31:37.290+0200`. Cockpit itself only ran `jira issue view --raw`.

## Results

| Step | Observed |
| --- | --- |
| Setup from the SCRUM-5 link, repository `sou`, Enter | Space `cockpit/source/SCRUM-5` created; Context → Resources lists the item as `jira · issue · fresh · unchanged`. |
| Refresh, no remote change | "fresh, unchanged; materialized". File SHA-256 unchanged (`6ebf18aa…`). |
| Remote comment 1, Refresh | `changed · materialized`, `source_provider_changed` with previous/current revision (the item's `updated` time) and content hashes. The file now has 2 `## Jira comment` sections including the new text (`JIRA-refresh-updated.png`). |
| Local edit to the imported file, remote comment 2, Refresh | `conflict · conflict`, "Local edits preserved; refresh needs reconciliation", `source_materialization_conflict`. The file kept the local edit byte for byte (SHA-256 `6a79af7a…` before and after) and did not gain comment 2; the new provider content went to the immutable cache (`JIRA-refresh-conflict.png`). |
| Import `…/browse/SCRUM-99999` | `source_not_found: Jira work item does not exist or is not visible to the configured login`. Jira answers 404 both for a missing item and one the login may not see, so permission and absence share this message. |
| Import `https://other-site.atlassian.net/browse/SCRUM-5` | `Jira URL does not belong to the configured Jira site`. |
| Import a board URL | `Jira URL must open a work item (…/browse/KEY-123)`. |
| Import `…/browse/scrum-5` | `Jira work item key is malformed`. |
| Import SCRUM-5 with the fixture `.netrc` replaced by an empty file | `source_auth_required: Jira CLI is not logged in to this site; run \`jira init\`` (`JIRA-auth-required.png`). After restoring it, the missing-item import returned `source_not_found` again. |

Bounded comments are covered by the provider test `renders_facts_description_and_comments_and_reports_partial_comments` (incomplete when Jira's `total` exceeds the included comments); the document is capped by the shared source byte limit.

## Defect found and repaired

The Import form showed every failure as `http_error: …`, because the HTTP client puts the server's code in `operationCode`. `SourceImport.tsx` now shows `operationCode` when present. Verified above: `source_auth_required` and `source_not_found` are displayed.

## Checks

- `bunx tsc --noEmit` passes; `bun run test` 242 passed; frontend rebuilt before the auth/not-found checks.
- The Rust provider and resolver tests from `e6709202` are unchanged.

## Not covered

- Reconciling a conflict (choosing local or provider content) is the existing generic source behavior and was not exercised for Jira.
- Native (Tauri) Context panel with a Jira source; the change is shared frontend code.

## Cleanup

Browser session `cjira` closed. Herdr and gateway exited 0, PIDs 553535 and 553566 gone, port 53573 released, no process referenced the root, root removed. The two comments on `SCRUM-5` are kept as fixture history.
