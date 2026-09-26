# Local toolchain bootstrap inventory

## Scope and conclusion

This inventory targets the local Fedora Workstation and Herdr 0.8.2 installation described by the project context. The repository currently contains architecture documents and research notes but no `Cargo.toml` or frontend manifest, so this is a prerequisite inventory rather than a project build check.

This inventory predates the local Herdr upgrade. Protocol-20 and unavailable-native-dependency observations below are historical; current runtime and verification authority lives in `DECISIONS.md`.

The Rust compiler, Cargo, rustup, Bun, Node.js, npm, C/C++ compiler toolchain, linker, Make, CMake, OpenSSL development files, and Herdr are present. The Tauri Linux WebKitGTK/GTK development stack is not installed, and the Tauri CLI is not installed as a Cargo subcommand. Those are the immediate native-build blockers. The `pnpm` executable is a Corepack shim, and its harmless version/help invocation currently fails signature verification before pnpm starts.

Bun can own frontend dependency installation and package scripts: Tauri's own project generator lists Bun as a supported JavaScript package manager, and Bun documents itself as a standalone package manager for existing `package.json` projects. Bun does not replace Rust, Cargo, the native linker, or WebKitGTK development libraries.

## Observed local versions and paths

All checks below were read-only version/help/status checks; no install, update, activation, or configuration mutation was run.

| Area | Observation |
|---|---|
| OS | Fedora Linux 44 Workstation (`/etc/os-release`); x86_64 kernel 7.1.12-200.fc44.x86_64 |
| Rust | `rustc 1.98.0 (88d9e12ae 2026-08-18)`; executable `/home/nnex/.cargo/bin/rustc` |
| Cargo | `cargo 1.98.0 (797e8a9bc 2026-08-05)`; executable `/home/nnex/.cargo/bin/cargo` |
| rustup | `rustup 1.29.0 (28d1352db 2026-03-05)`; active `stable-x86_64-unknown-linux-gnu`; installed target `x86_64-unknown-linux-gnu` |
| Bun | `1.3.14`; executable `/home/nnex/.bun/bin/bun` |
| Node.js | `v26.8.1`; executable `/home/linuxbrew/.linuxbrew/bin/node` |
| npm | `11.19.0`; executable `/home/linuxbrew/.linuxbrew/bin/npm`; npm prefix `/home/linuxbrew/.linuxbrew` |
| Corepack | `0.29.4`; executable `/home/nnex/.nvm/versions/node/v24.11.1/bin/corepack` |
| pnpm | `/home/nnex/.nvm/versions/node/v24.11.1/bin/pnpm`, resolving to Corepack's `dist/pnpm.js`; `pnpm --version` fails before reporting a pnpm version |
| C compiler | GCC 16.2.1 (Fedora package `gcc`) |
| C++ compiler | G++ 16.2.1 (Fedora package `g++`) |
| Linker/build tools | GNU ld 2.46.1-1.fc44; GNU Make 4.4.1; CMake 4.3.0 |
| pkg-config | 2.5.1 |
| OpenSSL development files | `openssl-devel-3.5.8-1.fc44.x86_64`; `pkg-config --modversion openssl` reports 3.5.8 |
| GLib development files | `glib2-devel-2.88.3-1.fc44.x86_64`; `pkg-config --modversion glib-2.0` reports 2.88.3 |
| Herdr | `herdr 0.8.2`; executable `/home/linuxbrew/.linuxbrew/bin/herdr`; stable channel |
| Herdr runtime | `herdr status --json`: server running, version 0.8.2, protocol 20, compatible; socket `/home/nnex/.config/herdr/herdr.sock` |
| Herdr config/schema | `herdr config check`: `config: ok`; `herdr api schema`: protocol 20, schema version 1, responses/events/subscription schemas present |
| Tauri CLI | `cargo tauri --version` fails: Cargo has no `tauri` subcommand (only a similar `miri` command) |

## Missing Tauri Linux prerequisites

The official Tauri v2 prerequisite page lists Linux development dependencies including `libwebkit2gtk-4.1-dev`, `build-essential`, `libxdo-dev`, `libssl-dev`, `libayatana-appindicator3-dev`, and `librsvg2-dev` ([Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)). Its package example is Debian-oriented. Fedora package names should be confirmed against Fedora repositories before installation; the expected WebKitGTK package is `webkit2gtk4.1-devel`.

Read-only checks found all of the following absent as RPM packages and/or pkg-config modules:

- `webkit2gtk4.1-devel` and `webkit2gtk-4.1` (`pkg-config` cannot find `webkit2gtk-4.1`);
- older `webkit2gtk4.0-devel` and `webkit2gtk-4.0` also absent;
- `gtk3-devel` / `gtk+-3.0` absent;
- `libsoup3-devel` / `libsoup-3.0` absent;
- `libxdo-devel` absent;
- `libayatana-appindicator-gtk3-devel` absent;
- `librsvg2-devel` absent.

The installed GCC/G++, linker, Make, CMake, OpenSSL, GLib, and `pkg-config` cover only part of the native prerequisite set. Severity: **blocking for a Tauri Linux compile or native smoke run**, but not blocking for Rust-only work, Bun dependency work, or Herdr socket integration.

Do not install these packages as part of a repository bootstrap that promises no workstation-global mutation. If native development is required, use an explicitly approved Fedora package transaction (or a disposable Fedora toolbox/container with the required `-devel` packages) and then re-check `pkg-config` names. The Tauri docs' `sudo apt ...` command is not a Fedora command.

## Bun versus Node/npm/pnpm ownership

Tauri's official project creation guide explicitly offers `pnpm`, `yarn`, `npm`, and **`bun`** as JavaScript package-manager choices ([Tauri create project](https://v2.tauri.app/start/create-project/)). Bun documents `bun install` as a standalone, Node-compatible package manager usable in any project with `package.json`; it installs dependencies and writes `bun.lock` ([Bun install](https://bun.com/docs/pm/cli/install)). Bun also provides `bun run` for package scripts, and the installed help confirms `bun run <script>` and `bunx` support.

Recommendation: choose Bun as the single frontend package manager and script runner. Use `bun install --frozen-lockfile` once a repository `bun.lock` exists, and invoke scripts through `bun run <name>`. Use a project-local Tauri CLI dev dependency and `bunx tauri ...`/a package script rather than a globally installed Cargo CLI. This avoids Corepack and avoids mutating npm, pnpm, or Cargo global state.

Bun is not a universal Node replacement. Bun's compatibility documentation lists several Node APIs as partial (including `child_process`, `crypto`, `module`, `worker_threads`, and `node:test`) ([Bun Node.js compatibility](https://bun.com/docs/runtime/nodejs-compat)). Frontend tooling should therefore be smoke-tested under Bun when selected; a package that requires a missing Node API remains a legitimate blocker. Tauri's prerequisite page still describes installing Node.js for JavaScript frameworks and says Corepack is optional for pnpm/yarn ([Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)). Keeping Node installed is prudent for fallback tooling, but it need not own this repository's normal install/scripts path.

For repo-level declarations, pin the Rust toolchain in `rust-toolchain.toml` once the intended Rust version is chosen (the current workstation compiler is 1.98.0), and declare the supported Bun range in the eventual `package.json` `engines.bun` field (for example, `>=1.3.14 <1.4`). Do not use a `packageManager` value of `bun@...` as a Corepack solution: Corepack's own documentation lists only `yarn`, `npm`, and `pnpm` as permitted package-manager names ([Corepack README](https://github.com/nodejs/corepack)). Record the exact Bun version in contributor/CI instructions and invoke that binary explicitly. A checked-in Bun lockfile is the reproducibility boundary; do not check in a pnpm lockfile while claiming Bun ownership.

## Corepack/pnpm signature failure

The observed failure is:

```text
Error: Cannot find matching keyid: {"signatures":[{"keyid":"SHA256:DhQ8wR5APBvFHLF/+Tc+AYvPOdTpcIDqOhxsBHRwC7U", ...}],"keys":[{"keyid":"SHA256:jl3bwswu80PjjokCgh0o2w5c2U4LhQAE57gj9cz1kzA", ...}]}
```

This is supportable as a stale Corepack trust-key problem, not evidence of a pnpm project failure. Corepack 0.29.4 is older than the upstream 0.31.0 release. Corepack's changelog records that 0.31.0 updated npm registry keys on 2025-01-27 ([Corepack changelog](https://github.com/nodejs/corepack/blob/main/CHANGELOG.md#0310-2025-01-27)). The upstream issue reproduces the same `pnpm@10.1.0` key mismatch, identifies the new `DhQ8...` and old `jl3b...` keys, and records that the fix shipped in Corepack 0.31.0 ([Corepack issue #612](https://github.com/nodejs/corepack/issues/612)). npm documents that signatures are in package metadata and public keys are served by `/-/npm/v1/keys` ([npm registry signatures](https://docs.npmjs.com/about-registry-signatures)).

The local PATH is also mixed: Node/npm resolve from Homebrew (`/home/linuxbrew/.linuxbrew`), while Corepack/pnpm resolve from an nvm Node 24.11.1 tree; the pnpm shim runs under the first `node` on PATH (Node 26.8.1). That split increases ambiguity, but the error's old embedded key set is sufficient to explain the failure. Do not disable integrity checks with `COREPACK_INTEGRITY_KEYS=0`; Corepack documents it as a bypass and it removes the supply-chain verification this check provides. Do not update Corepack globally for this bootstrap, because the assignment forbids global mutation.

## Safe bootstrap commands (no global mutation)

After project manifests and a lockfile exist:

```bash
# Confirm the selected tools without changing them
rustc --version
cargo --version
bun --version
node --version
herdr --version
herdr api schema

# Frontend install and scripts, owned by Bun
bun install --frozen-lockfile
bun run <script-name>

# Project-local Tauri CLI (after it is declared as a dev dependency)
bunx tauri --help
bun run tauri dev
```

`bun install --frozen-lockfile` is intentionally preferred over an unconstrained install for a checked-in lockfile; Bun documents that frozen mode refuses lockfile drift ([Bun install](https://bun.com/docs/pm/cli/install)). `bunx` may download a package into Bun's cache, but does not require a global package installation. Avoid `corepack enable`, `corepack install -g`, `corepack prepare`, `npm install -g`, `bun install -g`, `cargo install`, and `herdr update` in repository bootstrap instructions. Avoid `sudo apt ...` on Fedora; any required Fedora `dnf` transaction should be a separately approved host/container provisioning step.

## Blockers and severity

| Blocker | Severity | Effect | Resolution boundary |
|---|---|---|---|
| WebKitGTK 4.1 and related GTK/appindicator/XDo/Rsvg development packages absent | **High / native-build blocker** | Tauri Linux compile/link cannot begin reliably | Provision Fedora `-devel` packages in an approved host or disposable toolbox; then verify pkg-config modules |
| No project manifest/lockfile yet | **High / bootstrap-definition blocker** | Cannot exercise Bun install or Tauri scripts | Add project files in implementation work (outside this research assignment) |
| No `cargo-tauri` subcommand | **Medium** | `cargo tauri ...` unavailable | Prefer project-local `@tauri-apps/cli`; no global install needed |
| Corepack 0.29.4/pnpm shim signature mismatch | **Medium / pnpm-only** | pnpm commands fail before pnpm starts | Prefer Bun; otherwise separately update/select a supported Corepack without disabling verification |
| Bun compatibility gaps for some Node APIs | **Low to medium / workload-dependent** | Specific frontend tools may require Node | Smoke-test chosen dependency scripts under Bun; retain Node fallback |
| Herdr | **None observed** | Installed 0.8.2 server is running and schema-compatible | Keep schema-gating against protocol 20 as specified by the architecture |

## Bootstrap decisions

- **Frontend owner:** Bun 1.3.14, with checked-in `bun.lock`; use `bun install --frozen-lockfile` and `bun run`.
- **Native owner:** Rust/Cargo 1.98.0 and system linker; do not install `cargo-tauri` globally. Declare a project-local Tauri CLI dev dependency and call it with Bun.
- **Repo declarations:** add an explicit Rust toolchain file when implementation selects the supported Rust release; declare Bun compatibility in `package.json` (prefer `engines.bun`) and document the exact CI/workstation Bun version. Do not rely on Corepack's unsupported `packageManager: bun@...` convention.
- **Herdr:** target and schema-gate Herdr 0.8.2 / protocol 20; the installed server and `herdr api schema` are healthy. The documented Herdr CLI/socket API remains the primary integration surface ([Herdr socket API](https://herdr.dev/docs/socket-api/)).
- **Rejected alternatives:** pnpm as the default (current Corepack shim fails key verification); disabling `COREPACK_INTEGRITY_KEYS` (unsafe verification bypass); global `npm install -g`, `corepack enable/install -g`, `cargo install`, `bun install -g`, or `herdr update` (mutate workstation-global state); Debian `apt` instructions on Fedora (wrong distribution).
- **Observed commands/versions:** Rust/Cargo 1.98.0, rustup 1.29.0, Bun 1.3.14, Node 26.8.1, npm 11.19.0, Corepack 0.29.4, GCC/G++ 16.2.1, ld 2.46.1, Make 4.4.1, CMake 4.3.0, Herdr 0.8.2/protocol 20.
- **Unresolved blocker:** Fedora's exact package availability and dependency names for Tauri's WebKitGTK 4.1 stack must be confirmed during an approved package-provisioning step; this report intentionally performed no package install.
