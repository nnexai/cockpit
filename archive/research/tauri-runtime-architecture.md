# Tauri v2 runtime architecture for Cockpit and Herdr

## Executive conclusion

Tauri v2 is a native application toolkit, not a browser runtime target. It compiles a Rust application for desktop or mobile, renders a static web frontend in an operating-system WebView, and provides a WebView-to-Rust message bridge. The same static frontend can also be built for ordinary web hosting, but a browser has neither the Tauri host process nor its IPC bridge; browser deployment therefore needs a normal network API rather than direct Tauri commands. Tauri’s CLI confirms this split: `tauri build` creates native bundles/installers for a Rust target triple, while Android and iOS have distinct native build commands; there is no browser build target ([CLI reference](https://v2.tauri.app/reference/cli/#build)).

For Cockpit, keep **Herdr (or a companion Cockpit daemon) as the single, long-lived authority** and put one transport-neutral application core behind it. The Tauri app and browser UI should share frontend code and a typed client contract, but use different transport adapters: Tauri commands/channels to a Rust adapter that talks to Herdr’s Unix socket, and HTTP/WebSocket to a tightly bound loopback gateway backed by the same core. This preserves behavior across clients and allows multiple simultaneous sessions without making the desktop window the server or owner of durable state.

## 1. Runtime and build targets

Tauri’s native architecture combines compiled Rust with HTML rendered in a WebView; the WebView controls system functionality through message passing to the Rust backend. Tauri uses the operating system’s WebView rather than shipping a browser runtime ([architecture](https://v2.tauri.app/concept/architecture/)). Its documented native platform set is Linux, macOS, Windows, Android, and iOS; capabilities may be restricted to those target names ([capability target platforms](https://v2.tauri.app/security/capabilities/#target-platform)).

The frontend is a static asset input to the native application. Tauri describes itself conceptually as a static web host and expects HTML, CSS, JavaScript, and optionally WASM. SPA, SSG, and MPA outputs are supported, but server-side rendering is not natively supported ([frontend checklist](https://v2.tauri.app/start/frontend/#configuration-checklist)). During development, `build.devUrl` commonly points the WebView at a frontend development server. During release, `tauri build` runs the configured frontend build and consumes `build.frontendDist` to create the native binary and bundles ([configuration](https://v2.tauri.app/reference/config/#buildconfig), [CLI build](https://v2.tauri.app/reference/cli/#build)).

### Browser deployment

A browser deployment is a **separate deployment of the frontend’s static output**, not a Tauri target. The UI framework can remain the same because Tauri is frontend-agnostic, but any path that imports or calls native functionality must sit behind a client abstraction. Tauri exposes `isTauri(): boolean` for detecting whether the host bridge exists ([core API](https://v2.tauri.app/reference/javascript/api/namespacecore/#istauri)); use that once at composition/bootstrap time to select an adapter, rather than scattering environment checks through components.

Tauri can also load a remote URL in its own WebView, and v2 capabilities can explicitly grant selected commands to remote origins. By default, however, Tauri APIs are accessible only to bundled application code, and remote access must be opted into ([remote API access](https://v2.tauri.app/security/capabilities/#remote-api-access)). That feature does **not** make Tauri IPC available to users visiting the same URL in Chrome or Firefox: the native host bridge exists only inside a Tauri WebView.

## 2. Frontend-to-Rust IPC

The primary request/response primitive is a Tauri **command**:

1. Rust annotates a function with `#[tauri::command]` and registers it with `tauri::Builder::invoke_handler`.
2. Frontend code calls `invoke(name, arguments)` from `@tauri-apps/api/core`.
3. Arguments deserialize through Serde; return values serialize through Serde; `Result` errors reject the JavaScript promise; commands may be asynchronous ([calling Rust](https://v2.tauri.app/develop/calling-rust/#commands)).

For server-initiated updates, Tauri offers events and channels. Events are bidirectional, JSON-only, dynamically typed, and intended for small payloads or multi-producer/multi-consumer notification patterns; they are not intended for low-latency or high-throughput transfer. Events can be global or directed to a particular WebView ([calling the frontend](https://v2.tauri.app/develop/calling-frontend/#event-system)). Channels are the recommended ordered/high-throughput streaming mechanism ([channels](https://v2.tauri.app/develop/calling-rust/#channels)).

Capabilities constrain which core/plugin permissions are exposed to which windows and WebViews. Custom commands registered directly with `invoke_handler` are allowed to all application windows/WebViews by default unless the application declares its command manifest, so Cockpit should explicitly register and grant only its intended command surface ([capabilities](https://v2.tauri.app/security/capabilities/)).

**Design implication:** commands and channels should be thin transport adapters. They should validate/deserialize a versioned Cockpit protocol and delegate to the shared application service; business behavior should not live in Tauri handlers.

## 3. Servers, subprocesses, and sidecars

Tauri itself serves static frontend assets and does not natively supply an SSR or application API server ([frontend checklist](https://v2.tauri.app/start/frontend/#configuration-checklist)). It can nevertheless participate in a server architecture in three ways:

- Rust code in the Tauri process can start an HTTP/WebSocket server as ordinary Rust application code.
- Tauri can bundle and spawn an external executable (a **sidecar**). The official guide explicitly lists bundled API servers as a sidecar use case and requires architecture-specific binaries configured with `bundle.externalBin` ([embedding external binaries](https://v2.tauri.app/develop/sidecar/)).
- The official localhost plugin can expose the app’s frontend assets through localhost instead of Tauri’s default custom protocol, but its own v2 README warns that this carries considerable security risk ([v2 localhost plugin](https://github.com/tauri-apps/plugins-workspace/tree/v2/plugins/localhost)). It is an asset-serving choice, not a substitute for a designed Cockpit/Herdr API.

The shell plugin can execute or spawn child processes and exchange stdout/stdin, but dangerous commands and scopes are blocked by default and must be explicitly enabled in capabilities ([shell plugin](https://v2.tauri.app/plugin/shell/)). Prefer spawning any required helper from Rust and exposing task-specific commands; do not give frontend JavaScript a general shell surface.

For Cockpit, **do not make the Tauri process the normal Herdr server lifecycle owner**. Closing or restarting the UI must not terminate persistent terminal sessions, and two Cockpit windows must not start competing authorities. Connect to the already-running unprivileged Herdr daemon. A bundled sidecar is appropriate only if distribution requires shipping a missing helper; even then, implement single-instance discovery and attach to an existing daemon before spawning.

## 4. Unix sockets and browser-safe access

Native Rust can connect directly to Herdr’s Unix-domain socket. Rust’s standard library exposes `std::os::unix::net::UnixStream::connect(path)` for exactly this purpose ([Rust `UnixStream`](https://doc.rust-lang.org/std/os/unix/net/struct.UnixStream.html#method.connect)). A Tauri command can call a Cockpit Rust client that frames Herdr JSON-RPC over that stream, while a channel forwards subscription updates to the WebView.

An ordinary browser cannot use that socket directly. The WebSocket Web API intentionally provides no raw network access, permits only `ws`, `wss`, `http`, and `https` URL schemes, and communicates with server processes through the WebSocket protocol ([WHATWG WebSockets](https://websockets.spec.whatwg.org/#network-intro), [WebSocket constructor](https://websockets.spec.whatwg.org/#the-websocket-interface)). Therefore a browser needs a gateway:

```text
Tauri UI ── invoke/channel ── Tauri Rust adapter ── Unix JSON-RPC ──┐
                                                                    │
Browser UI ── HTTP/WebSocket ── loopback gateway ───────────────────┤
                                                                    ▼
                                                     Cockpit core / Herdr daemon
                                                     sessions, workspaces, events
```

The loopback gateway should bind only to the intended local interface, authenticate every session, validate browser origins, expose task-level operations rather than raw socket forwarding, and use explicit capability/scopes on the Tauri side. These are Cockpit design requirements, not behavior supplied automatically by Tauri. If remote browser access is later required, put TLS and real authentication in front of the same gateway rather than broadening the native IPC capability.

## 5. Recommended shared-core shape

### Backend boundaries

1. **Protocol crate/package:** versioned request, response, error, and event DTOs for operations such as workspace listing, session creation, pane input, resize, status subscription, and teardown. Preserve stable operation names and error codes across transports.
2. **Application core:** authorization, workspace/session invariants, idempotency, lifecycle rules, and orchestration. It must not depend on Tauri, HTTP, WebSocket, or Unix framing.
3. **Herdr adapter:** the only layer aware of `$HERDR_SOCKET_PATH`, JSON-RPC framing, reconnect behavior, and Herdr-specific identifiers.
4. **Native adapter:** narrow `#[tauri::command]` handlers for request/response plus a Tauri channel for each subscription. Keep connection/subscription handles in managed Rust state where necessary; Tauri documents application state management through `Manager` and command injection ([state management](https://v2.tauri.app/develop/state-management/)).
5. **Web gateway:** HTTP for bounded commands/queries and WebSocket for ordered session/status output. It delegates to the same application core and emits the same protocol events as the native adapter.

If Herdr already owns all business invariants, the “application core” may be a thin Cockpit façade around Herdr rather than a second daemon. The important constraint is one authoritative state machine, not one executable.

### Frontend boundary

Define one injected interface, for example `CockpitClient`, with methods and streams expressed only in protocol types. Provide:

- `TauriCockpitClient`: `invoke` for commands and Tauri channels for streams.
- `WebCockpitClient`: `fetch` for commands and WebSocket for streams.

Select the implementation once at startup using build configuration or `isTauri()`. UI stores and components consume only `CockpitClient`. Contract tests should run the same behavior cases against both adapters: success/error mapping, cancellation, reconnect/resubscribe, ordering, and authorization failure.

### Multi-session behavior

Herdr/daemon state must outlive every frontend connection. Give each connected UI a client/session ID and each terminal stream a stable stream ID plus monotonic sequence number. On reconnect, request a fresh snapshot and then resume from a cursor where Herdr supports it; otherwise explicitly replace local state from the snapshot. Fan out state/status events to all authorized subscribers, but route interactive terminal input and acknowledgements by workspace/pane and client identity. This avoids treating Tauri global events as the source of truth and gives native and browser clients identical observable behavior.

## Decision

Adopt **one static frontend, two frontend transport adapters, one daemon-backed application core**. Ship the static assets inside Tauri for native desktop and through an ordinary web host (preferably the local gateway itself for same-origin browser use) for browsers. Keep Herdr persistent and authoritative; let Tauri bridge to its Unix socket, and let browsers reach the same behavior through a narrow authenticated HTTP/WebSocket gateway. Do not expose raw shell commands, raw Unix-socket forwarding, or remote Tauri capabilities to achieve parity.