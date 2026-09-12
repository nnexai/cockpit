# Complete delivery scope

The user has explicitly authorized all twelve packets. The README's original one-packet handoff restriction does not limit this implementation. Earlier scoped receipts establish only the behavior they actually exercised.

Deliver each coherent increment with lightweight checks, real browser interaction in disposable Herdr sessions, and a scoped commit. Follow `../VIEWERS.md` for both viewer surfaces. Keep implementation, runtime verification, and native installation distinct. Preserve unrelated work and never use the user's running session for verification.

## Current work

| Packet | Implemented | Verified | Installed | Remaining delivery |
| --- | --- | --- | --- | --- |
| 01 Clipboard | Terminal integration and explicit Linux Tauri clipboard adapter committed (`9efa6ac`) | Real browser Unicode/multiline roundtrip; focused native `wl-copy`/`wl-paste` Unicode/multiline roundtrip passed | No; install predates `9efa6ac` | Native UI click path still needs the final desktop smoke after install |
| 02 Focus | Focus coordinator and terminal transition gate committed | Busy-output switching and confirmed-target input passed in `cn12` | Yes in current debug install | Repeat after final all-packet integration |
| 03 Sidebar | Integrated (`e09e0e6`) | Shell worker runtime gate pending | No | Resize, collapse, portrait drawer, readable hierarchy |
| 04 Resource interactions | Integrated (`1ad69ed`) | Shell worker runtime gate pending | No | Stable-ID drag/drop, correctly targeted menus and keyboard/touch access |
| 05 Commands | Integrated (`87f3779`) | Shell worker runtime gate pending | No | Searchable bounded action list, accessible capability reasons |
| 06 Files/Context | Shared viewer, narrow picker, and bounded continuation integrated (`1f37928`, `61600a1`, `586f885`, `f10d0b2`, `120f2ad`, `36ceaf1`, `42b9d45`, `6ca0b2c`) | Focused Rust/context tests and frontend build passed; final combined browser gate pending | No final install | Final Files/Context browser proof with the shell increment |
| 07 Review | Lazy inventory, immutable paging, and bounded source reads committed (`eed8718`, `d81b716`) | Real browser listed 260 changed files, paged a 6.6 MB source in 512 KiB chunks, and rejected a continuation after a worktree mutation | No final install | Pair with final shell/viewer flow and native install |
| 08 Annotation | Browser extension increment integrated as `84c8be0` | Isolated MV3 Chrome proof passed per worker receipt | Extension source integrated; native install predates this commit | Pair with final overall workflow |
| 09 Feedback | Integrated (`a89f3ae`, `96ecdbb`) | Shell worker runtime gate pending | No | Compact capture-bound feedback, delivery and recovery |
| 10 Setup | Compact two-stage create/open draft, inline provider identity validation, source/viewer disclosure, and failed-resource recovery committed (`51cbeaa`, `b20af29`) | Real browser create completed with linked worktree/companion/context terminal; reopen preselected its parent repository; open-by-path produced a borrow-only reviewed plan | No; install predates `b20af29` | Repeat retained-source-failure recovery after final install |
| 11 Terminal edge | Decorative scrollbar removal committed | Input and focus path passed; full scroll/native appearance proof remains partial | Yes in current debug install | Complete scrolling and native appearance proof |
| 12 Task readiness | GitHub import and direct Context committed | Real issue #4 workflow recorded in README; current setup create/open review preserves exact effects and companion binding | No final install | Integrate final shell/viewer flow and repeat the complete workflow |

The current native install includes the focus and terminal increment. The annotation extension, setup cutover, and Review/Context backend commits landed afterward, so the next final all-packet install must refresh them if the native bundle embeds those surfaces. The user's existing process still holds an older executable image. Leave it untouched.

## Ownership and sequence

- `terminal_followup`: current focus/clipboard integration, including App and session state.
- `native_delivery`: disposable runtime, scoped integration commits, build/install and evidence. Keep one integration owner.
- `review_scale`: isolated Review backend/client increment in `/tmp/crv12`.
- Next independent work: production browser annotation and setup presenters.
- Serialize sidebar, resource interactions, Commands and feedback edits to App/styles after the current focus increment.
- Coordinate Files/Context access with Review's final revision and source contracts before parallel viewer edits.

Use short disposable roots and session names. Retain proof receipts; stop only run-owned services. An unavailable native or extension check is INCONCLUSIVE, not PASS.
