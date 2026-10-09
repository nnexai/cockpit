import type { LibrarySpace } from "./libraryState";
import { pageCount, resolutionSpaceName } from "./libraryState";
import type { AddContextSourceModel } from "./AddContextSource";

export function AddContextOptionsStep({ source, space, fieldId, followChoiceId, followModeId, destinationId }: {
  source: AddContextSourceModel;
  space: LibrarySpace | null;
  fieldId: string;
  followChoiceId: string;
  followModeId: string;
  destinationId: string;
}) {
  const { folder, label, resolution, setLabel, choices, chosen, setChosenProviderId, setDepthChoice, followOffered,
    spaceResolution, followChoice, setFollowChoice, spaceName, pageTotal, follow, existingFollow, overLimit,
    spacePageLimit, jiraFollow, followMode, setModeChoice, modeChoice, overCap, attachmentsOffered,
    downloadAttachments, setDownloadAttachments, attachmentLimit, jiraDownloads, credentials, jiraToken,
    depthOffered, referenceDepth, depthOptions, existing, refreshExisting, setRefreshExisting,
    spaceChoice, destination, setDestination, followedItems } = source;
  return <>
  {folder && resolution ? <div className="task-setup-row">
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
  {jiraFollow && resolution ? <div className="task-setup-row">
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
  </>;
}
