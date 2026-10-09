import type { HerdrCommand, SpaceGitAction, SpaceGitStatus, ViewerSourceOptions, WidgetSummary } from "../../protocol/generated/v1";
import type { UiIconName } from "../UiIcon";
import { herdrCommandShortcut, type HerdrChord } from "../input/herdrBindings";
import { SHORTCUTS, formatShortcut, shortcutEntry, type LocalShortcutId, type PrefixCommand } from "../input/shortcuts";
import { gitActionReason, gitTargetDetail, type GitActionState } from "../session/spaceGitActions";
import type { Space } from "../sidebar/spaceTree";
import { widgetKey } from "../widgets/widgetStore";
import type { WorkbenchView } from "./model";

export type CommandGroup = "Navigate" | "Space" | "Tab" | "Pane" | "Browser" | "Library" | "Herdr";
export type CommandAction = { id: string; label: string; icon?: UiIconName; shortcut?: string; group: CommandGroup; primary?: boolean; disabled?: boolean; reason?: string; reasonDetail?: string; run: () => void };
export type RendererKind = "review" | "files" | "context";
export type RendererActionDefinition = { id: string; label: string; icon: UiIconName; kind: RendererKind; primary?: true };
export const rendererActionDefinitions: RendererActionDefinition[] = [
  { id: "review", label: "Open Review", icon: "file", kind: "review", primary: true },
  { id: "files", label: "Open Files", icon: "folder", kind: "files" },
  { id: "context", label: "Open Context", icon: "library", kind: "context" },
];

export type ViewerSourcesState =
  | { status: "pending"; source: boolean }
  | { status: "failed"; message: string }
  | { status: "ready"; options: ViewerSourceOptions };

export function viewerCapability(kind: RendererKind, state: ViewerSourcesState): boolean {
  if (state.status !== "ready") return false;
  if (kind === "review") return state.options.review_repository_ids.length > 0;
  return kind === "files" ? state.options.files_folder_root_id !== null : state.options.files_context_root_id !== null;
}

export function viewerSourcesDetail(state: ViewerSourcesState): string {
  if (state.status === "ready") return state.options.reason;
  if (state.status === "failed") return state.message;
  return state.source ? "Loading viewer sources" : "No terminal in this tab";
}

export type RendererAvailability = { enabled: boolean; reason: string; detail: string };
export function rendererAvailability(kind: RendererKind, state: ViewerSourcesState, live: boolean, busy: boolean): RendererAvailability {
  const capability = viewerCapability(kind, state);
  const detail = viewerSourcesDetail(state);
  return {
    enabled: !busy && live && capability,
    reason: !live ? "Herdr is not live" : kind === "context" && !capability ? "Library context unavailable" : detail,
    detail,
  };
}

export interface CommandInput {
  herdr: { commands: readonly HerdrCommand[]; prefixes: readonly HerdrChord[]; reason: string | undefined; popupOpen: boolean };
  view: Pick<WorkbenchView, "spaces" | "allTabs" | "selectedSpace" | "selectedTab" | "selectedLeaf" | "selectedPane" | "live" | "busy">;
  libraryOpen: boolean;
  notesOpen: boolean;
  browser: { open: boolean; reason: string | null };
  git: { target: Space | undefined; status: SpaceGitStatus | undefined; pending: GitActionState | undefined; blocked: string | undefined };
  widgets: { pending: boolean; dockTabId: string | null; list: readonly WidgetSummary[] };
  viewerSources: ViewerSourcesState;
  local: Partial<Record<LocalShortcutId, () => void>>;
  run: {
    herdr(command: HerdrCommand): void;
    prefix(command: PrefixCommand): void;
    supervisor(start: boolean): void;
    git(spaceId: string, action: SpaceGitAction): void;
    recovery(): void;
    toggleNotes(): void;
    openBrowser(): void;
    closeBrowser(): void;
    retryBrowserCleanup(): void;
    showWidgets(): void;
    cycleWidget(tabId: string, step: 1 | -1): void;
    removeWidget(tabId: string): void;
    goToWidget(widget: WidgetSummary): void;
    libraryAdd(): void;
    library(command: { kind: "refresh" } | { kind: "tokens" }): void;
    openViewer(kind: RendererKind): void;
  };
}

export function buildCommands(input: CommandInput): CommandAction[] {
  const { herdr, view, libraryOpen, notesOpen, browser, git, widgets, viewerSources, local, run } = input;
  const { spaces, allTabs, selectedSpace, selectedTab, selectedLeaf, selectedPane, live, busy } = view;
  const dockTabId = widgets.dockTabId;
  return [
    ...herdr.commands.filter(command => command.action !== "unknown").map((command): CommandAction => ({
      id: `herdr:${command.command_id}`, label: command.description ? command.description.charAt(0).toUpperCase() + command.description.slice(1) : "Custom command", icon: command.action === "popup" ? "more" : command.action === "plugin_action" ? "file" : "terminal", shortcut: herdrCommandShortcut(command, herdr.prefixes), group: "Herdr",
      disabled: Boolean(herdr.reason || herdr.popupOpen), reason: herdr.reason, run: () => run.herdr(command),
    })),
    { id: "show-supervisor", label: shortcutEntry("show-supervisor").label, primary: shortcutEntry("show-supervisor").primary, group: "Navigate", run: () => run.supervisor(false) },
    { id: "start-supervisor", label: shortcutEntry("start-supervisor").label, primary: shortcutEntry("start-supervisor").primary, group: "Navigate", run: () => run.supervisor(true) },
    ...SHORTCUTS.filter(entry => entry.palette === undefined).flatMap((entry): CommandAction[] => {
      if (!entry.prefix) {
        const handler = local[entry.id as LocalShortcutId];
        return handler ? [{ id: entry.id, label: entry.label, primary: entry.primary, group: entry.group, run: handler }] : [];
      }
      const command = entry.id as PrefixCommand;
      const reason = entry.needs === "space" && !selectedSpace ? "Select a Space first"
        : entry.needs === "tab" && !selectedTab ? "Select a tab first"
        : entry.needs === "pane" && !selectedLeaf ? "Select a pane first"
        : command === "rename-pane" && !selectedPane ? "Terminals only"
        : command === "setup-space" && !live ? "Herdr is not live"
        : command === "toggle-browser" ? browser.open ? undefined : browser.reason ?? undefined : undefined;
      return [{
        id: `prefix:${command}`, label: command === "toggle-library" && libraryOpen ? "Close Library" : entry.label, primary: entry.primary, shortcut: formatShortcut(command), group: entry.group,
        disabled: reason !== undefined, reason, run: () => run.prefix(command),
      }];
    }),
    ...(["pull-space", "push-space"] as const).map((id): CommandAction => {
      const entry = shortcutEntry(id);
      const action = id === "pull-space" ? "pull" : "push";
      const { target, status, pending, blocked } = git;
      const reason = !target ? "Select a Space first" : gitActionReason(status, action, pending, blocked);
      const detail = target ? gitTargetDetail(target.label, status, action) : undefined;
      return { id: entry.id, label: entry.label, primary: entry.primary, group: entry.group, icon: action === "pull" ? "down" : "up", shortcut: formatShortcut(id), disabled: Boolean(reason), reason: reason ?? detail, reasonDetail: detail ? `${detail}${reason ? ` · ${reason}` : ""}` : reason, run: () => { if (target) run.git(target.id, action); } };
    }),
    { id: "recovery:cleanup", label: "Recover task cleanup…", group: "Navigate", run: run.recovery },
    { id: "notes:toggle", label: notesOpen ? "Close Notes" : "Open Notes", icon: "file", primary: true, group: "Navigate", disabled: !selectedSpace, reason: !selectedSpace ? "Select a Space first" : undefined, run: run.toggleNotes },
    { id: "browser:open", label: "Open Browser", primary: true, shortcut: formatShortcut("toggle-browser"), group: "Browser", disabled: Boolean(browser.reason), reason: browser.reason ?? undefined, run: run.openBrowser },
    { id: "browser:close", label: "Close browser", shortcut: formatShortcut("toggle-browser"), group: "Browser", disabled: !browser.open, reason: !browser.open ? "No browser in this tab" : undefined, run: run.closeBrowser },
    { id: "browser:cleanup", label: "Retry browser cleanup", group: "Browser", run: run.retryBrowserCleanup },
    ...(widgets.pending ? [{ id: "show-widgets", label: shortcutEntry("show-widgets").label, group: "Pane" as const, run: run.showWidgets }] : []),
    ...(dockTabId ? [
      { id: "next-widget", label: shortcutEntry("next-widget").label, group: "Pane" as const, run: () => run.cycleWidget(dockTabId, 1) },
      { id: "previous-widget", label: shortcutEntry("previous-widget").label, group: "Pane" as const, run: () => run.cycleWidget(dockTabId, -1) },
      { id: "remove-widget", label: shortcutEntry("remove-widget").label, group: "Pane" as const, run: () => run.removeWidget(dockTabId) },
    ] : []),
    ...widgets.list.map(widget => ({
      id: `widget:${widgetKey(widget.key)}`, label: `Go to widget: ${widget.title} — ${spaces.find(space => space.id === widget.space_id)?.label ?? "Space"} · ${allTabs.find(tab => tab.id === widget.key.tab_id)?.label ?? "tab"}`, group: "Navigate" as const,
      run: () => run.goToWidget(widget),
    })),
    { id: "library:add", label: "Add to Library…", group: "Library", run: run.libraryAdd },
    { id: "library:refresh", label: "Refresh Library", group: "Library", run: () => run.library({ kind: "refresh" }) },
    { id: "library:tokens", label: "Provider tokens…", group: "Library", run: () => run.library({ kind: "tokens" }) },
    ...rendererActionDefinitions.map(({ id, label, icon, kind, primary }): CommandAction => {
      const availability = rendererAvailability(kind, viewerSources, live, busy);
      return { id: `renderer:${id}`, label, icon, primary, group: "Pane", disabled: !availability.enabled, reason: availability.reason, reasonDetail: availability.detail, run: () => run.openViewer(kind) };
    }),
  ];
}
