# Complete delivery scope

The user has explicitly authorized all twelve packets. The README's original one-packet handoff restriction does not limit this implementation. Earlier scoped receipts establish only the behavior they actually exercised.

Deliver each coherent increment with lightweight checks, real browser interaction in disposable Herdr sessions, and a scoped commit. Follow `../VIEWERS.md` for both viewer surfaces. Keep implementation, runtime verification, and native installation distinct. Preserve unrelated work and never use the user's running session for verification.

## Current work

| Packet | Implemented | Verified | Installed | Remaining delivery |
| --- | --- | --- | --- | --- |
| 01 Clipboard | Terminal integration and explicit Linux Tauri clipboard adapter committed (`9efa6ac`) | Real browser Unicode/multiline roundtrip; focused native `wl-copy`/`wl-paste` Unicode/multiline roundtrip passed | No; install predates `9efa6ac` | Native UI click path still needs the final desktop smoke after install |
| 02 Focus | Focus coordinator and terminal transition gate committed | Busy-output switching and confirmed-target input passed in `cn12` | Yes in current debug install | Repeat after final all-packet integration |
| 03 Sidebar | Pending | Pending | No | Resize, collapse, portrait drawer, readable hierarchy |
| 04 Resource interactions | Pane-local overflow prepared | Pending | No | Stable-ID drag/drop, correctly targeted menus and keyboard/touch access |
| 05 Commands | Pending | Pending | No | Searchable bounded action list, accessible capability reasons |
| 06 Files/Context | Shared viewer and narrow picker committed; narrow repair prepared | Earlier actual-content browser proof | No final install | Scalable directory/read/search access and completed reader UI |
| 07 Review | Draft continuity and shared viewer committed | Earlier real checkout draft/reattach proof | No final install | Remove global untracked inspection failure, on-demand diffs and complete large-file access |
| 08 Annotation | Browser extension increment integrated as `84c8be0` | Isolated MV3 Chrome proof passed per worker receipt | Extension source integrated; native install predates this commit | Pair with final overall workflow |
| 09 Feedback | Pending | Pending | No | Compact capture-bound feedback, delivery and recovery |
| 10 Setup | Compact create/open flow, inline provider identity validation, source/viewer disclosure, and failed-resource recovery committed (`51cbeaa`) | Focused SetupDialog/provider tests and typecheck passed; earlier real setup/source-lock recovery proof retained | No; install predates `51cbeaa` | Repeat short real create/open and retained-source-failure flow after final install |
| 11 Terminal edge | Decorative scrollbar removal committed | Input and focus path passed; full scroll/native appearance proof remains partial | Yes in current debug install | Complete scrolling and native appearance proof |
| 12 Task readiness | GitHub import and direct Context committed | Real issue #4 workflow recorded in README | No final install | Integrate final setup/viewer changes and repeat the complete workflow |

The current native install includes the focus and terminal increment. The annotation extension and the `9efa6ac`/`51cbeaa` commits landed afterward, so the next final all-packet install must refresh them if the native bundle embeds those surfaces. The user's existing process still holds an older executable image. Leave it untouched.

## Ownership and sequence

- `terminal_followup`: current focus/clipboard integration, including App and session state.
- `native_delivery`: disposable runtime, scoped integration commits, build/install and evidence. Keep one integration owner.
- `review_scale`: isolated Review backend/client increment in `/tmp/crv12`.
- Next independent work: production browser annotation and setup presenters.
- Serialize sidebar, resource interactions, Commands and feedback edits to App/styles after the current focus increment.
- Coordinate Files/Context access with Review's final revision and source contracts before parallel viewer edits.

Use short disposable roots and session names. Retain proof receipts; stop only run-owned services. An unavailable native or extension check is INCONCLUSIVE, not PASS.
