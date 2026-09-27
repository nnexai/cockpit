# Cloud CLI evidence

Captured through the existing `default` profile, reported by `confluence profile list` as `[read-only]` for `nnexai.atlassian.net`. The CLI was run only with `CONFLUENCE_READ_ONLY=true` and `CONFLUENCE_CLI_ANALYTICS=false`; no CLI configuration was changed. Version: `2.25.2` (`cli-version.txt`).

The successful `spaces --all --json` call returns `SD` (`Software Development`) and the account's personal space. Output is scrubbed for account IDs, display names, email addresses, and credential-shaped values. `api space/SD -X GET -f expand=homepage` returned the space identity and links but no `homepage` field. `api content/search` with the planned CQL returned `size: 0`, `results: []`, and no `_links.next`; consequently no result carried `ancestors`, `extensions.position`, or a page ID. `find --space SD --json -- "Release Checklist"` returned `NOT_FOUND`; a search for `Software Development` returned the space object, not a page.

No readable page in `SD` was available to capture the planned `info`, Markdown `read`, page/label metadata, or attachment-list/download output. Calls against page ID `1` returned `NOT_FOUND`; they are retained only as negative/error-shape evidence and are not substitutes for a page fixture. No remote write was attempted. The existing profile and site configuration remain untouched.

For CLI `2.25.2`, a nonexistent profile name returns `VALIDATION` (`Profile … not found`), not `AUTH_FAILED`; `info 1 --json` returns `NOT_FOUND` with HTTP 404. The real-profile successful `spaces` call verifies the configured profile is usable for read-only access, but page and attachment output shapes remain unavailable live. The credential redaction check from the implementation plan scanned 25 captured files against 2 credential values and returned exit 0.

Live OQ status: Cloud `content/search` supplied no page rows, so live pagination/ancestor/position shape is unresolved (OQ1); no page attachment existed to check CLI `destination`/`savedTo` output (OQ2). These shapes must be contract-tested with the local fake server. No credentials or raw account identity fields are stored here.

## Synthetic page fixtures (`page/`)

Because no live page was readable, `page/` holds hand-written Cloud page output, not captures. The shapes follow confluence-cli `2.25.2` source (`normalizePage` for `info --json`, `normalizeAttachment` plus the `attachments` command for `attachments --json`, raw REST v1 bodies for `api content/<id>` and `api content/<id>/label`, and `read --format markdown` text). Identity fields use placeholders (`<display-name>`, `<account-id>`). Folder ancestors (`type: folder`) are Cloud-only. `info-auth-failed.stderr.json` is the CLI's `--json` error envelope for HTTP 401. The same shapes are served over loopback by `tests/support/fake_confluence.rs` for the real-CLI harness (`COCKPIT_CONFLUENCE_CLI`).
