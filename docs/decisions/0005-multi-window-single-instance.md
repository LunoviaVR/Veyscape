# 0005 — Multiple windows in one process

Status: accepted (2026-11-13, requested by the project owner)

## Context

The owner asked for multiple browser windows or instances, with history kept in sync between them.
Two shapes were possible:

- **Several processes**, one window each. Each process would open the same SQLite database and the
  same WebView2 user-data folder. History would need cross-process change notification (polling
  `PRAGMA data_version` or a file watcher), and every process would overwrite the others' saved
  session.
- **One process, several windows**, as in Chromium and Firefox.

## Decision

One process owns every window. `browser::Browser` holds what windows share (downloads, the saved
session, which window is focused); `browser::Window` holds one window's tabs, groups, split view,
prompts and engine (its own content host, sharing the WebView2 environment and `Browsing` profile).
History, bookmarks, settings and permissions live in the single database, so they are shared by
construction. Changes are broadcast to every window's UI (`history:changed` and so on).

Launching the executable again opens a window in the running instance
(`tauri-plugin-single-instance`). The second process exits immediately, so it never touches the
database or the profile.

Labels carry the window number (`window-<n>`, `shell-<n>`, `overlay-<n>`). The capability uses the
globs `shell-*` / `overlay-*`. The invoke-handler caller check accepts only those labels with a numeric
suffix, and commands act only on the caller's window. Tab ids embed the window id in their upper 32
bits, so they stay unique across windows.

## Consequences

- The session file stores a list of windows. Single-window sessions from earlier versions still
  load.
- Closing a window forgets its tabs. Closing the last window quits and keeps it in the session.
- Native popup-menu events carry no window, so they go to the window that showed the menu. Only one
  popup menu can be open at a time. Tracking the focused window instead failed: clicking into another
  window's webview does not focus the top-level window, so menu actions landed in the first window.
- Window position, size, maximized state and split view are saved per window.
- Not done yet: dragging tabs between windows, private-only windows.
