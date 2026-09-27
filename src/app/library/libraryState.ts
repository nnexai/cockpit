import type {
  LibraryItemState,
  LibraryItemSummary,
  LibraryRefreshReport,
  LibraryReportOutcome,
  LibraryResolution,
  ProjectProvider,
  SpaceTarget,
} from "../../protocol/generated/v1";

/**
 * The Space a Library surface can add to: the selected Space in the Library
 * view, or a Context pane's own Space. `live` is false while Herdr isn't live,
 * when Space-targeted actions are unavailable (design §4.1).
 */
export type LibrarySpace = { target: SpaceTarget; label: string; live: boolean };

export function sameSpaceTarget(left: SpaceTarget | null | undefined, right: SpaceTarget | null | undefined): boolean {
  return Boolean(left && right && left.session_id === right.session_id && left.space_id === right.space_id);
}

/** Provider families Cockpit can snapshot into the Library, keyed by CLI executable. */
export type ProviderFamily = { key: "gitlab" | "github" | "gitea" | "jira" | "confluence" | "other"; name: string; review: string };

const FAMILIES: Record<string, ProviderFamily> = {
  glab: { key: "gitlab", name: "GitLab", review: "MR" },
  gh: { key: "github", name: "GitHub", review: "PR" },
  tea: { key: "gitea", name: "Gitea", review: "PR" },
  jira: { key: "jira", name: "Jira", review: "issue" },
  confluence: { key: "confluence", name: "Confluence", review: "page" },
};

function executableName(executable: string): string {
  return executable.slice(Math.max(executable.lastIndexOf("/"), executable.lastIndexOf("\\")) + 1);
}

export function providerFamily(providers: readonly ProjectProvider[], providerId: string | null): ProviderFamily {
  const provider = providers.find((candidate) => candidate.id === providerId);
  const family = provider ? FAMILIES[executableName(provider.executable)] : undefined;
  return family ?? { key: "other", name: providerId ?? "Source", review: "review" };
}

export function jiraProviders(providers: readonly ProjectProvider[]): ProjectProvider[] {
  return providers.filter((provider) => executableName(provider.executable) === "jira");
}

export function confluenceProviders(providers: readonly ProjectProvider[]): ProjectProvider[] {
  return providers.filter((provider) => executableName(provider.executable) === "confluence");
}

/** A Library item or lookup result that is a Confluence page. */
export function isConfluencePage(item: Pick<LibraryItemSummary, "kind" | "resource_type">): boolean {
  return item.kind === "provider_snapshot" && item.resource_type === "page";
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
export type StateChip = { glyph: string; word: string; tone: StateTone };

/** Library item states (design §4.6). Only `fresh` reads `Up to date`. */
export function libraryStateChip(state: LibraryItemState): StateChip {
  switch (state) {
    case "fresh": return { glyph: "✓", word: "Up to date", tone: "idle" };
    case "changed": return { glyph: "↑", word: "Updated", tone: "working" };
    case "unknown": return { glyph: "?", word: "Not checked", tone: "muted" };
    case "removed_at_source": return { glyph: "⊘", word: "Removed at source", tone: "muted" };
    case "conflict": return { glyph: "✎", word: "Edited in Library", tone: "working" };
    case "failed": return { glyph: "✕", word: "Refresh failed", tone: "blocked" };
    case "partial": return { glyph: "◐", word: "Partial", tone: "working" };
    default: {
      const unreachable: never = state;
      return unreachable;
    }
  }
}

export function relativeTime(iso: string | null, now: number): string | null {
  if (!iso) return null;
  const time = Date.parse(iso);
  if (!Number.isFinite(time)) return null;
  const seconds = Math.max(0, Math.round((now - time) / 1000));
  if (seconds < 60) return "just now";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  const days = Math.round(hours / 24);
  if (days < 14) return `${days} d ago`;
  return `on ${shortDate(iso)}`;
}

function shortDate(iso: string): string {
  const date = new Date(iso);
  return Number.isFinite(date.getTime()) ? date.toLocaleDateString(undefined, { day: "numeric", month: "short" }) : iso;
}

export function partialText(item: Pick<LibraryItemSummary, "partial">): string | null {
  const partial = item.partial;
  if (!partial) return null;
  return `${partial.have} of ${partial.total ?? "?"} ${partial.unit} (${partial.reason})`;
}

export function itemFailureReason(item: Pick<LibraryItemSummary, "diagnostics">): string | null {
  return item.diagnostics.find((diagnostic) => diagnostic.code !== "source_markup_unconverted")?.message ?? null;
}

/** Freshness phrase shown after the state chip, and the notice text for states that need one. */
export function libraryFreshness(item: LibraryItemSummary, now: number): { phrase: string; notice: string | null } {
  const checked = relativeTime(item.checked_at ?? item.fetched_at, now);
  switch (item.state) {
    case "fresh": return { phrase: checked ? `checked ${checked}` : "checked", notice: null };
    case "changed": return { phrase: `updated on last refresh${checked ? `, ${checked}` : ""}`, notice: null };
    case "unknown": return { phrase: "not checked", notice: "The source has not been checked yet." };
    case "removed_at_source": return { phrase: checked ? `checked ${checked}` : "checked", notice: `Not found at source${item.checked_at ? ` on ${shortDate(item.checked_at)}` : ""}. The Library copy is kept.` };
    case "conflict": return { phrase: "edited outside Cockpit", notice: "This Library file was changed outside Cockpit. Refresh keeps it and skips updates." };
    case "failed": return { phrase: checked ? `last checked ${checked}` : "not refreshed", notice: `${itemFailureReason(item) ?? "The source could not be refreshed."} The previous content is kept.` };
    case "partial": return { phrase: partialText(item) ?? "partial", notice: partialText(item) };
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

export type LibraryContainerNode = {
  kind: "container";
  key: string;
  label: string;
  instance: string | null;
  containerId: string | null;
  items: LibraryItemSummary[];
};

export type LibraryInstanceNode = {
  kind: "instance";
  key: string;
  label: string;
  instance: string | null;
  unavailable: boolean;
  containers: LibraryContainerNode[];
};

const UNAVAILABLE_CODES: Record<string, true> = { source_cli_unavailable: true, source_auth_failed: true };
const FOLDERS_KEY = "folders";

function containerFor(item: LibraryItemSummary): { id: string | null; label: string } {
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

/**
 * Library tree ordering (design §4.3): provider instances alphabetically, then
 * `Folders`; containers alphabetically; forge and Jira items by id, newest
 * first; Confluence pages in page-tree order.
 */
export function libraryTree(items: readonly LibraryItemSummary[], providers: readonly ProjectProvider[]): LibraryInstanceNode[] {
  const instances = new Map<string, LibraryInstanceNode>();
  for (const item of items) {
    const folder = item.kind === "folder_copy";
    const instanceKey = folder ? FOLDERS_KEY : `instance:${item.provider_id ?? ""}\u0000${item.provider_instance ?? ""}`;
    let instance = instances.get(instanceKey);
    if (!instance) {
      const family = providerFamily(providers, item.provider_id);
      instance = {
        kind: "instance",
        key: instanceKey,
        label: folder ? "Folders" : `${family.name} · ${family.key === "confluence" ? confluenceSite(item.provider_instance) : instanceHost(item.provider_instance)}`,
        instance: folder ? null : item.provider_instance,
        unavailable: false,
        containers: [],
      };
      instances.set(instanceKey, instance);
    }
    const container = folder ? { id: null, label: "" } : containerFor(item);
    const containerKey = `${instanceKey}\u0000${container.id ?? container.label}`;
    let node = instance.containers.find((candidate) => candidate.key === containerKey);
    if (!node) {
      node = { kind: "container", key: containerKey, label: container.label, instance: instance.instance, containerId: container.id, items: [] };
      instance.containers.push(node);
    }
    node.items.push(item);
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

export const REPORT_OUTCOMES: readonly LibraryReportOutcome[] = ["new", "updated", "unchanged", "removed_at_source", "partial", "conflict", "failed"];

export function reportOutcomeLabel(outcome: LibraryReportOutcome): string {
  switch (outcome) {
    case "new": return "new";
    case "updated": return "updated";
    case "unchanged": return "unchanged";
    case "removed_at_source": return "removed at source";
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
  const family = providerFamily(providers, resolution.provider_id);
  const host = instanceHost(resolution.provider_instance);
  if (family.key === "jira") return `Jira issue ${resolution.canonical_id ?? ""} · ${resolution.title} · ${host}`;
  const review = resolution.canonical_id?.includes("!") ?? false;
  const id = itemDisplayId({ canonical_id: resolution.canonical_id, resource_type: review ? "review" : "issue", provider_id: resolution.provider_id }, providers);
  const kind = review ? family.review : "Issue";
  return `${kind}${id ? ` ${id}` : ""} · ${resolution.title} · ${family.name} · ${host}`;
}

export type LookupFailure = { title: string; detail: string; retry: boolean };

export function errorCode(error: unknown): string | null {
  if (typeof error !== "object" || error === null) return null;
  if ("operationCode" in error && typeof error.operationCode === "string") return error.operationCode;
  return "code" in error && typeof error.code === "string" ? error.code : null;
}

export function errorText(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : typeof error === "object" && error !== null && "message" in error && typeof error.message === "string" && error.message ? error.message : fallback;
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
  const provider = selected ?? providers.find((candidate) => {
    try { return new URL(candidate.base_url).host === host; } catch { return false; }
  });
  const family = provider ? providerFamily(providers, provider.id) : null;
  const executable = provider ? executableName(provider.executable) : "The provider CLI";
  const confluence = family?.key === "confluence";
  switch (code) {
    case "unsupported_artifact":
    case "source_provider_unsupported":
      return provider
        ? { title: "✕ Not recognized", detail: `${message}. Enter an issue, merge request or pull request link, or a Jira key.`, retry: false }
        : { title: `✕ No provider configured for ${host}`, detail: "Add this instance to the Cockpit configuration file, then retry.", retry: true };
    case "invalid_artifact_url":
    case "library_input_unrecognized":
      return { title: "✕ Not recognized", detail: "Enter an issue, merge request or pull request link, a Jira key, a Confluence page link or id, or an absolute or ~ folder path.", retry: false };
    case "library_folder_refused":
      return { title: "Can't copy this folder", detail: message, retry: false };
    case "library_folder_unavailable":
      return { title: "Folder unavailable", detail: message, retry: true };
    case "source_cli_unavailable":
      return confluence
        ? { title: `✕ ${executable} isn't installed`, detail: "Install it with brew install pchuri/tap/confluence-cli, configure a read-only profile, then retry.", retry: true }
        : { title: `✕ ${executable} isn't installed`, detail: `Install ${executable} and sign in with it, then retry.`, retry: true };
    case "source_auth_failed":
      return { title: `✕ ${family?.name ?? "Provider"} sign-in failed`, detail: `${host} rejected the ${executable} CLI's credentials. Cockpit doesn't store credentials: sign in with the CLI${confluence ? "'s read-only profile" : ""}, then retry.`, retry: true };
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
