import type {
  LibraryFollowSummary,
  LibraryItemState,
  LibraryItemSummary,
  LibraryRefreshReport,
  LibraryReportOutcome,
  LibraryResolution,
  ProjectProvider,
  ProviderKind,
  ProviderCredentialStatus,
  SpaceTarget,
} from "../../protocol/generated/v1";
import type { UiIconName } from "../UiIcon";

/**
 * The Space a Library surface can add to: the selected Space in the Library
 * view, or a Context pane's own Space. `live` is false while Herdr isn't live,
 * when Space-targeted actions are unavailable (design §4.1).
 */
export type LibrarySpace = { target: SpaceTarget; label: string; live: boolean };

export function sameSpaceTarget(left: SpaceTarget | null | undefined, right: SpaceTarget | null | undefined): boolean {
  return Boolean(left && right && left.session_id === right.session_id && left.space_id === right.space_id);
}

/** Provider families Cockpit can snapshot into the Library, keyed by explicit provider kind. */
export type ProviderFamily = { key: ProviderKind | "other"; name: string; review: string };

const FAMILIES: Record<ProviderKind, ProviderFamily> = {
  gitlab: { key: "gitlab", name: "GitLab", review: "MR" },
  github: { key: "github", name: "GitHub", review: "PR" },
  gitea: { key: "gitea", name: "Gitea", review: "PR" },
  jira: { key: "jira", name: "Jira", review: "issue" },
  confluence: { key: "confluence", name: "Confluence", review: "page" },
};

export function providerFamily(providers: readonly ProjectProvider[], providerId: string | null): ProviderFamily {
  const provider = providers.find((candidate) => candidate.id === providerId);
  const family = provider ? FAMILIES[provider.kind] : undefined;
  return family ?? { key: "other", name: providerId ?? "Source", review: "review" };
}

export function jiraProviders(providers: readonly ProjectProvider[]): ProjectProvider[] {
  return providers.filter((provider) => provider.kind === "jira");
}

export function confluenceProviders(providers: readonly ProjectProvider[]): ProjectProvider[] {
  return providers.filter((provider) => provider.kind === "confluence");
}

/** A Library item or lookup result that is a Confluence page. */
export function isConfluencePage(item: Pick<LibraryItemSummary, "kind" | "resource_type">): boolean {
  return item.kind === "provider_snapshot" && item.resource_type === "page";
}

/**
 * Whether Cockpit can fetch a Jira issue's attachment bytes with a token stored in Cockpit.
 * Null for anything that isn't a Jira issue; `loading` until the token states were read (`statuses` null).
 */
export function jiraAttachmentAccess(item: Pick<LibraryItemSummary, "kind" | "resource_type" | "provider_id">, providers: readonly ProjectProvider[], statuses: readonly ProviderCredentialStatus[] | null): "stored" | "needs_token" | "loading" | null {
  if (item.kind !== "provider_snapshot" || item.resource_type !== "issue" || providerFamily(providers, item.provider_id).key !== "jira") return null;
  if (statuses === null) return "loading";
  return statuses.find((status) => status.provider_id === item.provider_id)?.state === "stored" ? "stored" : "needs_token";
}

/** `https://gitlab.test:9443/subfolder/` → `gitlab.test:9443/subfolder`. */
export function instanceHost(instance: string | null): string {
  if (!instance) return "Unknown instance";
  try {
    const url = new URL(instance);
    return `${url.host}${url.pathname.replace(/\/+$/, "")}`;
  } catch {
    return instance;
  }
}

/** A Confluence site as the design names it: `nnexai.atlassian.net`, not `nnexai.atlassian.net/wiki`. */
export function confluenceSite(instance: string | null): string {
  return instanceHost(instance).replace(/\/wiki$/, "");
}

/** A bare Jira work-item key such as `OPS-311`. */
export const JIRA_KEY = /^[A-Z][A-Z0-9_]+-[1-9][0-9]*$/;

/** The Library only resolves links; a bare Jira key becomes that site's canonical browse link. */
export function libraryInputUrl(input: string, provider: ProjectProvider | undefined): string {
  const trimmed = input.trim();
  if (!JIRA_KEY.test(trimmed) || !provider) return trimmed;
  return `${provider.base_url.replace(/\/+$/, "")}/browse/${trimmed}`;
}

/** A bare Confluence page id; it needs a selected Confluence provider. */
export const CONFLUENCE_PAGE_ID = /^[0-9]{1,20}$/;

/** A Confluence page reference typed into Add (design §4.5), recognized before any lookup. */
export type ConfluencePageInput = {
  /** From a bare id, a Cloud `/spaces/<KEY>/pages/<id>` link or a `viewpage.action?pageId=<id>` link. */
  pageId: string | null;
  spaceKey: string | null;
  /** From a Data Center `/display/<KEY>/<title>` link. */
  title: string | null;
  /** The link's host; null for a bare page id. */
  host: string | null;
  /** Configured Confluence providers that can read it: all of them for a bare id, else those whose base URL contains the link. */
  providers: ProjectProvider[];
};

function urlWithin(baseUrl: string, url: URL): boolean {
  try {
    const base = new URL(baseUrl);
    if (base.protocol !== url.protocol || base.host !== url.host) return false;
    const prefix = base.pathname.replace(/\/+$/, "");
    return url.pathname === prefix || url.pathname.startsWith(`${prefix}/`);
  } catch {
    return false;
  }
}

function pathSegment(segment: string): string | null {
  try {
    return decodeURIComponent(segment.replace(/\+/g, " ")).trim() || null;
  } catch {
    return null;
  }
}

export function confluencePageInput(input: string, providers: readonly ProjectProvider[]): ConfluencePageInput | null {
  const trimmed = input.trim();
  const confluence = confluenceProviders(providers);
  if (CONFLUENCE_PAGE_ID.test(trimmed)) return { pageId: trimmed, spaceKey: null, title: null, host: null, providers: confluence };
  let url: URL;
  try { url = new URL(trimmed); } catch { return null; }
  if (url.protocol !== "https:" && url.protocol !== "http:") return null;
  const cloud = /\/spaces\/([^/]+)\/pages\/([0-9]{1,20})(?:\/[^/]*)?\/?$/.exec(url.pathname);
  const display = /\/display\/([^/]+)\/([^/]+)\/?$/.exec(url.pathname);
  const idParameter = url.searchParams.get("pageId");
  let page: Pick<ConfluencePageInput, "pageId" | "spaceKey" | "title">;
  if (cloud) page = { pageId: cloud[2]!, spaceKey: pathSegment(cloud[1]!), title: null };
  else if (display && pathSegment(display[2]!)) page = { pageId: null, spaceKey: pathSegment(display[1]!), title: pathSegment(display[2]!) };
  else if (/\/pages\/viewpage\.action$/.test(url.pathname) && idParameter !== null && CONFLUENCE_PAGE_ID.test(idParameter)) page = { pageId: idParameter, spaceKey: null, title: null };
  else return null;
  const matching = confluence.filter((provider) => urlWithin(provider.base_url, url));
  // A link on another configured provider's site is that provider's to read.
  if (matching.length === 0 && providers.some((provider) => !confluence.includes(provider) && urlWithin(provider.base_url, url))) return null;
  return { ...page, host: url.host, providers: matching };
}

/** A Confluence space reference typed into Add (design §4.5): the space to follow. */
export type ConfluenceSpaceInput = {
  spaceKey: string;
  /** The link's host; null for a bare key. */
  host: string | null;
  /** Configured Confluence providers that can read it: all of them for a bare key, else those whose base URL contains the link. */
  providers: ProjectProvider[];
};

/** Confluence space keys (`SD`, `~jdoe`); a bare `~` is the home folder, and a bare key needs a configured Confluence provider. */
const CONFLUENCE_SPACE_KEY = /^(?!~$)(?=.*[A-Za-z~_-])[A-Za-z0-9~_-]{1,255}$/;

/**
 * A space link (`…/spaces/<KEY>[/overview]`, Data Center `…/display/<KEY>`) or
 * a bare space key while a Confluence provider is configured.
 */
export function confluenceSpaceInput(input: string, providers: readonly ProjectProvider[]): ConfluenceSpaceInput | null {
  const trimmed = input.trim();
  const confluence = confluenceProviders(providers);
  if (CONFLUENCE_SPACE_KEY.test(trimmed)) return confluence.length > 0 ? { spaceKey: trimmed, host: null, providers: confluence } : null;
  let url: URL;
  try { url = new URL(trimmed); } catch { return null; }
  if (url.protocol !== "https:" && url.protocol !== "http:") return null;
  const match = /\/(?:spaces\/([^/]+)(?:\/overview)?|display\/([^/]+))\/?$/.exec(url.pathname);
  const key = match ? pathSegment(match[1] ?? match[2]!) : null;
  if (!key || !CONFLUENCE_SPACE_KEY.test(key)) return null;
  const matching = confluence.filter((provider) => urlWithin(provider.base_url, url));
  if (matching.length === 0 && providers.some((provider) => !confluence.includes(provider) && urlWithin(provider.base_url, url))) return null;
  return { spaceKey: key, host: url.host, providers: matching };
}

/** `SD · Software Development`: how a followed or resolved space is named everywhere. */
export function spaceDisplayName(space: { space_key: string; space_name: string }): string {
  return space.space_name && space.space_name !== space.space_key ? `${space.space_key} · ${space.space_name}` : space.space_key;
}

/** What a follow is called everywhere: `SD · Software Development` for a space, the JQL for a query. */
export function followTitle(follow: Pick<LibraryFollowSummary, "source">): string {
  return follow.source.kind === "jira_query" ? follow.source.jql : spaceDisplayName(follow.source);
}

/** `1 issue`, `12 issues`; `100+ issues` when the count is a lower bound. */
export function issueCount(count: number, exact = true): string {
  return `${count}${exact ? "" : "+"} ${count === 1 && exact ? "issue" : "issues"}`;
}

/** `38 pages` for a followed space, `12 issues` for a followed query. */
export function followCountText(follow: Pick<LibraryFollowSummary, "source" | "item_count" | "reference_depth">): string {
  if (follow.source.kind !== "jira_query") return pageCount(follow.item_count);
  // Followed references add pages and other trackers' items, so a query that follows them counts items.
  return follow.reference_depth ? `${follow.item_count} ${follow.item_count === 1 ? "item" : "items"}` : issueCount(follow.item_count);
}

/** A bare Jira project key, which Add reads as `project = KEY`. */
const JIRA_PROJECT_KEY = /^[A-Z][A-Z0-9_]{1,31}$/;
const JQL_OPERATOR = /[=~<>]|\s(?:in|is|was|changed)\s/i;

/** A Jira query typed into Add (design: follows). */
export type JiraQueryInput = {
  jql: string;
  /** A bare project key such as `OPS`; a Confluence provider could also read it as a space key. */
  bare: boolean;
  /** Configured Jira providers that can run it. */
  providers: ProjectProvider[];
};

/**
 * A bare project key or text with a JQL operator, while a Jira provider is
 * configured. Links, Jira issue keys and folder paths are other inputs.
 */
export function jiraQueryInput(input: string, providers: readonly ProjectProvider[]): JiraQueryInput | null {
  const trimmed = input.trim();
  const jira = jiraProviders(providers);
  if (jira.length === 0 || !trimmed || /^https?:\/\//i.test(trimmed) || JIRA_KEY.test(trimmed)) return null;
  if (trimmed.startsWith("/") || trimmed.startsWith("~/") || trimmed === "~" || /^[A-Za-z]:[\\/]/.test(trimmed)) return null;
  if (JIRA_PROJECT_KEY.test(trimmed)) return { jql: trimmed, bare: true, providers: jira };
  return JQL_OPERATOR.test(trimmed) ? { jql: trimmed, bare: false, providers: jira } : null;
}

/** Queries offered as chips under the Add field; the project ones need a project key. */
export function jiraQueryPresets(projectKey: string | null): { label: string; jql: string }[] {
  return [
    ...(projectKey ? [
      { label: "Whole project", jql: `project = ${projectKey}` },
      { label: "Not done", jql: `project = ${projectKey} AND statusCategory != Done` },
      { label: "Updated in last 14 days", jql: `project = ${projectKey} AND updated >= -14d` },
    ] : []),
    { label: "Assigned to me, unresolved", jql: "assignee = currentUser() AND resolution = Unresolved" },
  ];
}

/** The project a bare key or a `project = KEY …` query names. */
export function jiraQueryProject(jql: string): string | null {
  const trimmed = jql.trim();
  if (JIRA_PROJECT_KEY.test(trimmed)) return trimmed;
  return /^project\s*=\s*"?([A-Z][A-Z0-9_]{1,31})"?(?:\s|$)/i.exec(trimmed)?.[1]?.toUpperCase() ?? null;
}

/** The space a `confluence_space` resolution names, from its container label or its key and title. */
export function resolutionSpaceName(resolution: Pick<LibraryResolution, "container_label" | "canonical_id" | "title">): string {
  return resolution.container_label ?? spaceDisplayName({ space_key: resolution.canonical_id ?? resolution.title, space_name: resolution.title });
}

/** `1 page`, `38 pages`. */
export function pageCount(count: number): string {
  return `${count} ${count === 1 ? "page" : "pages"}`;
}

function trailingNumber(value: string | null): number | null {
  const match = value ? /(\d+)$/.exec(value) : null;
  return match ? Number(match[1]) : null;
}

export function itemDisplayId(item: Pick<LibraryItemSummary, "canonical_id" | "resource_type" | "provider_id">, providers: readonly ProjectProvider[]): string | null {
  const id = item.canonical_id;
  if (!id) return null;
  const family = providerFamily(providers, item.provider_id);
  if (family.key === "jira") return id;
  // A page id is metadata; the tree and header show the page by title (design §4.3).
  if (family.key === "confluence" || item.resource_type === "page") return null;
  const number = trailingNumber(id);
  if (number === null) return id;
  if (item.resource_type === "review") return family.key === "gitlab" ? `!${number}` : `#${number}`;
  return `#${number}`;
}

/** Kind chip: `GitLab MR`, `GitLab issue`, `GitHub PR`, `Jira issue`, `Confluence page`, `Folder`. */
export function itemKindLabel(item: Pick<LibraryItemSummary, "kind" | "resource_type" | "provider_id">, providers: readonly ProjectProvider[]): string {
  if (item.kind === "folder_copy") return "Folder";
  if (isConfluencePage(item)) return "Confluence page";
  const family = providerFamily(providers, item.provider_id);
  if (family.key === "jira") return "Jira issue";
  return `${family.name} ${item.resource_type === "review" ? family.review : "issue"}`;
}

export function itemTreeLabel(item: LibraryItemSummary, providers: readonly ProjectProvider[]): string {
  const id = itemDisplayId(item, providers);
  return id && id !== item.title ? `${id} ${item.title}` : item.title;
}

export type StateTone = "idle" | "working" | "blocked" | "muted";
/** Bare SVG shapes shared by every Library state mark (design §4.1a); the names are `UiIcon` names. */
export type StateShape = Extract<UiIconName, "check" | "up" | "ring" | "half-ring" | "slash-ring" | "dot-ring" | "close" | "edit">;
export type StateChip = { shape: StateShape; word: string; tone: StateTone };

/** Library item states (design §4.6). Only `fresh` reads `Up to date`. */
export function libraryStateChip(state: LibraryItemState): StateChip {
  switch (state) {
    case "fresh": return { shape: "check", word: "Up to date", tone: "idle" };
    case "changed": return { shape: "up", word: "Updated", tone: "working" };
    case "unknown": return { shape: "ring", word: "Not checked", tone: "muted" };
    case "removed_at_source": return { shape: "slash-ring", word: "Removed at source", tone: "muted" };
    case "conflict": return { shape: "edit", word: "Edited in Library", tone: "working" };
    case "failed": return { shape: "close", word: "Refresh failed", tone: "blocked" };
    case "partial": return { shape: "half-ring", word: "Partial", tone: "working" };
    default: {
      const unreachable: never = state;
      return unreachable;
    }
  }
}

/**
 * A Library timestamp as epoch milliseconds (design §4.9). The Library index
 * writes `fetched_at`/`checked_at` as 13-digit decimal epoch milliseconds
 * (`Date.parse` reads those as NaN); provider frontmatter writes ISO-8601.
 * Anything else is unknown.
 */
export function parseLibraryTime(value: string | null | undefined): number | null {
  if (!value) return null;
  if (/^\d{13}$/.test(value)) return Number(value);
  const time = Date.parse(value);
  return Number.isFinite(time) ? time : null;
}

const RECENT_DAYS = 14;

/** `27 Sep` this year, `27 Sep 2025` otherwise. */
export function formatDate(ms: number, now: number = Date.now()): string {
  const date = new Date(ms);
  return date.toLocaleDateString(undefined, { day: "numeric", month: "short", ...(date.getFullYear() === new Date(now).getFullYear() ? {} : { year: "numeric" }) });
}

/** `2026-09-27 16:17` in the viewer's time zone, 24 h, no seconds. */
export function formatDateTime(ms: number): string {
  const date = new Date(ms);
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

/** The relative phrase and whether it is a relative one (under 14 days); older times read as dates. */
function agePhrase(ms: number, now: number): { text: string; recent: boolean } {
  const seconds = Math.max(0, Math.round((now - ms) / 1000));
  if (seconds < 60) return { text: "just now", recent: true };
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return { text: `${minutes} min ago`, recent: true };
  const hours = Math.round(minutes / 60);
  if (hours < 24) return { text: `${hours} h ago`, recent: true };
  const days = Math.round(hours / 24);
  if (days < RECENT_DAYS) return { text: `${days} d ago`, recent: true };
  return { text: formatDate(ms, now), recent: false };
}

/** `just now`, `13 min ago`, `3 h ago`, `5 d ago`, then a date from 14 days. A future time reads `just now`. */
export function formatAgo(ms: number, now: number): string {
  return agePhrase(ms, now).text;
}

/** `formatAgo` of a raw Library timestamp; null when it is empty or unknown. */
export function relativeTime(value: string | null | undefined, now: number): string | null {
  const time = parseLibraryTime(value);
  return time === null ? null : formatAgo(time, now);
}

/** A Library timestamp for the Details popover: local date and time, the age while recent, and the ISO UTC tooltip. */
export type TimeDetail = { text: string; ago: string | null; iso: string };

export function timeDetail(value: string | null | undefined, now: number): TimeDetail | null {
  const time = parseLibraryTime(value);
  if (time === null) return null;
  const age = agePhrase(time, now);
  return { text: formatDateTime(time), ago: age.recent ? age.text : null, iso: new Date(time).toISOString() };
}

/** The source clock of a page: `v3 edited 3 h ago by Konni Hartmann`; unknown parts are left out. */
export function sourceEditPhrase({ version, editedAt, by }: { version: string | null; editedAt: string | null | undefined; by: string | null }, now: number): string | null {
  const edited = relativeTime(editedAt, now);
  const parts = [version, edited ? `edited ${edited}` : null, by ? `by ${by}` : null].filter((part): part is string => part !== null);
  return parts.length > 0 ? parts.join(" ") : null;
}

export function partialText(item: Pick<LibraryItemSummary, "partial">): string | null {
  const partial = item.partial;
  if (!partial) return null;
  return `${partial.have} of ${partial.total ?? "?"} ${partial.unit} (${partial.reason})`;
}

export function itemFailureReason(item: Pick<LibraryItemSummary, "diagnostics">): string | null {
  return item.diagnostics.find((diagnostic) => diagnostic.code !== "source_markup_unconverted")?.message ?? null;
}

/** `1 attachment · downloaded`, `3 attachments · 1 downloaded`, `2 attachments`, `2 attachments · all downloaded`. */
export function attachmentSummary(item: Pick<LibraryItemSummary, "attachments">): string {
  const total = item.attachments.length;
  const downloaded = item.attachments.filter((attachment) => attachment.state === "downloaded").length;
  const noun = `${total} ${total === 1 ? "attachment" : "attachments"}`;
  if (downloaded === 0) return noun;
  return downloaded === total ? `${noun} · ${total === 1 ? "downloaded" : "all downloaded"}` : `${noun} · ${downloaded} downloaded`;
}

/** The tree's `Attachments` group meta: `not downloaded`, `1 downloaded`, `1 of 3 downloaded`. */
export function attachmentTreeMeta(item: Pick<LibraryItemSummary, "attachments">): string {
  const total = item.attachments.length;
  const downloaded = item.attachments.filter((attachment) => attachment.state === "downloaded").length;
  return downloaded === 0 ? "not downloaded" : downloaded === total ? `${downloaded} downloaded` : `${downloaded} of ${total} downloaded`;
}

/** Freshness phrase shown after the state pill, and the notice text for states that need one. */
export function libraryFreshness(item: LibraryItemSummary, now: number): { phrase: string; notice: string | null } {
  const checkedTime = parseLibraryTime(item.checked_at ?? item.fetched_at);
  const checked = checkedTime === null ? null : formatAgo(checkedTime, now);
  // The Cockpit clock is never printed bare: without a known time the segment is omitted.
  switch (item.state) {
    case "fresh": return { phrase: checked ? `Checked ${checked}` : "", notice: null };
    case "changed": return { phrase: `Updated on last refresh${checked ? `, ${checked}` : ""}`, notice: null };
    case "unknown": return { phrase: "Not checked", notice: "The source has not been checked yet." };
    case "removed_at_source": {
      const at = parseLibraryTime(item.checked_at);
      return { phrase: checked ? `Checked ${checked}` : "", notice: `Not found at source${at === null ? "" : ` on ${formatDate(at, now)}`}. The Library copy is kept.` };
    }
    case "conflict": return { phrase: "Edited outside Cockpit", notice: "This Library file was changed outside Cockpit. Refresh keeps it and skips updates." };
    case "failed": return { phrase: checked ? `Last checked ${checked}` : "Not refreshed", notice: `${itemFailureReason(item) ?? "The source could not be refreshed."} The previous content is kept.` };
    case "partial": return { phrase: partialText(item) ?? "Partial", notice: partialText(item) };
    default: {
      const unreachable: never = item.state;
      return unreachable;
    }
  }
}

/** One-line accessible description, e.g. `!482 Fix race, GitLab MR, updated on last refresh`. */
export function itemAccessibleName(item: LibraryItemSummary, providers: readonly ProjectProvider[]): string {
  const state = item.state === "changed" ? "updated on last refresh" : libraryStateChip(item.state).word.toLowerCase();
  return `${itemTreeLabel(item, providers)}, ${itemKindLabel(item, providers)}, ${state}`;
}

/** The item is held by at least one follow. */
export function hasFollowRef(item: Pick<LibraryItemSummary, "refs">): boolean {
  return item.refs.some((ref) => ref.kind === "follow");
}

/** `Keep in Library` applies until the item holds its own `manual` reference. */
export function isKeepable(item: Pick<LibraryItemSummary, "refs" | "kind">): boolean {
  return item.kind !== "folder_copy" && !item.refs.some((ref) => ref.kind === "manual");
}

/** Tooltip for an item nothing references any more: when the Library deletes it, and how to prevent that. */
export function purgeNotice(item: Pick<LibraryItemSummary, "purge_after">): string {
  const time = parseLibraryTime(item.purge_after);
  return `No follow or Space holds this item. It is deleted from the Library ${time === null ? "soon" : `on ${formatDateTime(time)}`} unless you keep it, or it has edits.`;
}

export type LibraryContainerNode = {
  kind: "container";
  key: string;
  label: string;
  instance: string | null;
  containerId: string | null;
  items: LibraryItemSummary[];
  /** The followed space this container shows (`Following`), or null for pages added one by one. */
  follow: LibraryFollowSummary | null;
};

export type LibraryInstanceNode = {
  kind: "instance";
  key: string;
  label: string;
  instance: string | null;
  /** The provider configuration behind this instance, for its monogram tile; null for `Folders`. */
  providerId: string | null;
  unavailable: boolean;
  containers: LibraryContainerNode[];
};

const UNAVAILABLE_CODES: Record<string, true> = { source_cli_unavailable: true, source_auth_failed: true };
const FOLDERS_KEY = "folders";

function itemContainer(item: LibraryItemSummary): { id: string | null; label: string } {
  if (item.container) return { id: item.container.container_id, label: item.container.label };
  const id = item.canonical_id;
  if (id && /[#!]\d+$/.test(id)) return { id: null, label: id.replace(/[#!]\d+$/, "") };
  if (id && JIRA_KEY.test(id)) return { id: null, label: id.replace(/-\d+$/, "") };
  return { id: null, label: "Other" };
}

function compareItems(left: LibraryItemSummary, right: LibraryItemSummary): number {
  // Confluence pages follow the provider's page-tree order, never their numeric ids.
  const leftNumber = isConfluencePage(left) ? null : trailingNumber(left.canonical_id);
  const rightNumber = isConfluencePage(right) ? null : trailingNumber(right.canonical_id);
  if (leftNumber !== null && rightNumber !== null && leftNumber !== rightNumber) return rightNumber - leftNumber;
  if (left.order !== null && right.order !== null && left.order !== right.order) return left.order - right.order;
  return left.title.localeCompare(right.title) || left.item_id.localeCompare(right.item_id);
}

export type LibraryItemNode = { item: LibraryItemSummary; children: LibraryItemNode[] };

/**
 * Items nested under the item their `parent_item_id` names (a Jira subtask under its parent issue). Top-level
 * nodes keep the order given; a Jira issue's subtasks read oldest key first (`SCRUM-4` before `SCRUM-5`),
 * unlike the newest-first list around them. Other providers retain list order. Only a parent in the same list nests a child: a child
 * whose parent is elsewhere, or not in the Library, stays a top-level node. A parent chain that loops back to
 * the child (never produced by a provider) leaves that child top-level rather than dropping the cycle.
 */
export function nestUnderParents(items: readonly LibraryItemSummary[], providers: readonly ProjectProvider[] = []): LibraryItemNode[] {
  const nodes = new Map<string, LibraryItemNode>(items.map((item) => [item.item_id, { item, children: [] }]));
  const parentOf = (item: LibraryItemSummary) => item.parent_item_id === null || item.parent_item_id === item.item_id ? undefined : nodes.get(item.parent_item_id);
  const loops = (item: LibraryItemSummary) => {
    const seen = new Set<string>();
    for (let parent = parentOf(item); parent && !seen.has(parent.item.item_id); parent = parentOf(parent.item)) {
      if (parent.item.item_id === item.item_id) return true;
      seen.add(parent.item.item_id);
    }
    return false;
  };
  const roots: LibraryItemNode[] = [];
  for (const item of items) {
    const node = nodes.get(item.item_id)!;
    const parent = parentOf(item);
    if (parent && !loops(item)) parent.children.push(node); else roots.push(node);
  }
  const oldestFirst = (left: LibraryItemNode, right: LibraryItemNode) => {
    const leftNumber = trailingNumber(left.item.canonical_id);
    const rightNumber = trailingNumber(right.item.canonical_id);
    return leftNumber !== null && rightNumber !== null && leftNumber !== rightNumber ? leftNumber - rightNumber : 0;
  };
  for (const node of nodes.values()) {
    const parent = node.item;
    if (parent.kind === "provider_snapshot" && parent.resource_type === "issue" && providerFamily(providers, parent.provider_id).key === "jira"
      && node.children.every(({ item }) => item.kind === "provider_snapshot" && item.resource_type === "issue"
        && item.provider_id === parent.provider_id && item.provider_instance === parent.provider_instance)) node.children.sort(oldestFirst);
  }
  return roots;
}

/**
 * Library tree ordering (design §4.3): provider instances alphabetically, then
 * `Folders`; containers alphabetically; forge and Jira items by id, newest
 * first; Confluence pages in page-tree order. A followed space is its space's
 * container, and it is listed even while it holds no pages.
 */
export function libraryTree(items: readonly LibraryItemSummary[], providers: readonly ProjectProvider[], follows: readonly LibraryFollowSummary[] = []): LibraryInstanceNode[] {
  const instances = new Map<string, LibraryInstanceNode>();
  const instanceFor = (providerId: string | null, providerInstance: string | null, folder: boolean): LibraryInstanceNode => {
    const instanceKey = folder ? FOLDERS_KEY : `instance:${providerId ?? ""}\u0000${providerInstance ?? ""}`;
    let instance = instances.get(instanceKey);
    if (!instance) {
      const family = providerFamily(providers, providerId);
      instance = {
        kind: "instance",
        key: instanceKey,
        label: folder ? "Folders" : `${family.name} · ${family.key === "confluence" ? confluenceSite(providerInstance) : instanceHost(providerInstance)}`,
        instance: folder ? null : providerInstance,
        providerId: folder ? null : providerId,
        unavailable: false,
        containers: [],
      };
      instances.set(instanceKey, instance);
    }
    return instance;
  };
  const containerFor = (instance: LibraryInstanceNode, container: { id: string | null; label: string }): LibraryContainerNode => {
    const containerKey = `${instance.key}\u0000${container.id ?? container.label}`;
    let node = instance.containers.find((candidate) => candidate.key === containerKey);
    if (!node) {
      node = { kind: "container", key: containerKey, label: container.label, instance: instance.instance, containerId: container.id, items: [], follow: null };
      instance.containers.push(node);
    }
    return node;
  };
  const jiraFollows = follows.filter((follow) => follow.source.kind === "jira_query");
  for (const item of items) {
    const folder = item.kind === "folder_copy";
    const instance = instanceFor(item.provider_id, item.provider_instance, folder);
    // A Jira issue sits under the first query of its own provider instance that follows it, else under its project.
    // A page or issue another provider pulled in through a followed reference keeps its own provider's place: two
    // providers can share a host (Jira and Confluence on one Atlassian site), so only the provider id and instance match.
    const followed = folder ? undefined : jiraFollows.find((follow) => follow.provider_id === item.provider_id && follow.provider_instance === item.provider_instance
      && item.refs.some((ref) => ref.kind === "follow" && ref.follow_id === follow.follow_id));
    containerFor(instance, folder ? { id: null, label: "" } : followed ? { id: followed.follow_id, label: followTitle(followed) } : itemContainer(item)).items.push(item);
  }
  for (const follow of follows) {
    const instance = instanceFor(follow.provider_id, follow.provider_instance, false);
    containerFor(instance, { id: follow.source.kind === "jira_query" ? follow.follow_id : follow.source.space_key, label: followTitle(follow) }).follow = follow;
  }
  for (const instance of instances.values()) {
    const all = instance.containers.flatMap((container) => container.items);
    instance.unavailable = instance.key !== FOLDERS_KEY && all.length > 0
      && all.every((item) => item.state === "failed" && item.diagnostics.some((diagnostic) => UNAVAILABLE_CODES[diagnostic.code] === true));
    instance.containers.sort((left, right) => left.label.localeCompare(right.label));
    for (const container of instance.containers) container.items.sort(compareItems);
  }
  return [...instances.values()].sort((left, right) => {
    if (left.key === FOLDERS_KEY) return 1;
    if (right.key === FOLDERS_KEY) return -1;
    return left.label.localeCompare(right.label);
  });
}

export const REPORT_OUTCOMES: readonly LibraryReportOutcome[] = ["new", "updated", "unchanged", "removed_at_source", "dropped", "partial", "conflict", "failed"];

export function reportOutcomeLabel(outcome: LibraryReportOutcome): string {
  switch (outcome) {
    case "new": return "new";
    case "updated": return "updated";
    case "unchanged": return "unchanged";
    case "removed_at_source": return "removed at source";
    case "dropped": return "no longer followed";
    case "partial": return "partial";
    case "conflict": return "edited in Library";
    case "failed": return "failed";
    default: {
      const unreachable: never = outcome;
      return unreachable;
    }
  }
}

/** `1 new · 3 updated · 30 unchanged`; zero counts are omitted. */
export function reportSummary(report: LibraryRefreshReport): string {
  const parts = REPORT_OUTCOMES.filter((outcome) => report[outcome] > 0).map((outcome) => `${report[outcome]} ${reportOutcomeLabel(outcome)}`);
  return parts.length ? parts.join(" · ") : "nothing to refresh";
}

/** Recognition line for a resolved link (design §4.5). */
export function resolutionNote(resolution: LibraryResolution, providers: readonly ProjectProvider[]): string {
  if (resolution.kind === "folder") return `Folder${resolution.git_working_tree ? " · Git working tree" : ""}${resolution.file_count !== null ? ` · ${resolution.file_count} files` : ""} · ${resolution.title}`;
  // `KEY · Space name`: the note names the space by its key.
  if (resolution.kind === "confluence_page") return `Confluence page · ${resolution.title}${resolution.container_label ? ` · ${resolution.container_label.split(" · ")[0]}` : ""}`;
  if (resolution.kind === "confluence_space") return `Confluence space · ${resolutionSpaceName(resolution)}${resolution.item_count !== null ? ` · ${pageCount(resolution.item_count)}` : ""}`;
  if (resolution.kind === "jira_query") return `Jira query · ${resolution.title}${resolution.item_count !== null ? ` · ${issueCount(resolution.item_count, resolution.item_count_exact)}` : ""}`;
  const family = providerFamily(providers, resolution.provider_id);
  const host = instanceHost(resolution.provider_instance);
  if (family.key === "jira") return `Jira issue ${resolution.canonical_id ?? ""} · ${resolution.title} · ${host}`;
  const review = resolution.canonical_id?.includes("!") ?? false;
  const id = itemDisplayId({ canonical_id: resolution.canonical_id, resource_type: review ? "review" : "issue", provider_id: resolution.provider_id }, providers);
  const kind = review ? family.review : "Issue";
  return `${kind}${id ? ` ${id}` : ""} · ${resolution.title} · ${family.name} · ${host}`;
}

/** `credentialProviderId` names the provider whose token dialog can fix the failure. */
export type LookupFailure = { title: string; detail: string; retry: boolean; credentialProviderId?: string };

export function errorCode(error: unknown): string | null {
  if (typeof error !== "object" || error === null) return null;
  if ("operationCode" in error && typeof error.operationCode === "string") return error.operationCode;
  return "code" in error && typeof error.code === "string" ? error.code : null;
}

export function errorText(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : typeof error === "object" && error !== null && "message" in error && typeof error.message === "string" && error.message ? error.message : fallback;
}

/**
 * The configured provider a typed link belongs to when the request did not name one. Providers can share a host
 * (Jira and Confluence on one Atlassian site), so the link's path decides: the provider whose base URL contains
 * it, deepest first, else the first on the host.
 */
function providerForInput(providers: readonly ProjectProvider[], input: string, host: string): ProjectProvider | undefined {
  let url: URL | null = null;
  try { url = new URL(input); } catch { /* not a link */ }
  const onHost = providers.filter((candidate) => {
    try { return new URL(candidate.base_url).host === host; } catch { return false; }
  });
  const within = url ? onHost.filter((candidate) => urlWithin(candidate.base_url, url)) : [];
  const basePath = (provider: ProjectProvider) => new URL(provider.base_url).pathname.replace(/\/+$/, "").length;
  return [...within].sort((left, right) => basePath(right) - basePath(left))[0] ?? onHost[0];
}

/**
 * Refusal block under the Add field: what failed and what is safe to do next.
 * `selected` is the provider the request named, when the input itself has no host (a Jira key or Confluence page id).
 */
export function lookupFailure(error: unknown, input: string, providers: readonly ProjectProvider[], selected?: ProjectProvider): LookupFailure {
  const code = errorCode(error);
  const message = errorText(error, "The link could not be read.");
  let host = input;
  try { host = new URL(input).host; } catch {
    if (selected) try { host = new URL(selected.base_url).host; } catch { /* a bare key or path is its own label */ }
  }
  const provider = selected ?? providerForInput(providers, input, host);
  const family = provider ? providerFamily(providers, provider.id) : null;
  const executable = provider?.executable?.split(/[\\/]/).pop() ?? "The provider CLI";
  const confluence = family?.key === "confluence";
  // Only Jira and Confluence take a stored token; the host reports every other provider as unsupported.
  const credentialProviderId = provider && (family?.key === "jira" || confluence) ? provider.id : undefined;
  switch (code) {
    case "unsupported_artifact":
    case "source_provider_unsupported":
      return provider
        ? { title: "✕ Not recognized", detail: `${message}. Enter an issue, merge request or pull request link, or a Jira key.`, retry: false }
        : { title: `✕ No provider configured for ${host}`, detail: "Add this instance to the Cockpit configuration file, then retry.", retry: true };
    case "invalid_artifact_url":
    case "library_input_unrecognized":
      return { title: "✕ Not recognized", detail: "Enter an issue, merge request or pull request link, a Jira key, a Confluence page or space link, page id or space key, or an absolute or ~ folder path.", retry: false };
    case "library_folder_refused":
      return { title: "Can't copy this folder", detail: message, retry: false };
    case "library_folder_unavailable":
      return { title: "Folder unavailable", detail: message, retry: true };
    case "source_cli_unavailable":
      return { title: `✕ ${executable} isn't installed`, detail: `Install ${executable} and sign in with it, then retry.`, retry: true };
    case "source_auth_failed":
      return credentialProviderId
        ? { title: `✕ ${family?.name} sign-in failed`, detail: `${host} rejected the token stored in Cockpit for this site. Replace it in Provider tokens, then retry.`, retry: true, credentialProviderId }
        : { title: `✕ ${family?.name ?? "Provider"} sign-in failed`, detail: `${host} rejected the ${executable} CLI's credentials. Sign in with the CLI, then retry.`, retry: true };
    case "source_auth_required":
      return { title: `✕ ${family?.name ?? "Provider"} sign-in required`, detail: message, retry: true, credentialProviderId };
    case "source_credential_required":
      return { title: "✕ A token is needed", detail: message, retry: true, credentialProviderId };
    case "source_capability_unavailable":
      return { title: "✕ Not available on this Confluence Data Center instance", detail: message, retry: false };
    case "source_not_found":
      return { title: "✕ Not found at source", detail: message, retry: true };
    case "library_unavailable":
      return { title: "✕ Library unavailable", detail: `${message}. Space context is unaffected.`, retry: true };
    default:
      return { title: "✕ Lookup failed", detail: code ? `${code}: ${message}` : message, retry: true };
  }
}
