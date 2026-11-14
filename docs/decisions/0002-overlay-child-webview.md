# 0002 — Popups over web content use a trusted overlay webview

Status: accepted (2026-11-12)

## Context

Web pages are native child windows. Anything rendered by the shell webview sits *behind* them, so an
address-bar dropdown or the shield popup drawn in the shell would be hidden by the page. Options:

1. Hide the page while a popup is open (visible flicker, page disappears).
2. Screenshot the page and show the image under the popup (latency, platform-specific capture).
3. A separate top-level popup window (activation/focus side effects, must track window moves).
4. A second small trusted webview stacked above the content host.

## Decision

Option 4: an `overlay` Tauri child webview (same React bundle, `?view=overlay`), created lazily,
sized to the popup and kept above the content host. The content host re-asserts its z-order directly
beneath the overlay whenever it is shown. The overlay is created with `focused(false)` so suggestions
never steal focus from the address bar. Native menus are used for the main menu and the tab context
menu instead (accessible, no overlay needed).

This needs Tauri's `unstable` feature (`Window::add_child`). Commands that create the overlay or open
native menus are `async`: creating a webview inside a synchronous command (which runs inside
WebView2's IPC callback) deadlocks on Windows (tauri-apps/wry#583). This was the cause of the first
"app froze" bug.

## Consequences

- One extra renderer process once a popup has been opened.
- Shell ↔ overlay communication goes through Tauri events (`overlay:render`, `overlay:action`).
- The `unstable` API may change between Tauri minor versions; it is isolated in `src-tauri/src/shell`.
