# Bounded graph clarity revision — 2026-10-07

Recommendation artifacts only. Mandatory design agent `GraphClarityDesign` revised existing DESIGN.md and PRESENTATION.html after the exact Execute grant. No product code, user state, providers, Notes, bindings, commits, pushes or widget publication. User-reported failures were accepted without re-running the broken presentation.

## Changes

- Consistent navigation labels: **Show in Tasks** / **Show in Graph**, including details and attention rows. DOM/Herdr focus remains technical keyboard/terminal terminology, not navigation copy.
- Graph edges now use the direct `.gv-canvas > svg.gv-edges` layer; node glyphs no longer inherit absolute positioning. Status and document glyphs have explicit viewBoxes and fixed 16×16 boxes within 20×20 icon slots.
- Nodes use a 240×48 grid: separate icon, title, tier, status and provenance regions. Title/provenance may ellipsize; icons, tier and status do not shrink. Static figure and demo layout/edges use matching geometry. Historical Before schematics retain their original geometry/content; accidental icon rendering is corrected.
- All initial recommendations remain: attention-first Tasks default, optional full-workarea Graph, attention tiers/queue, result-first details, dialog/recovery polish, relationships and keyboard access.

## Browser evidence

Actual standalone HTML served on owned loopback server and exercised in hidden Chromium. No live Cockpit/Herdr/provider actions.

| Viewport | Details | Page horizontal overflow | Graph node geometry failures | Close returns focus |
|---|---|---|---|---|
| 1440×1000 | Side panel | None | 0 | Worker 4 |
| 760×900 | Side panel | None | 0 | Worker 4 |
| 360×800 | Bottom sheet | None | 0 | Worker 4 |
| 1440×600 | Side panel | None | 0 | Worker 4 |
| 360×600 | Full overlay | None | 0 | Worker 4 |

- Checked all **44 concept nodes**: 18 static + 26 demo. Every icon was 16×16, position static, inside its node and disjoint from title/metadata. Status and tier did not clip; title and tier did not intersect. All five glyph states present: working, idle, blocked, done, unknown/unobserved; task document icon present. SVG symbols resolved. All 13 historical Before graph-node icons also measured 16×16 and did not intersect their title text.
- Clicked all 26 demo nodes; each opened matching details: supervisor, 11 open tasks, 9 workers, 5 internal subagents. Unassigned tasks and nested subagents included. Task activation again cleared selection.
- Tasks was initial after reset. Show in Tasks from t4 selected/focused t4 in Tasks; Show in Graph selected/focused t4 in Graph. Attention action labels changed with the current view.
- Graph keyboard: ArrowRight w4→s2→s3, Enter selected Doc linter, Escape closed details without changing Graph, End reached t11.
- Side splitter ArrowLeft: 340→356; sheet splitter ArrowUp: 283→299. Values updated through aria-valuenow.
- Attention filter dimmed 21 nodes without removing any. Subagents off: 21 visible nodes; on: 26. Narrow Recover counter opened overlay; Show in Graph selected w4, closed queue, and opened sheet.
- Graph offsets [350,120] survived Tasks→Graph return unchanged at 760×900. Review-lane task details retained State/Notice and relationship navigation. Result-first and dialog/recovery designs remain static recommendations, not demo implementations.
- Visually reviewed wide, medium, narrow, short and short-narrow screenshots; static graph at left and right (including nested node and detail panel); Before/proposed workarea, relationship and narrow figures. Graph overflow is contained within graph/figure scrollports; deliberate title/provenance ellipses and historical Before clipping remain.
- No browser JavaScript errors observed.

## Files

Structured observations: [bounded-redesign-checks.json](bounded-redesign-checks.json). Screenshots: `bounded-redesign-wide.png`, `-medium.png`, `-narrow.png`, `-short.png`, `-short-narrow.png`, `-static.png`, `-static-full.png`, `-static-right.png`, `-workarea.png`, `-relationships.png`, `-narrow-figures.png` (all share the bounded-redesign prefix). The static-full image remains bounded by the presentation max-width; static-right records the scrollport’s right side.

## Limits and cleanup

This verifies the illustrative standalone presentation, not product/native WebKit, a published Cockpit widget, real OMP/Herdr behavior or task-control authority. Pointer dragging of splitters was not separately exercised; keyboard resizing was. No claim of live result/recovery implementation. Prior full-graph-verification.md is historical evidence; its old navigation wording is superseded by this report. Owned browser tab released and owned presentation server stopped; shared Chromium and unrelated resources untouched. Parent reviews and updates the same widget.
