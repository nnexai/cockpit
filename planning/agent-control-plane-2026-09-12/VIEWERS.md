# One design for Files, Context, and Review

The user requested a unified design after noticing different font sizes in the mock readers. Files, Context source, diff text, and expanded diff source use one set of typography and geometry rules. Implement packets 06 and 07 against this document together. Do not let separate agents choose separate viewer styles.

The live issue-4 trial found that production Context source and diff text already use 13 px type with a 19 px line height. Preserve that baseline while consolidating the styles. The mockups now explicitly share [viewer.css](mocks/viewer.css), including a loaded monospace font. Compare them [side by side](mocks/viewers.html).

| Role | Shared treatment |
| --- | --- |
| Source, diff, expanded source, line numbers | IBM Plex Mono, 13 px / 19 px, same fallback stack |
| Toolbar controls and file-list labels | IBM Plex Sans, 13 px / 20 px |
| Secondary metadata | IBM Plex Sans, 12 px / 16 px |
| Rendered Markdown prose | One shared prose scale; code blocks use the source scale. Heading levels describe document content. |
| File rows | 30 px minimum; same hover, selection, focus, and truncation behavior |
| Toolbars and document headers | 36 px minimum; expand only if controls wrap |
| Document inset | 12 px; 8 px in a narrow pane |
| Source gutter | 4 character widths; diff reserves two number columns within 8 character widths |
| Border, focus, disabled and selection colors | Shared Cockpit tokens; differences in diff backgrounds indicate additions and deletions |

Narrow panes change available navigation and insets, never the font size. Line numbers keep the source line height. Use the same file-picker, tree collapse, loading, retry, comment editor, and draft-overview patterns. Root selection belongs to Files/Context; comparison selection belongs to Review. These distinct controls should still look and behave as parts of one product.

Keep stable geometry when showing selection, hover actions, pending work, or comments. Do not replace the document with an empty panel while loading another file. Preserve the previous content and position until the next source is ready. A comment count that is loading must not become a confirmed zero.

## Acceptance for both packets

- Compare the same file as source, as a diff, and as expanded diff source. Font family, size, line height, gutter baseline, and document inset match their shared roles.
- Check computed styles and screenshots after fonts have loaded at 1440×900, 800×1000, 600×900, and 480×900, plus a narrow split pane on desktop.
- Verify file switching, line selection, commenting, refresh, and return from another tab without losing position or drafts.
- Remove divergent component-local font and spacing rules during the cutover. Shared roles have one owner; feature-specific code supplies data and actions.
- Keep Herdr terminal fonts, terminal content, tab order, and pane layout outside this change.
