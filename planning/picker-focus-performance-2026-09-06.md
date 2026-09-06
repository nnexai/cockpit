# Files, Review, and terminal activity follow-up

## Observations and implementation

1. Picker selection reset when a parent supplied an equivalent candidate array. Preserve the selected file by identity, including when another result is inserted before it. Highlight the actual fuzzy subsequence, with Unicode character positions.
2. Loading a single-child directory compresses its path and removes the focused tree button. Restore focus to the resulting row, unless the user moved elsewhere during loading. Skip unavailable entries with arrow navigation.
3. Equivalent pane-inspection responses replaced React state every 2.5 seconds. Retain state identity for unchanged presentations and repeated errors; still apply changed presentations and error recovery.
4. Review arrows used the selected file rather than the focused row. Use the focused row as the starting point, with the selected file as fallback for container shortcuts.
5. Session subscriptions replay historical pane creation events. Refreshing subscriptions after every such event causes a recurring snapshot loop. Distinguish changes to the subscribed pane set from historical events. Exclude scroll-only notifications from the workbench subscription; terminal frames have their own stream.

## Acceptance

- Reproduce picker reset before the patch, then keep selection and focus across multiple inspection polls.
- Exercise Files tree arrows and compressed paths, including delayed responses after focus moves to content.
- Check Review file arrows, picker selection, and match highlighting.
- Run a bounded terminal workload alongside graphical panes. Count frames, snapshots, DOM mutations, and CPU time; verify focus during output.
- Run frontend tests/build and affected Rust tests. Review the integrated changes and commit only this increment.

## Runtime isolation

All live checks use the ledger-owned `daily` Herdr session under `/tmp/cdu`, its fixture repository, and a gateway on port 4183. The user's `default` session and running installed native application are excluded.

## Evidence

Completed and independently reviewed.

- The original live picker reproduction reset `focus-file-07.md` to `focus-file-00.md` after six seconds. The same scenario now retains the selected file and focused button. A regression also covers insertion before the selected result.
- Live browser checks passed for Files compressed-directory arrows, fuzzy subsequence highlighting, and Files/Review picker stability across six-second polling intervals. Review arrows were reproduced failing when DOM focus and selected file differed; the fixed path follows the focused row.
- During a separate interaction check, 236 terminal frames arrived while Files picker selection, Review arrow focus, and Review picker selection remained stable. Screenshot: `/tmp/cdu/evidence/output-focus.png`.
- The ten-second output sample delivered 200 terminal frames before and after. Full session snapshots fell from 53 to 0 during that interval. Files DOM mutations remained zero, and focus remained on `focus-file-10.md`.
- Measured service CPU fell from 15.95% to 11.56% of one core; Chromium CPU from 24.02% to 20.13%. These are short local samples, not a controlled benchmark: the after run includes Review as a third pane, and startup warmup differs. The idle sample still contains finite historical event catch-up; startup replay is not eliminated. See the adjacent evidence JSON for raw counts and limitations.
- `bun run test`: 138 passed. `bun run build`: passed.
- `cargo test -p cockpit-herdr`: 88 passed across unit and integration suites. Replay retains the existing event transport; real pane-set changes refresh scoped subscriptions. Identity replacement now terminates before publishing Changed, and the regression requires that stronger behavior.
- `cargo test -p cockpit-host`: 11 passed. `cargo fmt --all -- --check` and `git diff --check`: passed.
- `bunx tauri build --debug --no-bundle`: passed. The actual native build launched against the owned session and rendered terminal, Files, and Review. Screenshot: `/tmp/cdu/evidence/followup-native.png`. Detailed focus/workload automation ran through the browser; the native check was a build-and-launch smoke.

## Cleanup

Stopped the owned native process and loopback gateway, then stopped `daily` through the ledger-guarded helper. Fixture data and screenshots remain under `/tmp/cdu`; no live user installation or protected session was changed.
