# Ideas inbox

Untriaged ideas and rough edges to remember. This is an inbox, not a roadmap, implementation plan, or set of commitments. Items here do not change active feature plans or define their acceptance criteria. Promote an idea into planning only after deciding to scope it.

## Ideas

- [ ] **Support Herdr custom keybindings, including `herdr-nnex-actions`.** Explore compatibility with the custom actions plugin and custom keybindings generally. This likely depends on terminal overlay panes or dialogs for actions that need an interactive UI; that dependency is part of the idea, not a planned implementation commitment.
- [ ] **Revisit how Library items relate to main repositories.** Explore whether forge-backed repositories should be represented by a repository node linked to the checked-out repository instead of copied into the Library. For forge artifacts such as GitHub pull requests or GitLab merge requests, consider linking back to the repository checkout and possible repository actions—for example, a clean-state fast-forward merge as an “update” command—with a small amount of repository state surfaced. Model and safety are undecided.
- [ ] **Improve browser annotation text input.** The current text input takes up too much space and looks unattractive; revisit its footprint and presentation.
- [ ] **Check macOS display scaling / DPR.** The default appears slightly too large and blurry across the sidebar, menus, and terminal. Investigate device-pixel-ratio handling and native rendering on macOS.
- [ ] **Support merge-request notes and opening the MR externally.** Explore reading/adding notes on an MR and opening its URL in the system browser, not Cockpit's embedded browser. The right home is undecided: this could belong in the Review pane, while opening the MR could also be a Space-level action.
- [ ] **Show CI status for review artifacts.** Surface GitLab CI/CD pipelines and GitHub Actions for an MR/PR so it is clear when the pipeline completes. Details such as refresh behavior and status display are undecided.
- [ ] **Drag and drop panes.** Allow dragging panes to switch their positions and change their order.
- [ ] **Revisit the terminal font.** The current terminal font looks wider than the preferred font in the system terminal; compare and adjust the default.
