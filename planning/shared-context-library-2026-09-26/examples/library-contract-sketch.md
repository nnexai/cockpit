# Example: Library contracts (sketch)

Status: example for [IMPLEMENTATION_PLAN.md](../IMPLEMENTATION_PLAN.md), revision 2 (review R1–R12). The plan's "Fixed contracts" and Decisions D1–D23 are normative; this file shows the same contracts in full so workers can copy names. Where the two disagree, the plan wins. Nothing here has been compiled.

## 1. Configuration (`cockpit.toml`)

```toml
version = 1
# Optional. Default: $XDG_DATA_HOME/cockpit/library (~/.local/share/cockpit/library).
# Env override: COCKPIT_LIBRARY_ROOT. Must not equal, contain, or sit inside
# state_root, companion_root, or worktree_root.
library_root = "/home/me/.local/share/cockpit/library"

[limits]
library_folder_files = 512          # files per copied folder
library_folder_bytes = 33554432     # 32 MiB per copied folder
library_file_bytes = 4194304        # 4 MiB per copied file
library_space_pages = 200           # pages per followed space
library_attachment_bytes = 26214400 # 25 MiB per attachment
library_item_attachment_bytes = 104857600 # 100 MiB of downloaded attachments per page
library_max_items = 20000

[[providers]]
id = "confluence"
base_url = "https://nnexai.atlassian.net/wiki"   # Cloud; DC example: https://confluence.example.com/confluence
executable = "confluence"                       # pchuri/confluence-cli
login = "cockpit-readonly"                      # confluence-cli profile name; the CLI owns the credential
```

## 2. On-disk layout of `library_root`

```text
<library_root>/
  .cockpit/                      # reserved; hidden from every Library read
    index.json                   # LibraryIndex (schema 1), see §2.1
    library.lock                 # fs2: exclusive = mutate/publish/recover, shared = open item files (D2)
    journal/<intent_id>.json     # PublishIntent, written + fsynced before the first rename (D2)
    locks/<sha256>.lock          # per-item/per-follow nonblocking operation lease
    staging/<uuid>/              # item being assembled; also dl-<n>/ private attachment download dirs (D21)
    trash/<intent_id>/           # backup of a replaced/removed tree; name = journal intent id
    operations/<operation_id>.json
    # No transition/import record: legacy source cache is untouched and never read.
  gitlab/gitlab.example.com/platform-api/review/482/
    .cockpit-item.json           # ItemMarker: item_id, revision, files[{path,hash,bytes}]
    document.md                  # envelope Markdown (CONTEXT.md §7.4 + Library fields)
  github/github.com/acme-widgets/review/7/document.md       # GitHub PR acme/widgets!7 (D19)
  jira/jira.example.com/OPS/issue/OPS-311/document.md
  confluence/nnexai.atlassian.net/SD/page/123456789-release-checklist/
    .cockpit-item.json
    document.md                  # frontmatter attachments[].path = "attachments/release-flow.png"
    attachments/release-flow.png # Cockpit-chosen safe name; original title in frontmatter/metadata
  folders/design-notes-3f2a1c9b/  # FolderCopy: copied tree lives directly here
    .cockpit-item.json
    README.md
    ...
```

Directory names are readable slugs chosen at first publish and never renamed (same rule as `context_assets.rs:121-124`), including when a Confluence page moves to another parent (D20).

### 2.1 Internal records (not on the wire)

```rust
struct LibraryIndex {
    schema_version: u32,              // 1
    generation: u64,                  // bumps on every commit
    items: Vec<LibraryIndexEntry>,
    follows: Vec<IndexFollow>,
}
struct LibraryIndexEntry {
    item_id: String, logical_id: String, item_path: String,
    revision: String,                 // D4 content_revision (provider) | file-list hash (folder)
    source_url: Option<String>, original_url: Option<String>,   // provenance; excluded from revision
    container: Option<LibraryContainer>,                         // presentation; excluded from revision
    folder: Option<LibraryFolderInfo>,                             // includes origin metadata
    state: LibraryItemState, checked_at: Option<String>, diagnostics: Vec<ProjectDiagnostic>,
    follow_id: Option<String>, ancestors: Vec<LibraryAncestor>, version: Option<String>,
    // …remaining LibraryIndexEntry fields
}
struct PublishIntent {                // .cockpit/journal/<intent_id>.json
    schema_version: u32, intent_id: String,
    op: IntentOp,                     // Publish | Remove
    method: IntentMethod,             // Exchange | TwoRename | NewTarget | Remove
    item_id: String, target: String,  // paths relative to library_root
    staging: Option<String>, backup: Option<String>,   // backup = ".cockpit/trash/<intent_id>"
    previous_revision: Option<String>,
    new_entry: Option<LibraryIndexEntry>, // complete new entry, including folder origin metadata, durable before first rename
}
// No transition/import disposition record; the legacy source cache is never imported or deleted by Cockpit.
```

Recovery runs before `open()` returns and decides from the journaled complete `new_entry` and filesystem facts (marker revision at `target`, `staging` and `backup`). For `new_target`, if the rename already moved staging to target but the index commit did not happen, recovery commits `new_entry` from the intent; the complete target and its metadata remain recoverable. Existing-target methods retain the prior target in staging/backup through the index commit and only remove it after recovery can safely roll forward. An operation removes only its own staging on normal completion/failure; open never sweeps unjournaled staging, so a crash orphan is retained rather than risking deletion of another host's active content.

## 3. Protocol DTO sketch (`crates/cockpit-protocol/src/library.rs`)

```rust
#[serde(rename_all = "snake_case")]
pub enum LibraryItemKind { ProviderSnapshot, FolderCopy }

#[serde(rename_all = "snake_case")]
pub enum LibraryItemState { Fresh, Changed, Unknown, RemovedAtSource, Conflict, Failed, Partial }

pub struct LibraryContainer { pub container_id: String, pub label: String }
pub struct LibraryAncestor { pub id: String, pub title: String }
pub struct LibraryPartial { pub unit: String /* "pages" | "files" | "bytes" */, pub have: u64, pub total: Option<u64>, pub reason: String }
pub struct LibraryConflictFile { pub path: String, pub current_hash: String }

// No UnsafeName state: every name downloads through the CLI into a private dir and is stored under a
// Cockpit-chosen safe name (D21). `Failed` carries the reason (e.g. CLI without safe download output).
#[serde(rename_all = "snake_case")]
pub enum LibraryAttachmentState { NotDownloaded, Downloaded, OverLimit, Failed }
pub struct LibraryAttachment {
    pub attachment_id: String, pub original_name: String, pub stored_name: String,
    pub media_type: Option<String>, pub bytes: Option<u64>, pub version: Option<String>,
    pub state: LibraryAttachmentState, pub relative_path: Option<String>,
}

pub struct LibraryFolderInfo {
    pub origin_path: String, pub git_working_tree: bool, pub files: u64, pub bytes: u64,
    pub skipped_symlinks: u32, pub skipped_special: u32, pub skipped_ignored: u32, pub skipped_other: u32,
}

pub struct LibraryItemSummary {
    pub item_id: String,               // "source:<sha256>" (sources.rs:1709) | "folder:<uuid>"
    pub logical_id: String,            // "source:{provider}:{instance}:{type}:{canonical}" | item_id
    pub kind: LibraryItemKind,
    pub provider_id: Option<String>, pub provider_instance: Option<String>,
    pub resource_type: Option<String>, pub canonical_id: Option<String>,
    pub container: Option<LibraryContainer>,
    pub parent_item_id: Option<String>, pub ancestors: Vec<LibraryAncestor>, pub order: Option<u32>,
    pub title: String,
    pub document_path: Option<String>, // relative to library_root, e.g. ".../document.md"
    pub item_path: String,             // item directory relative to library_root
    pub source_url: Option<String>, pub original_url: Option<String>, pub source_revision: Option<String>,
    pub revision: String,              // provider: semantic content hash; folder: hash of file list
    pub state: LibraryItemState,
    pub partial: Option<LibraryPartial>,
    pub conflict: Vec<LibraryConflictFile>,
    pub fetched_at: Option<String>, pub checked_at: Option<String>,
    pub follow_id: Option<String>,
    pub attachments: Vec<LibraryAttachment>,
    pub folder: Option<LibraryFolderInfo>,
    pub diagnostics: Vec<ProjectDiagnostic>,
}

pub struct LibraryFollowSummary {
    pub follow_id: String, pub provider_id: String, pub provider_instance: String,
    pub space_key: String, pub space_name: String, pub include_attachments: bool,
    pub page_count: u32, pub partial: Option<LibraryPartial>, pub excluded_page_ids: Vec<String>,
    pub last_refreshed_at: Option<String>, pub state: LibraryItemState,
}

// No transition notice is returned; the source cache is outside the Library contract.

pub struct LibraryListing {
    pub root: ContextRoot,             // kind = ContextRootKind::Library, root_id = "library:<fs identity>"
    pub generation: String,            // changes on every index commit
    pub items: Vec<LibraryItemSummary>, pub follows: Vec<LibraryFollowSummary>,
    pub next_offset: Option<u32>,
    // No transition field.
    pub diagnostics: Vec<ProjectDiagnostic>,
}

pub struct SpaceTarget { pub session_id: String, pub space_id: String }

#[serde(rename_all = "snake_case")]
pub enum LibraryInputKind { Artifact, ConfluencePage, ConfluenceSpace, Folder }
pub struct LibraryResolveRequest { pub input: String, pub provider_id: Option<String> }
pub struct LibraryResolution {
    pub kind: LibraryInputKind, pub provider_id: Option<String>, pub provider_instance: Option<String>,
    pub title: String, pub canonical_id: Option<String>, pub container_label: Option<String>,
    pub existing_item_id: Option<String>, pub existing_follow_id: Option<String>,
    pub page_count: Option<u32>, pub git_working_tree: Option<bool>, pub file_count: Option<u64>,
    pub diagnostics: Vec<ProjectDiagnostic>,
}

pub struct LibraryAddRequest {
    pub input: String, pub provider_id: Option<String>,
    #[serde(default)] pub hydrate_references: bool,
    #[serde(default)] pub follow_space: bool,
    #[serde(default)] pub download_attachments: bool,
    #[serde(default)] pub refresh_existing: bool,
    pub label: Option<String>,
    pub target: Option<SpaceTarget>,   // S2+
}

#[serde(tag = "scope", rename_all = "snake_case")]
pub enum LibraryRefreshRequest {
    Items { item_ids: Vec<String> },
    Follow { follow_id: String },
    Container { provider_instance: String, container_id: String },
    All,
}

pub struct LibraryReplaceRequest { pub item_id: String, pub confirmed: Vec<LibraryConflictFile> }

#[serde(tag = "mode", rename_all = "snake_case")]
pub enum LibraryRemoveRequest {
    Item { item_id: String, expected_revision: String },
    StopFollowing { follow_id: String },
    Follow { follow_id: String },      // remove followed space and its pages
}

#[serde(rename_all = "snake_case")]
pub enum LibraryAttachmentAction { Download, RemoveDownloaded }
pub struct LibraryAttachmentRequest { pub item_id: String, pub attachment_ids: Vec<String>, pub action: LibraryAttachmentAction }

#[serde(rename_all = "snake_case")]
pub enum LibraryOperationKind { Add, Refresh, SpaceAdd, SpaceUpdate, Attachments }
#[serde(rename_all = "snake_case")]
pub enum LibraryPhaseName { Library, Space }
#[serde(rename_all = "snake_case")]
pub enum LibraryPhaseState { Pending, Running, Done, Partial, Failed, Cancelled }
pub struct LibraryPhase {
    pub phase: LibraryPhaseName, pub state: LibraryPhaseState,
    pub done: u32, pub total: Option<u32>, pub message: Option<String>, pub error: Option<ErrorResponse>,
}
#[serde(rename_all = "snake_case")]
pub enum LibraryReportOutcome { New, Updated, Unchanged, RemovedAtSource, Partial, Failed, Conflict }
pub struct LibraryReportRow { pub item_id: Option<String>, pub follow_id: Option<String>, pub title: String, pub outcome: LibraryReportOutcome, pub reason: Option<String> }
pub struct LibraryRefreshReport {
    pub new: u32, pub updated: u32, pub unchanged: u32, pub removed_at_source: u32,
    pub partial: u32, pub failed: u32, pub conflict: u32, pub rows: Vec<LibraryReportRow> /* <= 256 */,
    pub truncated_rows: bool,
}
#[serde(rename_all = "snake_case")]
pub enum SpaceCopyMode { Reflink, Copy, Mixed }
pub struct SpacePhaseResult {
    pub space_id: String, pub copy_mode: Option<SpaceCopyMode>,
    pub written: Vec<String>, pub skipped_edited: Vec<String>, pub companion_root_id: Option<String>,
}
pub struct LibraryOperation {
    pub operation_id: String, pub kind: LibraryOperationKind, pub phases: Vec<LibraryPhase>,
    pub item_ids: Vec<String>, pub report: Option<LibraryRefreshReport>, pub space: Option<SpacePhaseResult>,
    pub target: Option<SpaceTarget>, pub cancel_requested: bool, pub finished: bool,
    pub created_at: String, pub updated_at: String,
}

// Reads (bodies mirror the pane-scoped Context DTOs; responses reuse ContextDirectory/ContextDocument/ContextMedia
// with binding_id = "library" and root_id = listing.root.root_id).
pub struct LibraryDirectoryRequest { pub path: String, pub offset: Option<u32>, pub revision: Option<String> }
pub struct LibraryDocumentRequest { pub path: String, pub expected_revision: Option<String>, pub offset: Option<u32> }
pub struct LibraryMediaRequest { pub path: String, pub expected_revision: Option<String> }

// Space (S2/S3/S6)
#[serde(rename_all = "snake_case")]
pub enum SpaceCopyState { UpToDate, LibraryNewer, EditedInSpace, RemovedAtSource, MissingInSpace, NotInLibrary, NotLinked }
pub struct SpaceFollowSummary {
    pub follow_id: String, pub space_key: String, pub page_count: u32,
    pub new_pages: u32,            // follow pages not in the Space's known_page_item_ids (D8)
    pub changed_pages: u32,        // unedited copies whose copied revision is not current (D4)
    pub edited_pages: u32, pub removed_at_source_pages: u32,
}
pub struct SpaceCopyRow {
    pub item_id: Option<String>,   // None for NotLinked rows and for follow aggregate rows
    pub logical_id: String,        // item logical id, or "follow:<id>" for an aggregate row
    pub title: String,
    pub provider_id: Option<String>, pub resource_type: Option<String>, pub kind: LibraryItemKind,
    pub state: SpaceCopyState, pub library_newer: bool,
    pub paths: Vec<String>, pub edited: Vec<LibraryConflictFile>,
    pub copy_mode: Option<SpaceCopyMode>,
    pub library_revision_copied: Option<String>, pub current_library_revision: Option<String>,
    pub follow: Option<SpaceFollowSummary>,
}
#[serde(rename_all = "snake_case")]
pub enum SpaceAddAttemptState { Pending, Failed }
pub struct SpaceAddAttempt {       // durable phase-2 record, stored in the Library (D10)
    pub target: SpaceTarget, pub space_label: Option<String>,
    pub item_id: Option<String>, pub follow_id: Option<String>,   // exactly one is Some
    pub title: String, pub state: SpaceAddAttemptState, pub error: Option<ErrorResponse>,
    pub operation_id: String, pub updated_at: String,
}
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SpaceCompanionStatus {
    Available { companion_root_id: String, companion_label: String },
    Unavailable { error: ErrorResponse },   // rows is empty; attempts are still listed
}
pub struct SpaceContextRequest { pub target: SpaceTarget }
pub struct SpaceContextListing {
    pub target: SpaceTarget, pub companion: SpaceCompanionStatus,
    pub attempts: Vec<SpaceAddAttempt>,    // failed/pending adds for this target, shown first (D23)
    pub rows: Vec<SpaceCopyRow>, pub behind: u32, pub diagnostics: Vec<ProjectDiagnostic>,
}
// Also the retry path: re-adding saved ids runs phase 2 only and clears matching attempts on success.
pub struct SpaceAddRequest { pub target: SpaceTarget, pub item_ids: Vec<String>, pub follow_ids: Vec<String> }
pub struct SpaceAttemptsDismissRequest { pub target: SpaceTarget, pub item_ids: Vec<String>, pub follow_ids: Vec<String> }
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum SpaceUpdateScope {
    // Only the listed items and followed spaces in this Space. A follow id updates only that
    // follow's pages (new + changed unedited); other follows and items stay byte-identical.
    Selection { item_ids: Vec<String>, follow_ids: Vec<String> },
    All,   // every LibraryNewer/MissingInSpace row and every follow in this Space; edited copies skipped
}
pub struct SpaceUpdateRequest { pub target: SpaceTarget, pub scope: SpaceUpdateScope, pub replace_edited: Vec<LibraryConflictFile> }
// logical_id may be "follow:<id>": removes that follow's page copies (CAS on edited) and its library_follows record.
pub struct SpaceRemoveRequest { pub target: SpaceTarget, pub logical_id: String, pub confirmed: Vec<LibraryConflictFile> }

// No legacy-cache routes or DTOs; obsolete cache remains untouched on disk.
```

## 4. Companion manifest v2 (JSON, `context-manifest.json`)

Single-file provider item (existing path rule, D22):

```json
{
  "logical_id": "source:gitlab:https://gitlab.example.com:review:platform/api!482",
  "relative_path": "sources/gitlab/review/platform-api-482.md",
  "kind": "review",
  "source": "gitlab",
  "generated": true,
  "revision": "2026-09-25T10:11:12Z",
  "content_hash": "sha256:…bytes as written (= Library marker hash for document.md)…",
  "bytes": 18234,
  "copy_mode": "reflink",
  "status": "complete",
  "updated_at": "…",
  "source_repository_id": "https://gitlab.example.com",
  "source_checkout_path": "",
  "source_identity": "platform/api!482",
  "source_hash_before": "sha256:…Library content revision copied…",
  "source_hash_after": "sha256:…bytes as written…",
  "library_item_id": "source:3b1f…",
  "library_revision": "sha256:…Library content revision copied…",
  "library_file": "document.md"
}
```

Multi-file Confluence page in a followed space (shared relative layout, D22). Top-level manifest fields gain `library_follows`:

```json
{
  "schema_version": 2,
  "library_follows": [
    { "follow_id": "follow:9c0e…", "known_page_item_ids": ["source:aa…", "source:bb…"], "added_at": "…", "updated_at": "…" }
  ],
  "entries": [
    { "logical_id": "source:confluence:https://nnexai.atlassian.net/wiki:page:123456789",
      "relative_path": "sources/confluence/page/SD/123456789-release-checklist/document.md",
      "library_item_id": "source:aa…", "library_revision": "sha256:…", "library_file": "document.md",
      "library_follow_id": "follow:9c0e…", "content_hash": "sha256:…", "…": "…" },
    { "logical_id": "source:confluence:https://nnexai.atlassian.net/wiki:page:123456789#attachments/release-flow.png",
      "relative_path": "sources/confluence/page/SD/123456789-release-checklist/attachments/release-flow.png",
      "library_item_id": "source:aa…", "library_revision": "sha256:…", "library_file": "attachments/release-flow.png",
      "library_follow_id": "follow:9c0e…", "content_hash": "sha256:…same bytes as the Library file…", "…": "…" }
  ]
}
```

`document.md` frontmatter lists `attachments: [{id, original_name, stored_name, media_type, bytes, version, path: "attachments/release-flow.png"}]`; the path is relative to `document.md` and resolves in the Library item and in the Space copy alike.

v1 companion entries remain byte-for-byte unchanged and unlinked while the Library is introduced. When the user explicitly re-adds an item, Cockpit may match it to an existing companion entry by source identity without rewriting its file or manifest unless the normal add flow requires a manifest update; an old copied hash that does not equal the current Library revision is reported as behind.

## 5. Credential redaction check (verification helper)

Prints only paths and counts, never a secret value. Exit 0 = clean, 1 = match, 2 = inconclusive (no secret found to check).

```python
#!/usr/bin/env python3
"""Usage: redaction_check.py <dir> [<dir> ...]  (reads confluence-cli config, never prints values)."""
import json, os, pathlib, sys

cfg_dir = pathlib.Path(os.environ.get("CONFLUENCE_CONFIG_DIR") or pathlib.Path.home() / ".config/confluence-cli")
legacy = pathlib.Path.home() / ".confluence-cli"
if not (cfg_dir / "config.json").exists() and (legacy / "config.json").exists():
    cfg_dir = legacy
secrets = set()
def walk(value):
    if isinstance(value, dict):
        for key, inner in value.items():
            if key.lower() in {"token", "apitoken", "api_token", "password", "email", "username", "cookie"} \
                    and isinstance(inner, str) and len(inner) >= 6:
                secrets.add(inner.encode())
            walk(inner)
    elif isinstance(value, list):
        for inner in value:
            walk(inner)
try:
    walk(json.loads((cfg_dir / "config.json").read_text()))
except FileNotFoundError:
    pass
if not secrets:
    print("inconclusive: no credential values found in the confluence-cli config (keychain/.netrc?)")
    sys.exit(2)
matches = 0
for base in sys.argv[1:]:
    for path in pathlib.Path(base).rglob("*"):
        if path.is_file() and not path.is_symlink():
            data = path.read_bytes()
            if any(secret in data for secret in secrets):
                matches += 1
                print(f"MATCH: {path}")
print(f"checked {len(secrets)} credential values; files with matches: {matches}")
sys.exit(1 if matches else 0)
```

## 6. Confluence argv allowlist (`confluence_args`, D16)

Every call is `confluence --profile <login> …` with env additions `CONFLUENCE_READ_ONLY=true CONFLUENCE_CLI_ANALYTICS=false`. Nothing outside this table is ever spawned. `<p>` = digits (1–20), `<K>` = `[A-Za-z0-9~_-]{1,255}`, `<T>` = title (1–255 chars, no control characters), `<C>` = `[A-Za-z0-9._~%+=/-]{1,1024}`.

| Call | argv after `--profile <login>` | Used by |
|---|---|---|
| `Spaces` | `spaces --all --json` | S6 browse spaces |
| `Info{p}` | `info <p> --json` | S5 fetch/instance proof, S6 removal/move confirmation, DC display follow-up |
| `Read{p}` | `read <p> --format markdown` | S5 body |
| `Find{K,T}` | `find --space <K> --json -- <T>` | S5 DC `/display/<K>/<T>` resolution |
| `Attachments{p}` | `attachments <p> --json` | S5 metadata |
| `DownloadAttachment{p,pattern,dest}` | `attachments <p> --download --dest <dest> --pattern=<pattern> --json` | S7 (D21) |
| `Api{Content{p}}` | `api content/<p> -X GET -f expand=<subset of ancestors,version,space,history.lastUpdated,metadata.labels>` | S5 ancestors/space/last-modified; OQ1 (b) |
| `Api{Labels{p}}` | `api content/<p>/label -X GET` | S5 labels |
| `Api{Space{K}}` | `api space/<K> -X GET -f expand=homepage` | S6 homepage + space name |
| `Api{Search{K,limit,expand,cursor\|start}}` | `api content/search -X GET -f cql=space="<K>" and type=page -f limit=<1..100> -f expand=<subset> [-f cursor=<C> \| -f start=<n>]` | S6 enumeration (D20) |

Refused before spawn (table-tested): any other subcommand; `api` without `-X GET`, or with `-X` ≠ `GET`; an `api` endpoint not in the four templates or given as a full URL or absolute path; `--input`, `-H`, `-i`, `--jq`, `--silent`; `--token`, `--email`, `--cookie`; a `cql` other than the template; a `_links.next` whose path is not `…/rest/api/content/search` or whose `cql` differs.

Download mapping (D21). The CLI prints the following; Cockpit keeps only `attachments[i]` whose `id` equals the requested id, then requires `destination == dest` and `savedTo` to be `dest/<one component>`:

```json
{ "attachmentCount": 2, "downloaded": 2, "destination": "<dest>",
  "attachments": [ { "title": "../x", "id": "att101", "savedTo": "<dest>/x" },
                   { "title": "X", "id": "att102", "savedTo": "<dest>/X" } ] }
```

Here the pattern `../x` also matched `X` (case-insensitive, `bin/commands/attachments.js` + `globToRegExp`): `att102` is discarded, and `att101` is stored as `attachments/<readable_name("../x")>`.

## 7. Space-copy presentation map (`src/app/library/spaceCopyPresentation.ts`, D23)

```ts
type Chip = { glyph: string; word: string; tone: "idle" | "working" | "blocked" | "secondary" };

export function spaceCopyChip(row: SpaceCopyRow): Chip {
  switch (row.state) {
    case "up_to_date": return { glyph: "✓", word: "Up to date", tone: "idle" };
    case "library_newer": return { glyph: "↑", word: "Library newer", tone: "working" };
    case "edited_in_space": return { glyph: "✎", word: row.library_newer ? "Edited in Space · Library newer" : "Edited in Space", tone: "working" };
    case "removed_at_source": return { glyph: "⊘", word: "Removed at source", tone: "secondary" };
    case "missing_in_space": return { glyph: "○", word: "Missing in Space", tone: "working" };
    case "not_in_library": return { glyph: "·", word: "Not in Library", tone: "secondary" };
    case "not_linked": return { glyph: "·", word: "Not linked", tone: "secondary" };
    default: { const never: never = row.state; return never; }
  }
}

type HeaderAction =
  | { kind: "add" }                                   // "Add to <Space>"
  | { kind: "in_space_up_to_date" }                   // "In <Space> · ✓ Up to date"
  | { kind: "update" }                                // "Update in <Space>"
  | { kind: "edited"; libraryNewer: boolean }         // "✎ Edited in Space[ · Library newer]" + Replace… + View Library version
  | { kind: "removed_at_source" }                     // "⊘ Removed at source" + Remove from this Space…
  | { kind: "not_in_library" };                       // "· Not in Library" + Remove from this Space… + Add to Library again

export function headerSpaceAction(row: SpaceCopyRow | undefined): HeaderAction {
  if (!row) return { kind: "add" };
  switch (row.state) {
    case "up_to_date": return { kind: "in_space_up_to_date" };
    case "library_newer": return { kind: "update" };
    case "edited_in_space": return { kind: "edited", libraryNewer: row.library_newer };
    case "removed_at_source": return { kind: "removed_at_source" };
    case "not_in_library": return { kind: "not_in_library" };
    case "missing_in_space": return { kind: "add" };  // OQ7 (a): Resources-only state; SpaceAdd restores the file
    case "not_linked": return { kind: "add" };        // unreachable: NotLinked rows have item_id = null
    default: { const never: never = row.state; return never; }
  }
}
```

Resources order when the dialog opens (D23, design §4.8): attempt rows `✕ Not added — Retry`; then `Library newer`, `Missing in Space`, `Edited in Space`, `Removed at source`, `Not in Library`; then up-to-date and `Not linked` rows; by title within a group.
