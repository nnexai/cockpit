import { useEffect, useState, type Dispatch, type ReactNode, type SetStateAction } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { LibraryResolution, ProjectProvider } from "../../protocol/generated/v1";
import { JIRA_KEY, confluencePageInput, confluenceProviders, confluenceSpaceInput, errorText, jiraProviders, jiraQueryInput, libraryInputUrl, lookupFailure, providerFamily, resolutionSpaceName, type ConfluencePageInput, type ConfluenceSpaceInput, type JiraQueryInput, type LibrarySpace, type LookupFailure } from "./libraryState";
import { useLibraryListing, useSpaceContextListing, type LibraryListingState } from "./useLibraryOperation";
import { useProviderCredentialActions, type ProviderCredentialActions } from "./useProviderCredentials";

const LOOKUP_DELAY_MS = 400;

type Lookup =
  | { status: "idle" }
  | { status: "pending" }
  | { status: "ok"; resolution: LibraryResolution }
  | { status: "error"; failure: LookupFailure };

export function useAddContextSource(client: CockpitClient, space: LibrarySpace | null, defaultDestination: "library" | "space"): AddContextSourceModel {
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
  const credentials = useProviderCredentialActions(client, providers);
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
  const reset = () => {
    setInput("");
    setLabel(null);
    setPicked(null);
    setFollowChoice(false);
    setDepthChoice(null);
    setModeChoice(null);
    setDownloadAttachments(false);
    setLookup({ status: "idle" });
    setRefreshExisting(false);
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
  const failure = lookup.status === "error" ? lookup.failure : null;
  return { providers, spacePageLimit, attachmentLimit, providersLoaded, providersError,
    input, label, setLabel, setChosenProviderId, depthChoice, setDepthChoice, refreshExisting, setRefreshExisting,
    followChoice, setFollowChoice, downloadAttachments, setDownloadAttachments, modeChoice, setModeChoice,
    browseOpen, setBrowseOpen, pickedFocus, lookup, setLookupRetry, spaceChoice, setDestination, destination,
    credentials, trimmed, jira, confluence, confluencePage, confluenceSpace, jiraQuery, choices, chosen,
    requestUrl, requestProviderId, editInput, resolution, spaceResolution, jiraFollow, followMode, followOffered,
    follow, spaceName, existingFollow, existingItem, existing, folder, jiraDownloads, jiraToken, attachmentsOffered,
    depthOffered, referenceDepth, depthOptions, destinationSpace, followedItems, existingItemIds, existingInSpace,
    spaceAddable, pageTotal, overLimit, overCap, failure, reset, pickSpace };
}

/** Source identity, lookup and the choices that belong to that source. */
export type AddContextSourceModel = {
  providers: ProjectProvider[];
  spacePageLimit: number | null;
  attachmentLimit: number | null;
  providersLoaded: boolean;
  providersError: string | null;
  input: string;
  label: string | null;
  setLabel: Dispatch<SetStateAction<string | null>>;
  setChosenProviderId: Dispatch<SetStateAction<string | null>>;
  depthChoice: number | null;
  setDepthChoice: Dispatch<SetStateAction<number | null>>;
  refreshExisting: boolean;
  setRefreshExisting: Dispatch<SetStateAction<boolean>>;
  followChoice: boolean;
  setFollowChoice: Dispatch<SetStateAction<boolean>>;
  downloadAttachments: boolean;
  setDownloadAttachments: Dispatch<SetStateAction<boolean>>;
  modeChoice: "live" | "accumulate" | null;
  setModeChoice: Dispatch<SetStateAction<"live" | "accumulate" | null>>;
  browseOpen: boolean;
  setBrowseOpen: Dispatch<SetStateAction<boolean>>;
  pickedFocus: number;
  lookup: Lookup;
  setLookupRetry: Dispatch<SetStateAction<number>>;
  spaceChoice: LibrarySpace | null;
  setDestination: Dispatch<SetStateAction<"library" | "space" | null>>;
  destination: "library" | "space";
  credentials: { actions: ProviderCredentialActions; dialog: ReactNode };
  trimmed: string;
  jira: ProjectProvider[];
  confluence: ProjectProvider[];
  confluencePage: ConfluencePageInput | null;
  confluenceSpace: ConfluenceSpaceInput | null;
  jiraQuery: JiraQueryInput | null;
  choices: ProjectProvider[];
  chosen: ProjectProvider | undefined;
  requestUrl: string;
  requestProviderId: string | null;
  editInput: (value: string) => void;
  resolution: LibraryResolution | null;
  spaceResolution: boolean;
  jiraFollow: boolean;
  followMode: "live" | "accumulate" | null;
  followOffered: boolean;
  follow: boolean;
  spaceName: string | null;
  existingFollow: string | null;
  existingItem: string | null;
  existing: string | null;
  folder: boolean;
  jiraDownloads: boolean;
  jiraToken: boolean;
  attachmentsOffered: boolean;
  depthOffered: boolean;
  referenceDepth: number;
  depthOptions: number[];
  destinationSpace: LibrarySpace | null;
  followedItems: LibraryListingState;
  existingItemIds: string[];
  existingInSpace: boolean;
  spaceAddable: boolean;
  pageTotal: number | null;
  overLimit: boolean;
  overCap: boolean;
  failure: LookupFailure | null;
  reset: () => void;
  pickSpace: (provider: ProjectProvider, chosenSpace: LibraryResolution) => void;
};
