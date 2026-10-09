import { useEffect, useState, type RefObject } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { LibraryResolution, ProjectProvider } from "../../protocol/generated/v1";
import { StatePill } from "./StatePill";
import { confluenceSite, jiraQueryPresets, jiraQueryProject, lookupFailure, pageCount, resolutionNote, resolutionSpaceName, type LookupFailure } from "./libraryState";
import type { AddContextSourceModel } from "./AddContextSource";

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

export function AddContextSourceStep({ client, source, fieldId, failureId, browseId, inputRef }: {
  client: CockpitClient;
  source: AddContextSourceModel;
  fieldId: string;
  failureId: string;
  browseId: string;
  inputRef: RefObject<HTMLInputElement | null>;
}) {
  const { input, editInput, failure, providersLoaded, jira, confluence, trimmed, jiraQuery, browseOpen, setBrowseOpen,
    lookup, confluencePage, chosen, confluenceSpace, resolution, jiraFollow, providers, existingFollow, spaceName,
    existingItem, existingInSpace, credentials, setLookupRetry, providersError, pickSpace } = source;
  return (
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
        {failure.credentialProviderId ? <button type="button" className="task-setup-link" onClick={() => credentials.actions.open(failure.credentialProviderId!)}>Provider token…</button> : null}
        {failure.retry ? <button type="button" className="task-setup-link" onClick={() => setLookupRetry((value) => value + 1)}>Retry lookup</button> : null}
      </div> : null}
      {providersError ? <p className="task-setup-note">{providersError}</p> : null}
      {providersLoaded && confluence.length > 0 && browseOpen ? <div id={browseId} className="library-browse">
        {confluence.map((provider) => <ConfluenceSpaceList key={provider.id} client={client} provider={provider} providers={providers} onPick={pickSpace} />)}
      </div> : null}
    </div>
  </div>
  );
}
