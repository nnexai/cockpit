import { UiIcon } from "../UiIcon";
import { dispatchFileNavigation } from "../input/fileNavigation";
import { gitActionReason, gitTargetDetail } from "../session/spaceGitActions";
import { ContextMenu } from "./ContextMenu";
import { rendererActionDefinitions, viewerCapability, viewerSourcesDetail } from "./commands";
import { byId } from "./model";
import type { WorkbenchContentProps } from "./WorkbenchContent";

export function ResourceContextMenu(props: WorkbenchContentProps) {
  const { menu, mutationBusy, spaces, spaceGit, gitActions, gitBlocked, dismissMenu, menuAction, beginRename, closeSpace, setTeardownSpaceId, runGitAction, allTabs, closeTab, localLeaves, tabLayout, panes, zoom, viewerSources, state, openViewer, browserReason, openBrowser, setDialog, closeLeaf } = props;
  if (!menu) return null;
  const disabled = mutationBusy;
  if (menu.target.kind === "space") {
    const space = spaces.find((candidate) => candidate.id === menu.target.id);
    if (!space) return null;
    const git = spaceGit.get(space.id);
    const gitPending = gitActions.forStatus(git);
    return <ContextMenu menu={menu} onDismiss={dismissMenu}>
      <button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => beginRename(menu.target))}><UiIcon name="edit" />Rename</button>
      <button role="menuitem" type="button" disabled={disabled} className="destructive" onClick={() => menuAction(() => closeSpace(space))}><UiIcon name="close" />Close</button>
      <button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => setTeardownSpaceId(space.id))}><UiIcon name="trash" />Review task cleanup…</button>
      {git ? <>
        <div className="context-menu-separator" role="presentation" />
        <p className="context-menu-heading" role="presentation">Branch · {git.checkout.state === "branch" ? git.checkout.branch : git.checkout.state === "detached" ? "detached HEAD" : "unavailable"}</p>
        {(["pull", "push"] as const).map(action => {
          const reason = gitActionReason(git, action, gitPending, gitBlocked);
          const detail = gitTargetDetail(space.label, git, action);
          return <button key={action} role="menuitem" type="button" disabled={Boolean(reason)} title={`${detail}${reason ? ` · ${reason}` : ""}`} onClick={() => menuAction(() => runGitAction(space.id, action))}><UiIcon name={action === "pull" ? "down" : "up"} /><span>{action === "pull" ? "Pull (fast-forward only)" : "Push"}</span><small className="context-menu-reason">{reason ?? detail}</small></button>;
        })}
      </> : null}
    </ContextMenu>;
  }
  if (menu.target.kind === "tab") {
    const tab = allTabs.find((candidate) => candidate.id === menu.target.id);
    if (!tab) return null;
    return <ContextMenu menu={menu} onDismiss={dismissMenu}><button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => beginRename(menu.target))}><UiIcon name="edit" />Rename</button><button role="menuitem" type="button" disabled={disabled} className="destructive" onClick={() => menuAction(() => closeTab(tab))}><UiIcon name="close" />Close</button></ContextMenu>;
  }
  const leaf = localLeaves.find(candidate => candidate.id === menu.target.id);
  if (!leaf || !tabLayout) return null;
  const pane = byId(panes, leaf.id);
  return <ContextMenu menu={menu} onDismiss={dismissMenu}>
    <p className="context-menu-heading" role="presentation">Selected pane · {leaf.kind.charAt(0).toUpperCase() + leaf.kind.slice(1)}</p>
    <button role="menuitem" type="button" onClick={() => menuAction(() => zoom(leaf.id))}><UiIcon name="expand" />Expand / restore pane</button>
    {(leaf.kind === "files" || leaf.kind === "review") ? <button role="menuitem" type="button" onClick={() => menuAction(() => dispatchFileNavigation("open-picker"))}><UiIcon name="search" />Go to file…</button> : null}
    <div className="context-menu-separator" role="presentation" />
    <p className="context-menu-heading" role="presentation">Open view</p>
    {rendererActionDefinitions.map(({ id, icon, kind }) => <button key={id} role="menuitem" type="button" disabled={disabled || !viewerCapability(kind, viewerSources) || state.sync !== "live"} title={viewerSources.status === "pending" ? "Loading viewer sources" : viewerSourcesDetail(viewerSources)} onClick={() => menuAction(() => openViewer(kind))}><UiIcon name={icon} /><span>{kind === "review" ? "Review" : kind === "files" ? "Files" : "Context"}</span>{kind === "context" && !viewerCapability(kind, viewerSources) ? <small className="context-menu-reason">no Space context</small> : null}</button>)}
    <button role="menuitem" type="button" disabled={Boolean(browserReason)} title={browserReason ?? undefined} onClick={() => menuAction(openBrowser)}><UiIcon name="browser" />Browser</button>
    <div className="context-menu-separator" role="presentation" />
    {pane ? <button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => beginRename(menu.target))}><UiIcon name="edit" />Rename pane…</button> : null}
    <button role="menuitem" type="button" aria-haspopup="dialog" disabled={localLeaves.length < 2} onClick={() => menuAction(() => setDialog({ kind: "swap", paneId: leaf.id }))}><UiIcon name="refresh" /><span>Swap with…</span><span className="context-menu-chevron"><UiIcon name="right" /></span></button>
    {pane ? <button role="menuitem" type="button" aria-haspopup="dialog" disabled={disabled} onClick={() => menuAction(() => setDialog({ kind: "move", paneId: pane.id }))}><UiIcon name="forward" /><span>Move to…</span><span className="context-menu-chevron"><UiIcon name="right" /></span></button> : null}
    <div className="context-menu-separator" role="presentation" />
    <button role="menuitem" type="button" disabled={disabled} className="destructive" onClick={() => menuAction(() => closeLeaf(leaf.id))}><UiIcon name="close" />Close pane</button>
  </ContextMenu>;
}
