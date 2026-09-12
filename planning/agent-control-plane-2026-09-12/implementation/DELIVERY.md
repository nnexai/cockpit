# Complete delivery scope

The user has explicitly authorized all twelve packets. The README's original one-packet handoff restriction does not limit this implementation. Earlier scoped receipts establish only the behavior they actually exercised.

Deliver each coherent increment with lightweight checks, real browser interaction in disposable Herdr sessions, and a scoped commit. Follow `../VIEWERS.md` for both viewer surfaces. Keep implementation, runtime verification, and native installation distinct. Preserve unrelated work and never use the user's running session for verification.

## Current work

| Packet | Implemented | Verified | Remaining delivery |
| --- | --- | --- | --- |
| 01 Clipboard | Terminal integration changes prepared | Focused component checks | Real clipboard roundtrip, native adapter and final installation |
| 02 Focus | Initial handoff increment committed; further repair in progress | Initial simple browser handoff only | Busy-output switching, latest intent recovery, confirmation before selection, compact indicators |
| 03 Sidebar | Pending | Pending | Resize, collapse, portrait drawer, readable hierarchy |
| 04 Resource interactions | Pane-local overflow prepared | Pending | Stable-ID drag/drop, correctly targeted menus and keyboard/touch access |
| 05 Commands | Pending | Pending | Searchable bounded action list, accessible capability reasons |
| 06 Files/Context | Shared viewer and narrow picker committed; narrow repair prepared | Earlier actual-content browser proof | Scalable directory/read/search access and completed reader UI |
| 07 Review | Draft continuity and shared viewer committed | Earlier real checkout draft/reattach proof | Remove global untracked inspection failure, on-demand diffs and complete large-file access |
| 08 Annotation | Pending | Pending | Toolbar lifecycle, Select, element preview and guarded shortcuts |
| 09 Feedback | Pending | Pending | Compact capture-bound feedback, delivery and recovery |
| 10 Setup | Parent preselection and readiness slice committed | Earlier real setup/source-lock recovery proof | Compact form, inline prerequisites, complete recovery presentation |
| 11 Terminal edge | Decorative scrollbar removal prepared | Pending | Real scrolling/input and native appearance proof |
| 12 Task readiness | GitHub import and direct Context committed | Real issue #4 workflow recorded in README | Integrate final setup/viewer changes and repeat the complete workflow |

The installed intermediate native build does not include all ongoing changes. The user's existing process still holds an older executable image. Rebuild and atomically install completed increments; leave the user's running process untouched.

## Ownership and sequence

- `terminal_followup`: current focus/clipboard integration, including App and session state.
- `native_delivery`: disposable runtime, scoped integration commits, build/install and evidence. Keep one integration owner.
- `review_scale`: isolated Review backend/client increment in `/tmp/crv12`.
- Next independent work: production browser annotation and setup presenters.
- Serialize sidebar, resource interactions, Commands and feedback edits to App/styles after the current focus increment.
- Coordinate Files/Context access with Review's final revision and source contracts before parallel viewer edits.

Use short disposable roots and session names. Retain proof receipts; stop only run-owned services. An unavailable native or extension check is INCONCLUSIVE, not PASS.
