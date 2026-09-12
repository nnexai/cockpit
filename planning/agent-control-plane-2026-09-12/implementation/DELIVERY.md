# Complete delivery scope

The user has explicitly authorized all twelve packets. The README's original one-packet handoff restriction does not limit this implementation. Earlier scoped receipts establish only the behavior they actually exercised.

Deliver each coherent increment with lightweight checks, real browser interaction in disposable Herdr sessions, and a scoped commit. Follow `../VIEWERS.md` for both viewer surfaces. Keep implementation, runtime verification, and native installation distinct. Preserve unrelated work and never use the user's running session for verification.

## Current work

| Packet | Implemented | Verified | Installed | Remaining delivery |
| --- | --- | --- | --- | --- |
| 01 Clipboard | Terminal integration and explicit Linux Tauri clipboard adapter committed (`9efa6ac`) | Real browser Unicode/multiline roundtrip; focused native `wl-copy`/`wl-paste` Unicode/multiline roundtrip passed; native window input remains INCONCLUSIVE because isolated Tauri launch exposed no second niri window | Yes; final debug install hash verified | Ordinary relaunch required for PID 2523753 to load the new bundle |
| 02 Focus | Focus coordinator and terminal transition gate committed | Busy-output switching and confirmed-target input passed in `cn12` | Yes; final debug install hash verified | Ordinary relaunch required for PID 2523753 |
| 03 Sidebar | Integrated (`e09e0e6`, `662374d`) | Real Herdr resize/collapse/portrait drawer proof passed | Yes; final debug install hash verified | — |
| 04 Resource interactions | Integrated (`1ad69ed`, `662374d`) | Real Herdr label drag and fresh-order proof passed | Yes; final debug install hash verified | — |
| 05 Commands | Integrated (`87f3779`) | Shell worker's 53-test gate and real command proof passed | Yes; final debug install hash verified | — |
| 06 Files/Context | Shared viewer, narrow picker, and bounded continuation integrated (`1f37928`, `61600a1`, `586f885`, `f10d0b2`, `120f2ad`, `36ceaf1`, `42b9d45`, `6ca0b2c`, `4cf51a9`, `24618ea`) | Real `.editorconfig` source, 6.6 MB bounded continuation, and stale mutation proof passed; focused tests passed | Yes; final debug install hash verified | — |
| 07 Review | Lazy inventory, immutable paging, and bounded source reads committed (`eed8718`, `d81b716`) | Real browser listed 260 changed files, paged a 6.6 MB source in 512 KiB chunks, and rejected a continuation after a worktree mutation | Yes; final debug install hash verified | — |
| 08 Annotation | Browser extension increment integrated as `84c8be0` | Production MV3 Chrome mark/capture/pairing proof passed; capture `c77997e1-2067-4482-9e8b-ce7559dfeb5b` is 142,295-byte PNG-backed and paired to `w2/shell12b` | Yes; extension is embedded in rebuilt Cockpit core and final bundle | — |
| 09 Feedback | Integrated (`a89f3ae`, `96ecdbb`, `a089ffa`) | Real mouse feedback accepted with exactly one send, one eligible target, and one Herdr bracketed paste without Enter | Yes; final debug install hash verified | — |
| 10 Setup | Compact two-stage create/open draft, inline provider identity validation, source/viewer disclosure, and failed-resource recovery committed (`51cbeaa`, `b20af29`) | Real browser create completed with linked worktree/companion/context terminal; reopen preselected its parent repository; open-by-path produced a borrow-only reviewed plan | Yes; final debug install hash verified | — |
| 11 Terminal edge | Decorative scrollbar removal committed | Input/focus and shell worker scroll appearance proof passed; native window appearance remains INCONCLUSIVE under isolated launch | Yes; final debug install hash verified | — |
| 12 Task readiness | GitHub import and direct Context committed | Real issue #4 setup/create/open/Context/Review workflow and retained review drafts recorded; authenticated GitHub provider is now configured in the user's Cockpit config | Yes; final debug install hash verified | Ordinary relaunch required for PID 2523753 |

The current native install includes the focus and terminal increment. The annotation extension, setup cutover, and Review/Context backend commits landed afterward, so the next final all-packet install must refresh them if the native bundle embeds those surfaces. The user's existing process still holds an older executable image. Leave it untouched.

## Ownership and sequence

- `terminal_followup`: current focus/clipboard integration, including App and session state.
- `native_delivery`: disposable runtime, scoped integration commits, build/install and evidence. Keep one integration owner.
- `review_scale`: isolated Review backend/client increment in `/tmp/crv12`.
- Next independent work: production browser annotation and setup presenters.
- Serialize sidebar, resource interactions, Commands and feedback edits to App/styles after the current focus increment.
- Coordinate Files/Context access with Review's final revision and source contracts before parallel viewer edits.

Use short disposable roots and session names. Retain proof receipts; stop only run-owned services. An unavailable native or extension check is INCONCLUSIVE, not PASS.
