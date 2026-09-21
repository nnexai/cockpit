# GLAB-02 — standalone merge-request implementation

## Accepted plan

Accepted by Main, 2026-09-21, against settled GLAB-01 source interfaces under OBS-011. Baseline commit remains `6593cf4ecba87687c809ee9f3d2a295c757db787`; returned source changes are uncommitted and unaccepted. GLAB-01 relinquishes its source/provider/core-projects writing locks, not its acceptance requirements. All original GLAB-02 criteria and completion dependencies remain required.

Observable outcome: a standalone MR resolves/imports as `<full-project-path>!<iid>`, retains verified fork and review provenance, detects bounded metadata/discussion changes independently of SHA, and supplies its exact source branch to setup. No issue/Jira dependency or remote mutation.

### Ownership and fixed interfaces

- MR adapter worker owns only `cockpit-providers/src/gitlab.rs`: extend the existing GET-only, authority-checked, shared-budget adapter. Preserve issue behavior. Required and optional review fields, discussions/individual notes, available approvals, deterministic ordering/deduplication, escaping, component revision data, incomplete diagnostics and unsupported local-anchor status follow the original brief. Missing/deleted source refs do not prevent source import. Metadata-only setup lookup requires a valid source branch and full head commit.
- MR core worker owns `cockpit-core/src/repositories.rs`, `sources.rs`, `projects/defaults.rs`, and `projects.rs`, plus necessary existing gh/tea and core test constructor migrations. Extend the strict resolver for `/-/merge_requests/IID`; resource type `review`. Add only internal `SourceMetadata.source_commit: Option<String>`; GitLab review metadata supplies a validated full 40/64-hex commit, existing providers migrate to None without new behavior. No new protocol/source-cache model.
- Core setup uses provider source branch when no branch was explicitly supplied. For metadata carrying a source commit, verify the exact local source ref and expected commit without fetch/clone; a conflicting existing local branch or missing ref is diagnosed, not replaced. A verified origin tracking ref may supply a new local branch when no local branch exists. Pin reviewed setup base to the verified commit; preserve explicit branch choices and reject, rather than silently overwrite, an incompatible explicit base. Revalidation checks current provider/local commit before mutation using existing stale-plan behavior. Source import itself remains independent of local ref availability.
- Main integrates the publication preflight after the core worker hands back `projects.rs`. Publication worker owns `project_store.rs` separately. No concurrent `projects.rs` edits.
- Main owns generated files, task records, integration, eventual verification and commits. No MR worker edits browser/frontend, installer, publication store, protocol, or generated files.

### Verification and blockers

Writing workers skip formatters, linters, builds, tests and runtime. Retain meaningful deterministic regressions for identity, stable-SHA component changes, budget/permission incompleteness and source-ref safety; do not add plumbing assertions. Required provider/source-identity and standalone setup reviews, focused checks, captured GET-only commands, actual import/refresh/conflict flow and real authorized MR proof remain pending for the final verification rounds.

OBS-012 supersedes the original missing-fixture authorization blocker. Owned issue #2 and standalone draft MR !1 exist in project 86672117; the MR has a dedicated non-main branch and no issue/Jira dependency. See `gitlab-fixtures.json`. Production adapter requests remain GET-only; GH PR and Jira scope remain deferred.

## Evidence

A bounded real `GitlabSourceProvider::fetch`/metadata smoke through installed `glab` passed for standalone MR !1: canonical ID `nnex.ai/integration!1`, complete=true, empty diagnostics, owned description/discussion marker, source branch `cockpit-fixture/csg-a3e9b950-20260921` and exact head `8493e6e72e56df62006b084eb211f2f5156390e0`. It required no issue/Jira linkage, made no remote changes and removed its temporary Rust example afterward. Sanitized evidence: [GLAB-adapter-evidence.json](GLAB-adapter-evidence.json).

The integrated compiler, frontend typecheck/build and host build passed for the adapter delivery `7d06e6e4225cec48fe5847f9bc4e95e1c04bc1e0`. Source branch/head setup integration landed in `163ee85fba8c883f80b5b24871de84bd210a7823`. These checks do not claim the standalone MR setup or an exhaustive refresh/conflict matrix.

The real resource-panel smoke imported standalone MR !1, rendered the ready/materialized review row and opened its merge-request metadata, discussion/note, approvals and owned body marker. The MR row remained after refreshing the issue. See [GLAB-resource-ui-evidence.json](GLAB-resource-ui-evidence.json). Remaining integrated checks follow the OBS-015 batch, with no extra per-task verification loop.
