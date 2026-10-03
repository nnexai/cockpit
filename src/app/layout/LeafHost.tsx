import type { MouseEvent } from "react";
import type { SessionSnapshotResponse } from "../../protocol/generated/v1";
import { PaneChrome } from "./PaneChrome";
import { TerminalLeaf, type TerminalLeafProps } from "./TerminalLeaf";
import { FilesLeaf } from "./FilesLeaf";
import { ReviewLeaf } from "./ReviewLeaf";
import { BrowserLeaf } from "./BrowserLeaf";
import { WidgetDock } from "../widgets/WidgetDock";
import type { Leaf } from "./splitTree";
import type { Rect } from "./solveLayout";
import type { LeafCtx, TabLayoutState } from "./tabLayoutStore";

type Pane = SessionSnapshotResponse["panes"][number];
type Props = {
  ctx: LeafCtx; tab: TabLayoutState; leaf: Leaf; rect: Rect; pane?: Pane;
  selected: boolean; browserInputActive: boolean; browserLiveInputEnabled: boolean;
  terminal: TerminalLeafProps | null;
  focusStatus: "pending" | "error" | null; focusError?: string;
  closeDisabled: boolean;
  widgetInputBlocked: boolean; widgetLive: boolean; widgetLocation?: string;
  onWidgetAgent(paneId: string, tabId: string, spaceId: string): void;
  onSelect(): void; onZoom(): void;
  onClose(): void; onRetryFocus(): void; onMenu(event: MouseEvent<HTMLButtonElement>): void;
};

export function LeafHost(props: Props) {
  const { ctx, tab, leaf, rect, pane, selected, terminal } = props;
  const viewer = leaf.kind === "files" ? tab.viewers.files : leaf.kind === "review" ? tab.viewers.review : undefined;
  const title = leaf.kind === "terminal" ? pane?.title || "Terminal" : leaf.kind === "files" ? viewer?.selector.kind === "files_context" ? "Context" : "Files" : leaf.kind === "review" ? "Review" : "Browser";
  const root = viewer?.context?.roots.find(candidate => candidate.root_id === viewer.context?.default_root_id);
  if (leaf.kind === "widget") return <WidgetDock ctx={ctx} tab={tab} width={rect.width} selected={selected} inputBlocked={props.widgetInputBlocked} live={props.widgetLive} location={props.widgetLocation} onSelect={props.onSelect} onZoom={props.onZoom} onGoToAgent={props.onWidgetAgent} />;
  return <>
    <PaneChrome leaf={leaf} title={title} subtitle={root?.label} selected={selected} zoomed={tab.zoomLeafId === leaf.id}
      lastTerminal={leaf.kind === "terminal" && Object.keys(tab.terminals).length === 1}
      focusStatus={leaf.kind === "terminal" ? props.focusStatus : null} focusError={props.focusError}
      onRetryFocus={props.onRetryFocus} onZoom={props.onZoom} onClose={props.onClose}
      closeDisabled={props.closeDisabled} onMenu={props.onMenu} />
    {leaf.kind === "terminal" && terminal ? <TerminalLeaf key={`${ctx.serverInstance}:${pane?.terminal_id}`} {...terminal} /> : null}
    {leaf.kind === "files" && tab.viewers.files ? <FilesLeaf ctx={ctx} tabId={tab.tabId} slot={tab.viewers.files} selected={selected} onSelect={props.onSelect} /> : null}
    {leaf.kind === "review" && tab.viewers.review ? <ReviewLeaf ctx={ctx} tabId={tab.tabId} slot={tab.viewers.review} selected={selected} onSelect={props.onSelect} /> : null}
    {leaf.kind === "browser" && tab.viewers.browser ? <BrowserLeaf ctx={ctx} tabId={tab.tabId} slot={tab.viewers.browser} rect={rect} selected={selected} inputActive={props.browserInputActive} liveInputEnabled={props.browserLiveInputEnabled} onSelect={props.onSelect} /> : null}
  </>;
}
