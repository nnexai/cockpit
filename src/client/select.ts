import { isTauri } from "@tauri-apps/api/core";
import {
  createBrowserClient,
  type BrowserFetch,
  type BrowserWebSocketFactory,
} from "./browser";
import {
  createNativeClient,
  type NativeChannelFactory,
  type NativeInvoke,
} from "./native";
import type { CockpitClient } from "./CockpitClient";

export interface ClientSelectionDependencies {
  isNative?: () => boolean | Promise<boolean>;
  createBrowser?: (request?: BrowserFetch, webSocketFactory?: BrowserWebSocketFactory) => CockpitClient;
  createNative?: (invoke?: NativeInvoke, channelFactory?: NativeChannelFactory) => CockpitClient;
  request?: BrowserFetch;
  invoke?: NativeInvoke;
  webSocketFactory?: BrowserWebSocketFactory;
  channelFactory?: NativeChannelFactory;
}

/**
 * Build a selector whose result is memoized after the first startup decision.
 * Tests can inject host detection and transport factories without touching globals.
 */
export function createClientSelector(
  dependencies: ClientSelectionDependencies = {},
): () => Promise<CockpitClient> {
  const detectNative = dependencies.isNative ?? (() => isTauri());
  const browserFactory = dependencies.createBrowser ?? createBrowserClient;
  const nativeFactory = dependencies.createNative ?? createNativeClient;
  let selected: Promise<CockpitClient> | undefined;

  return async function select(): Promise<CockpitClient> {
    if (selected === undefined) {
      selected = Promise.resolve(detectNative()).then((native) =>
        native
          ? nativeFactory(dependencies.invoke, dependencies.channelFactory)
          : browserFactory(dependencies.request, dependencies.webSocketFactory),
      );
    }
    return selected;
  };
}

/** The sole application-startup selection used by the shared frontend. */
export const selectClient = createClientSelector();
