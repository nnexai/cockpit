import { useEffect, useId, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { LibraryOperation, LibraryResolution, ProjectProvider } from "../../protocol/generated/v1";
import { createPortal } from "react-dom";
import { UiIcon } from "../UiIcon";
import { trapDialogKeys, useRestoreFocus } from "./LibraryConfirmDialog";
import { JIRA_KEY, confluencePageInput, confluenceProviders, confluenceSite, confluenceSpaceInput, errorText, jiraProviders, libraryInputUrl, lookupFailure, pageCount, providerFamily, resolutionNote, resolutionSpaceName, type LibrarySpace, type LookupFailure } from "./libraryState";
import { headerSpaceAction } from "./spaceCopyPresentation";
import { useLibraryOperation, useSpaceContextListing } from "./useLibraryOperation";
import "../projects/setup.css";
import "../projects/taskSetup.css";
import "./library.css";

const LOOKUP_DELAY_MS = 400;

type Lookup =
  | { status: "idle" }
  | { status: "pending" }
  | { status: "ok"; resolution: LibraryResolution }
  | { status: "error"; failure: LookupFailure };

type Tone = "running" | "done" | "failed";

/**
 * Phase 1 (design §4.7): the Library step. `saved` follows the items the
 * operation actually saved, so a cancel or failure after some were published
 * still offers them; `complete` is false when part of the source wasn't added.
 * A Space-only retry starts from saved items.
 */
function libraryPhase(operation: LibraryOperation, unit: "items" | "pages"): { text: string; tone: Tone; saved: boolean; complete: boolean } {
  if (operation.kind === "space_add") return { text: "✓ Saved to Library", tone: "done", saved: true, complete: true };
  const phase = operation.phases.find((candidate) => candidate.phase === "library");
  const partialReason = operation.report?.rows.find((row) => row.outcome === "partial")?.reason;
  const unchanged = operation.report?.rows.find((row) => row.outcome === "unchanged");
  const count = operation.item_ids.length;
  // A followed space counts its pages (`✓ Saved to Library · 38 pages`).
  const summary = unit === "pages" ? ` · ${pageCount(count)}` : count > 1 ? ` · ${count} items` : "";
  switch (phase?.state ?? "pending") {
    case "pending":
    case "running":
      if (operation.cancel_requested) return { text: "Cancelling…", tone: "running", saved: false, complete: false };
      return { text: `Saving to Library…${phase?.total && phase.total > 1 ? ` ${phase.done} of ${phase.total}` : ""}`, tone: "running", saved: false, complete: false };
    case "done":
      if (operation.cancel_requested) return { text: "Already saved to Library.", tone: "done", saved: true, complete: true };
      if (count === 0 && unchanged) return { text: `✓ ${unchanged.reason ?? "Already saved in Library"}`, tone: "done", saved: true, complete: true };
      return { text: `✓ Saved to Library${summary}`, tone: "done", saved: true, complete: true };
    case "partial":
      return { text: `◐ Saved to Library, partial${partialReason ? `: ${partialReason}` : ""}${summary}`, tone: "done", saved: true, complete: true };
    case "failed": {
      const reason = (phase?.error?.message ?? phase?.message ?? "The source could not be saved").replace(/\.$/, "");
      if (count > 0) return { text: `◐ Saved to Library${summary}, then stopped: ${reason}. Anything not yet saved wasn't added.`, tone: "failed", saved: true, complete: false };
      return { text: `✕ Not saved. ${reason}. Nothing was added.`, tone: "failed", saved: false, complete: false };
    }
    case "cancelled":
      if (count > 0) return { text: `◐ Saved to Library${summary}, then cancelled. Anything not yet saved wasn't added.`, tone: "done", saved: true, complete: false };
      return { text: "Cancelled. Nothing was added.", tone: "failed", saved: false, complete: false };
  }
}

/**
 * Phase 2 (design §4.7): the copy into the Space, shown once the Library copy is
 * saved. A failure never implies the Library copy was lost, and the files the
 * operation recorded as written tell a partial copy from one that wrote nothing.
 */
function spacePhase(operation: LibraryOperation, space: string): { text: string; tone: Tone } | null {
  const phase = operation.phases.find((candidate) => candidate.phase === "space");
  if (!phase) return null;
  const written = operation.space?.written.length ?? 0;
  const files = `${written} ${written === 1 ? "file was" : "files were"} copied`;
  switch (phase.state) {
    case "pending":
      return operation.finished ? { text: `Not added to ${space}. The Library copy is saved.`, tone: "failed" } : null;
    case "running":
      return { text: `Adding to ${space}…${phase.total && phase.total > 1 ? ` ${phase.done} of ${phase.total}` : ""}`, tone: "running" };
    case "done":
    case "partial": {
      const result = operation.space;
      if (!result || written === 0) return { text: `✓ Already in ${space}. Nothing was written.`, tone: "done" };
      const how = result.copy_mode === "reflink" ? "reflinked" : result.copy_mode === "mixed" ? "reflinked and copied" : "copied (reflink not supported here)";
      return { text: `✓ Added to ${space} · ${how}`, tone: "done" };
    }
    case "failed": {
      const unverified = phase.error?.code === "source_companion_unavailable";
      const reason = unverified ? "Its context folder couldn't be verified" : (phase.error?.message ?? "The copy stopped").replace(/\.$/, "");
      if (written > 0) return { text: `✕ Only partly added to ${space}: ${files} before it stopped. ${reason}. The Library copy is saved; retrying copies the rest.`, tone: "failed" };
      return unverified
        ? { text: `✕ Not added to ${space}. ${reason}. The Library copy is saved; nothing was written to ${space}.`, tone: "failed" }
        : { text: `✕ Not added to ${space}. ${reason}. The Library copy is saved.`, tone: "failed" };
    }
    case "cancelled":
      return { text: written > 0 ? `Only partly added to ${space}: ${files}. The Library copy is saved.` : `Not added to ${space}. The Library copy is saved.`, tone: "failed" };
  }
}

/** What a Space-only copy asked for: saved items, followed spaces, or both. */
type SpaceSelection = { item_ids: string[]; follow_ids: string[] };

/** A followed space's source, so a Space retry can look up the follow the add created. */
type FollowSource = { input: string; provider_id: string | null };

/**
 * The dialog's accepted request outlives the dialog (design §4.7: closing does
 * not cancel). Reopening shows a request still starting, a running add, an
 * outcome that finished while closed, or a failure with its retry, until
 * `Add another` or a new add.
 */
type AcceptedAdd = {
  begin: () => Promise<LibraryOperation>;
  /**
   * What a Space-only copy asked for (no Library step), or null for an add. A
   * retry resends these: an interrupted operation may record fewer.
   */
  spaceSelection: SpaceSelection | null;
  /** Set when the request follows a space; its operation lists the space's pages, not the follow. */
  followSource: FollowSource | null;
  space: LibrarySpace | null;
  /** Settles after `operation` or `error` is recorded. */
  pending: Promise<LibraryOperation>;
  operation: LibraryOperation | null;
  error: string | null;
  seen: boolean;
};
let accepted: AcceptedAdd | null = null;

type SpaceList =
  | { status: "loading" }
  | { status: "ready"; spaces: LibraryResolution[] }
  | { status: "error"; failure: LookupFailure };

/**
 * One configured Confluence provider's spaces in `Browse Confluence spaces`
 * (design UQ5a): a read-only list of what its profile can read. Choosing a
 * space fills Add with it; nothing is saved until the dialog's primary action.
 * A sign-in or install failure stays with its provider and retries on its own.
 */
function ConfluenceSpaceList({ client, provider, providers, onPick }: {
  client: CockpitClient;
  provider: ProjectProvider;
  providers: readonly ProjectProvider[];
  onPick: (provider: ProjectProvider, space: LibraryResolution) => void;
}) {
  const [list, setList] = useState<SpaceList>({ status: "loading" });
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    let current = true;
    setList({ status: "loading" });
    client.libraryConfluenceSpaces({ provider_id: provider.id }).then((spaces) => {
      if (current) setList({ status: "ready", spaces });
    }, (cause: unknown) => {
      if (current) setList({ status: "error", failure: lookupFailure(cause, provider.base_url, providers, provider) });
    });
    return () => { current = false; };
    // `providers` only improves failure wording.
  }, [client, provider, retry]);
  const site = `Confluence · ${confluenceSite(provider.base_url)}`;
  return <section className="library-browse-provider" aria-label={site}>
    <h3>{site}</h3>
    {list.status === "loading" ? <p className="task-setup-note" role="status">Loading spaces…</p> : null}
    {list.status === "error" ? <div className="library-refusal" role="alert">
      <strong>{list.failure.title}</strong>
      <span>{list.failure.detail}</span>
      {list.failure.retry ? <button type="button" className="task-setup-link" onClick={() => setRetry((value) => value + 1)}>Retry</button> : null}
    </div> : null}
    {list.status === "ready" && list.spaces.length === 0 ? <p className="task-setup-note">No spaces this profile can read.</p> : null}
    {list.status === "ready" && list.spaces.length > 0 ? <ul className="library-browse-list">
      {list.spaces.map((space) => {
        const name = resolutionSpaceName(space);
        // An already followed space can still be chosen, to add it to the target Space.
        const verb = space.existing_follow_id ? "Select" : "Follow";
        return <li key={space.canonical_id ?? name}>
          <span className="library-browse-name" title={name}>{name}</span>
          {space.page_count !== null ? <span className="library-browse-detail">{pageCount(space.page_count)}</span> : null}
          {space.existing_follow_id ? <span className="library-state is-idle"><span aria-hidden="true">◉</span> Following</span> : null}
          <button type="button" aria-label={`${verb} ${name}`} onClick={() => onPick(provider, space)}>{verb}</button>
        </li>;
      })}
    </ul> : null}
  </section>;
}

/**
 * Add context (design §4.5): one field for a forge issue, MR or PR link, a
 * Jira key, a Confluence page link or id, a Confluence space link or key, or a
 * local folder path, plus `Browse Confluence spaces`. A page can be added alone
 * or its whole space followed. It always saves to the Library first; with a
 * live target Space the destination can also copy the saved item or followed
 * space there.
 */
export function AddContextDialog({ client, onClose, onOpenItem, space = null, defaultDestination = "library", openInSpace = null }: {
  client: CockpitClient;
  onClose: () => void;
  onOpenItem?: (itemId: string) => void;
  /** The Space an add can also copy into; absent with no session or Space. */
  space?: LibrarySpace | null;
  /** From the entry point (design §1): `Resources → Add…` presets the Space. */
  defaultDestination?: "library" | "space";
  /** The opening Context pane's companion root; `Open in <Space>` selects a written file there. */
  openInSpace?: { companionRootId: string; open: (path: string) => void } | null;
}) {
  const titleId = useId();
  const fieldId = useId();
  const failureId = useId();
  const destinationId = useId();
  const followChoiceId = useId();
  const browseId = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const closeActionRef = useRef<HTMLButtonElement>(null);
  const primaryActionRef = useRef<HTMLButtonElement>(null);
  const submitRef = useRef<HTMLButtonElement>(null);
  const restoredRef = useRef(accepted);
  const lastBeginRef = useRef<(() => Promise<LibraryOperation>) | null>(restoredRef.current?.begin ?? null);
  const [operationSpace, setOperationSpace] = useState<LibrarySpace | null>(restoredRef.current?.space ?? null);
  const [spaceSelection, setSpaceSelection] = useState<SpaceSelection | null>(restoredRef.current?.spaceSelection ?? null);
  const [followSource, setFollowSource] = useState<FollowSource | null>(restoredRef.current?.followSource ?? null);
  const [restoredError, setRestoredError] = useState<string | null>(restoredRef.current && !restoredRef.current.operation ? restoredRef.current.error : null);
  // Closed before the request was accepted: follow it until it settles.
  const [awaitingStart, setAwaitingStart] = useState(Boolean(restoredRef.current && !restoredRef.current.operation && !restoredRef.current.error));
  const [providers, setProviders] = useState<ProjectProvider[]>([]);
  const [spacePageLimit, setSpacePageLimit] = useState<number | null>(null);
  const [providersLoaded, setProvidersLoaded] = useState(false);
  const [providersError, setProvidersError] = useState<string | null>(null);
  const [input, setInput] = useState("");
  const [label, setLabel] = useState<string | null>(null);
  // The provider picked where several configured instances could read the input.
  const [chosenProviderId, setChosenProviderId] = useState<string | null>(null);
  const [linked, setLinked] = useState(false);
  const [refreshExisting, setRefreshExisting] = useState(false);
  // A Confluence page: `Only this page` (false, the default) or `Follow the whole space`.
  const [followChoice, setFollowChoice] = useState(false);
  const [browseOpen, setBrowseOpen] = useState(false);
  // A space chosen from the browser is already resolved; the lookup reuses it instead of asking again.
  const [picked, setPicked] = useState<{ input: string; providerId: string; resolution: LibraryResolution } | null>(null);
  const [pickedFocus, setPickedFocus] = useState(0);
  const [lookup, setLookup] = useState<Lookup>({ status: "idle" });
  const [lookupRetry, setLookupRetry] = useState(0);
  const spaceChoice = space?.live ? space : null;
  const [destination, setDestination] = useState<"library" | "space">(spaceChoice && defaultDestination === "space" ? "space" : "library");
  const spaceListing = useSpaceContextListing(client, spaceChoice?.target ?? null, spaceChoice !== null);
  const add = useLibraryOperation(client);
  const resume = add.resume;
  useRestoreFocus();
  useEffect(() => {
    const restored = restoredRef.current;
    if (restored?.operation) { resume(restored.operation); return; }
    if (!restored || restored.error) { if (!restored) inputRef.current?.focus(); return; }
    let current = true;
    // A newer request replaces this one; its late result must not overwrite the dialog.
    restored.pending.then((next) => {
      if (!current || accepted !== restored) return;
      setAwaitingStart(false);
      resume(next);
    }, () => {
      if (!current || accepted !== restored) return;
      setAwaitingStart(false);
      setRestoredError(restored.error);
    });
    return () => { current = false; };
  }, [resume]);
  useEffect(() => {
    let current = true;
    client.projectConfiguration().then((configuration) => {
      if (current) { setProviders(configuration.providers); setSpacePageLimit(configuration.limits?.library_space_pages ?? null); setProvidersLoaded(true); }
    }, (cause: unknown) => {
      if (current) { setProvidersError(errorText(cause, "Provider configuration could not be read.")); setProvidersLoaded(true); }
    });
    return () => { current = false; };
  }, [client]);
  const trimmed = input.trim();
  const jira = jiraProviders(providers);
  const confluence = confluenceProviders(providers);
  const jiraKey = JIRA_KEY.test(trimmed);
  const confluencePage = jiraKey ? null : confluencePageInput(trimmed, providers);
  const confluenceSpace = jiraKey || confluencePage ? null : confluenceSpaceInput(trimmed, providers);
  // A Jira key or Confluence page or space names its provider here; every other input by its own link.
  const choices = jiraKey ? jira : confluencePage?.providers ?? confluenceSpace?.providers ?? [];
  const chosen = choices.find((provider) => provider.id === chosenProviderId) ?? choices[0];
  const needsProvider = jiraKey || confluencePage !== null || confluenceSpace !== null;
  const requestUrl = libraryInputUrl(trimmed, jiraKey ? chosen : undefined);
  const requestProviderId = chosen?.id ?? null;
  const lookupKey = `${requestUrl}\u0000${requestProviderId ?? ""}\u0000${lookupRetry}\u0000${needsProvider && !providersLoaded}`;
  useEffect(() => {
    if (!trimmed) { setLookup({ status: "idle" }); return; }
    if (needsProvider && !providersLoaded) { setLookup({ status: "pending" }); return; }
    if (picked && picked.input === requestUrl && picked.providerId === requestProviderId && lookupRetry === 0) { setLookup({ status: "ok", resolution: picked.resolution }); return; }
    if (jiraKey && jira.length === 0) {
      setLookup({ status: "error", failure: { title: "✕ No Jira provider configured", detail: "Add a Jira instance to the Cockpit configuration file, or paste the issue's link.", retry: false } });
      return;
    }
    const unconfigured = confluencePage ?? confluenceSpace;
    if (unconfigured && unconfigured.providers.length === 0) {
      setLookup({ status: "error", failure: unconfigured.host
        ? { title: `✕ No provider configured for ${unconfigured.host}`, detail: "Add this Confluence instance to the Cockpit configuration file, then retry.", retry: false }
        : { title: "✕ No Confluence provider configured", detail: "Add a Confluence instance to the Cockpit configuration file, or paste the page's link.", retry: false } });
      return;
    }
    let current = true;
    setLookup({ status: "pending" });
    const timer = window.setTimeout(() => {
      client.libraryResolve({ input: requestUrl, provider_id: requestProviderId }).then((resolution) => {
        if (current) setLookup({ status: "ok", resolution });
      }, (cause: unknown) => {
        if (current) setLookup({ status: "error", failure: lookupFailure(cause, requestUrl, providers, chosen) });
      });
    }, LOOKUP_DELAY_MS);
    return () => { current = false; window.clearTimeout(timer); };
    // `providers` only improves failure wording and is loaded before a provider-named lookup starts.
  }, [client, lookupKey]);
  const resolution = lookup.status === "ok" ? lookup.resolution : null;
  const spaceResolution = resolution?.kind === "confluence_space";
  // A page can follow its whole space unless that space is already followed.
  const followOffered = resolution?.kind === "confluence_page" && !resolution.existing_follow_id;
  const follow = spaceResolution || (followOffered && followChoice);
  const spaceName = resolution && (spaceResolution || followOffered) ? resolutionSpaceName(resolution) : null;
  const existingFollow = follow ? resolution?.existing_follow_id ?? null : null;
  const existingItem = follow ? null : resolution?.existing_item_id ?? null;
  const existing = existingItem ?? existingFollow;
  const folder = resolution?.kind === "folder";
  const family = resolution && !folder ? providerFamily(providers, resolution.provider_id) : null;
  const forge = resolution?.kind === "artifact" && family !== null && family.key !== "jira" && family.key !== "confluence";
  const destinationSpace = destination === "space" ? spaceChoice : null;
  const companion = spaceListing.listing?.companion ?? null;
  // An item or followed space already in the target Space: the header's Space state decides whether adding applies.
  const existingRow = spaceListing.listing?.rows.find((row) => existingFollow ? row.follow?.follow_id === existingFollow : row.item_id === existingItem);
  const existingInSpace = existing && destinationSpace && spaceListing.listing ? headerSpaceAction(existingRow, destinationSpace.label) : null;
  const spaceAddable = !existingInSpace || existingInSpace.actions.some((action) => action.kind === "add");
  // A follow over the page limit saves only part of the space (design §4.6 `◐ Partial`).
  const pageTotal = resolution?.page_count ?? null;
  const overLimit = follow && !existingFollow && spacePageLimit !== null && pageTotal !== null && pageTotal > spacePageLimit;
  const operation = add.operation;
  const shownSpace = operationSpace?.label ?? "the Space";
  const unit = followSource ? "pages" : "items";
  const progress = operation ? libraryPhase(operation, unit) : null;
  const spaceStep = operation && progress?.saved ? spacePhase(operation, shownSpace) : null;
  const finished = operation?.finished ?? false;
  const startError = add.error ?? restoredError;
  const canAdd = resolution !== null && !add.starting && (!existing || refreshExisting || (destinationSpace !== null && spaceAddable));
  const primaryLabel = existing
    ? refreshExisting ? (destinationSpace ? `Refresh and add to ${destinationSpace.label}` : "Refresh from source") : destinationSpace ? `Add to ${destinationSpace.label}` : existingFollow ? "Already following" : "Already in Library"
    : follow ? (destinationSpace ? `Follow and add to ${destinationSpace.label}` : "Follow space")
    : destinationSpace ? `Add to Library and ${destinationSpace.label}` : "Add to Library";
  const operationRef = useRef(operation);
  operationRef.current = operation;
  // A displayed, complete success is done with; anything else stays for the next opening.
  useEffect(() => () => {
    const latest = operationRef.current;
    if (!accepted?.seen || !latest?.finished || accepted.operation?.operation_id !== latest.operation_id) return;
    const phase = libraryPhase(latest, accepted.followSource ? "pages" : "items");
    if (phase.saved && phase.complete && spacePhase(latest, "")?.tone !== "failed") accepted = null;
  }, []);
  useEffect(() => {
    if (finished && operation && accepted?.operation?.operation_id === operation.operation_id) accepted.seen = true;
  }, [finished, operation]);
  // A space chosen in the browser moves focus to the action it enables, or back to the field.
  useEffect(() => {
    if (pickedFocus > 0) (submitRef.current && !submitRef.current.disabled ? submitRef.current : inputRef.current)?.focus();
  }, [pickedFocus]);
  const begin = (request: () => Promise<LibraryOperation>, requestSpace: LibrarySpace | null, requestSelection: SpaceSelection | null, requestFollow: FollowSource | null) => {
    const pending = request();
    const entry: AcceptedAdd = { begin: request, spaceSelection: requestSelection, followSource: requestFollow, space: requestSpace, operation: null, error: null, seen: false, pending };
    // Record the result on this request only; a newer request owns `accepted`.
    entry.pending = pending.then((next) => { entry.operation = next; return next; }, (cause: unknown) => {
      entry.error = errorText(cause, "The Library operation could not start.");
      throw cause;
    });
    accepted = entry;
    lastBeginRef.current = request;
    setOperationSpace(requestSpace);
    setSpaceSelection(requestSelection);
    setFollowSource(requestFollow);
    setRestoredError(null);
    setAwaitingStart(false);
    void add.start(() => entry.pending);
  };
  const submit = () => {
    if (!canAdd || !resolution) return;
    const target = destinationSpace?.target ?? null;
    if (existing && target && !refreshExisting) {
      // Already saved: copy the Library item or followed space without asking the provider again.
      const selection: SpaceSelection = { item_ids: existingItem ? [existingItem] : [], follow_ids: existingFollow ? [existingFollow] : [] };
      begin(() => client.librarySpaceAdd({ target, ...selection }), destinationSpace, selection, null);
      return;
    }
    const providerId = requestProviderId ?? resolution.provider_id;
    begin(() => client.libraryAdd({
      input: requestUrl,
      provider_id: providerId,
      hydrate_references: forge && linked,
      follow_space: follow,
      download_attachments: false,
      refresh_existing: Boolean(existing) && refreshExisting,
      label: folder ? label?.trim() || resolution.title : null,
      target,
    }), destinationSpace, null, follow ? { input: requestUrl, provider_id: providerId } : null);
  };
  // The same source again, whether the request never started or stopped part way.
  const retryAll = () => { if (lastBeginRef.current) begin(lastBeginRef.current, operationSpace, spaceSelection, followSource); };
  // Copies the saved items again; items already in the Space are left unchanged. A
  // Space-only request resends what it asked for, since an interrupted operation
  // may have recorded only some of them; an add knows its items only from the operation.
  // A follow's operation lists its pages, so a follow is copied again by its follow id.
  const retrySpace = () => {
    const target = operation?.target ?? operationSpace?.target;
    if (!target || !operation) return;
    if (followSource) {
      const source = followSource;
      begin(async () => {
        const followed = await client.libraryResolve(source);
        if (!followed.existing_follow_id) throw new Error("The followed space isn't in the Library, so it can't be added to the Space.");
        return client.librarySpaceAdd({ target, item_ids: [], follow_ids: [followed.existing_follow_id] });
      }, operationSpace, { item_ids: [], follow_ids: [] }, source);
      return;
    }
    const followIds = spaceSelection?.follow_ids ?? [];
    const itemIds = followIds.length > 0 ? spaceSelection?.item_ids ?? [] : [...new Set([...(spaceSelection?.item_ids ?? []), ...operation.item_ids])];
    const selection: SpaceSelection = { item_ids: itemIds, follow_ids: followIds };
    begin(() => client.librarySpaceAdd({ target, ...selection }), operationSpace, selection, null);
  };
  const addAnother = () => {
    accepted = null;
    add.reset();
    setRestoredError(null);
    setAwaitingStart(false);
    setInput("");
    setLabel(null);
    setPicked(null);
    setFollowChoice(false);
    setLookup({ status: "idle" });
    setRefreshExisting(false);
    window.requestAnimationFrame(() => inputRef.current?.focus());
  };
  const pickSpace = (provider: ProjectProvider, chosenSpace: LibraryResolution) => {
    const key = chosenSpace.canonical_id ?? resolutionSpaceName(chosenSpace).split(" · ")[0]!;
    setInput(key);
    setChosenProviderId(provider.id);
    setPicked({ input: key, providerId: provider.id, resolution: chosenSpace });
    setLookupRetry(0);
    setLookup({ status: "ok", resolution: chosenSpace });
    setLabel(null);
    setRefreshExisting(false);
    setBrowseOpen(false);
    setPickedFocus((value) => value + 1);
  };
  const starting = add.starting || awaitingStart;
  const openedItemId = operation?.item_ids[0] ?? spaceSelection?.item_ids[0] ?? existingItem;
  const spaceFailed = spaceStep?.tone === "failed";
  const written = operation?.space?.written ?? [];
  // A partial copy still opens what it wrote.
  const openablePath = openInSpace && written.length > 0 && operation?.space?.companion_root_id === openInSpace.companionRootId ? written[0] : undefined;
  const libraryIncomplete = finished && progress !== null && !progress.complete && lastBeginRef.current !== null;
  // After success focus moves to `Open in …`; after a failure, to its retry.
  useEffect(() => {
    if (startError && !operation && !starting) primaryActionRef.current?.focus();
    else if (finished) (primaryActionRef.current ?? closeActionRef.current)?.focus();
    else if (starting || operation) closeActionRef.current?.focus();
  }, [startError, starting, finished, operation]);
  const failure = lookup.status === "error" ? lookup.failure : null;
  // On the body: a Context viewer is a size container and would clip a fixed overlay to its pane.
  return createPortal(<div className="setup-overlay library-dialog-overlay" role="presentation">
    <section className="setup-dialog task-setup library-add" role="dialog" aria-modal="true" aria-labelledby={titleId} onKeyDown={(event) => {
      if (event.key === "Enter" && !operation && event.target instanceof HTMLInputElement && event.target.type === "text" && !event.nativeEvent.isComposing) {
        event.preventDefault();
        submit();
        return;
      }
      trapDialogKeys(event, onClose);
    }}>
      <header className="task-setup-header"><h2 id={titleId}>Add context</h2><button type="button" className="task-setup-close" onClick={onClose} aria-label="Close Add context"><UiIcon name="close" /></button></header>
      <div className="task-setup-body">
        {operation || starting ? <div className="library-progress" aria-live="polite">
          <ol className="library-progress-steps">
            <li className={`library-progress-step is-${progress?.tone ?? "running"}`}>
              <span>{progress?.text ?? (spaceSelection ? `Adding to ${shownSpace}…` : followSource ? "Following the space…" : "Saving to Library…")}</span>
              {operation && !finished && !operation.cancel_requested && operation.kind !== "space_add" && !progress?.saved ? <button type="button" onClick={add.cancel}>Cancel</button> : null}
            </li>
            {spaceStep ? <li className={`library-progress-step is-${spaceStep.tone}`}>
              <span key={spaceStep.tone} role={spaceFailed ? "alert" : undefined}>{spaceStep.text}</span>
              {spaceFailed && written.length > 0 ? <ul className="library-progress-files" aria-label={`Copied to ${shownSpace}`}>
                {written.map((path) => <li key={path}><code>{path}</code></li>)}
              </ul> : null}
              {spaceFailed && finished ? <span className="library-progress-actions">
                <button ref={primaryActionRef} type="button" className="setup-primary" onClick={retrySpace}>{`Retry adding to ${shownSpace}`}</button>
                {openablePath && openInSpace ? <button type="button" onClick={() => { openInSpace.open(openablePath); onClose(); }}>{`Open in ${shownSpace}`}</button> : null}
                {onOpenItem && openedItemId ? <button type="button" onClick={() => { onOpenItem(openedItemId); onClose(); }}>Open in Library</button> : null}
              </span> : null}
            </li> : null}
          </ol>
          {add.error ? <p className="task-setup-note" role="status">{add.error}</p> : null}
        </div> : <>
          {startError ? <p className="task-setup-note is-error" role="alert">{startError}</p> : null}
          <div className="task-setup-row">
            <label htmlFor={fieldId}>Source</label>
            <div>
              <input ref={inputRef} id={fieldId} type="text" value={input} onChange={(event) => { setInput(event.target.value); if (event.target.value.trim() !== trimmed) { setLabel(null); setLookup({ status: "idle" }); setRefreshExisting(false); setFollowChoice(false); setPicked(null); } }}
                placeholder="Issue, MR or PR link, Jira key, Confluence page or space, or folder path" autoComplete="off" spellCheck={false}
                aria-invalid={failure ? "true" : undefined} aria-describedby={failure ? failureId : undefined} />
              {lookup.status === "pending" ? <p className="task-setup-note">{trimmed.startsWith("/") || trimmed === "~" || trimmed.startsWith("~/") ? "Checking the folder…"
                : confluencePage ? `Looking up Confluence page ${confluencePage.pageId ?? `“${confluencePage.title}”`}${confluencePage.spaceKey ? ` in ${confluencePage.spaceKey}` : ""}${chosen ? ` on ${confluenceSite(chosen.base_url)}` : ""}…`
                : confluenceSpace ? `Looking up Confluence space ${confluenceSpace.spaceKey}${chosen ? ` on ${confluenceSite(chosen.base_url)}` : ""}…`
                : "Looking up the link…"}</p> : null}
              {resolution ? <p className="task-setup-note is-valid">✓ {existingFollow ? `Already following · ${spaceName}` : existingItem ? `Already in Library · ${resolution.title}` : resolutionNote(resolution, providers)}</p> : null}
              {resolution && (resolution.kind === "confluence_page" || resolution.kind === "confluence_space") ? <p className="task-setup-note">{[resolution.kind === "confluence_page" ? resolution.container_label : null, confluenceSite(resolution.provider_instance)].filter(Boolean).join(" · ")}</p> : null}
              {resolution?.diagnostics.map((diagnostic, index) => <p className="task-setup-note" key={`${diagnostic.code}:${index}`}>{diagnostic.message}</p>)}
              {existingInSpace?.text ? <p className="task-setup-note">{existingInSpace.text}</p> : null}
              {failure ? <div id={failureId} className="library-refusal" role="alert">
                <strong>{failure.title}</strong>
                <span>{failure.detail}</span>
                {failure.retry ? <button type="button" className="task-setup-link" onClick={() => setLookupRetry((value) => value + 1)}>Retry lookup</button> : null}
              </div> : null}
              {providersError ? <p className="task-setup-note">{providersError}</p> : null}
              {providersLoaded && confluence.length > 0 ? <>
                <button type="button" className="task-setup-link library-browse-toggle" aria-expanded={browseOpen} aria-controls={browseOpen ? browseId : undefined} onClick={() => setBrowseOpen((open) => !open)}>
                  <UiIcon name={browseOpen ? "down" : "right"} /> Browse Confluence spaces
                </button>
                {browseOpen ? <div id={browseId} className="library-browse">
                  {confluence.map((provider) => <ConfluenceSpaceList key={provider.id} client={client} provider={provider} providers={providers} onPick={pickSpace} />)}
                </div> : null}
              </> : null}
            </div>
          </div>
          {folder ? <div className="task-setup-row">
            <label htmlFor={`${fieldId}-label`}>Label</label>
            <div>
              <input id={`${fieldId}-label`} type="text" value={label ?? resolution.title} onChange={(event) => setLabel(event.target.value)} autoComplete="off" />
              <p className="task-setup-note">Copies files into the Library, not a live link. Later source edits stay outside the Library until you explicitly re-copy.</p>
            </div>
          </div> : null}
          {choices.length > 1 ? <div className="task-setup-row">
            <label htmlFor={`${fieldId}-provider`}>Provider</label>
            <div><select id={`${fieldId}-provider`} className="library-select" value={chosen?.id ?? ""} onChange={(event) => setChosenProviderId(event.target.value)}>
              {choices.map((provider) => <option key={provider.id} value={provider.id}>{provider.id} · {provider.base_url}</option>)}
            </select></div>
          </div> : null}
          {followOffered || spaceResolution ? <div className="task-setup-row">
            <span id={followChoiceId} className="library-row-label">Add</span>
            <div>
              {followOffered ? <div role="radiogroup" aria-labelledby={followChoiceId} className="library-destination-choices">
                <label className="task-setup-check"><input type="radio" name={followChoiceId} checked={!followChoice} onChange={() => setFollowChoice(false)} /> Only this page</label>
                <label className="task-setup-check"><input type="radio" name={followChoiceId} checked={followChoice} onChange={() => setFollowChoice(true)} /> {`Follow the whole space (${spaceName}${pageTotal !== null ? ` · ${pageCount(pageTotal)}` : ""})`}</label>
              </div> : <p className="library-destination">{`Follow the whole space (${spaceName}${pageTotal !== null ? ` · ${pageCount(pageTotal)}` : ""})`}</p>}
              {follow && !existingFollow ? <p className="task-setup-note">Saves every page this profile can read in {spaceName}, including every top-level page tree. Refresh adds new pages and updates changed ones; Spaces change only when you update them.</p> : null}
              {overLimit ? <p className="task-setup-note library-note-partial" role="status"><span aria-hidden="true">◐</span> {`The page limit is ${spacePageLimit}, so this saves ${spacePageLimit} of ${pageTotal} pages. Refresh won't mark pages removed at source until the whole space fits.`}</p> : null}
            </div>
          </div> : null}
          {resolution?.kind === "confluence_page" && resolution.existing_follow_id ? <p className="task-setup-check">{`${resolutionSpaceName(resolution)} is already followed; refreshing it keeps this page current.`}</p> : null}
          {resolution && forge && !existing ? <label className="task-setup-check"><input type="checkbox" checked={linked} onChange={(event) => setLinked(event.target.checked)} /> Include linked issues and {family?.review}s within import limits</label> : null}
          {existing ? <label className="task-setup-check"><input type="checkbox" checked={refreshExisting} onChange={(event) => setRefreshExisting(event.target.checked)} /> Refresh from source first</label> : null}
          <div className="task-setup-row">
            <span id={destinationId} className="library-row-label">Destination</span>
            {spaceChoice ? <div role="radiogroup" aria-labelledby={destinationId} className="library-destination-choices">
              <label className="task-setup-check"><input type="radio" name={destinationId} checked={destination === "library"} onChange={() => setDestination("library")} /> Library only</label>
              <label className="task-setup-check"><input type="radio" name={destinationId} checked={destination === "space"} onChange={() => setDestination("space")} /> Library and {spaceChoice.label}</label>
              {destination === "space" ? <p className="task-setup-note">Saved to the Library first, then copied into {spaceChoice.label}. Later Library refreshes don't change {spaceChoice.label} until you update it.</p> : null}
              {companion?.status === "unavailable" ? <p className="task-setup-note is-error">{spaceChoice.label}'s context folder couldn't be verified: {companion.error.message.replace(/\.$/, "")}. The Library copy is saved either way; a failed Space step can be retried.</p> : null}
            </div> : <div>
              <p className="library-destination">Library</p>
              <p className="task-setup-note">{space ? `Herdr isn't live, so this adds to the Library only.` : "Select a Space to also add it there."}</p>
            </div>}
          </div>
        </>}
      </div>
      <footer className="task-setup-footer">
        {!operation && !starting ? <>
          {startError && lastBeginRef.current ? <>
            <button type="button" onClick={addAnother}>Add another</button>
            <button ref={primaryActionRef} type="button" className="setup-primary" onClick={retryAll}>Retry</button>
          </> : null}
          {existingItem && onOpenItem ? <button type="button" onClick={() => { onOpenItem(existingItem); onClose(); }}>Open in Library</button> : null}
          <button type="button" onClick={onClose}>Cancel</button>
          {!startError ? <button ref={submitRef} type="button" className="setup-primary" onClick={submit} disabled={!canAdd}>{primaryLabel}</button> : null}
        </> : !finished || !progress ? <button ref={closeActionRef} type="button" onClick={onClose}>Close</button> : <>
          <button type="button" onClick={addAnother}>Add another</button>
          <button ref={closeActionRef} type="button" onClick={onClose}>Close</button>
          {/* The same source again: primary when nothing was saved, beside the result otherwise. */}
          {libraryIncomplete ? <button ref={progress.saved ? undefined : primaryActionRef} type="button" className={progress.saved ? undefined : "setup-primary"} onClick={retryAll}>Retry</button> : null}
          {!progress.saved || spaceFailed ? null : openablePath && openInSpace
            ? <button ref={primaryActionRef} type="button" className="setup-primary" onClick={() => { openInSpace.open(openablePath); onClose(); }}>{`Open in ${shownSpace}`}</button>
            : onOpenItem && openedItemId ? <button ref={primaryActionRef} type="button" className="setup-primary" onClick={() => { onOpenItem(openedItemId); onClose(); }}>Open in Library</button> : null}
        </>}
      </footer>
    </section>
  </div>, document.body);
}
