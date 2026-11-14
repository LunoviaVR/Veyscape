// Mirrors of the Rust types sent over IPC (serde camelCase). Keep in sync with src-tauri/src.

export type TabId = number;
export type Security = 'secure' | 'insecure' | 'internal' | 'none';
export type InternalPage = 'newTab' | 'settings' | 'history' | 'bookmarks' | 'downloads';
export type PermissionKind = 'camera' | 'microphone' | 'location' | 'notifications' | 'popups' | 'automaticDownloads';

export type LoadFailure = 'certificate' | 'notFound' | 'timeout' | 'unreachable' | 'offline' | 'failed';

export type TabError =
  | { kind: 'httpsUnavailable'; httpUrl: string }
  | { kind: 'loadFailed'; url: string; reason: LoadFailure }
  | { kind: 'crashed' }
  | { kind: 'engine'; message: string };

export interface Tab {
  id: TabId;
  url: string;
  title: string;
  loading: boolean;
  canGoBack: boolean;
  canGoForward: boolean;
  pinned: boolean;
  private: boolean;
  zoom: number;
  blocked: number;
  /** Logins saved for this site; counted when the page shows a password field. */
  savedLogins: number;
  security: Security;
  internal: InternalPage | null;
  error: TabError | null;
  bookmarked: boolean;
  groupId: number | null;
}

export interface TabGroup {
  id: number;
  title: string;
  /** Preset name (grey, blue, ...) or a custom #rrggbb colour. */
  color: string;
  collapsed: boolean;
  /** Bookmark folder icon name shown on the chip, if any. */
  icon: string | null;
}

export type SplitLayout = 'columns' | 'rows' | 'mainLeft' | 'mainTop' | 'grid';

export interface PaneRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface SplitState {
  tabs: TabId[];
  layout: SplitLayout;
  shown: boolean;
  panes: { tabId: TabId; rect: PaneRect }[];
}

export type Prompt =
  | { type: 'permission'; id: number; tabId: TabId; origin: string; kind: PermissionKind }
  | { type: 'externalProtocol'; id: number; tabId: TabId; uri: string }
  | { type: 'popupBlocked'; id: number; tabId: TabId; origin: string; url: string }
  /** `update`: the username is saved already, with another password. The password never reaches the UI. */
  | { type: 'savePassword'; id: number; tabId: TabId; origin: string; username: string; update: boolean };

/** A saved login (never includes the password). */
export interface Login {
  id: number;
  origin: string;
  username: string;
  createdAt: number;
  lastUsedAt: number;
}

export interface BrowserState {
  /** A private window: every tab in it is private. */
  privateWindow: boolean;
  tabs: Tab[];
  activeTabId: TabId | null;
  canReopenClosed: boolean;
  prompts: Prompt[];
  fullscreen: boolean;
  groups: TabGroup[];
  split: SplitState | null;
}

export type DownloadStateName = 'inProgress' | 'completed' | 'interrupted' | 'cancelled';

export interface DownloadItem {
  id: number;
  url: string;
  path: string;
  fileName: string;
  state: DownloadStateName;
  received: number;
  total: number | null;
  startedAt: number;
  private: boolean;
  risky: boolean;
}

export interface InitialState {
  state: BrowserState;
  favicons: Record<string, string>;
  downloads: DownloadItem[];
}

export type StartupBehavior = 'newTab' | 'restoreSession' | 'homepage';
export type SearchEngine = 'duckDuckGo' | 'brave' | 'startpage' | 'google' | 'bing';
export type ThemeSetting = 'system' | 'light' | 'dark';
export type Density = 'comfortable' | 'compact';
export type TrackingPrevention = 'off' | 'basic' | 'balanced' | 'strict';
export type NewTabBackground = 'none' | 'mist' | 'dusk' | 'forest';

export interface Settings {
  startup: StartupBehavior;
  homepage: string;
  searchEngine: SearchEngine;
  downloadDir: string | null;
  theme: ThemeSetting;
  density: Density;
  newtabShowTopSites: boolean;
  newtabShowBookmarks: boolean;
  newtabBackground: NewTabBackground;
  adblockEnabled: boolean;
  cosmeticFiltering: boolean;
  trackingPrevention: TrackingPrevention;
  httpsOnly: boolean;
  clearCookiesOnExit: boolean;
  showHomeButton: boolean;
  showBookmarkBar: boolean;
  offerToSavePasswords: boolean;
}

export interface EngineStatus {
  ready: boolean;
  updating: boolean;
  ruleListsActive: number;
  lastError: string | null;
  totalBlocked: number;
}

export interface ProtectionStatus {
  adblockEnabled: boolean;
  host: string | null;
  siteEnabled: boolean;
  blockedOnPage: number;
  trackingPrevention: TrackingPrevention;
  /** Private tab: site choices last only until the last private tab closes. */
  private: boolean;
  engine: EngineStatus;
}

export interface FilterList {
  id: string;
  title: string;
  url: string;
  enabled: boolean;
  builtin: boolean;
  license: string;
  lastUpdated: number | null;
  lastError: string | null;
}

export interface HistoryEntry {
  id: number;
  url: string;
  title: string;
  visitCount: number;
  lastVisit: number;
}

export interface Bookmark {
  id: number;
  parentId: number | null;
  kind: 'bookmark' | 'folder';
  url: string | null;
  title: string;
  icon: string | null;
  /** Folder colour: preset name or #rrggbb. */
  color: string | null;
  /** Site icon (PNG data URL) once the site has been visited. */
  favicon: string | null;
  position: number;
  createdAt: number;
}

/** Result of a bookmark import; `skipped` items were over the import limit or not importable. */
export interface ImportSummary {
  imported: number;
  skipped: number;
}

export interface ImportSource {
  id: string;
  name: string;
}

export interface Suggestion {
  url: string;
  title: string;
  bookmarked: boolean;
  /** Set for history rows, which can be removed from the list (and history). */
  historyId: number | null;
}

export interface Suggestions {
  items: Suggestion[];
  /** The typed text completed to a site the user visits, shown selected after the caret. */
  completion: string | null;
}

export interface SitePermission {
  origin: string;
  kind: PermissionKind;
  state: 'allow' | 'deny';
}

export interface AppInfo {
  version: string;
  tauriVersion: string;
  engine: string;
  platform: string;
}

export type TimeRange = 'lastHour' | 'lastDay' | 'lastWeek' | 'allTime';

export interface ClearDataRequest {
  history: boolean;
  downloads: boolean;
  cookies: boolean;
  cache: boolean;
  permissions: boolean;
  range: TimeRange;
}
