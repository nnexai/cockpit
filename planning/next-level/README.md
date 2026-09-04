# Cockpit next-level plan

Prepared 2026-09-04 against commit `7e8fe25`. Planning and HTML prototypes only. Application code and Herdr-server are unchanged.

Status: selected direction is detect-and-replace for real Herdr extension panes. Initial planning verification is complete. No extension IPC, plugin changes, or Herdr-server changes are required.

## Start here

1. [Architecture and shared contracts](01-architecture-and-contracts.md): authority, identities, configuration, transport, and storage boundaries.
2. [Project setup and lifecycle](02-project-setup-and-lifecycle.md): local repository selection, artifact entry, worktree setup, environment handoff, recovery, and destruction.
3. [Context and sources](03-context-and-sources.md): companion folders, local repository snapshots, normalized cache, provider downloads, freshness, reviews, wiki, and telemetry.
4. [Viewer and reference comments](04-viewer-and-reference-comments.md): file tree, Markdown/Mermaid, other previews, search, collected comments, and paste-only delivery.
5. [Optional building blocks](05-optional-building-blocks.md): the rest of the product vision as independently selectable stories.
6. [Delivery sequence](06-delivery-sequence.md): dependencies, parallel lanes, integration ownership, milestones, and acceptance gates.
7. [Extension pane replacements](07-extension-panes.md): detection, Herdr-owned layout, complete graphical review, and optional TUI backport.
8. [UI design](08-ui-design.md) and [interactive mock index](mocks/index.html): final pane, setup, review, and recovery designs. All data and terminal activity are simulated.
9. [Maintainability pass](09-maintainability.md): separate pre-feature cleanup, a personal code-tweaking guide, and optional follow-up cleanup.

10. [Second interaction checkpoint](10-interaction-checkpoint.md) and [consolidated workflow lab](mocks/workflow.html): keyboard/mouse efficiency, fast setup, collected comments, and delivery outcomes.

11. [Deterministic quality gates](11-quality-gates.md): CLEAN-05 coverage/complexity/CRAP reporting, mutation testing, pinned tools, and changed-code agent feedback.

## Confirmed scope from the planning interview

The user requested a thorough plan for all missing features, including implementation instructions, a sequence, explicit parallel work, UI designs conveyed through HTML mocks, and a commit containing the completed planning artifacts. No feature implementation is authorized in this task.

The main experience is local-first. Repository selection is normally the first step; an issue or review URL can also start the flow, but the user still confirms a local primary repository. Arbitrary URL downloads and cloning remote source repositories are outside the main scope.

Additional existing local repositories from the same discovery catalog can be added to the companion context. Optimize independent snapshots with CoW reflinks and fall back to ordinary copies. Hardlinks would share writable inodes and are therefore unsuitable for the intended isolation.

The context viewer stays read-only. It provides a hierarchical file browser, Markdown with Mermaid, and safe other-content previews. Whole-file comments and selected-line comments can be collected across files before sending. The outgoing text contains file paths, comments, and the selected original lines with line numbers. Sending means pasting into an active agent pane in the same tab, never submitting it.

The existing dense graphical workbench is the visual starting point. Spaces and their state remain visible; Agents stays a separate attention queue. Herdr-server remains the authority and must not be modified. At the requested checkpoint, the user chose real extension panes instead of docks: detect the existing file-viewer/Reviewr pane and replace its presentation with a full Cockpit GUI. Do not communicate with the extension or change Herdr-server. Build/Review are not workbench modes.

## Main loop and optional scope

Begin with the separately scoped CLEAN maintainability increment. A usable main delivery then includes FND-01-03, PANE-01-02, LIFE-01-04, CTX-01-02, SRC-01-03, VIEW-01-03, and REF-01-02. It must support manually created context without a functioning provider. It must support an agent terminal without context, and context without a currently available agent.

The complete graphical review replacement has its own REV-01/02 stories. It can follow the main Context loop and does not depend on remote review downloads. A future TUI backport is PANE-03. Review/MR ingestion, wiki, and telemetry have their own stories, SRC-04-06. They are fully planned but can ship after the Gitea issue path. Provider expansion, settings, history, inbox extensions, optional automation, credentials, remote access, packaging, and release accessibility are separate selectable blocks in the optional plan. “Optional” means not required for the main loop, not forgotten or implicitly approved for implementation.

## Authority and decision handling

`CONTEXT.md` and `DECISIONS.md` remain the repository's architecture records. This package proposes the next contracts and identifies where those records need an explicit update. Historical bootstrap and session-mirror plans remain completed history; do not turn them into current implementation instructions.

Some earlier assumptions do not match installed Herdr capability:

- Herdr extensions are terminal-backed panes. Cockpit detects supported extensions and chooses a graphical renderer inside their real Herdr rectangles. The existing terminal process continues independently.
- Worktree create/open cannot inject environment into the initial terminal. Context-aware Cockpit-created tabs/panes can receive explicit environment afterward.
- Display metadata is not a durable arbitrary context association store. An owned companion manifest carries provenance; Herdr still decides live resource state.
- Closing a Space and deleting a worktree are different operations.
- Existing client-shell text input has no end-to-end paste acknowledgement. Reference delivery needs a narrow, validated operation with explicit accepted/rejected/unknown results.

These are evidence-backed design corrections, not requests to change Herdr-server. The implementation must test their observable behavior in disposable sessions before relying on them.

## Research and confidence

- [Current capability audit](../../research/next-level-capability-audit.md): installed schema, current source seams, and Herdr source behavior.
- [Reviewr reference](../../research/next-level-review-reference.md): installed plugin comment format and delivery semantics, with Cockpit differences called out.
- [Context feasibility](../../research/next-level-context-feasibility.md): official provider, filesystem, Markdown/Mermaid, search, and watcher references.

Installed/source inspection establishes feasibility, not a passing runtime implementation. The HTML mock proves its own interactions only. Each implementation story includes the tests and browser/native runtime evidence needed to claim completion later.

## Personal-tool priority

Cockpit is intended to become the user's primary way of engaging with local projects, not a product for other users. Favor direct, understandable code, local defaults, and a short edit/test/run loop. Make common behavior changes local to a clear owning module. Distribution, multi-user access, credentials infrastructure, and provider breadth remain elective plans, not prerequisites or reasons to build a framework.

## Initial-plan validation

Reviewed with three bounded Luna research/review lanes and root integration. Local Markdown links and whitespace checks pass. Chromium exercised the final initial mocks at 1440×900 and 1024×640, with no horizontal page overflow or JavaScript errors, plus selected comment, placement, fallback, setup/retry, and teardown interactions. No application build/test or live feature smoke is claimed: production code was not changed, and feature runtime probes are planned acceptance work.

## Quality infrastructure addition

The maintainability pass includes deterministic feedback before new feature work: a proposed CRAP target of at most 8 for new functions (6 as a preferred target), reviewed legacy baselines, coverage checks, and mutation testing. [Tooling research](../../research/next-level-quality-gates.md) records the formula, compatibility evidence, and adoption probes. The infrastructure is planned, not installed or implemented by this task.
