# Progress

_Last updated: 2026-11-13_

## Current phase

Phases 1–5 implemented on Windows; Phase 6 (polish, audit, profiling) in progress. Phase 7 (macOS,
Linux) not started.

## Verified in the running app (Windows 11, WebView2 157)

- Launch, session restore (lazy tabs), undecorated window with custom caption buttons
- App icon (window, taskbar, exe) and the About page logo come from `assets/logo.png` (1024 px, transparent); regenerate
  the icon set with `npx tauri icon assets/logo.png` (then delete the `android/` and `ios/` folders)
- Session restores each window's position and size, maximized state and split view (moved and
  resized window with a stacked split, then a maximized one, each brought back after Exit)
- Address bar: URL vs search resolution, local suggestions overlay above page content, Enter / Alt+Enter
- Address-bar inline completion ("gi" → github.com, "localh" → localhost:1420, rest selected; Backspace
  removes it without completing again); removing a history suggestion with its X or Shift+Delete, after
  which typing continues in the address bar
- Tabs: new (Ctrl+T, also from page focus), close, switch, private tabs, "Open link in new/private tab"
- Navigation, redirects, titles, favicons, back/forward state, reload (F5 from page)
- Network-level blocking (29 blocked on cnn.com; ad script + tracking pixel blocked on test page)
- Shield popup with per-page count and site toggle
- History page, Settings → Ad blocking, Downloads page, download of a file, find in page (1 of 9)
- Private tab cookie/localStorage isolation from normal tabs
- Private tabs: strict tracking prevention while normal tabs keep Balanced (profiles checked
  separately); a site turned off in the shield is not saved and is on again after the last private
  tab closes, without affecting normal tabs
- Isolation: no Tauri globals, no `chrome.webview`, IPC unreachable from pages
- No Edge surfaces: own error pages (DNS, certificate), UA without `Edg/`, client hints = Chromium,
  Edge context-menu items removed
- Home button (toggle in Appearance), bookmarks bar (shown by default, Ctrl+Shift+B) with folder
  dropdowns and per-folder icons, bookmark folders on the Bookmarks page
- Bookmark site icons: visiting a bookmarked site puts its favicon on the bar in every window, in
  folder dropdown menus and on the new-tab page; a new bookmark of a known site starts with the icon
- Bookmarks bar right-click menu: "Bookmark this page" (disabled on internal pages), "Add bookmark…"
  (custom name + address, prefilled from the page), "New folder…" with icon and colour (presets and
  the custom colour picker), "Edit…" and "Delete" on bar items; folder colours survive restart
- Bookmark import from an exported HTML file (nested folders kept, bookmarklets dropped); detection
  of installed browsers (Chrome found on the test machine)
- Tab groups: create, add/remove tabs, preset colours, minimize/expand, survive restart
- Drag a tab into a group: onto a one-tab group's tab, and onto the group chip (joins at the start)
- Tab tear-off: dragging a tab onto the page opens a window at the drop point with that tab; the page
  keeps its state (typed text survived, no reload), events reach the new window (password offer shown
  there), it keeps working after the window it came from closes; "Move to new window" in the tab menu
  does the same without a drop point; a window's only tab cannot be moved
- Drag a tab into another open window's tab strip: lands where dropped (before the tab under the
  pointer, or at the end on empty strip space), page state kept, that window comes to the front;
  dragging a window's last tab into another window closes the emptied one
- Private window (Ctrl+Shift+P and the main menu): tinted strip, "Private window" badge, title
  "Veyscape (private)", Ctrl+T opens private tabs, not in the saved session (checked in the database),
  a normal tab dragged onto its strip is refused, a tab torn off it opens another private window
- Torn-off window follows the pointer while the tab is still dragged (page state kept); over the
  original strip it hides and releasing there puts the tab back (the extra window closes); dragging
  out and straight back before the window is ready leaves the tab where it was
- Tab group editor (like bookmark folders): opens for a new group and from "Edit group…"; name, icon
  (or none) and colour; the chip shows the icon; editing reloads the current values
- Split view: 2×2 grid, one-left-two-right and one-top-two-below layouts with web and internal panes;
  clicking a pane makes it active
- Multiple windows: Ctrl+N and the menu open a window; launching the exe again opens a window in the
  running instance; each window keeps its own tabs; history recorded in one window appears live on
  another window's History page; popups, the main menu (New tab, New private tab) and tab menus act on the window they were opened in; closing a window's
  last tab or its caption button closes just that window; Exit with two windows restores both
- Password manager (`tests/passwords/login.html` over http): "Save password?" after a login that
  navigates and after a single-page login whose form goes away; a rejected single-page login is not
  offered; Save, Never for this site (listed in Settings, removable) and Not now; the key button
  appears on a login page with saved logins and its popup fills username and password (input events
  fire, focus stays in the page); private tabs fill but never offer to save; Settings → Passwords
  lists, searches and deletes (with confirmation); the page sees neither `__vsPw` nor `__vsFill`
- Split picker: visual layout thumbnails grouped by page count, current layout highlighted, "Close
  split view", toggles from the toolbar button, keyboard selection, Escape returns focus to the button

## Implemented, not yet exercised end-to-end in the app

Icons from imported HTML files (parser unit-tested), direct import from Chrome/Firefox/Brave/Vivaldi/Opera/Edge profiles (parsers unit-tested; not run
against real profiles to avoid touching the owner's data), group rename and custom colour picker,
the remaining split layouts, bookmark moves between folders, bar overflow menu, pinned tabs, drag reordering, reopen closed tab, permission prompts (camera, mic, location,
notifications), external-protocol prompt, pop-up blocking prompt, HTTPS-only interstitial,
showing, copying and editing a saved password (Windows Hello prompt; needs the owner's PIN), the
"Update password?" offer, logins on real sites, clear browsing data, clear-on-exit, zoom indicator, print, full screen, bookmarks page editing,
theme/density switching, keyboard-only and screen-reader passes, untitled (icon-only) bookmarks on
the bar (a blank name is kept, also on import; folders still need a name).

## Known issues

- Fixed after the full audit (2026-11-13): session kept when the last normal window closes while a
  private window is open (checked in the database); shield on/off no longer undone by the next
  settings change (checked in the app); "clear cookies on exit" can no longer be skipped or hang the
  exit; crafted bookmark files (huge PNG header, deep nesting) no longer crash; Clear browsing data
  also clears the downloads list on screen; stale app-link/pop-up bars dropped on site change and
  repeated ones replaced; pop-ups cannot open internal pages; no RefCell borrows across WebView2
  calls; HTTPS-only redirect loop; ad-block page context restored after a download/Stop; hostile
  class lists capped; filter lists: HTTPS-only, timeout, no truncation, periodic refresh, no lost
  toggles; suggestions keep bookmarked pages and run off the UI thread; Firefox import reads mobile
  bookmarks and always cleans its temp copy; dragged-in tabs keep their icon; seven unused commands
  removed. Most of these are covered by unit tests, not exercised in the app.
- Fixed after the second audit (2026-11-13): tearing a private tab out of a normal window opens a
  private window and can no longer empty the saved session; a permission request whose origin
  differs from the tab's is never remembered; Clear browsing data also clears WebView2's own history and download records
  (for the chosen range); AZERTY Ctrl+Z no longer closes the tab; HTTPS-only upgrades public IPs and
  `[::1]` navigates; closing a background tab keeps fullscreen and find; full split keeps the active
  pane; password fill re-checks the shown document's origin and reports a failed fill; ad-block uses
  the committed page while a navigation is pending; more risky download types; secure_delete and WAL
  truncation after history deletion; history older than 365 days pruned at start; History page and
  top sites off the UI thread; bookmark import caps before decoding icons and reports skipped items;
  an unknown setting value resets only that setting; tab strip arrows, bookmark bar overflow, split
  view ids, find bar on tab switch and several UI error paths. Unit-tested. Checked in the app: private
  tear-off + closing every normal window keeps the session; `[::1]` navigates; loopback stays http
  under HTTPS-only; find bar and highlights cleared on tab switch; site-permission list refreshes
  after clearing; History page deletes and truncates the WAL. Not yet exercised: split replace, fullscreen
  on background close, password fill, import summary, bookmark-bar overflow, ranged engine clear.
- Fixed after the third audit (2026-11-13): "Already in split view" no longer collapses a split;
  closing a group or other tabs no longer loads lazy tabs first; a normal tab closed before quitting
  no longer returns when only private tabs were left; find highlights cleared when clicking into
  another split pane or tearing a tab off; the "embedded frame" permission code removed (WebView2
  reports the top page); Clear browsing data's time range applies to the download list too; History
  "Clear all" also clears WebView2's history (the separate command is gone); a fill that typed nothing
  reports failure; one malformed cosmetic rule no longer disables the rest; the WAL is truncated
  after any clear or login delete; empty imports add no folder; History page loads cannot land out
  of order; zoom shortcuts follow the keyboard layout; pop-ups are not always-allowed by default.

- First launch has no filter lists until the download finishes (a few seconds).
- `npm run tauri dev` hot rebuild can fail with "Access is denied" while the old exe runs; restart.
- Fixed: scrolling Google search results froze for ~440 ms while loading (Chromium starting Google's
  hidden YouTube preview player; Chrome does it too). That request gets an empty response on Google
  search pages; a 50–150 ms hitch at first paint remains (also in Chrome).
- Fixed: pages froze while loading in dev builds (unoptimized ad-block engine on the UI thread, up to
  ~100 ms per request); dependencies are now optimized in dev builds (see performance.md).
- Dev-only: Vite HMR of `main.tsx` logs a duplicate `createRoot` warning.
- Workers see the engine's original client-hint brands.
- Fixed: after another window closed (or a tab was dragged in from it), the first shortcut in the
  remaining window was lost; focus was still in the closing window when this one was activated.
- Dropping a tab outside the strip after Esc-cancelling the drag also moves it (HTML5 drag reports
  both as "no drop target").
- Removing finished private downloads at the end of the private session is unit-tested, not tried with a
  real download.
- Bookmarks of sites not opened in Veyscape yet show a globe (direct profile imports carry no icons;
  icons are never downloaded separately).
- Bookmarks cannot be reordered by drag and drop yet (move between folders via the Bookmarks page).
- Passwords: no suggestions under the focused field yet (fill from the key button); no import/export,
  password generator or "Saved passwords" option in Clear browsing data; cross-site login iframes are
  not covered; a login rejected by a full page reload is still offered.
- Fixed: a tab going from an internal page back to its web page (Back, or a page with the same title)
  kept the internal page's title; the tab now keeps the view's title while an internal page covers it.

## Recent decisions

0001 raw WebView2 tabs · 0002 overlay webview for popups · 0003 adblock-rust · 0004 no Edge surfaces ·
0005 multiple windows in one process · 0006 password manager bridge

## Next steps

1. Exercise the "implemented, not exercised" list above; fix what breaks.
2. Accessibility audit (keyboard-only walkthrough, Narrator), contrast check of tokens.
3. Measure tab switch latency, per-tab memory, interception overhead (see performance.md).
4. Integration test harness that drives the app (WebDriver for WebView2) for the isolation and
   blocking checks now done manually.
5. Choose a project license (none chosen yet).
