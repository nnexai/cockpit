# LATER-GHPR — Add GitHub pull request source support

Status, dependencies, owner, locks and completion proof: [task ledger](../tasks.json). Follow the [orchestrator contract](../ORCHESTRATOR.md).

## Outcome

Future, separately authorized support for GitHub through gh pull requests. This is explicitly deferred and is not necessary to close the currently selected GitLab/polish campaign.

## Evidence and starting points

[GitHub #6](https://github.com/nnexai/cockpit/issues/6) requests this provider in addition to GitLab. Read its complete body and the completed GitLab source identity/cache contracts before activating this task. Current source starts: `crates/cockpit-providers/src/`, `crates/cockpit-core/src/repositories.rs`, `sources.rs`, `projects/defaults.rs`, and the existing source UI.

## Changes

On explicit scope activation, replace deferred status with a bounded implementation plan grounded in the installed CLI/selected host. Implement pull requests as independently addressable typed artifacts through the existing provider/source/setup/refresh pipeline. Keep credentials CLI-owned, host/project identity verified, pagination/bytes bounded, and remote reads non-mutating. Preserve actual PR source branch/review metadata, body/comments freshness and optional related issues/Jira links; no related ticket is required.

## Non-goals

Not an authorization to implement now. No remote writes, credential framework, automatic cross-provider traversal without bounds, or dependency imposed on standalone GitLab MRs.

## Acceptance

On activation: canonical URL/kind/identity, configured-host authority, malformed/auth/permission failures, bounded comments, independent primary artifact, refresh/local-edit conflicts and a real authorized end-to-end source import must all pass. Existing gh/tea/glab behavior must remain unchanged. Define concrete fixtures and platform evidence before dispatch, not during final closure.

## Verification

No verification is claimed or required while deferred. Future implementation uses focused resolver/provider regressions and real authenticated reads in an explicitly selected fixture, with browser/native evidence for changed UI/transport contracts.

## Handoff

Remain `deferred`, `required: false`, with no fabricated owner/evidence/commit. #6 remains partially open until this scope and its sibling are actually delivered or the user explicitly changes that issue's scope. Activation requires updating the campaign graph/coverage or creating a separate linked campaign.
