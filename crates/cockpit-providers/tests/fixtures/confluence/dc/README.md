# Data Center fixtures (hand-written)

Not live-validated: no Data Center instance was available. These files are written from confluence-cli `2.25.2` source and the Server/DC REST v1 documentation for an instance at `https://confluence.example.com/confluence` (context path `/confluence`, API `/rest/api`).

- `info.json`, `attachments.json`, `find-release-checklist.json`: CLI `--json` output (`normalizePage`, `attachments`, `findPageByTitle`). DC page URLs are display URLs (`/display/ENG/Release+Checklist`), so the page id is not in `info.url`.
- `content.json`, `labels.json`, `api-space-ENG.json`: raw REST bodies printed by `api … -X GET`.
- `api-search-page-{1,2}.json`: CQL `content/search` with `start`-style `_links.next` and `totalSize`; no folder ancestors (folders are Cloud-only).
- `read.md`: `read --format markdown` output.
- `info-auth-failed.stderr.json`: the CLI's `--json` error envelope for HTTP 401.

Identity fields use placeholders; no credentials are stored.
