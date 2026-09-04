# Herdr 0.8.2 bootstrap evidence

Target inspected: installed `herdr 0.8.2` on the local Linux workstation. No workspace, pane, terminal, session, or server mutation was issued. `herdr status server` and `herdr api snapshot` are read-only observations.

## Sources and observed commands

Installed CLI observations:

```text
$ herdr --version
herdr 0.8.2

$ herdr status server
status: running
version: 0.8.2
protocol: 20
compatible: yes
socket: /home/nnex/.config/herdr/herdr.sock
```

The path above is an observation of this workstation, not a path Cockpit should hard-code. The CLI help identifies `HERDR_CONFIG_PATH` as the config-file override and names `herdr api schema` as the schema inspection command. `herdr --skill` was also inspected; it confirms that the CLI is intended for operations from a Herdr-managed pane and that raw socket use is a separate integration layer.

```text
$ herdr api schema --json
JSON Schema draft 2020-12 document
protocol: 20
schema_version: 1
```

The full document has top-level schemas `request`, `success_response`, `error_response`, `event`, and `subscription_event`. The installed schema is the compatibility authority: startup should obtain and validate this document (or use the equivalent bundled schema obtained via the installed CLI), then gate required operations on `protocol`, `schema_version`, and method/type presence. The schema contains request method/type definitions and response structures; it does not define the separate terminal-session control stream described below.

Primary sources:

- Socket API documentation (latest documentation version shown as 0.8.2): https://herdr.dev/docs/socket-api/
- CLI reference (latest documentation version shown as 0.8.2): https://herdr.dev/docs/cli-reference/
- Agent skill: https://raw.githubusercontent.com/herdrdev/herdr/master/skills/herdr/SKILL.md
- Upstream request enum and serde wire names: https://raw.githubusercontent.com/herdrdev/herdr/master/src/api/schema.rs
- Upstream API server line framing and error serialization: https://raw.githubusercontent.com/herdrdev/herdr/master/src/api/server.rs
- Upstream terminal session stream implementation: https://raw.githubusercontent.com/herdrdev/herdr/master/src/client/terminal_sessions.rs
- Upstream repository: https://github.com/herdrdev/herdr

## Socket location and session selection

The official socket documentation says Unix uses a Unix-domain socket (Windows uses a named pipe). The default is under the Herdr config directory; named sessions use separate sockets:

```text
~/.config/herdr/herdr.sock
~/.config/herdr/sessions/<name>/herdr.sock
```

Documented resolution order is explicit CLI `--session <name>`, then `HERDR_SOCKET_PATH`, then `HERDR_SESSION=<name>`, then the default-session socket. `HERDR_SOCKET_PATH` is intended for low-level overrides. For Cockpit, make the endpoint configurable, select one named session at a time, and never derive a path from a workspace ID. Session selection is a Cockpit concern; Herdr remains authoritative for each selected session's state.

## Request framing, correlation, and errors

The socket transport is newline-delimited JSON over the local socket. Each request is one JSON object followed by `\n`, with a caller-chosen string ID:

```json
{"id":"req_1","method":"ping","params":{}}
```

A normal response carries the same ID:

```json
{"id":"req_1","result":{"type":"pong"}}
```

The upstream server reads one line at a time and serializes one JSON response line. A successful response has `{ "id": string, "result": ... }`; an error has `{ "id": string, "error": { "code": string, "message": string } }`. Error codes are server-defined strings, not a closed enum (the docs show `not_found`; upstream tests show `timeout`, `pane_not_found`, and `server_unavailable`). Preserve both code and message, map unknown codes to a generic structured Herdr error, and do not turn protocol errors into empty state.

The socket supports concurrent request/response traffic by ID, while subscriptions keep the connection open after the initial acknowledgement. The adapter must demultiplex responses from pushed event envelopes, preserve event arrival order, detect disconnects, and resnapshot/resubscribe after reconnect. The official protocol-stability guidance says to check `ping` or `herdr status`, handle unknown fields gracefully, and account for protocol changes.

## Discovery and authoritative snapshot

Use `ping` for live server identity/capability discovery. Its schema result is `type: "pong"` and includes required `version` and numeric `protocol`, plus optional `capabilities` (`live_handoff` is required when capabilities is present; `detached_server_daemon` defaults false). On this installation, `herdr status server` reported version `0.8.2`, protocol `20`, and `compatible: yes`.

Bootstrap the selected session with:

```json
{"id":"bootstrap","method":"session.snapshot","params":{}}
```

The result is `type: "session_snapshot"` containing `SessionSnapshot`: `version`, `protocol`, `workspaces`, `tabs`, `panes`, `layouts`, and `agents`; focus IDs (`focused_workspace_id`, `focused_tab_id`, `focused_pane_id`) are nullable. `PaneInfo` includes stable public `pane_id`, `terminal_id`, workspace/tab IDs, focus, agent status, revision, optional cwd/title/agent fields, scroll metrics, and token/state-label maps. Workspace records include optional worktree provenance. `session.snapshot` is one-time bootstrap, not a subscription; the official docs explicitly require subscribing after it and repeating it after reconnect or suspected staleness. The read-only `herdr api snapshot` command printed this live response successfully during inspection.

Relevant exact socket request method names from the installed schema include `ping`, `session.snapshot`, `workspace.list/get/focus/rename/close`, `worktree.list/create/open/remove`, `tab.list/get/focus/rename/close`, `pane.list/current/get/focus/layout/resize/send_text/send_keys/send_input/read`, `layout.export/apply/set_split_ratio`, `agent.list/get/read/explain/focus/prompt/wait`, and `events.subscribe/wait`. Additional methods exist; do not assume this list is exhaustive. The upstream `Method` enum is the primary source for exact wire names.

## Events and ordered cache updates

Subscribe on the same long-lived socket:

```json
{"id":"sub_1","method":"events.subscribe","params":{"subscriptions":[{"type":"workspace.created"},{"type":"pane.updated"},{"type":"layout.updated"}]}}
```

The acknowledgement result is `type: "subscription_started"`; later pushed lines are event envelopes with `event` and `data` and no request ID. Installed schema subscription types include workspace lifecycle (`workspace.created`, `updated`, `metadata_updated`, `renamed`, `moved`, `reordered`, `closed`, `focused`), worktree lifecycle (`worktree.created`, `opened`, `removed`), tab lifecycle (`created`, `closed`, `focused`, `renamed`, `moved`), pane lifecycle (`created`, `closed`, `updated`, `focused`, `moved`, `exited`, `agent_detected`), `layout.updated`, and filtered pane subscriptions `pane.output_matched`, `pane.agent_status_changed`, and `pane.scroll_changed`.

The broad `event` schema's `EventKind` includes the lifecycle names above plus `pane_output_changed` and `pane_agent_status_changed` (underscore form in event data). The `subscription_event` schema uses dotted event names for the filtered subscription stream. Preserve unknown event kinds for diagnostics and ignore unknown fields. Apply events in arrival order to the snapshot cache; on reconnect, sequence uncertainty, malformed event, or stale-state detection, discard the affected derived state and obtain a fresh snapshot plus subscription.

## Terminal attach, output, input, resize, and takeover

Terminal attachment is **not** a normal `pane.*` request in the JSON API schema. The documented CLI exposes a separate terminal session stream:

```text
herdr terminal session observe <target> [--cols N] [--rows N]
herdr terminal session control <target> [--takeover] [--cols N] [--rows N]
herdr terminal attach <terminal_id> [--takeover]
herdr agent attach <target> [--takeover]
```

`target` may be a pane, terminal, or agent target for terminal session commands. `terminal.session observe` is read-only and permits multiple observers. `terminal session control` is writable and allows only one controller; `--takeover` replaces the current controller. Direct `terminal attach` and `agent attach` likewise document `--takeover`. A second writable attach without takeover fails; this is an expected conflict, not a reason to close the Herdr process. The current Cockpit architecture deliberately chooses automatic takeover when selecting a pane, but this remains disruptive and must be surfaced in UI/error state.

The upstream terminal session implementation confirms the stream details. After a binary local-socket handshake, server terminal frames are emitted to stdout as newline-delimited JSON records:

```json
{"type":"terminal.frame","seq":7,"encoding":"ansi","width":120,"height":40,"full":true,"bytes":"<base64>"}
{"type":"terminal.closed","reason":"..."}
```

`bytes` is standard-base64 encoded ANSI bytes. The initial rendered state is sent first, followed by live ANSI frames; `seq`, dimensions, and `full` must be retained by the renderer. `terminal.closed` carries a reason. The stream source explicitly discards non-terminal messages and treats EOF as stream completion.

Control stdin accepts one JSON command per line. Exact command tags are `terminal.input`, `terminal.resize`, `terminal.scroll`, and `terminal.release`:

```json
{"type":"terminal.input","text":"ls\n"}
{"type":"terminal.input","bytes":"<base64>"}
{"type":"terminal.resize","cols":120,"rows":40,"cell_width_px":8,"cell_height_px":16}
{"type":"terminal.release"}
```

`terminal.input` accepts either `text` or base64 `bytes`, not both; omitted values produce empty input. `terminal.resize` requires nonzero `cols` and `rows`; pixel cell dimensions default to zero. `terminal.scroll` additionally carries `direction` (`up`/`down`), `lines`, and optional `source`, column, row, and modifiers. `terminal.release` relinquishes control. The upstream source sets `pixel_mouse: false` for this CLI control path. **Uncertainty:** the public docs describe the stream and command names, while the binary handshake itself is an internal protocol rather than a documented Cockpit-facing API; prefer invoking the installed CLI as a subprocess for terminal streams initially, or capture/implement the handshake only after a fixture-backed decision.

For non-stream one-shot terminal operations, the JSON API provides `pane.read` with required `pane_id` and `source` (`visible`, `recent`, `recent_unwrapped`, or `detection`), optional `lines`, `format` (`text`/`ansi`), and `strip_ansi`; result `type` is `pane_read` and includes text, revision, and `truncated`. `pane.send_text`, `pane.send_keys`, and `pane.send_input` are request methods. Key strings support printable keys, `enter`, `esc`, modifier chords such as `ctrl+h`, and function keys; they are logical keys, not Herdr prefix-binding strings.

## Minimal adapter interface recommendation

Keep transport and Herdr semantics behind a deep Rust adapter boundary:

```text
HerdrConnector::discover(endpoint, session) -> HerdrIdentity
HerdrConnector::snapshot() -> SessionSnapshot
HerdrConnector::subscribe(filters) -> OrderedEventStream
HerdrConnector::resync() -> SnapshotAndSubscription
HerdrConnector::workspace/tab/pane/agent operations(...) -> typed results
HerdrConnector::terminal_observe(target, size) -> TerminalFrameStream
HerdrConnector::terminal_control(target, size, takeover) -> TerminalControl
TerminalControl::send_text | send_bytes | resize | scroll | release
```

Expose typed `UnsupportedCapability` separately from transport failure, protocol mismatch, request error, takeover conflict, and disconnected/stale stream. The adapter owns request IDs, newline framing, response demultiplexing, schema/protocol gating, reconnect backoff, snapshot replacement, event ordering, and terminal stream lifecycle. Application code should consume authoritative typed snapshots/events rather than raw JSON.

## Fixture strategy and scope boundary

Capture immutable fixtures without mutating live sessions:

1. Save the installed `herdr api schema --json` output as the protocol-20/schema-1 schema fixture (redact no schema fields; it contains type definitions, not session secrets).
2. Save `herdr status server` identity output and a representative `herdr api snapshot` response only after removing local paths, agent session paths, and other personal identifiers.
3. Hand-author protocol fixtures for `ping`, `session.snapshot`, `subscription_started`, representative workspace/tab/pane/layout/agent events, unknown fields/events, malformed JSON, EOF, and each error envelope shape.
4. Capture terminal stream fixtures with synthetic `terminal.frame` records (base64 ANSI, full and incremental frames, sequence changes), `terminal.closed`, and stdin control lines for text/bytes input, resize, scroll, and release. Do not record or replay actual user terminal bytes or live agent output.
5. Test takeover as a deterministic fake-server conflict/success transition; do not request takeover against a live terminal during bootstrap research.

Represent now: schema/version/protocol discovery; configurable Unix socket and named-session selection; newline JSON request/response; authoritative snapshot; lifecycle/event subscriptions; workspace/tab/pane/agent reads and supported mutations; pane read/send operations; terminal observe/control subprocess integration; ANSI frame decoding; input, resize, scroll, release, and explicit takeover conflict handling. Defer: graphics streams, plugin/integration administration, undocumented binary-handshake reimplementation, multi-session simultaneous UI state, remote sockets, and any capability not present in the installed schema. Keep raw socket forwarding and arbitrary shell execution out of the browser gateway.

## Bootstrap decisions

- Pin the initial compatibility target to installed Herdr `0.8.2`, API `protocol 20`, schema `1`; reject or mark incompatible when required schema methods/types are absent rather than probing undocumented methods.
- Discover identity with `ping`/`herdr status`, then bootstrap using `session.snapshot`; subscribe with `events.subscribe`; resnapshot and resubscribe after reconnect or stale state.
- Make socket endpoint and session configurable. Default Linux layout is `~/.config/herdr/herdr.sock`; named sessions use `~/.config/herdr/sessions/<name>/herdr.sock`; do not hard-code the observed home path.
- Use newline-delimited JSON with caller-generated string IDs and preserve structured `{code,message}` errors. Handle unknown fields and unknown event kinds gracefully.
- Use `terminal session observe/control` for terminal attachment initially, because the documented stream offers exact frame/input/resize/release semantics while the underlying binary handshake is not a stable public JSON method. Invoke the installed CLI or isolate handshake code behind a capability gate.
- Honor documented one-controller ownership and takeover. Cockpit's current automatic takeover policy is accepted for the proof of concept but must report takeover/disconnect conflicts inline and never terminate the Herdr pane/process.
- Represent snapshot/event state, pane reads, ANSI frames, text/bytes input, resize, scroll, release, and explicit unsupported capabilities now. Defer graphics, plugin/integration management, remote transport, simultaneous sessions, and undocumented handshake reimplementation.
- Commands observed: `herdr --version` -> `0.8.2`; `herdr status server` -> running, protocol 20, compatible; `herdr api schema --json`; `herdr api snapshot`; `herdr --skill`; `herdr terminal session observe/control` syntax. No mutating command was run.
- Unresolved blockers: the public docs and upstream source expose terminal stream records and control commands but not a supported high-level Rust library or stable public binary-handshake contract; a future implementation must choose CLI subprocess mediation versus maintaining a tested handshake implementation. Exact server-side takeover error code/message should be fixture-backed from a controlled test server, not guessed from live sessions.
