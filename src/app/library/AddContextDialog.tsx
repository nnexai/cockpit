import { useEffect, useId, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { LibraryOperation, LibraryResolution, ProjectProvider } from "../../protocol/generated/v1";
import { createPortal } from "react-dom";
import { UiIcon } from "../UiIcon";
import { ErrorSlot } from "../ErrorSlot";
import { trapDialogKeys, useRestoreFocus } from "./LibraryConfirmDialog";
import { StatePill } from "./StatePill";
import { JIRA_KEY, confluencePageInput, confluenceProviders, confluenceSite, confluenceSpaceInput, errorText, issueCount, jiraProviders, jiraQueryInput, jiraQueryPresets, jiraQueryProject, libraryInputUrl, lookupFailure, pageCount, providerFamily, resolutionNote, resolutionSpaceName, type LibrarySpace, type LookupFailure } from "./libraryState";
import { useLibraryListing, useLibraryOperation, useSpaceContextListing } from "./useLibraryOperation";
import { useProviderCredentialActions } from "./useProviderCredentials";
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
 */
function libraryPhase(operation: LibraryOperation, unit: "items" | "pages" | "issues"): { text: string; tone: Tone; saved: boolean; complete: boolean } {
  if (operation.kind === "space_add") return { text: "✓ Saved to Library", tone: "done", saved: true, complete: true };
  const phase = operation.phases.find((candidate) => candidate.phase === "library");
  const partialReason = operation.report?.rows.find((row) => row.outcome === "partial")?.reason;
  const unchanged = operation.report?.rows.find((row) => row.outcome === "unchanged");
  const count = operation.item_ids.length;
  // A followed space counts its pages (`✓ Saved to Library · 38 pages`), a followed query its issues, or its items once references add other kinds.
  const summary = unit === "pages" ? ` · ${pageCount(count)}` : unit === "issues" ? ` · ${issueCount(count)}` : count > 1 ? ` · ${count} items` : "";
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

/** Selecting saved Library items never changes their paths or contents. */
function spacePhase(operation: LibraryOperation, space: string): { text: string; tone: Tone } | null {
  const phase = operation.phases.find((candidate) => candidate.phase === "space");
  if (!phase) return null;
  const reason = (phase.error?.message ?? phase.message ?? "The selection could not be saved").replace(/\.$/, "");
  switch (phase.state) {
    case "pending":
      return operation.finished ? { text: `Saved to Library, but couldn't select for ${space}. ${reason}.`, tone: "failed" } : null;
    case "running":
      return { text: `Adding to ${space}…${phase.total && phase.total > 1 ? ` ${phase.done} of ${phase.total}` : ""}`, tone: "running" };
    case "done":
    case "partial":
      return { text: `✓ Added to ${space}`, tone: "done" };
    case "failed":
    case "cancelled":
      return { text: `Saved to Library, but couldn't select for ${space}. ${phase.state === "cancelled" ? "Selection was cancelled" : reason}.`, tone: "failed" };
  }
}

/**
 * The dialog's accepted request outlives the dialog (design §4.7: closing does
 * not cancel). Reopening shows a request still starting, a running add, an
 * outcome that finished while closed, until `Add another` or a new add.
 */
type AcceptedAdd = {
  begin: () => Promise<LibraryOperation>;
  savedItemId: string | null;
  unit: "pages" | "issues" | "items";
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
          {space.item_count !== null ? <span className="library-browse-detail">{pageCount(space.item_count)}</span> : null}
          {space.existing_follow_id ? <StatePill shape="dot-ring" tone="idle" word="Following" /> : null}
          <button type="button" aria-label={`${verb} ${name}`} onClick={() => onPick(provider, space)}>{verb}</button>
        </li>;
      })}
    </ul> : null}
  </section>;
}

/**
 * Add context (design §4.5): one field for a forge issue, MR or PR link, a
 * Jira key, a Confluence page link or id, a Confluence space link or key, or a
 * local folder path, plus `Browse Confluence spaces`. It always saves to the
 * Library first, then selects the saved items for a live target Space.
 */
export function AddContextDialog({ client, onClose, onOpenItem, space = null, defaultDestination = "space" }: {
  client: CockpitClient;
  onClose: () => void;
  onOpenItem?: (itemId: string) => void;
  /** The live Space that can select the saved Library items. */
  space?: LibrarySpace | null;
  defaultDestination?: "library" | "space";
}) {
  const titleId = useId();
  const fieldId = useId();
  const failureId = useId();
  const destinationId = useId();
  const followChoiceId = useId();
  const followModeId = useId();
  const browseId = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const closeActionRef = useRef<HTMLButtonElement>(null);
  const primaryActionRef = useRef<HTMLButtonElement>(null);
  const submitRef = useRef<HTMLButtonElement>(null);
  const restoredRef = useRef(accepted);
  const lastBeginRef = useRef<(() => Promise<LibraryOperation>) | null>(restoredRef.current?.begin ?? null);
  const [operationSpace, setOperationSpace] = useState<LibrarySpace | null>(restoredRef.current?.space ?? null);
  const [savedItemId, setSavedItemId] = useState<string | null>(restoredRef.current?.savedItemId ?? null);
  const [operationUnit, setOperationUnit] = useState<"pages" | "issues" | "items">(restoredRef.current?.unit ?? "items");
  const [restoredError, setRestoredError] = useState<string | null>(restoredRef.current && !restoredRef.current.operation ? restoredRef.current.error : null);
  // Closed before the request was accepted: follow it until it settles.
  const [awaitingStart, setAwaitingStart] = useState(Boolean(restoredRef.current && !restoredRef.current.operation && !restoredRef.current.error));
  const [providers, setProviders] = useState<ProjectProvider[]>([]);
  const [spacePageLimit, setSpacePageLimit] = useState<number | null>(null);
  const [attachmentLimit, setAttachmentLimit] = useState<number | null>(null);
  const [providersLoaded, setProvidersLoaded] = useState(false);
  const [providersError, setProvidersError] = useState<string | null>(null);
  const [input, setInput] = useState("");
  const [label, setLabel] = useState<string | null>(null);
  // The provider picked where several configured instances could read the input.
  const [chosenProviderId, setChosenProviderId] = useState<string | null>(null);
  // The user's `Follow references` pick; null keeps the default for the resolved source.
  const [depthChoice, setDepthChoice] = useState<number | null>(null);
  const [refreshExisting, setRefreshExisting] = useState(false);
  // A Confluence page: `Only this page` (false, the default) or `Follow the whole space`.
  const [followChoice, setFollowChoice] = useState(false);
  // A new Confluence page or followed space downloads attachments only when asked (Q6); unchecked by default.
  const [downloadAttachments, setDownloadAttachments] = useState(false);
  // A Jira query follows `live` or `accumulate`; null keeps the resolution's suggestion.
  const [modeChoice, setModeChoice] = useState<"live" | "accumulate" | null>(null);
  const [browseOpen, setBrowseOpen] = useState(false);
  // A space chosen from the browser is already resolved; the lookup reuses it instead of asking again.
  const [picked, setPicked] = useState<{ input: string; providerId: string; resolution: LibraryResolution } | null>(null);
  const [pickedFocus, setPickedFocus] = useState(0);
  const [lookup, setLookup] = useState<Lookup>({ status: "idle" });
  const [lookupRetry, setLookupRetry] = useState(0);
  const spaceChoice = space?.live ? space : null;
  const [destinationChoice, setDestination] = useState<"library" | "space" | null>(null);
  const spaceListing = useSpaceContextListing(client, spaceChoice?.target ?? null, spaceChoice !== null);
  const destination = spaceChoice && (destinationChoice ?? defaultDestination) === "space" ? "space" : "library";
  const add = useLibraryOperation(client);
  const credentials = useProviderCredentialActions(client, providers);
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
      if (current) { setProviders(configuration.providers); setSpacePageLimit(configuration.limits?.library_space_pages ?? null); setAttachmentLimit(configuration.limits?.library_attachment_bytes ?? null); setProvidersLoaded(true); }
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
  const jiraQuery = jiraKey || confluencePage ? null : jiraQueryInput(trimmed, providers);
  // A Jira key, Confluence page or space, or Jira query names its provider here; every other input by its own link.
  // A bare key could be a Confluence space or a Jira project: Confluence providers stay first, so the field keeps reading it as a space.
  const choices = jiraKey ? jira : confluencePage?.providers ?? (jiraQuery ? [...(jiraQuery.bare ? confluenceSpace?.providers ?? [] : []), ...jiraQuery.providers] : confluenceSpace?.providers) ?? [];
  const chosen = choices.find((provider) => provider.id === chosenProviderId) ?? choices[0];
  const needsProvider = jiraKey || confluencePage !== null || confluenceSpace !== null || jiraQuery !== null;
  const requestUrl = libraryInputUrl(trimmed, jiraKey ? chosen : undefined);
  const requestProviderId = chosen?.id ?? null;
  // Typing, or choosing a preset, replaces the source: choices made for the old one no longer apply.
  const editInput = (value: string) => {
    setInput(value);
    if (value.trim() !== trimmed) { setLabel(null); setLookup({ status: "idle" }); setRefreshExisting(false); setFollowChoice(false); setModeChoice(null); setDepthChoice(null); setDownloadAttachments(false); setPicked(null); }
  };
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
  // A Jira query is always followed; its mode is the user's pick, else the resolution's suggestion.
  const jiraFollow = resolution?.kind === "jira_query";
  const followMode = jiraFollow ? modeChoice ?? resolution.follow_mode ?? "live" : null;
  // A page can follow its whole space unless that space is already followed.
  const followOffered = resolution?.kind === "confluence_page" && !resolution.existing_follow_id;
  const follow = spaceResolution || jiraFollow || (followOffered && followChoice);
  const spaceName = resolution && (spaceResolution || followOffered) ? resolutionSpaceName(resolution) : null;
  // Following the same query again is allowed: it applies the chosen mode.
  const existingFollow = follow && !jiraFollow ? resolution?.existing_follow_id ?? null : null;
  const existingItem = follow ? null : resolution?.existing_item_id ?? null;
  const existing = existingItem ?? existingFollow;
  // Attachment bytes are fetched only for a new page, followed space, Jira issue or Jira query follow, and only when checked; a follow keeps the choice for its refreshes.
  // A Jira download needs a token stored in Cockpit: without one the field offers the token dialog instead.
  const folder = resolution?.kind === "folder";
  const family = resolution && !folder ? providerFamily(providers, resolution.provider_id) : null;
  const jiraDownloads = !existing && (jiraFollow || (resolution?.kind === "artifact" && family?.key === "jira"));
  const jiraToken = jiraDownloads ? credentials.actions.statuses?.find((status) => status.provider_id === resolution?.provider_id)?.state === "stored" : false;
  const ensureTokens = credentials.actions.ensure;
  useEffect(() => { if (jiraDownloads) ensureTokens(); }, [jiraDownloads, ensureTokens]);
  const attachmentsOffered = ((resolution?.kind === "confluence_page" || spaceResolution) && !existing) || (jiraDownloads && jiraToken);
  // One hop per step, from the ticket, page or query results through relations, descriptions and comments. Jira tickets default to 1, everything else to Off,
  // and an item or follow already in the Library offers the depth it was saved with. An existing item applies a depth only when refreshed.
  const depthOffered = jiraFollow || (resolution?.kind === "artifact" && (!existing || refreshExisting));
  const referenceDepth = depthChoice ?? resolution?.reference_depth ?? (resolution?.kind === "artifact" && family?.key === "jira" ? 1 : 0);
  const depthOptions = [0, 1, 2, 3, ...(referenceDepth > 3 ? [referenceDepth] : [])];
  const destinationSpace = destination === "space" ? spaceChoice : null;
  const followedItems = useLibraryListing(client, Boolean(existingFollow && destinationSpace));
  const existingItemIds = existingItem ? [existingItem] : existingFollow
    ? followedItems.listing?.items.filter((item) => item.refs.some((ref) => ref.kind === "follow" && ref.follow_id === existingFollow)).map((item) => item.item_id) ?? []
    : [];
  const existingInSpace = Boolean(existing && destinationSpace && existingItemIds.length > 0
    && existingItemIds.every((itemId) => spaceListing.listing?.items.some((item) => item.item_id === itemId)));
  const spaceAddable = !existingInSpace && (!existingFollow || (followedItems.status === "ready" && followedItems.listing?.next_offset === null && existingItemIds.length > 0));
  // A follow over the page limit saves only part of the space (design §4.6 `◐ Partial`).
  const pageTotal = resolution?.item_count ?? null;
  const overLimit = follow && !jiraFollow && !existingFollow && spacePageLimit !== null && pageTotal !== null && pageTotal > spacePageLimit;
  // A query's preview reads one window, so it can only show that the cap is at or past it.
  const overCap = jiraFollow && spacePageLimit !== null && pageTotal !== null && pageTotal >= spacePageLimit;
  const operation = add.operation;
  const shownSpace = operationSpace?.label ?? "the Space";
  const progress = operation ? libraryPhase(operation, operationUnit) : null;
  const spaceStep = operation && progress?.saved ? spacePhase(operation, shownSpace) : null;
  const finished = operation?.finished ?? false;
  const startError = add.error ?? restoredError;
  const canAdd = resolution !== null && !add.starting && (!existing || refreshExisting || (destinationSpace !== null && spaceAddable));
  const primaryLabel = existing
    ? refreshExisting ? (destinationSpace ? `Refresh and add to ${destinationSpace.label}` : "Refresh from source") : destinationSpace ? existingInSpace ? "Already in Space" : "Add to Space" : existingFollow ? "Already following" : "Already in Library"
    : destinationSpace ? `Add to Library and ${destinationSpace.label}`
    : follow ? jiraFollow ? "Follow query" : "Follow space" : "Add to Library";
  const operationRef = useRef(operation);
  operationRef.current = operation;
  // A displayed, complete success is done with; anything else stays for the next opening.
  useEffect(() => () => {
    const latest = operationRef.current;
    if (!accepted?.seen || !latest?.finished || accepted.operation?.operation_id !== latest.operation_id) return;
    const phase = libraryPhase(latest, accepted.unit);
    if (phase.saved && phase.complete && spacePhase(latest, "")?.tone !== "failed") accepted = null;
  }, []);
  useEffect(() => {
    if (finished && operation && accepted?.operation?.operation_id === operation.operation_id) accepted.seen = true;
  }, [finished, operation]);
  // A space chosen in the browser moves focus to the action it enables, or back to the field.
  useEffect(() => {
    if (pickedFocus > 0) (submitRef.current && !submitRef.current.disabled ? submitRef.current : inputRef.current)?.focus();
  }, [pickedFocus]);
  const begin = (request: () => Promise<LibraryOperation>, requestSpace: LibrarySpace | null, requestSavedItemId: string | null, unit: "pages" | "issues" | "items") => {
    const pending = request();
    const entry: AcceptedAdd = { begin: request, savedItemId: requestSavedItemId, unit, space: requestSpace, operation: null, error: null, seen: false, pending };
    // Record the result on this request only; a newer request owns `accepted`.
    entry.pending = pending.then((next) => { entry.operation = next; return next; }, (cause: unknown) => {
      entry.error = errorText(cause, "The Library operation could not start.");
      throw cause;
    });
    accepted = entry;
    lastBeginRef.current = request;
    setOperationSpace(requestSpace);
    setSavedItemId(requestSavedItemId);
    setOperationUnit(unit);
    setRestoredError(null);
    setAwaitingStart(false);
    void add.start(() => entry.pending);
  };
  const submit = () => {
    if (!canAdd || !resolution) return;
    const target = destinationSpace?.target ?? null;
    if (existing && target && !refreshExisting) {
      // Select the existing Library items without asking the provider again.
      begin(() => client.librarySpaceAdd({ target, item_ids: existingItemIds }), destinationSpace, existingItemIds[0] ?? null, "items");
      return;
    }
    const providerId = requestProviderId ?? resolution.provider_id;
    begin(() => client.libraryAdd({
      input: requestUrl,
      provider_id: providerId,
      reference_depth: depthOffered ? referenceDepth : 0,
      follow,
      follow_mode: followMode,
      download_attachments: attachmentsOffered && downloadAttachments,
      refresh_existing: Boolean(existing) && refreshExisting,
      label: folder ? label?.trim() || resolution.title : null,
      target,
    }), destinationSpace, null, follow ? jiraFollow ? (referenceDepth > 0 ? "items" : "issues") : "pages" : "items");
  };
  // The same source again, whether the request never started or stopped part way.
  const retryAll = () => { if (lastBeginRef.current) begin(lastBeginRef.current, operationSpace, savedItemId, operationUnit); };
  const addAnother = () => {
    accepted = null;
    add.reset();
    setRestoredError(null);
    setAwaitingStart(false);
    setInput("");
    setLabel(null);
    setPicked(null);
    setFollowChoice(false);
    setDepthChoice(null);
    setModeChoice(null);
    setDownloadAttachments(false);
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
    setDepthChoice(null);
    setDownloadAttachments(false);
    setBrowseOpen(false);
    setPickedFocus((value) => value + 1);
  };
  const starting = add.starting || awaitingStart;
  const openedItemId = operation?.item_ids[0] ?? operation?.space?.item_ids[0] ?? savedItemId ?? existingItem;
  const spaceFailed = spaceStep?.tone === "failed";
  const libraryIncomplete = finished && progress !== null && !progress.complete && lastBeginRef.current !== null;
  // Progress polls and failures must not steal focus; successful completion can offer the result.
  const actionFailed = Boolean(startError || spaceFailed || progress?.tone === "failed");
  const footerMessage = startError ?? (spaceFailed ? spaceStep?.text : progress?.tone === "failed" ? progress.text : null);
  const focusPhase = finished ? actionFailed ? "failed" : "finished" : starting || operation ? "running" : "idle";
  useEffect(() => {
    if (focusPhase === "finished") (primaryActionRef.current ?? closeActionRef.current)?.focus();
    else if (focusPhase === "running") closeActionRef.current?.focus();
    else if (focusPhase === "failed" && !closeActionRef.current?.closest("[role='dialog']")?.contains(document.activeElement)) closeActionRef.current?.focus();
  }, [focusPhase]);
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
              <span>{progress?.tone === "failed" ? "Library" : progress?.text ?? (savedItemId ? `Adding to ${shownSpace}…` : operationUnit === "pages" ? "Following the space…" : operationUnit === "issues" ? "Following the query…" : "Saving to Library…")}</span>
              {operation && !finished && !operation.cancel_requested && operation.kind !== "space_add" && !progress?.saved ? <button type="button" onClick={add.cancel}>Cancel</button> : null}
            </li>
            {operationSpace || spaceStep ? <li className={`library-progress-step is-${spaceStep?.tone ?? "running"}`}>
              <span>{spaceFailed ? shownSpace : spaceStep?.text ?? `Waiting to add to ${shownSpace}…`}</span>
            </li> : null}
          </ol>
        </div> : <>
          <div className="task-setup-row">
            <label htmlFor={fieldId}>Source</label>
            <div>
              <input ref={inputRef} id={fieldId} type="text" value={input} onChange={(event) => editInput(event.target.value)}
                placeholder="Issue, MR or PR link, Jira key or query, Confluence page or space, or folder path" autoComplete="off" spellCheck={false}
                aria-invalid={failure ? "true" : undefined} aria-describedby={failure ? failureId : undefined} />
              {providersLoaded && (jira.length > 0 || confluence.length > 0) ? <div className="library-source-links">
                {jira.length > 0 && (trimmed === "" || jiraQuery !== null) ? <div className="library-query-presets" role="group" aria-label="Jira query presets">
                  {jiraQueryPresets(jiraQuery ? jiraQueryProject(jiraQuery.jql) : null).map((preset) => <button key={preset.label} type="button" className="task-setup-link" title={preset.jql} onClick={() => { editInput(preset.jql); inputRef.current?.focus(); }}>{preset.label}</button>)}
                </div> : null}
                {confluence.length > 0 ? <button type="button" className="task-setup-link" aria-expanded={browseOpen} aria-controls={browseOpen ? browseId : undefined} onClick={() => setBrowseOpen((open) => !open)}>Browse Confluence spaces</button> : null}
              </div> : null}
              {lookup.status === "pending" ? <p className="task-setup-note">{trimmed.startsWith("/") || trimmed === "~" || trimmed.startsWith("~/") ? "Checking the folder…"
                : confluencePage ? `Looking up Confluence page ${confluencePage.pageId ?? `“${confluencePage.title}”`}${confluencePage.spaceKey ? ` in ${confluencePage.spaceKey}` : ""}${chosen ? ` on ${confluenceSite(chosen.base_url)}` : ""}…`
                : jiraQuery && chosen && jiraQuery.providers.includes(chosen) ? "Counting the issues this query matches…"
                : confluenceSpace ? `Looking up Confluence space ${confluenceSpace.spaceKey}${chosen ? ` on ${confluenceSite(chosen.base_url)}` : ""}…`
                : "Looking up the link…"}</p> : null}
              {resolution ? <p className="task-setup-note is-valid">✓ {jiraFollow ? resolutionNote(resolution, providers) : existingFollow ? `Already following · ${spaceName}` : existingItem ? `Already in Library · ${resolution.title}` : resolutionNote(resolution, providers)}</p> : null}
              {resolution && (resolution.kind === "confluence_page" || resolution.kind === "confluence_space") ? <p className="task-setup-note">{[resolution.kind === "confluence_page" ? resolution.container_label : null, confluenceSite(resolution.provider_instance)].filter(Boolean).join(" · ")}</p> : null}
              {resolution?.diagnostics.map((diagnostic, index) => <p className="task-setup-note" key={`${diagnostic.code}:${index}`}>{diagnostic.message}</p>)}
              {existingInSpace ? <p className="task-setup-note library-space-state">Already in Space</p> : null}
              {failure ? <div id={failureId} className="library-refusal" role="alert">
                <strong>{failure.title}</strong>
                <span>{failure.detail}</span>
                {failure.credentialProviderId ? <button type="button" className="task-setup-link" onClick={() => credentials.actions.open(failure.credentialProviderId!)}>Store a token…</button> : null}
                {failure.retry ? <button type="button" className="task-setup-link" onClick={() => setLookupRetry((value) => value + 1)}>Retry lookup</button> : null}
              </div> : null}
              {providersError ? <p className="task-setup-note">{providersError}</p> : null}
              {providersLoaded && confluence.length > 0 && browseOpen ? <div id={browseId} className="library-browse">
                {confluence.map((provider) => <ConfluenceSpaceList key={provider.id} client={client} provider={provider} providers={providers} onPick={pickSpace} />)}
              </div> : null}
            </div>
          </div>
          {folder ? <div className="task-setup-row">
            <label htmlFor={`${fieldId}-label`}>Label</label>
            <div>
              <input id={`${fieldId}-label`} type="text" value={label ?? resolution.title} onChange={(event) => setLabel(event.target.value)} autoComplete="off" />
              <p className="task-setup-note">Captures files in the Library, not a live link. Later source edits stay outside the Library until you explicitly capture them again.</p>
            </div>
          </div> : null}
          {choices.length > 1 ? <div className="task-setup-row">
            <label htmlFor={`${fieldId}-provider`}>Provider</label>
            <div><select id={`${fieldId}-provider`} className="library-select" value={chosen?.id ?? ""} onChange={(event) => { setChosenProviderId(event.target.value); setDepthChoice(null); }}>
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
              {follow && !existingFollow ? <p className="task-setup-note">Saves every page this profile can read in {spaceName}, including every top-level page tree. Refresh adds new pages and updates changed ones; selected items show those changes immediately in Spaces.</p> : null}
              {overLimit ? <p className="task-setup-note library-note-partial" role="status"><span aria-hidden="true">◐</span> {`The page limit is ${spacePageLimit}, so this saves ${spacePageLimit} of ${pageTotal} pages. Refresh won't mark pages removed at source until the whole space fits.`}</p> : null}
            </div>
          </div> : null}
          {jiraFollow ? <div className="task-setup-row">
            <span id={followModeId} className="library-row-label">Mode</span>
            <div>
              <div role="radiogroup" aria-labelledby={followModeId} className="library-destination-choices">
                <label className="task-setup-check"><input type="radio" name={followModeId} checked={followMode === "live"} onChange={() => setModeChoice("live")} /> Live — mirror the query</label>
                <label className="task-setup-check"><input type="radio" name={followModeId} checked={followMode === "accumulate"} onChange={() => setModeChoice("accumulate")} /> Accumulate — keep every issue that ever matched</label>
              </div>
              {resolution.follow_mode === "accumulate" && !resolution.existing_follow_id && modeChoice === null ? <p className="task-setup-note">This query uses relative dates, so a live follow would drop issues as they age out of the window. Accumulate is preselected; it keeps every issue that ever matched.</p> : null}
              <p className="task-setup-note">Each issue is saved once, even when several follows match it. A live follow drops issues that stop matching; an issue nothing else holds is removed from the Library after a grace period unless you keep it or it has edits.</p>
              {resolution.existing_follow_id ? <p className="task-setup-note">This query is already followed. Following it again applies the chosen mode and restores issues you removed from it.</p> : null}
              {overCap ? <p className="task-setup-note library-note-partial" role="status"><span aria-hidden="true">◐</span> {`The issue limit is ${spacePageLimit}, so this saves ${spacePageLimit} issues. Refresh won't drop issues until the whole query fits.`}</p> : null}
            </div>
          </div> : null}
          {resolution?.kind === "confluence_page" && resolution.existing_follow_id ? <p className="task-setup-check">{`${resolutionSpaceName(resolution)} is already followed; refreshing it keeps this page current.`}</p> : null}
          {attachmentsOffered ? <label className="task-setup-check"><input type="checkbox" checked={downloadAttachments} onChange={(event) => setDownloadAttachments(event.target.checked)} /> {`Download attachments${attachmentLimit !== null ? ` (up to ${Math.round(attachmentLimit / (1024 * 1024))} MB each)` : ""}`}</label> : null}
          {jiraDownloads && credentials.actions.statuses !== null && !jiraToken && resolution?.provider_id ? <p className="task-setup-check">
            <span>Downloading Jira attachments needs a token stored in Cockpit.</span>
            <button type="button" className="task-setup-link" onClick={() => credentials.actions.open(resolution.provider_id!)}>Provider token…</button>
          </p> : null}
          {depthOffered ? <div className="task-setup-row">
            <label htmlFor={`${fieldId}-depth`}>Follow references</label>
            <div>
              <select id={`${fieldId}-depth`} className="library-select" value={referenceDepth} onChange={(event) => setDepthChoice(Number(event.target.value))}>
                {depthOptions.map((steps) => <option key={steps} value={steps}>{steps === 0 ? "Off" : `${steps} ${steps === 1 ? "step" : "steps"}`}</option>)}
              </select>
              <p className="task-setup-note">Also saves items these link to or mention: relations, descriptions and comments. Each step follows one more hop.</p>
            </div>
          </div> : null}
          {existing ? <label className="task-setup-check"><input type="checkbox" checked={refreshExisting} onChange={(event) => setRefreshExisting(event.target.checked)} /> Refresh from source first</label> : null}
          <div className="task-setup-row">
            <span id={destinationId} className="library-row-label">Destination</span>
            {spaceChoice ? <div role="radiogroup" aria-labelledby={destinationId} className="library-destination-choices">
              <label className="task-setup-check"><input type="radio" name={destinationId} checked={destination === "library"} onChange={() => setDestination("library")} /> Library only</label>
              <label className="task-setup-check"><input type="radio" name={destinationId} checked={destination === "space"} onChange={() => setDestination("space")} /> Library and {spaceChoice.label}</label>
              {destination === "space" ? <p className="task-setup-note">Saved to the Library first, then selected for {spaceChoice.label}. Library refreshes are visible in the Space immediately.</p> : null}
              {destination === "space" && existingFollow && followedItems.error ? <p className="task-setup-note" role="alert">{followedItems.error} <button type="button" onClick={followedItems.reload}>Reload Library</button></p> : null}
              {destination === "space" && existingFollow && followedItems.listing?.next_offset !== null && followedItems.status === "ready" ? <p className="task-setup-note">The full Library could not be read. Open it in the Library to select its items.</p> : null}
            </div> : <div>
              <p className="library-destination">Library</p>
              <p className="task-setup-note">{space ? `Herdr isn't live, so this adds to the Library only.` : "Select a Space to also add it there."}</p>
            </div>}
          </div>
        </>}
      </div>
      <footer className="task-setup-footer">
        <ErrorSlot placement="dialog" message={footerMessage} />
        <div className="library-footer-actions">
        <button ref={closeActionRef} type="button" onClick={onClose}>{operation || starting ? "Close" : "Cancel"}</button>
        {!operation && !starting ? <>
          {startError && lastBeginRef.current ? <>
            <button type="button" onClick={addAnother}>Add another</button>
            <button ref={primaryActionRef} type="button" className="setup-primary" onClick={retryAll}>Retry</button>
          </> : null}
          {existingItem && onOpenItem ? <button type="button" onClick={() => { onOpenItem(existingItem); onClose(); }}>Open in Library</button> : null}
          {!startError ? <button ref={submitRef} type="button" className="setup-primary" onClick={submit} disabled={!canAdd}>{primaryLabel}</button> : null}
        </> : !finished || !progress ? null : <>
          <button type="button" onClick={addAnother}>Add another</button>
          {/* The same source again: primary when nothing was saved, beside the result otherwise. */}
          {libraryIncomplete ? <button ref={progress.saved ? undefined : primaryActionRef} type="button" className={progress.saved ? undefined : "setup-primary"} onClick={retryAll}>Retry</button> : null}
          {spaceFailed && finished ? <>
            {onOpenItem && openedItemId ? <button type="button" onClick={() => { onOpenItem(openedItemId); onClose(); }}>Open in Library</button> : null}
          </> : null}
          {!progress.saved || spaceFailed ? null : onOpenItem && openedItemId ? <button ref={primaryActionRef} type="button" className="setup-primary" onClick={() => { onOpenItem(openedItemId); onClose(); }}>Open in Library</button> : null}
        </>}
        </div>
      </footer>
    </section>
    {credentials.dialog}
  </div>, document.body);
}
