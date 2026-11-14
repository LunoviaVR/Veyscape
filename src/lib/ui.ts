// Shell-only UI state (not browser state): find bar, address-bar focus requests, announcements.
import { useSyncExternalStore } from 'react';
import { createStore } from './store';

interface UiState {
  findOpen: boolean;
  /** Incremented to ask the address bar to take focus and select its text. */
  focusAddressBar: number;
}

const uiStore = createStore<UiState>({ findOpen: false, focusAddressBar: 0 });

export function useUi(): UiState {
  return useSyncExternalStore(uiStore.subscribe, uiStore.get);
}

export function patchUi(patch: Partial<UiState>) {
  uiStore.set({ ...uiStore.get(), ...patch });
}

export function requestAddressBarFocus() {
  patchUi({ focusAddressBar: uiStore.get().focusAddressBar + 1 });
}

/** Polite screen-reader announcement via the shell's live region. */
export function announce(message: string) {
  const region = document.getElementById('live-region');
  if (!region) return;
  region.textContent = '';
  // A fresh text node after a tick makes repeated identical messages re-announce.
  window.setTimeout(() => (region.textContent = message), 30);
}
