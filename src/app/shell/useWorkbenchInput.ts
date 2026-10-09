import { useCallback, useEffect } from "react";
import type { HerdrCommand } from "../../protocol/generated/v1";
import { focusSelectedDivider } from "../layout/TabCanvas";
import { dispatchFileNavigation } from "../input/fileNavigation";
import { focusSidebarList, shortcutEntry, type PrefixCommand } from "../input/shortcuts";
import { routeWorkbenchKeydown, type WorkbenchKeyEvent } from "../input/keymap";
import { byId } from "./model";
import { acknowledgeWidgetPrefix } from "./useWorkbenchUI";
import type { WorkbenchProps } from "./Workbench";
import type { WorkbenchState } from "./useWorkbenchState";
import type { WorkbenchActions, PaneFocusDirection } from "./useWorkbenchActions";
export interface WorkbenchInput {
  customCommandReason: string | undefined; runCommand(command: PrefixCommand): void; runHerdrCommand(command: HerdrCommand): void;
}
export function useWorkbenchInput(props: WorkbenchProps, runtime: WorkbenchState, actions: WorkbenchActions): WorkbenchInput {
  const { state, selection, tabLayout, onMutate, onSplit } = props;
  const { spaces, tabs, panes, snapshot, localLeaves, selectedLeaf, modalOpen, mutationBusy, libraryOpen, openLibrary, closeLibrary, openSessionChooser, canvasRef, view, exitForPaneCommand, setPrefixHint, setSetupOpen, setCommandsOpen, runGitAction, narrowViewport, drawerOpen, sidebarCollapsed, openDrawer, toggleSidebarCollapsed, toggleSidebar, shell, popup, popupPending, setPopupPending, setCommandNotice, prefixActive, armedPrefix, customPrefixes, customBindings, setArmedPrefix, setPrefixActive } = runtime;
  const { beginRename, closeSpace, closeTab, closeLeaf, zoom, focusTab, selectLeaf, neighbour, swap, toggleBrowser } = actions;
  const runCommand = (command: PrefixCommand) => {
    setPrefixHint(null);
    const space = byId(spaces, selection.spaceId);
    const tab = byId(tabs, selection.tabId);
    const pane = byId(panes, selection.paneId);
    const execute = () => {
      if (command === "new-space") onMutate("space:new", { type: "space_create", label: null, cwd: null }, true);
      if (command === "setup-space" && state.sync === "live") setSetupOpen(true);
      if (command === "rename-space" && space) beginRename({ kind: "space", id: space.id });
      if (command === "close-space") closeSpace(space);
      if (command === "pull-space" || command === "push-space") {
        const target = byId(spaces, state.focusPending ? snapshot?.focused_space_id ?? null : selection.spaceId);
        if (target) runGitAction(target.id, command === "pull-space" ? "pull" : "push");
      }
      if (command === "new-tab" && selection.spaceId) onMutate("tab:new", { type: "tab_create", space_id: selection.spaceId, label: null }, true);
      if (command === "rename-tab" && tab) beginRename({ kind: "tab", id: tab.id });
      if (command === "close-tab") closeTab(tab);
      if (command === "split-right" && tabLayout && selectedLeaf) onSplit(tabLayout.tabId, selectedLeaf.id, "right");
      if (command === "split-down" && tabLayout && selectedLeaf) onSplit(tabLayout.tabId, selectedLeaf.id, "down");
      if (command === "close-pane" && selectedLeaf) closeLeaf(selectedLeaf.id);
      if (command === "zoom-pane" && selectedLeaf) zoom(selectedLeaf.id);
      if (command === "rename-pane" && pane) beginRename({ kind: "pane", id: pane.id });
      if (command === "previous-tab" && tab) { const index = tabs.indexOf(tab); if (index > 0) focusTab(tabs[index - 1]); }
      if (command === "next-tab" && tab) { const index = tabs.indexOf(tab); if (index >= 0 && index < tabs.length - 1) focusTab(tabs[index + 1]); }
      if (command.startsWith("select-tab-")) { const target = tabs[Number(command.slice("select-tab-".length)) - 1]; if (target) focusTab(target); }
      if ((command === "previous-pane" || command === "next-pane") && selectedLeaf) {
        const index = localLeaves.findIndex(leaf => leaf.id === selectedLeaf.id);
        const offset = command === "next-pane" ? 1 : -1;
        selectLeaf(localLeaves[(index + offset + localLeaves.length) % localLeaves.length].id);
      }
      if (["focus-left", "focus-right", "focus-up", "focus-down"].includes(command)) {
        const target = neighbour(command.slice("focus-".length) as PaneFocusDirection);
        if (target) selectLeaf(target);
      }
      if (["swap-left", "swap-right", "swap-up", "swap-down"].includes(command) && selectedLeaf) {
        const target = neighbour(command.slice("swap-".length) as PaneFocusDirection);
        if (target) swap(selectedLeaf.id, target);
      }
      if (command === "resize" && !mutationBusy && canvasRef.current && selection.paneId) focusSelectedDivider(canvasRef.current, selection.paneId);
      if (command === "open-file-picker") dispatchFileNavigation("open-picker");
      if (command === "switch-session") openSessionChooser();
      if (command === "toggle-browser") toggleBrowser();
      if (command === "toggle-library") { if (libraryOpen) closeLibrary(); else openLibrary(); }
      if (command === "toggle-sidebar") toggleSidebar();
      if (command === "focus-spaces" || command === "focus-agents") {
        const list = command === "focus-spaces" ? "spaces" : "agents";
        const visible = narrowViewport ? drawerOpen : !sidebarCollapsed;
        if (!visible) { if (narrowViewport) openDrawer(); else toggleSidebarCollapsed(); }
        // The sidebar may still be mounting (and the drawer focuses its close button first): retry until a row holds focus.
        const attempt = (remaining: number) => {
          focusSidebarList(list);
          const focused = document.activeElement;
          if (remaining > 0 && !(focused?.closest("#cockpit-sidebar") && !focused.matches(".sidebar-close"))) window.setTimeout(() => attempt(remaining - 1), 30);
        };
        window.setTimeout(() => attempt(20), visible ? 0 : 60);
      }
    };
    if (command === "help") { setCommandsOpen(true); return; }
    if (mutationBusy && !shortcutEntry(command).allowWhileBusy) return;
    if (shortcutEntry(command).paneScoped && view.kind !== "terminal") { exitForPaneCommand(execute); return; }
    execute();
  };
  const customCommandReason = state.sync !== "live" || shell?.status !== "live" ? shell?.error ?? "Herdr commands are not live"
    : state.focusPending || state.focusError || !snapshot?.focused_space_id || !snapshot.focused_tab_id ? "Waiting for Herdr focus"
    : mutationBusy ? "Another Herdr action is pending" : undefined;
  const runHerdrCommand = useCallback((command: HerdrCommand) => {
    setPrefixHint(null);
    if (customCommandReason || popup) { setCommandNotice(customCommandReason ?? "The popup owns keyboard input"); return; }
    if (!shell?.commands.some(candidate => candidate.command_id === command.command_id)) { setCommandNotice("Custom command is not available on this endpoint; reload configuration"); return; }
    const accepted = onMutate(`command:${command.command_id}`, { type: "command_invoke", command_id: command.command_id, space_id: snapshot!.focused_space_id!, tab_id: snapshot!.focused_tab_id!, pane_id: snapshot!.focused_pane_id }, false);
    if (accepted) {
      setCommandNotice(null);
      if (command.action === "popup") setPopupPending(command.command_id);
    }
  }, [customCommandReason, popup, shell, snapshot, onMutate]);
  useEffect(() => {
    const keydown = (event: WorkbenchKeyEvent) => {
      routeWorkbenchKeydown(event, { modalOpen, serverModalOpen: Boolean(popup), popupPending: Boolean(popupPending), prefixActive, prefixOrigin: armedPrefix.origin, herdrPrefixes: customPrefixes, onPrefixArm: (origin, label) => setArmedPrefix({ origin, label }), runCommand, herdrBindings: customBindings, runHerdrCommand, setPrefixActive: active => { setPrefixActive(active); acknowledgeWidgetPrefix(active, event.target); }, setCommandsOpen, onUnboundPrefixKey: setPrefixHint });
    };
    const widgetShortcut = (event: Event) => {
      if (!(event instanceof CustomEvent)) return;
      const detail: unknown = event.detail;
      if (typeof detail !== "object" || detail === null || Array.isArray(detail)) return;
      const payload = detail as Record<string, unknown>;
      const frame = payload.target;
      if (!(frame instanceof HTMLIFrameElement) || !frame.isConnected || document.activeElement !== frame
        || !frame.matches(".widget-frame:not([data-pending])") || !frame.closest("[data-widget-tab]")
        || frame.closest('[inert], [aria-hidden="true"], [data-input-blocked="true"]')
        || document.body.classList.contains("is-pane-dragging")
        || typeof payload.key !== "string" || payload.key.length === 0 || payload.key.length > 64
        || typeof payload.code !== "string" || payload.code.length > 64
        || typeof payload.ctrlKey !== "boolean" || typeof payload.altKey !== "boolean"
        || typeof payload.shiftKey !== "boolean" || typeof payload.metaKey !== "boolean"
        || typeof payload.repeat !== "boolean" || payload.metaKey
        || (!payload.ctrlKey && !payload.altKey && !prefixActive)) return;
      // The frame bridge has authenticated this shortcut. Route only its typed
      // fields; never synthesize a DOM keydown or forward text to a terminal.
      keydown({
        key: payload.key, code: payload.code, ctrlKey: payload.ctrlKey, altKey: payload.altKey,
        shiftKey: payload.shiftKey, metaKey: false, repeat: payload.repeat, target: frame,
        isComposing: false, preventDefault() {}, stopPropagation() {},
      });
    };
    window.addEventListener("keydown", keydown, true);
    window.addEventListener("cockpit-widget-shortcut", widgetShortcut);
    return () => {
      window.removeEventListener("keydown", keydown, true);
      window.removeEventListener("cockpit-widget-shortcut", widgetShortcut);
    };
  }, [prefixActive, armedPrefix, runCommand, modalOpen, popup, popupPending, shell, runHerdrCommand]);
  return { customCommandReason, runCommand, runHerdrCommand };
}
