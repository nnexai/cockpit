# VS Code, xterm.js, and Kitty protocol support

## Bottom line

VS Code's integrated terminal supports parts of two protocols originated by Kitty: the **Kitty keyboard protocol** for richer input reporting and the **Kitty graphics protocol** for inline image output. This is protocol compatibility, not use of the Kitty terminal emulator: VS Code constructs an `@xterm/xterm` terminal, loads `@xterm/addon-webgl` as its accelerated renderer, and conditionally loads `@xterm/addon-image` for images ([VS Code xterm wrapper](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/terminal/browser/xterm/xtermTerminal.ts#L6-L13), [renderer and image-addon wiring](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/terminal/browser/xterm/xtermTerminal.ts#L925-L1012), [addon importer](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/terminal/browser/xterm/xtermAddonImporter.ts#L43-L53)). Kitty-aware programs therefore communicate with xterm.js through escape sequences while the integrated terminal remains VS Code's xterm.js-based UI.

## Older versus current support

- **VS Code 1.109 (January 2026): keyboard.** The release introduced Kitty keyboard protocol support, initially described as rolling out to Stable; the 1.109.5 update says it became available to all users. The release notes identify richer modifier combinations, press/repeat/release events, and disambiguation of keys such as Escape, and state that the terminal program must support and request the protocol ([1.109 release notes](https://code.visualstudio.com/updates/v1_109#_new-vt-features), [1.109.5 update note](https://code.visualstudio.com/updates/v1_109#_january-2026-version-1109)).
- **VS Code 1.110 (February 2026): graphics.** The next release added Kitty graphics rendering, including direct base64 transmission, chunking, zlib compression, PNG/RGB/RGBA data, cropping/scaling/offsets, z-order, stored-image placement, deletion, and cursor control ([1.110 release notes](https://code.visualstudio.com/updates/v1_110#_kitty-graphics-protocol)).
- **Older xterm.js releases lacked both implementations.** Core keyboard support landed in January 2026, followed by the image addon's graphics MVP in February 2026 ([keyboard PR #5600](https://github.com/xtermjs/xterm.js/pull/5600), [graphics PR #5619](https://github.com/xtermjs/xterm.js/pull/5619)). Current xterm.js keeps Kitty keyboard handling in core behind `ITerminalOptions.vtExtensions.kittyKeyboard`; Kitty graphics lives in `@xterm/addon-image`, whose documentation still calls it work in progress and alpha quality ([xterm.js public API](https://github.com/xtermjs/xterm.js/blob/master/typings/xterm.d.ts#L462-L482), [image-addon README](https://github.com/xtermjs/xterm.js/blob/master/addons/addon-image/README.md#kitty-graphics-support-tgp), [status](https://github.com/xtermjs/xterm.js/blob/master/addons/addon-image/README.md#status)).

## Keyboard: negotiation and input flow

1. VS Code maps `terminal.integrated.enableKittyKeyboardProtocol` into xterm.js as `vtExtensions.kittyKeyboard`, both when constructing the terminal and when configuration changes ([VS Code wiring](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/terminal/browser/xterm/xtermTerminal.ts#L267-L279), [configuration refresh](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/terminal/browser/xterm/xtermTerminal.ts#L600-L616)).
2. A terminal application writes Kitty control sequences into its PTY output. VS Code passes process output to `xterm.raw.write`, and xterm.js parses the Kitty set/query/push/pop forms `CSI = u`, `CSI ? u`, `CSI > u`, and `CSI < u`, maintaining separate main/alternate-buffer flag state and answering queries through its data channel ([VS Code output bridge](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/terminal/browser/terminalInstance.ts#L1669-L1708), [xterm.js handlers](https://github.com/xtermjs/xterm.js/blob/master/src/common/InputHandler.ts#L3537-L3635)).
3. Once an application has requested nonzero enhancement flags, xterm.js's keyboard service selects the Kitty encoder instead of legacy encoding. It distinguishes press, repeat, and—when requested—release events; the encoder supports the protocol's five reporting flags and emits legacy, modified CSI, or CSI-u forms as appropriate ([keyboard service](https://github.com/xtermjs/xterm.js/blob/master/src/browser/services/KeyboardService.ts#L31-L67), [encoder flags and event types](https://github.com/xtermjs/xterm.js/blob/master/src/common/input/KittyKeyboard.ts#L14-L43), [encoding decision](https://github.com/xtermjs/xterm.js/blob/master/src/common/input/KittyKeyboard.ts#L386-L532)).
4. xterm.js emits the resulting bytes through `onData`; VS Code forwards that string to the terminal process manager/PTY ([xterm.js `onData` contract](https://github.com/xtermjs/xterm.js/blob/master/typings/xterm.d.ts#L1054-L1074), [VS Code input bridge](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/terminal/browser/terminalInstance.ts#L882-L886)).

`terminal.integrated.enableKittyKeyboardProtocol` currently defaults to `true` and is an advanced, restricted setting. Enabling it only permits negotiation: the running application still has to request enhanced reporting, so ordinary applications continue on legacy keyboard encoding ([setting definition](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/terminal/common/terminalConfiguration.ts#L591-L598), [xterm.js protocol-selection condition](https://github.com/xtermjs/xterm.js/blob/master/src/browser/services/KeyboardService.ts#L58-L66)).

## Graphics: output and rendering flow

1. A program writes a Kitty APC graphics sequence (`APC G ... ST`) to PTY output; VS Code feeds that output to `xterm.raw.write` ([VS Code output bridge](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/terminal/browser/terminalInstance.ts#L1669-L1708)).
2. When images are enabled and WebGL is active, VS Code dynamically imports and loads `@xterm/addon-image`; without both conditions it does not keep the addon loaded ([VS Code image-addon gate](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/terminal/browser/xterm/xtermTerminal.ts#L1004-L1028)).
3. The addon enables Kitty handling by default and registers `KittyGraphicsHandler` for APC final byte `G`. That handler parses control data, streams base64 payload into a decoder, stores image data, and routes transmit, transmit-and-display, query, placement, and deletion actions into the addon's browser/canvas-backed image storage and renderer ([addon defaults and registration](https://github.com/xtermjs/xterm.js/blob/master/addons/addon-image/src/ImageAddon.ts#L59-L72), [APC handler registration](https://github.com/xtermjs/xterm.js/blob/master/addons/addon-image/src/ImageAddon.ts#L195-L204), [handler implementation](https://github.com/xtermjs/xterm.js/blob/master/addons/addon-image/src/kitty/KittyGraphicsHandler.ts#L32-L80)).

`terminal.integrated.enableImages` currently defaults to `false`. VS Code documents that it requires GPU acceleration; Kitty graphics works on all platforms, but Windows additionally requires ConPTY v2, images are not restored after window reload/reconnect, and enabling the setting also enables terminal transparency ([setting definition](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/terminal/common/terminalConfiguration.ts#L656-L662)). The 1.110 instructions likewise require `terminal.integrated.gpuAcceleration` to be `on` or `auto`, plus `terminal.integrated.windowsUseConptyDll` on Windows ([1.110 release notes](https://code.visualstudio.com/updates/v1_110#_kitty-graphics-protocol)).

## Known limitations

- The current addon labels Kitty graphics **alpha** and **work in progress**; this is not a claim of full Kitty terminal parity ([image-addon README](https://github.com/xtermjs/xterm.js/blob/master/addons/addon-image/README.md#kitty-graphics-support-tgp), [status](https://github.com/xtermjs/xterm.js/blob/master/addons/addon-image/README.md#status)).
- Animation frame/control/composition actions remain unimplemented, and only direct inline transmission is accepted; file, temporary-file, and shared-memory media are rejected ([graphics handler](https://github.com/xtermjs/xterm.js/blob/master/addons/addon-image/src/kitty/KittyGraphicsHandler.ts#L308-L371), [transmission limitation](https://github.com/xtermjs/xterm.js/blob/master/addons/addon-image/src/kitty/KittyGraphicsHandler.ts#L392-L408)).
- VS Code's 1.110 notes also call out missing animations, relative placements, Unicode placeholders, and file-based transmission ([1.110 release notes](https://code.visualstudio.com/updates/v1_110#_kitty-graphics-protocol)).
- Image behavior remains constrained by xterm.js's text grid: resize reflow can split images, writing characters over image cells erases those cells' image information, and the addon cannot provide arbitrary foreground/background composition or transparency composition ([image-addon terminal interaction notes](https://github.com/xtermjs/xterm.js/blob/master/addons/addon-image/README.md#terminal-interaction)).

## Local Herdr 0.9.0 attachment POC

### Scope and conclusion

This POC tested the exact path under consideration:

```text
Kitty-producing process → Herdr 0.9.0 terminal attachment → xterm.js browser terminal
```

The test used Herdr `0.9.0`, protocol `22`, and disposable named sessions. No
Tauri native wrapper was involved. No Cockpit source was changed.

**Conclusion:** xterm.js can render the Kitty graphics sequence, but Herdr's
direct terminal attachment path does not transmit the PTY's Kitty APC graphics
as either `ESC_G...ESC\` bytes inside `TerminalFrame` messages or separate
`Graphics` messages. The image therefore cannot reach xterm.js on this path.

Herdr's separate pane/client-shell graphics subsystem is a different path and
must not be conflated with direct terminal attachment.

### Test fixture

The fixture was `scripts/kitty_image_smoke.py`. It generates a dependency-free
PNG and writes:

```text
ESC _ G a=T,f=100,t=d,q=2,i=1,c=32,r=8 ; <base64 PNG> ESC \
```

The fixture output was 930 bytes. It includes a pink/teal checkerboard made of
colored squares and surrounding diagnostic text.

### Test A — browser attachment through the current Cockpit gateway

An isolated Herdr session was created with workspace `w1`, pane `w1:p1`, and a
temporary browser page using:

- `@xterm/xterm` `6.1.0-beta.304`;
- `@xterm/addon-image` `0.10.0-beta.301`;
- `@xterm/addon-fit`.

The browser opened a control terminal WebSocket attachment and the fixture was
run through `herdr pane run`.

Observed:

- text from the fixture reached xterm.js;
- terminal frames were received in order;
- delivered frame bytes contained no `ESC_G` Kitty APC sequence;
- no xterm image canvas was created;
- no checkerboard appeared.

The official Herdr CLI direct-attach capture also contained the fixture text
but no Kitty APC bytes.

This test established the symptom but was not treated as the decisive
transport test because the Cockpit gateway intentionally parks graphics
messages.

### Test B — direct Herdr client socket, bypassing Cockpit

A second isolated session used a temporary bridge connected directly to
Herdr's `herdr-client.sock`. The bridge implemented the protocol-22 framing
needed for:

1. `TerminalHello` with `80x24` cells and `8x16` cell metrics;
2. `ControlTerminal` for the disposable terminal;
3. decoding `TerminalFrame` and `Graphics` server messages;
4. forwarding any received bytes directly into xterm.js.

The fixture was run while the browser was attached to this direct bridge.

Observed on the direct Herdr wire:

- ordinary terminal frames arrived;
- no `Graphics` server messages arrived;
- no terminal frame contained `ESC_G`;
- xterm.js displayed the fixture text only;
- the image addon created no image canvas;
- no checkerboard appeared.

This is the decisive result for the Herdr → attached terminal → xterm.js
question: **the current Herdr direct terminal attachment does not forward the
PTY Kitty graphics command.**

### Test C — xterm.js positive control

The exact 930-byte fixture output was written directly into the same xterm.js
version and image addon without Herdr.

Observed:

- the `xterm-image-layer-top` canvas was created;
- the image canvas measured `1159 × 634` in the browser viewport;
- the pink/teal checkerboard visibly rendered.

This isolates xterm.js and confirms that the missing image in Test B is not an
xterm.js Kitty parsing or rendering failure.

### Source and documentation evidence

The live result agrees with the Herdr 0.9.0 source and documentation:

- `src/protocol/wire.rs` defines `ServerMessage::Graphics { bytes }` as raw
  Kitty bytes.
- `src/server/headless/render.rs` renders `TerminalAttach` and
  `TerminalObserve` through `render_terminal_virtual`, builds a `FrameData`,
  and sends it with the ordinary terminal-frame path. The client-shell branch
  separately builds a pane surface with graphics assets.
- `src/client/terminal_sessions.rs` explicitly ignores
  `ServerMessage::Graphics` for terminal-session clients.
- Herdr's Socket API documents `pane.graphics.*` and
  `file_frame_transport: "direct-kitty"` for the pane/client-shell graphics
  subsystem, not for arbitrary Kitty APC output from a process in a direct
  terminal attachment.
- Cockpit's current `crates/cockpit-herdr/src/terminal_wire.rs` recognizes a
  graphics wire tag but consumes the bounded graphics payload as parked data.
  This explains the earlier Cockpit result but is not the cause of the direct
  Herdr result in Test B.

### Implication for future implementation

Loading `@xterm/addon-image` is necessary but insufficient. A future
implementation needs one of these explicit contracts:

1. Herdr extends direct terminal attachment so PTY Kitty graphics are emitted
   as `ServerMessage::Graphics` and the client forwards their raw Kitty bytes;
   or
2. Cockpit consumes Herdr's client-shell/pane-graphics surface instead of
   treating the pane as a direct ANSI terminal stream.

The current evidence does not support claiming that direct
`Herdr terminal attach → xterm.js` Kitty rendering is available in Herdr
0.9.0.

### Cleanup and reproducibility

Both POCs used uniquely named disposable Herdr sessions and temporary files
outside the repository. The sessions, browser pages, bridges, Herdr servers,
and temporary roots were stopped or removed after verification. The protected
default Herdr session and unrelated running services were not touched.
