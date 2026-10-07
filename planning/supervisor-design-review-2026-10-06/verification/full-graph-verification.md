# Full-graph concept verification — 2026-10-07

Recommendations-only artifacts, not product implementation. Mandatory design agent revised DESIGN.md and PRESENTATION.html. Verified the local HTML in owned hidden Chromium through loopback static server; no Cockpit widget publication or Herdr/provider/product actions.

## Exercised

- Viewports: 1440×1000, 760×900, 360×800, 1440×600. No document horizontal overflow at any size. Screenshots: full-graph-wide.png, full-graph-medium.png, full-graph-narrow.png, full-graph-short.png. All four visually reviewed.
- Initial Tasks mode; Graph switch exposes 26 selectable topology/task nodes (10 root/worker agents, 5 internal subagents, 11 tasks). Subagents off leaves 21 visible nodes. Nested s2 → s3 keyboard navigation observed. Illustrative data only.
- Worker 4 selection opens side details at wide/medium/short-wide, bottom sheet at 360×800. Close restores DOM focus to w4 in all four layouts. Graph remains available beside side details and above the narrow sheet.
- Graph ArrowRight w4 → s2, ArrowRight s2 → s3; Enter selection; End reaches t11 and scrolls. Escape closes details without changing view.
- Show on Board from selected t4 switches to Tasks, preserves task selection and focuses t4. Medium graph scroll offsets [350,120] survive Tasks → Graph return exactly.
- Side splitter ArrowLeft changed value 340 → 356; narrow horizontal splitter ArrowUp changed sheet height 283 → 299. Separator values track resizing.
- Recover counter opens narrow attention overlay; expanded Recover row Locate selects w4 and closes queue. Wide queue remains inline; narrow and short-wide queue use overlay.
- Additional 360×600 check: details full overlay, focus moves to sv-details-close; Escape restores w4 focus. Short-wide 1280×520 preset intentionally retains side details.
- Attention dim marks 21 non-attention nodes while retaining all graph nodes; Subagents checkbox restores hidden internal nodes. Section navigation reaches #coverage. No browser JavaScript errors observed.
- PRESENTATION.html measured 169,145 UTF-8 bytes (<1 MiB); body computed margin 0px; no external HTTP(S) src/href dependencies. Demo actions are local; no host SDK calls.

## Limits

Browser proof covers the standalone concept presentation, not Cockpit product, native WebKit, live OMP/Herdr behavior or real task control. Existing frames remain inert schematics; only the added demo is interactive. Native/product scenarios intentionally not run. Resize was exercised through keyboard splitter and viewport changes; pointer dragging was not separately exercised.
