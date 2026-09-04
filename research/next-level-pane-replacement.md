# Pane replacement feasibility (no extension communication)

Status: evidence audit, 2026-09-04. Herdr source is `/tmp/herdr-cockpit-master` at `3150bd92d6162ba248bc28fd4d40bd0d6238e1af`. No Cockpit or Herdr source is changed by this note.

## Selected shape

Cockpit can launch a normal Herdr terminal pane and replace that pane's rectangle with a Cockpit rendered context surface. The helper process is only a terminal fallback. Cockpit does not require an extension handshake, IPC channel, helper protocol, or server modification. Herdr remains authoritative for pane identity, focus, layout rectangle, terminal lifetime, and close events; Cockpit owns the GUI surface and its local selection/render state.

The preferred launch path is a formally installed plugin entrypoint. `plugin.pane.open` accepts `plugin_id`, `entrypoint`, `workspace_id`, placement, cwd, focus, and env (`/tmp/herdr-cockpit-master/src/api/schema/plugins.rs:417-439`) and returns `PluginPaneInfo` with the resulting pane (`:465-469`). Herdr injects `HERDR_PLUGIN_ID` and `HERDR_PLUGIN_ENTRYPOINT_ID` into the launched terminal (`/tmp/herdr-cockpit-master/src/app/api/plugins/panes.rs:177-238`). The open response is a receipt, not a durable client registry.

## Detection after launch or reconnect

1. Call `plugin.list` read-only and locate the configured plugin ID and pane entrypoint. `InstalledPluginInfo` exposes manifest path, plugin root, enabled state, pane manifests, and warnings (`/tmp/herdr-cockpit-master/src/api/schema/plugins.rs:37-68`). Reject disabled, warning-bearing, or ambiguous manifests unless the user explicitly repairs the configuration.
2. Open the pane and retain the returned `pane_id`, `terminal_id`, plugin ID, entrypoint, workspace/tab IDs, and a random Cockpit association generation in the companion manifest. The `plugin.pane.open` result is the strongest provenance for this process lifetime.
3. On reconnect, enumerate live panes from `session.snapshot`, then call `pane.process_info` for candidate pane IDs. `PaneProcessInfo` exposes shell PID and foreground process records with PID, name, optional argv0/argv/cmdline/cwd (`/tmp/herdr-cockpit-master/src/api/schema/panes.rs:570-599`); the handler collects these from the live foreground job (`/tmp/herdr-cockpit-master/src/app/api/panes.rs:503-548`). Match against the enabled manifest's canonical command, executable path under the known plugin root, and expected entrypoint. Require one candidate and a live pane/terminal identity; otherwise show an explicit association choice or TUI fallback.
4. Treat the process match as detection evidence, not authentication. Do not display or persist raw cmdline because it may contain secrets. A PID alone is insufficient; require current pane identity and, when available, process generation/argv/cwd consistency. Herdr's ordinary `session.snapshot` and `PaneInfo` do not expose plugin ID/entrypoint (`/tmp/herdr-cockpit-master/src/api/schema/session.rs:8-22`, `src/api/schema/panes.rs:527-560`), so preexisting unknown panes cannot be safely identified from titles.

For an already open pane with no launch receipt, offer “render as Cockpit context” as an explicit user action. A configurable title/cwd/process heuristic can prefill candidates, but it must be labelled best effort and never authorize attachment, deletion, or replacement automatically. Reviewr's label-based toggle precedent (`/home/nnex/.config/herdr/plugins/github/persiyanov.reviewr-e87b654a74f8/herdr-plugin.toml:1-29`, its changelog) is not a sufficient identity contract.

## Rendering, input, and lifecycle

The replaced pane remains an ordinary Herdr terminal pane, with the same authoritative rectangle from the session snapshot/layout update. Cockpit omits xterm mounting for a proven replacement pane while neighboring terminal panes continue through the existing `TerminalPane` stream. Focus requests still go through Herdr and GUI input is routed to Cockpit DOM state after confirmed pane focus; context navigation is not encoded as terminal bytes. A TUI placeholder/helper remains available when detection is uncertain or the GUI is unavailable. There is no automatic agent launch, silent root-pane closure, or implicit pane deletion.

Resize, move, split, zoom, tab changes, and external focus changes consume Herdr's authoritative layout/focus state. GUI desired dimensions may request a Herdr resize, but DOM dimensions never become a competing rectangle. Closing the GUI requests the existing Herdr pane close operation and waits for the pane-closed event. A helper crash leaves the pane visible and marked fallback/disconnected until explicit retry or close.

## Limits and probes

- Herdr's internal `plugin_panes` map records plugin ID/entrypoint (`/tmp/herdr-cockpit-master/src/app/state.rs:11-15,873-878`) but is not part of `session.snapshot`; Cockpit cannot query it through a dedicated plugin-pane list. The open response plus process inspection is therefore the supported no-IPC reconciliation sequence.
- `plugin.pane.open` launches a terminal process and supports ordinary layout placement; Herdr v1 has no native nonterminal pane (`docs/versions/0.8.2/website/src/content/docs/cli-reference.mdx:460-471`).
- `--env` and open-time env affect the new process only. They cannot retrofit environment into Herdr-created terminals or guarantee inheritance for terminals created later by another actor.
- Probe in a disposable session: capture `plugin.pane.open`, snapshot, `pane.get`, process info, layout updates, process exit, reconnect, and close. Verify command matching for shell wrappers, PID reuse, duplicate candidates, missing argv, stale panes, and manifest warnings. Confirm the GUI follows layout while an adjacent xterm remains interactive.

This supports a genuine Herdr pane replacement without extension communication. The remaining product choice is whether to require the installed plugin entrypoint for automatic detection or permit explicit user association for ordinary/preexisting panes.
