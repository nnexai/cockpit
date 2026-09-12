# Current visual atlas

This folder is the small, current reference gallery for Cockpit at the capture commit. It collects settled screenshots from two owned runtime lanes:

- `app-*` covers the Cockpit shell, commands and menus, tabs and panes, Files, Context, Review, and setup.
- `browser-*` covers the production browser extension page, capture and feedback states, and popup status.

The gallery contains 24 images. The app lane contributes 17 images and the browser lane contributes 7. Desktop uses 1440x900. Narrow views use 480x900 or 800x1000 when the layout changes there. `app-context-split-360.png` records the real narrow split geometry at 1440x900, and the receipt records its 371.39 pixel Context pane. A worker receipt beside each lane records the source commit, runtime identity, viewport, pane bounds, and screenshot hashes.

The screenshot list in [`index.json`](./index.json) records the worker filenames. App captures live beside this README. Browser captures live under [`screenshots/`](./screenshots/). Keep the list short when two images show the same settled state. A receipt may contain extra action and diagnostic captures; those belong to runtime evidence and do not need gallery cards. The browser receipt explicitly omits its exploratory camera permission error state.

Open [`index.html`](./index.html) directly as a local file to browse the cards. Use the surface filter to narrow the view, and click a card to open the full-size PNG. After editing `index.json`, run `python3 scripts/sync-index.py` to refresh the embedded manifest used by the local page. The app worker can be rerun with `python3 scripts/capture-app.py` in its owned disposable runtime. From the repository root, rerun the browser lane with `COCKPIT_EXTENSION_DIR=/path/to/paired-production-bundle planning/product-atlas-2026-09-12/current/scripts/capture-browser.sh`. The old atlas remains available through the historical links in the page and the original surface indexes:

- [`workbench-terminal`](../workbench-terminal/index.json)
- [`setup-context-review`](../setup-context-review/index.json)
- [`browser-extension`](../browser-extension/index.json)

Those indexes and their runtime trees are historical evidence catalogs. They are not current gallery entries.

The selected captures were recorded on 2026-09-12 against source commit `d7da954fbbeaa294425b7b50de6a09efe3658f51`. The lane receipts carry the runtime identity, viewport or pane geometry, artifact SHA-256 values, and cleanup result. This gallery is a visual reference and carries no runtime PASS or recovery coverage claim.
