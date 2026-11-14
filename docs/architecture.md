# Architecture

## Process and window model

One process, any number of browser windows. Each window (`window-<n>`) has the same three layers:

```
Browser window (undecorated; tab strip doubles as title bar)
├── shell webview       `shell-<n>`, full window, React UI, has IPC          (bottom)
├── content host HWND   plain Win32 child window, one WebView2 controller/tab (middle)
└── overlay webview     `overlay-<n>`, popup-sized, React popups, has IPC    (top)
```

- **Shell** renders the tab strip, toolbar, infobars, find bar and every internal page
  (`veyscape://newtab|settings|history|bookmarks|downloads`). Internal pages are never loaded in a web
  content view.
- **Content host** holds the web pages. Each tab's view is a raw `ICoreWebView2Controller` created from
  the shell's WebView2 environment but in a separate profile (`Browsing`), so tabs share the browser
  process (saves ~100 MB) while cookies/storage never mix with the UI's. Private tabs use InPrivate
  controllers of the same profile.
- **Overlay** exists because native child windows always paint over sibling webviews: anything that
  must cover web content (suggestions, shield popup) is drawn in a separate, small, trusted webview
  stacked above the content host. Main menu and tab context menu are native menus for the same
  reason (and because native menus are accessible out of the box).

Why raw WebView2 controllers and not Tauri child webviews for tabs: see
`decisions/0001-raw-webview2-tabs.md`.

**Multiple windows** (ADR 0005). Every window has its own tabs, groups, split view, prompts and
engine (`browser::Window`), but all windows browse in one WebView2 profile and one SQLite database,
so history, bookmarks, settings, permissions and downloads are shared. `browser::Browser` holds the
shared parts and broadcasts changes to every window (`history:changed`, `bookmarks:changed`,
`downloads:state`, `settings:changed`, `protection:changed`); per-window events (`browser:state`,
`shell:action`, …) go to that window's shell only. The UI listens on its own webview
(`getCurrentWebview().listen`), so it never sees another window's events. Commands find their
window from the caller's label (`browser::with_caller`); native menu events, which carry no window,
go to the window that showed the menu (`shell::popup` records it; one popup menu is open at a time). Tab ids carry the window id in their upper 32 bits, so they are
unique across windows (a moved tab keeps its id, so nothing may derive the window from it).

**Private windows.** `WindowSeed::Private` (or a tear-off from a private window) sets
`Window::private`: `create_tab` makes every tab private, the snapshot's `privateWindow` tints the
strip, `save_session` skips the window, and `drop_tab` refuses non-private tabs.

**Moving a tab between windows.** A tab strip registers the tab it starts dragging in Rust
(`tab_drag_start`) and marks the drag with an `application/x-veyscape-tab` type, so every window's
strip accepts it. A drop calls `tab_drop(index, group)` on the window it lands in, which moves the
registered tab: within the window, or out of its window into this one (`Browser::drop_tab`: detach,
adopt, position, focus this window; a window left empty closes). Drops act only on a tab registered
in the last minute, so a web page faking the drag type cannot move tabs.
Dragging a tab away from its strip (the source tab's `drag` events) calls `tab_tear_off` with
`follow`: the new window is created at once and `shell/follow.rs` keeps it under the pointer from a
background thread (the drag loop runs in WebView2's browser process, so the UI thread is free) until
the button is released. Over another window's tab strip it hides, so the drop lands in that strip
(`tab_drop`) and the emptied window closes; released elsewhere, it stays and comes to the front. A
drop before the new window is ready cancels the move (`browser::begin_tear_off` / `TEARING_OFF`).
Without drag positions, a drop away from every strip (`dragend` with no drop target) tears off there,
and "Move to new window" in the tab menu does the same without a position; both call `tab_tear_off`.
It opens a window with `WindowSeed::Adopt { from, tab, at }`; when that window's engine is ready,
`Browser::attach_window` takes the tab out of the source window (`Window::detach_tab`: no "reopen
closed tab" entry, leaves its group) and gives it to the new one (`adopt_tab`). The page is not
reloaded: `BrowserEngine::detach_view` hides the WebView2 controller and `attach_view` re-parents it
into the new window's content host. Its event handlers still report through the engine it was
created in, so `Browser::handle_event` routes every event by `EngineEvent::tab()` to the window that
owns the tab now; pending popups, permission requests and downloads live in one map shared by all
engines (`platform/win` `Requests`). A view still being created is dropped and recreated instead. Ctrl+N or the menu opens a window; launching Veyscape again opens one in the
running instance (`tauri-plugin-single-instance`). Closing a window forgets it; closing the last
one quits and keeps it in the session, which stores every open window: tabs, groups, split view,
maximized state and, for a normal-sized window, its position and size (`session::WindowBounds`).
A saved position that is no longer on any monitor is ignored.

## Modules (Rust)

| Module | Owns | Depends on |
|---|---|---|
| `browser` | Windows (`Window`: tab list, active tab, closed-tab stack, layout, prompts) and what they share (`Browser`: downloads list, session, focus); reacts to `EngineEvent`s and publishes state | `platform` (trait), `storage`, `adblock` (via services), `permissions` |
| `browser::url` | Input → URL/search resolution, internal pages, scheme policy, HTTPS upgrade | — (pure) |
| `browser::tabs` | `Tab`, `TabManager` ordering rules | — (pure) |
| `platform` | `BrowserEngine` trait, `EngineEvent`, `win` adapter | `adblock` (`AdBlockEngine` for request decisions), `browser::url` (policy) |
| `adblock` | Filter engine, cosmetic CSS, lists, exceptions, stats | `storage` |
| `passwords` | Save/update decision, DPAPI encryption, Windows Hello check | — (Windows APIs) |
| `storage` | SQLite schema, migrations, queries | — |
| `commands` | Validated Tauri entry points | everything above |
| `shell` | Browser window/webview creation and window events, overlays, native menus | `browser`, `platform` |

The browser core lives in a `thread_local` on the UI thread (`browser::with_browser`). Synchronous
commands and window events run on that thread; engine callbacks also fire there. Engine events raised
while the core is borrowed are queued (`browser::dispatch`) and processed when the borrow ends, so
WebView2's synchronous callbacks can never re-enter the core.

## Data flows

**Navigation (address bar → page)**
1. `AddressBar` → `api.nav.go(tabId, text)` → `nav_go` (sync, UI thread).
2. `Browser::navigate` resolves text with `url::resolve_input` (URL, internal page or search).
3. Internal page → tab URL changes, content host hidden, shell renders the page.
   Web URL → `engine.create_view` (async controller creation, navigation queued) or `engine.navigate`.
4. WebView2 `NavigationStarting` → `url::navigation_policy` (forbidden schemes cancelled) and optional
   HTTPS-only upgrade → `EngineEvent::NavigationStarting`.
5. Core updates the tab, marks state dirty; `flush()` emits `browser:state` (≤30/s) to the shell.

**Address-bar suggestions and inline completion**
Each keystroke calls `suggest` (local only): up to six history/bookmark matches plus `completion`,
the typed text completed to the most visited site that starts with it, ignoring `http(s)://` and
`www.` (`Db::completion_candidates` + `url::inline_completion`; past a `/` it completes the address).
The shell shows the completion selected after the caret, only for typed characters (not after
Backspace, paste or IME input). History rows carry their `historyId`: the row's X (mouse) or
Shift+Delete (keyboard) deletes that history entry and refreshes the list; after a click in the
overlay, `overlay_show(refocusShell)` hands keyboard focus back to the address bar.

**Request filtering**
WebView2 `WebResourceRequested` (filter `*`, all contexts, all source kinds incl. workers) →
`AdBlockEngine::should_block(url, top-level page URL, kind)` → if blocked, the handler sets a 403
response *before the request is sent* and emits `RequestBlocked` (per-page counter). The top-level
document is never blocked (that is navigation policy's job).

**Cosmetic filtering**
`ContentLoading` → hostname-specific hide selectors injected as a `<style>`; `DOMContentLoaded` →
class/id names collected (bounded) → generic selectors → second `<style>`. CSS travels as a JSON
string literal. Main frame only.

**Filter lists**
Startup: `initialize()` deserializes `engine.dat` when its fingerprint matches (list ids + update
times + custom rules + app version); otherwise recompiles from cached list files. Missing/stale lists
(>4 days) are downloaded on a background thread, checked again every 6 hours while the browser runs,
then the engine is rebuilt and swapped atomically. Catalogue edits (toggle, add, remove) and an
update's results are merged under one lock, so neither overwrites the other; a change made during an
update runs one more pass when it ends. Any window's engine can cancel or resume a download (one
shared request map), so closing the window a download started in does not strand it.

**Popups, permissions, external apps, downloads**
All arrive as engine events with a request id. The core decides (stored decision, user gesture) or
shows an infobar `Prompt`; the user's answer resolves the stored WebView2 deferral. Popups become tabs
via `NewWindowRequested.NewWindow`, so `window.opener` keeps working. Downloads are performed by the
engine; the core tracks items, persists non-private ones and throttles progress events (4/s).

**Split view**
`browser/split.rs` computes pane rectangles (CSS px) for 2–4 tabs and a layout (columns, rows,
main-left, main-top, grid). The core sends them to the shell in the snapshot and converts them to
physical pixels for `engine.layout`. Web panes are native views; the content host gets a window
region covering only those panes, so the gaps (active-pane outline) and panes showing internal pages
are drawn by the shell underneath. Clicking into a pane raises `ViewFocused` and makes it active.
The split persists while switching away and reappears when one of its tabs is selected again, and
is saved with the session (a split with a private pane is not). The toolbar's split button opens `features/split/SplitPicker` in the overlay:
one thumbnail per layout, drawn from `layouts.ts::previewRects` (a preview-scale copy of `pane_rects`).

**Tab groups**
`TabManager` keeps groups contiguous: joining moves a tab next to its group, tabs opened from a grouped
tab join it, dragging a tab between two members joins, dragging it away leaves, empty groups vanish.
Dropping a tab onto a grouped tab or a group's chip joins that group there (`tab_drop` with `group`,
`TabManager::move_into_group`); the chip is outlined while a drop would join it.
Collapsing a group that contains the active tab first activates the nearest tab outside it. Groups are
saved with the session. Colours are preset names (themed tokens) or custom `#rrggbb`; a group may also
have one of the bookmark folder icons. "Edit group…" in the chip menu (and every new group) emits
`group:editor`; the shell shows `tabs/GroupEditor` in the overlay, which shares the name, icon and
colour pickers and the form styles (`components/popupForm.module.css`) with the bookmark editor.

**Bookmarks**
A tree (folders + bookmarks, `position` order, optional folder icon and colour). Top-level items form
the bookmarks bar; folder dropdowns and the bar/item right-click menus are native menus (they draw over
web content). Their "Add bookmark…", "New folder…" and "Edit…" items emit `bookmark:editor`; the shell
then shows `BookmarkEditor` in the overlay. While the editor's colour picker is open the overlay's blur
does not close it. Imports go into a new "Imported from …" folder; only http(s) links are kept.

Bookmark icons (`bookmarks.favicon`, a PNG data URL) are learned, never fetched. When a
non-private page reports its favicon, `Db::bookmark_set_site_favicon` gives that icon to every
bookmark of the page's origin, including pages not visited yet. Sites without bookmarks store
nothing. New bookmarks take the page's current icon, or one already known for the site. Imported HTML
files keep their embedded `ICON` attributes. `bookmarks::favicon` shrinks icons to at most 32 px
before storing them, and draws them at 16 px (times the display scale) in the native folder menus.

**Saved passwords** (ADR 0006)
At DOMContentLoaded of each http(s) page, `platform/win/passwords.rs` creates a private JavaScript
world through the DevTools protocol and runs the password script there. It reports a password field
(`LoginFormFound`: the core counts saved logins for the origin, `Tab.savedLogins` shows the key
button), a submission (`LoginSubmitted`: held in `Window.pending_logins`) and the form going away
(`LoginSucceeded`). The pending login becomes a `Prompt::SavePassword` infobar on the next
NavigationStarting/Completed or LoginSucceeded, unless that exact login is saved already
(`passwords::save_action`). Save encrypts with DPAPI and stores it (`browser/logins.rs`). The key
popup (`features/passwords`) lists `passwords_for_tab`; picking one calls `password_fill`, which
checks the origin, decrypts and has the adapter evaluate `__vsFill` in the private world.
Settings → Passwords lists logins without passwords; show/copy/edit go through `password_reveal`
and `password_update`, which ask Windows Hello first.

**Shortcuts**
`shared/shortcuts.json` maps key chords to action names. Shell focus: `ShellApp` keydown → `runAction`.
Page focus: WebView2 `AcceleratorKeyPressed` → Rust maps the virtual key → `shell:action` event →
the same `runAction`. One table, one executor.

## Persistence

SQLite (`veyscape.sqlite3`, WAL): `kv` (settings JSON, session JSON, filter-list catalogue, custom
rules, site exceptions), `bookmarks`, `history` (one row per URL), `site_permissions`, `downloads`,
`logins` (passwords as DPAPI blobs), `login_never`.
Migrations are append-only SQL batches keyed by `PRAGMA user_version`.

## UI (React)

- `lib/ipc.ts` is the single typed boundary; `lib/store.ts` mirrors backend state via
  `useSyncExternalStore`. No component mutates browser state locally.
- One bundle serves both trusted webviews; `?view=overlay` selects `OverlayApp`.
- Chrome height is measured with a `ResizeObserver` and reported (`layout_update`) so web content
  starts exactly below the toolbar/infobars; Rust converts CSS px to physical px.
