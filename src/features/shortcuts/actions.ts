// The single executor for browser actions. Keyboard shortcuts (shell or page focus), native
// menu items and toolbar buttons all end up here, so they always behave identically.
import { api, errorMessage } from '../../lib/ipc';
import { activeTabId, browserStore, settingsStore } from '../../lib/store';
import { announce, patchUi, requestAddressBarFocus } from '../../lib/ui';

const INTERNAL: Record<string, string> = {
  openHistory: 'veyscape://history',
  openDownloads: 'veyscape://downloads',
  openBookmarks: 'veyscape://bookmarks',
  openSettings: 'veyscape://settings',
  openAbout: 'veyscape://settings/about',
};

/** Switches to an open internal page if there is one, otherwise opens it in a new tab. */
export async function openInternal(url: string) {
  const page = (u: string) => u.replace('veyscape://', '').split(/[/?#]/)[0];
  const existing = browserStore.get().tabs.find((t) => t.internal && !t.private && page(t.url) === page(url));
  if (!existing) return void (await api.tabs.create({ url }));
  await api.tabs.activate(existing.id);
  if (existing.url !== url) await api.nav.go(existing.id, url);
}

export async function runAction(action: string): Promise<void> {
  const id = activeTabId();
  try {
    if (action in INTERNAL) return await openInternal(INTERNAL[action]!);
    const select = /^selectTab(\d)$/.exec(action);
    if (select) return await api.tabs.activateTarget(`#${Number(select[1]) - 1}`);
    switch (action) {
      case 'newTab':
        await api.tabs.create();
        return requestAddressBarFocus();
      case 'newWindow':
        return await api.windows.create();
      case 'newPrivateWindow':
        return await api.windows.create(true);
      case 'newPrivateTab':
        await api.tabs.create({ private: true });
        announce('Private tab opened');
        return requestAddressBarFocus();
      case 'reopenClosedTab':
        return await api.tabs.reopenClosed();
      case 'nextTab':
        return await api.tabs.activateTarget('next');
      case 'previousTab':
        return await api.tabs.activateTarget('previous');
      case 'selectLastTab':
        return await api.tabs.activateTarget('last');
      case 'focusAddressBar':
        return requestAddressBarFocus();
      case 'findInPage':
        return patchUi({ findOpen: true });
      case 'toggleBookmarkBar': {
        const s = settingsStore.get();
        if (s) settingsStore.set(await api.settings.update({ ...s, showBookmarkBar: !s.showBookmarkBar }));
        return;
      }
      case 'toggleFullscreen':
        return await api.layout.update(document.getElementById('chrome')?.getBoundingClientRect().height ?? 0, 'toggleFullscreen');
    }
    if (id == null) return;
    switch (action) {
      case 'closeTab':
        return await api.tabs.close(id);
      case 'reload':
        return await api.nav.reload(id);
      case 'goBack':
        return await api.nav.back(id);
      case 'goForward':
        return await api.nav.forward(id);
      case 'goHome':
        return await api.nav.go(id, settingsStore.get()?.homepage || 'veyscape://newtab');
      case 'zoomIn':
        return await api.nav.zoom(id, 1);
      case 'zoomOut':
        return await api.nav.zoom(id, -1);
      case 'zoomReset':
        return await api.nav.zoom(id, 0);
      case 'print':
        return await api.nav.print(id);
      case 'bookmarkPage': {
        const saved = await api.library.toggleBookmark(id);
        return announce(saved ? 'Bookmark added' : 'Bookmark removed');
      }
    }
  } catch (e) {
    announce(errorMessage(e));
  }
}
