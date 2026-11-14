# Platform support

| Capability | Windows 10/11 (WebView2) | macOS (WKWebView) | Linux (WebKitGTK) |
|---|---|---|---|
| Browser UI (shell) | ✅ verified | builds via Tauri, untested | builds via Tauri, untested |
| Web content views (tabs) | ✅ raw WebView2 controllers | ❌ not implemented (`unsupported.rs`) | ❌ not implemented |
| Network-level request blocking | ✅ `WebResourceRequested`, all contexts + worker source kinds | ❌ planned: `WKContentRuleList` (precompiled rules, capped rule count, no per-request callback) | ❌ planned: `WebKitUserContentFilter` (same JSON format) |
| Cosmetic filtering | ✅ main frame | ❌ | ❌ |
| Private browsing | ✅ InPrivate controllers (shared in-memory session, wiped when last private view closes; strict tracking prevention) | ❌ planned: non-persistent `WKWebsiteDataStore` | ❌ planned: ephemeral `WebKitWebContext` |
| Tracking prevention levels | ✅ engine Basic/Balanced/Strict | ❌ | ❌ |
| Find in page | ✅ WebView2 Find API (`ICoreWebView2_28`; verified on runtime 157) | ❌ | ❌ |
| Permissions prompts | ✅ | ❌ | ❌ |
| Downloads with progress/cancel/resume | ✅ (resume within a session only) | ❌ | ❌ |
| Popups with `window.opener` | ✅ | ❌ | ❌ |
| Own error pages (no engine branding) | ✅ | ❌ | ❌ |
| Developer tools for pages | ❌ disabled (WebView2 DevTools are Edge DevTools) | ❌ | ❌ |

macOS/Linux builds compile the `UnsupportedEngine`, which reports "not implemented" for every web
view operation so the UI fails honestly rather than pretending to browse.

## Windows details

- **Runtime**: Evergreen WebView2 (tested 157.0.4341.46 on Windows 11 26300). The installer uses the
  download bootstrapper if the runtime is missing.
- **Request interception** runs on the UI thread, per request. The adblock check itself is
  microseconds; the WebView2 round trip adds latency per request (not yet measured separately).
- **Blocked responses** are synthesized 403s, so the request never reaches the network.
- **Interception coverage**: main frame, iframes, dedicated/shared/service workers
  (`AddWebResourceRequestedFilterWithRequestSourceKinds`, falls back to the older filter API).
- **Not available in WebView2**: per-site third-party cookie blocking switch, extension APIs (unused),
  reputation checks without SmartScreen.
- **Multi-webview**: the overlay uses Tauri's `unstable` multi-webview API (`Window::add_child`).
  Creating webviews from synchronous commands deadlocks on Windows (tauri-apps/wry#583); all such
  commands are `async`.
- **Window**: undecorated with custom caption buttons. Windows 11 Snap Layouts flyout (hover on
  maximize) is not available with custom buttons; snapping by dragging and Win+Arrow work.

## macOS / Linux plan (Phase 7)

Implement `platform::BrowserEngine` with WKWebView / WebKitGTK views parented to the main window,
convert filter lists to the WebKit content-blocker JSON format (adblock-rust has a `content-blocking`
feature for this), and document the different capability set. Shortcuts need a Cmd mapping on macOS.
