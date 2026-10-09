import { useRef } from "react";
import type { BrowserViewCommand } from "../../protocol/generated/v1";
import { addressBarUrl, context, errorMessage, navigationUrl } from "./browserPaneModel";
import type { BrowserPaneState } from "./useBrowserPaneState";
import type { BrowserViewCommandApi } from "./useBrowserViewCommand";
import type { BrowserInputQueue } from "./useBrowserInputQueue";
import type { BrowserDrafts } from "./useBrowserDrafts";

export interface BrowserNavigationApi {
  navigation(action: "back" | "forward" | "reload" | "stop" | "navigate", address?: string): void;
  tabCommand(value: Extract<BrowserViewCommand, { type: "tab" }>): void;
  reconnectView(): Promise<void>;
}

export function useBrowserNavigation(state: BrowserPaneState, view: BrowserViewCommandApi, input: BrowserInputQueue, drafts: BrowserDrafts, onReconnect: (() => void | Promise<void>) | undefined): BrowserNavigationApi {
  const { liveInputEnabledRef, hoverInspectTimerRef, inspectRequestRef, elementIntentRef, snapshotRef, urlEditing, draftRequestRef, gestureRef, editorDirtyRef, draftRef, associationOwner, pendingCaptureRef, setUrl, setSelectedId, setInspection, setMessage, setRetry, setStatus } = state;
  const { command, ensureControl } = view;
  const { enqueueInput, releaseRemotePointer } = input;
  const { persistEditor } = drafts;
  const navigationRequestRef = useRef(0);
  const navigation = (action: "back" | "forward" | "reload" | "stop" | "navigate", address?: string): void => {
    if (!liveInputEnabledRef.current) return;
    const value = action === "navigate" ? { type: "navigate" as const, url: navigationUrl(address ?? "") } : { type: action };
    if (hoverInspectTimerRef.current !== null) window.clearTimeout(hoverInspectTimerRef.current);
    hoverInspectTimerRef.current = null;
    ++inspectRequestRef.current;
    elementIntentRef.current = null;
    const attempt = ++navigationRequestRef.current;
    if (value.type === "navigate") setUrl(addressBarUrl(snapshotRef.current?.navigation?.url));
    urlEditing.current = false;
    draftRequestRef.current += 1;
    setSelectedId(null);
    setInspection(null);
    gestureRef.current = null;
    void enqueueInput("boundary", async () => {
      if (editorDirtyRef.current && draftRef.current) {
        try {
          await persistEditor();
        } catch (error) {
          setMessage(`Could not preserve browser editor changes before navigation: ${errorMessage(error)}`);
          return;
        }
      }
      const controlled = await ensureControl(); const current = controlled ? snapshotRef.current : null; const documentContext = current ? context(current) : null;
      if (!documentContext) return;
      const outcome = await command({ type: "navigation", context: documentContext, command: value });
      if (!outcome && navigationRequestRef.current === attempt) setUrl(addressBarUrl(snapshotRef.current?.navigation?.url));
    });
  };
  const tabCommand = (commandValue: Extract<BrowserViewCommand, { type: "tab" }>) => {
    if (!liveInputEnabledRef.current) return;
    void releaseRemotePointer();
    void enqueueInput("boundary", async () => {
      try {
        if (editorDirtyRef.current && draftRef.current) await persistEditor();
        await associationOwner.mutationTail;
        if (associationOwner.pendingAnnotationMutations.length || pendingCaptureRef.current) {
          setMessage("Save or resolve retained browser annotations before switching tabs.");
          return;
        }
      } catch (error) {
        setMessage(`Could not preserve browser editor changes before switching tabs: ${errorMessage(error)}`);
        return;
      }
      if (await ensureControl()) await command(commandValue);
    });
  };
  const reconnectView = async (): Promise<void> => {
    try {
      await onReconnect?.();
      setRetry((value) => value + 1);
    } catch (error) {
      setStatus("error");
      setMessage(`Browser reconnect failed: ${errorMessage(error)}`);
    }
  };
  return { navigation, tabCommand, reconnectView };
}
