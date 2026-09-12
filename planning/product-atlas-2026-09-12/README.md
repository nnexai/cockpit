# Product atlas

The [current visual reference](./current/index.html) is the preferred entrypoint. It opens directly as a local file, shows the current runtime screenshots, and links to full-size images. Its canonical card list is [`current/index.json`](./current/index.json); refresh the embedded copy in the HTML after changing that file:

```sh
cd planning/product-atlas-2026-09-12/current
python3 scripts/sync-index.py
```

The current gallery is a screenshot reference. Its cards do not claim that a feature passed runtime verification or that recovery coverage exists. The app and browser worker receipts record the capture timestamp, source commit, runtime identity, viewport or pane geometry, artifact hashes, and cleanup state. The receipt files are [`current/app-receipt.json`](./current/app-receipt.json) and [`current/browser-receipt.json`](./current/browser-receipt.json) now that both lane receipts are complete.

The older evidence catalogs remain available for historical context: [`workbench-terminal`](./workbench-terminal/index.json), [`setup-context-review`](./setup-context-review/index.json), [`browser-extension`](./browser-extension/index.json), and the [full workflow](./workflow/). Historical entries are not copied into the current gallery.
