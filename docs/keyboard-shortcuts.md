# Keyboard navigation

Press Ctrl+B, release it, then press the command. Shift and other modifier keys keep the prefix armed. Escape cancels it. The prefix has no timeout.

| After Ctrl+B | Action |
| --- | --- |
| ? | Open command help |
| 1 through 9 | Select tab by displayed position |
| p / n | Previous / next tab |
| o / Shift+O | Next / previous pane |
| h / j / k / l | Focus the adjacent pane left / down / up / right |
| f | Find a file in the focused Files or Review pane |
| [ / ] | Focus the file tree / document in Files or Review |
| Shift+T / Shift+P | Rename tab / pane |
| c | Create tab |
| v / - | Split right / below |
| z | Toggle pane zoom |
| r | Focus a resize border, then use arrows |

Files and Review also support Ctrl+P or Cmd+P to open the file picker, and Alt+1 / Alt+2 to focus the tree / content. These local shortcuts leave text editors and terminal input alone. Use the prefix from a file tree row or document region. Clicking a file or pressing Enter opens it; renderable Markdown and HTML start in preview. The single View source / Rendered preview button switches modes.

The picker matches characters in order anywhere in a path. For example, `ctv` can match `context/ContextViewer.tsx`. Use arrows or Ctrl+N/P to move, Enter to open, and Escape to close. Files indexes nested directories on demand while the picker is open, up to 512 directories and 10,000 files, subject to the configured directory listing and depth limits. An incomplete index is labeled. Closing the picker cancels further reads. Review searches the changed files in its current comparison.

Tab and pane selection follows Herdr confirmation. Focus requests run in order; when several clicks arrive during a pending request, the newest queued selection is sent next.
