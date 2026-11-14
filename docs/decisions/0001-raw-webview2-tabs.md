# 0001 — Web tabs are raw WebView2 controllers, not Tauri webviews

Status: accepted (2026-11-12)

## Context

Tauri v2 can host several webviews in one window (`Window::add_child`, behind the `unstable`
feature). Using that for tabs was the obvious first option. Reading the Tauri 2.12 source
(`manager/webview.rs::prepare_pending_webview`) shows that every Tauri-created webview receives
initialization scripts defining `window.isTauri`, `window.__TAURI_INTERNALS__` (including the invoke
key) and the IPC/`tauri:` custom protocol handlers, regardless of the URL it loads. The capability ACL
would still reject commands from remote origins, but:

- every website could detect Veyscape (and Tauri) — a fingerprinting signal in a privacy browser;
- every website would hold a live IPC endpoint, so one ACL mistake becomes a full compromise;
- the request interception, permissions, downloads, find, accelerator and focus APIs we need are not
  exposed by Tauri anyway, so we would reach into WebView2 COM for all of them.

## Decision

Create tab views directly with WebView2 (`ICoreWebView2Environment10::CreateCoreWebView2ControllerWithOptions`)
in our Windows adapter, parented to a plain child HWND ("content host"). Reuse the shell webview's
environment (one browser process) with a separate profile (`Browsing`) so storage is separate.
Keep Tauri webviews only for trusted UI (shell, overlay).

## Consequences

- Web pages have no IPC, no Tauri globals; verified with `tests/isolation/index.html`.
- We own controller lifecycle, bounds, visibility, focus and z-order (`platform/win`).
- macOS/Linux need their own adapters (`BrowserEngine` trait) instead of getting tabs "for free".
- The adapter must never block the UI thread: controller creation is asynchronous and reported via
  `EngineEvent::ViewReady`.
