# Veyscape

Escape the clutter, advertising, and tracking of the modern internet. Your web. Your privacy. No distractions.

Veyscape is a lightweight, privacy-first desktop browser built with Tauri v2, Rust and React, using the
system's native web engine (Microsoft Edge WebView2 on Windows). Ads and trackers are blocked at the
network level by a built-in engine based on Brave's adblock-rust. No accounts, no sync, no telemetry.

> Status: early development. Windows 10/11 works; macOS and Linux are not supported yet.
> See [docs/progress.md](docs/progress.md) and [docs/platform-support.md](docs/platform-support.md).

## Features

- Tabs: open, close, reorder (drag or Ctrl+Shift+←/→), pin, duplicate, reopen closed, middle-click
  close, context menu, lazy loading of restored tabs
- Unified address/search bar with local-only suggestions (history + bookmarks), inline completion of
  sites you visit, and an X (or Shift+Delete) to remove a history suggestion; DuckDuckGo by default
- Built-in ad, tracker and malware blocking (EasyList, EasyPrivacy, optional lists, custom rules,
  per-site exceptions), cosmetic filtering, shield popup with per-page counts
- Private tabs and private windows (InPrivate storage, wiped when the last private tab closes; a private
  window only ever holds private tabs and is never restored)
- Password manager: offers to save after you sign in, fills from the key button, Settings → Passwords
  (encrypted with your Windows account, Windows Hello before showing one; nothing leaves the device)
- Engine tracking prevention, HTTPS-only mode, delete data on exit, browsing-data deletion
- Site permissions (camera, microphone, location, notifications, pop-ups, automatic downloads) with
  prompts and a review page
- Bookmarks bar with folders and folder icons, bookmark import (Chrome, Firefox, Brave, Vivaldi,
  Opera, Edge, or an exported HTML/JSON file)
- Tab groups with colours (presets or custom) that can be minimized; drag a tab onto a group to add it
- Drag tabs between windows, or off the tab strip into a new window that follows the pointer (or
  "Move to new window"); the page stays as it was
- Split view: up to four pages side by side in columns, rows, a grid or a main+side layout
- History, downloads (progress, cancel, retry), session restore, find in page, zoom, print, home button
- Light/dark/system theme, compact density, keyboard shortcuts, screen-reader labels

## Requirements

- Windows 10 or 11 with the WebView2 Runtime (preinstalled on Windows 11)
- Rust (stable, 1.85+), Node.js 20+ and npm
- Microsoft C++ Build Tools (for Rust on Windows)

## Develop

```bash
npm install
npm run tauri dev
```

## Build

```bash
npm run tauri build
```

The installer is written to `src-tauri/target/release/bundle/nsis/`. For just the executable use
`npm run tauri build -- --no-bundle` (output: `src-tauri/target/release/veyscape.exe`).

## Test

```bash
npm test && npm run lint && npm run typecheck
cd src-tauri && cargo test && cargo clippy --all-targets
```

Manual isolation check: serve `tests/isolation/` (e.g. `python -m http.server 8765` in that folder)
and open `http://127.0.0.1:8765/` in Veyscape; every line should read PASS. The password manager has a
sample sign-in page in `tests/passwords/` (serve it the same way and open `login.html`).

## Project layout

| Path | Contents |
|---|---|
| `src-tauri/` | Rust backend: browser core, WebView2 adapter, ad blocking, storage, passwords, commands |
| `src/` | React/TypeScript UI: shell, overlay popups, internal pages |
| `shared/` | Keyboard shortcut table shared by Rust and TypeScript |
| `docs/` | Architecture, security, platform support, performance, decisions, progress |
| `tests/` | Manual test pages (web isolation, password save/fill) |

## Documentation

- [docs/architecture.md](docs/architecture.md) — modules and data flows
- [docs/security.md](docs/security.md) — threat model and isolation
- [docs/platform-support.md](docs/platform-support.md) — engine capabilities and limitations
- [docs/performance.md](docs/performance.md) — measurements and method
- [docs/decisions/](docs/decisions/) — architecture decision records
- [docs/progress.md](docs/progress.md) — current status, known issues and next steps

## Privacy

Veyscape only contacts the sites you visit and, when lists are a few days old, the filter-list maintainers
(easylist.to and any optional lists you enable). Suggestions never leave your device. The WebView2
runtime is a Windows component whose diagnostic data follows your Windows privacy settings.

## Third-party licenses

adblock-rust (MPL-2.0), Tauri (MIT/Apache-2.0), React (MIT), Lucide (ISC), rusqlite/SQLite,
webview2-com and windows-rs (MIT/Apache-2.0). Filter lists keep their own licenses (EasyList:
GPL-3.0/CC BY-SA 3.0) and are downloaded from their maintainers at runtime, not distributed here.

## License

No license has been chosen yet; all rights reserved until one is added.
