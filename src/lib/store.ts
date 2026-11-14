// Read-only mirror of backend state. Rust owns browser state; the UI never mutates it locally,
// it calls a command and waits for the next `browser:state` event.
import { useSyncExternalStore } from 'react';
import type { BrowserState, DownloadItem, Settings, Tab, TabId } from '../types/browser';
import { api, onEvent } from './ipc';

export function createStore<T>(initial: T) {
  let value = initial;
  const listeners = new Set<() => void>();
  return {
    get: () => value,
    set(next: T) {
      if (next === value) return;
      value = next;
      listeners.forEach((l) => l());
    },
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
}

const emptyState: BrowserState = {
  privateWindow: false,
  tabs: [],
  activeTabId: null,
  canReopenClosed: false,
  prompts: [],
  fullscreen: false,
  groups: [],
  split: null,
};

export const browserStore = createStore<BrowserState>(emptyState);
export const faviconStore = createStore<Record<TabId, string>>({});
export const downloadsStore = createStore<DownloadItem[]>([]);
export const settingsStore = createStore<Settings | null>(null);

export function useBrowserState(): BrowserState {
  return useSyncExternalStore(browserStore.subscribe, browserStore.get);
}

export function useActiveTab(): Tab | undefined {
  const s = useBrowserState();
  return s.tabs.find((t) => t.id === s.activeTabId);
}

export function useFavicons(): Record<TabId, string> {
  return useSyncExternalStore(faviconStore.subscribe, faviconStore.get);
}

export function useDownloads(): DownloadItem[] {
  return useSyncExternalStore(downloadsStore.subscribe, downloadsStore.get);
}

export function useSettings(): Settings | null {
  return useSyncExternalStore(settingsStore.subscribe, settingsStore.get);
}

export function activeTabId(): TabId | null {
  return browserStore.get().activeTabId;
}

/** Subscribes to backend events and loads the initial snapshot (retrying while the core starts). */
export async function connectShellStores(): Promise<void> {
  await Promise.all([
    onEvent('browser:state', (s) => browserStore.set(s)),
    onEvent('browser:favicon', ({ tabId, dataUrl }) => {
      const rest = Object.fromEntries(Object.entries(faviconStore.get()).filter(([id]) => Number(id) !== tabId));
      faviconStore.set(dataUrl ? { ...rest, [tabId]: dataUrl } : rest);
    }),
    onEvent('downloads:state', (d) => downloadsStore.set(d)),
    onEvent('settings:changed', (s) => settingsStore.set(s)),
  ]);
  // An event that arrived while a fetch was in flight is newer than the fetch: keep it.
  const settings = await api.settings.get();
  if (!settingsStore.get()) settingsStore.set(settings);
  const initialDownloads = downloadsStore.get();
  for (let attempt = 0; ; attempt++) {
    try {
      const init = await api.snapshot();
      const icons = Object.fromEntries(Object.entries(init.favicons).map(([k, v]) => [Number(k), v]));
      faviconStore.set({ ...icons, ...faviconStore.get() });
      if (downloadsStore.get() === initialDownloads) downloadsStore.set(init.downloads);
      if (browserStore.get() === emptyState) browserStore.set(init.state);
      return;
    } catch (e) {
      if (attempt > 50) throw e;
      await new Promise((r) => setTimeout(r, 50));
    }
  }
}
