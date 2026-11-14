// Typed wrappers for every backend command and event. The only module that calls `invoke`.
import { invoke, type InvokeArgs } from '@tauri-apps/api/core';
import { emitTo, type UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import type {
  AppInfo,
  Bookmark,
  ClearDataRequest,
  ImportSource,
  ImportSummary,
  DownloadItem,
  FilterList,
  HistoryEntry,
  InitialState,
  Login,
  ProtectionStatus,
  Settings,
  SitePermission,
  SplitLayout,
  Suggestions,
  TabId,
} from '../types/browser';

/** Commands whose result is irrelevant (they report errors by rejecting). */
const run = (cmd: string, args?: InvokeArgs): Promise<void> => invoke(cmd, args).then(() => undefined);

export const api = {
  snapshot: () => invoke<InitialState>('browser_snapshot'),

  tabs: {
    create: (opts: { url?: string; private?: boolean; background?: boolean } = {}) => invoke<TabId>('tab_create', opts),
    close: (id: TabId) => run('tab_close', { id }),
    activate: (id: TabId) => run('tab_activate', { id }),
    activateTarget: (target: 'next' | 'previous' | 'last' | `#${number}`) => run('tab_activate', { target }),
    move: (id: TabId, index: number) => run('tab_move', { id, index }),
    /** Registers the tab a tab strip starts dragging; only that tab can be dropped. */
    dragStart: (id: TabId) => run('tab_drag_start', { id }),
    /** Drops the dragged tab (from any window) on this window's strip; `group`: joins it. */
    drop: (index: number, group: number | null) => run('tab_drop', { index, group }),
    /** Moves the tab (page as it is) to a new window at screen point x, y (logical px). */
    /** `follow`: still dragging; the new window follows the pointer until the drop. */
    tearOff: (id: TabId, x: number, y: number, follow = false) => run('tab_tear_off', { id, x, y, follow }),
    reopenClosed: () => run('tab_reopen_closed'),
    contextMenu: (id: TabId, info: TabMenuInfo, x: number, y: number) => run('tab_context_menu', { id, info, x, y }),
  },

  groups: {
    /** `icon: ''` removes the group's icon. */
    update: (group: number, patch: { title?: string; color?: string; collapsed?: boolean; icon?: string }) =>
      run('tab_group_update', { group, ...patch }),
    menu: (id: number, collapsed: boolean, x: number, y: number) => run('tab_group_menu', { id, collapsed, x, y }),
  },

  windows: {
    /** `private`: a private window, where every tab is private. */
    create: (privateWindow = false) => run('window_new', { private: privateWindow }),
  },

  split: {
    set: (count: number, layout: SplitLayout) => run('split_set', { count, layout }),
    close: () => run('split_close'),
  },

  nav: {
    go: (id: TabId, input: string) => run('nav_go', { id, input }),
    back: (id: TabId) => run('nav_back', { id }),
    forward: (id: TabId) => run('nav_forward', { id }),
    reload: (id: TabId) => run('nav_reload', { id }),
    stop: (id: TabId) => run('nav_stop', { id }),
    zoom: (id: TabId, step: -1 | 0 | 1) => run('zoom_change', { id, step }),
    find: (text: string, forward: boolean) => run('find_in_page', { text, forward }),
    findStop: () => run('find_stop'),
    print: (id: TabId) => run('print_page', { id }),
    focusContent: () => run('focus_content'),
    httpsContinue: (id: TabId) => run('https_continue', { id }),
  },

  layout: {
    update: (chromeHeight: number, action?: 'toggleFullscreen') => run('layout_update', { chromeHeight, action }),
    /** `refocusShell`: give keyboard focus back to the shell (after a click in the overlay). */
    overlayShow: (r: { x: number; y: number; width: number; height: number }, focus: boolean, refocusShell = false) =>
      invoke('overlay_show', { ...r, focus, refocusShell }),
    overlayHide: (refocusShell: boolean) => run('overlay_hide', { refocusShell }),
    mainMenu: (canReopen: boolean, x: number, y: number) => run('menu_open_main', { canReopen, x, y }),
  },

  prompts: {
    respond: (id: number, allow: boolean, remember: boolean) => run('permission_respond', { id, allow, remember }),
    respondExternal: (id: number, allow: boolean) => run('external_protocol_respond', { id, allow }),
  },

  library: {
    suggest: (query: string) => invoke<Suggestions>('suggest', { query }),
    topSites: () => invoke<HistoryEntry[]>('top_sites'),
    bookmarks: () => invoke<Bookmark[]>('bookmarks_list'),
    toggleBookmark: (id: TabId) => invoke<boolean>('bookmark_toggle', { id }),
    updateBookmark: (id: number, title: string, opts: { icon?: string | null; color?: string | null; url?: string | null } = {}) =>
      run('bookmark_update', { id, title, icon: opts.icon ?? null, color: opts.color ?? null, url: opts.url ?? null }),
    createFolder: (parent: number | null, title: string, icon: string | null, color: string | null = null) =>
      invoke<number>('bookmark_create_folder', { parent, title, icon, color }),
    addBookmark: (parent: number | null, title: string, url: string) => invoke<number>('bookmark_add', { parent, title, url }),
    barMenu: (canBookmarkPage: boolean, x: number, y: number) => run('bookmark_bar_menu', { canBookmarkPage, x, y }),
    moveBookmark: (id: number, parent: number | null) => run('bookmark_move', { id, parent }),
    importSources: () => invoke<ImportSource[]>('bookmark_import_sources'),
    importBrowser: (id: string) => invoke<ImportSummary>('bookmark_import_browser', { id }),
    importFile: (name: string, content: string) => invoke<ImportSummary>('bookmark_import_file', { name, content }),
    folderMenu: (id: number, x: number, y: number) => run('bookmark_folder_menu', { id, x, y }),
    itemMenu: (id: number, x: number, y: number) => run('bookmark_item_menu', { id, x, y }),
    removeBookmark: (id: number) => run('bookmark_remove', { id }),
    history: (query: string, offset: number, limit: number) => invoke<HistoryEntry[]>('history_list', { query, offset, limit }),
    deleteHistory: (id: number) => run('history_delete', { id }),
  },

  downloads: {
    list: () => invoke<DownloadItem[]>('downloads_list'),
    cancel: (id: number) => run('download_cancel', { id }),
    resume: (id: number) => run('download_resume', { id }),
    open: (id: number) => run('download_open', { id }),
    show: (id: number) => run('download_show', { id }),
    remove: (id: number) => run('download_remove', { id }),
    clear: () => run('downloads_clear'),
  },

  settings: {
    get: () => invoke<Settings>('settings_get'),
    update: (settings: Settings) => invoke<Settings>('settings_update', { settings }),
  },

  protection: {
    status: (id: TabId | null) => invoke<ProtectionStatus>('protection_status', { id }),
    setEnabled: (enabled: boolean) => run('adblock_set_enabled', { enabled }),
    /** `tab`: the tab the choice is made in; a private tab's choice is never saved. */
    setSiteEnabled: (host: string, enabled: boolean, tab: TabId | null = null) => run('adblock_set_site_enabled', { host, enabled, tab }),
    exceptions: () => invoke<string[]>('site_exceptions_list'),
    lists: () => invoke<FilterList[]>('filter_lists'),
    addList: (url: string, title: string) => invoke<FilterList>('filter_list_add', { url, title }),
    removeList: (id: string) => run('filter_list_remove', { id }),
    setListEnabled: (id: string, enabled: boolean) => run('filter_list_set_enabled', { id, enabled }),
    updateLists: () => run('filter_lists_update'),
    customRules: () => invoke<string>('custom_rules_get'),
    setCustomRules: (rules: string) => run('custom_rules_set', { rules }),
  },

  privacy: {
    clear: (request: ClearDataRequest) => run('clear_browsing_data', { request }),
    permissions: () => invoke<SitePermission[]>('permissions_list'),
    setPermission: (origin: string, kind: string, state: 'allow' | 'deny' | null) =>
      run('permission_set', { origin, kind, state }),
  },

  passwords: {
    list: () => invoke<Login[]>('passwords_list'),
    forTab: (tabId: TabId) => invoke<Login[]>('passwords_for_tab', { tabId }),
    fill: (tabId: TabId, id: number) => run('password_fill', { tabId, id }),
    /** Asks Windows Hello first. */
    reveal: (id: number) => invoke<string>('password_reveal', { id }),
    /** Asks Windows Hello first (unless it confirmed within the last minute). */
    update: (id: number, username: string, password: string) => run('password_update', { id, username, password }),
    remove: (id: number) => run('password_delete', { id }),
    neverList: () => invoke<string[]>('passwords_never_list'),
    neverRemove: (origin: string) => run('passwords_never_remove', { origin }),
  },

  appInfo: () => invoke<AppInfo>('app_info'),
};

/** Backend → UI events. */
export interface EventMap {
  'browser:state': InitialState['state'];
  'browser:favicon': { tabId: TabId; dataUrl: string | null };
  'downloads:state': DownloadItem[];
  'downloads:started': number;
  'find:result': { tabId: TabId; matches: number; active: number };
  'shell:action': string;
  'settings:changed': Settings;
  'protection:changed': null;
  'bookmarks:changed': null;
  'passwords:changed': null;
  'group:editor': number;
  'bookmark:editor': BookmarkEditorRequest;
  // Shell ↔ overlay (both trusted UI webviews).
  'history:changed': null;
  'overlay:render': OverlayContent | null;
  'overlay:action': OverlayAction;
  'overlay:ready': null;
}

interface TabMenuInfo {
  pinned: boolean;
  canReopen: boolean;
  groupId: number | null;
  groups: { id: number; title: string }[];
  inSplit: boolean;
  /** The window's only tab: "Move to new window" is disabled. */
  onlyTab: boolean;
}

export interface BookmarkEditorRequest {
  mode: 'bookmark' | 'folder';
  /** Existing item to edit, or null to create one. */
  id: number | null;
  /** Folder to create in (null = bookmarks bar). */
  parentId: number | null;
}

export type OverlayContent =
  | { kind: 'suggestions'; items: SuggestionRow[]; selected: number }
  | { kind: 'shield'; tabId: TabId }
  | { kind: 'passwords'; tabId: TabId }
  | ({ kind: 'bookmarkEditor'; pageUrl: string; pageTitle: string } & BookmarkEditorRequest)
  | { kind: 'split'; current: SplitChoice | null }
  | ({ kind: 'groupEditor' } & GroupEditorRequest);

export interface GroupEditorRequest {
  groupId: number;
  title: string;
  color: string;
  icon: string | null;
}

/** A split-view layout with its pane count. */
export interface SplitChoice {
  count: number;
  layout: SplitLayout;
}

export interface SuggestionRow {
  key: string;
  kind: 'search' | 'url' | 'history' | 'bookmark';
  primary: string;
  secondary: string;
  value: string;
  /** History rows can be removed (X button, Shift+Delete). */
  historyId: number | null;
}

export type OverlayAction =
  | { type: 'pick'; index: number }
  | { type: 'removeSuggestion'; index: number }
  /** `page`: the popup handed focus to the web page. */
  | { type: 'closed'; reason: 'escape' | 'blur' | 'done' | 'page'; kind?: OverlayContent['kind'] }
  | { type: 'reloadTab'; tabId: TabId }
  | { type: 'openSettings'; section: string };

// Each browser window has its own shell and overlay webviews, labelled `shell-<n>` and `overlay-<n>`.
const windowNumber = () => {
  const label = getCurrentWebview().label;
  return label.slice(label.indexOf('-') + 1);
};

/** Listens on this webview only: another window's events never arrive, broadcasts to all windows do. */
export function onEvent<K extends keyof EventMap>(name: K, handler: (payload: EventMap[K]) => void): Promise<UnlistenFn> {
  return getCurrentWebview().listen<EventMap[K]>(name, (e) => handler(e.payload));
}

/** Sends to this window's shell or overlay. */
export function emitToWebview<K extends keyof EventMap>(label: 'shell' | 'overlay', name: K, payload: EventMap[K]) {
  return emitTo(`${label}-${windowNumber()}`, name, payload);
}

/** Error text from a rejected invoke (commands reject with a plain string). */
export function errorMessage(e: unknown): string {
  return typeof e === 'string' ? e : e instanceof Error ? e.message : 'Something went wrong';
}
