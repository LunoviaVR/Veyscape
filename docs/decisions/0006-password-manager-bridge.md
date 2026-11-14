# 0006 — Password manager: a host-only bridge into pages

Status: accepted (2026-11-13, requested by the project owner)

## Context

The owner asked for a built-in password manager like Chrome's: offer to save a password after a
login, fill saved logins, and manage them. That needs two things the isolation model rules out for
pages: reading what the user typed into a login form, and typing a saved login back in.

Options considered:

- **WebView2's own password manager** (`IsPasswordAutosaveEnabled`). Its save prompt is engine UI,
  there is no API to list, edit or delete what it saved, and it stores data in the WebView2 profile.
  No management page would be possible. Rejected (also an Edge surface, see ADR 0004).
- **Web messages** (`chrome.webview.postMessage`) from an injected script. Any page script could
  call the same channel. Rejected: pages must have no channel to native code.
- **Parsing login POST bodies** in `WebResourceRequested`. Most sites log in with `fetch` and JSON;
  fragile, and it would add work to the per-request hot path. Rejected.
- **DevTools protocol from the host, with a private JavaScript world** (chosen).

## Decision

For every top-level http(s) document, the adapter (`platform/win/passwords.rs`) calls, through
`ICoreWebView2::CallDevToolsProtocolMethod` (host API, not reachable from pages; page DevTools stay
disabled):

1. `Page.createIsolatedWorld` for the main frame: a separate JavaScript world that shares the DOM
   but none of the page's globals or prototypes.
2. `Runtime.addBinding` with that world's name, which installs the `__vsPw` function there only.
3. `Runtime.evaluate` of the password script in that world's context.

The script reports a password field, form submissions and a submitted form disappearing. Calls arrive
as `Runtime.bindingCalled`; the adapter drops any call whose execution context is not the one it
created. Filling evaluates `__vsFill(username, password)` in the same context, with the values as JSON
string literals.

The `Runtime` domain is never enabled: binding by world name after the world exists works without it,
whereas enabling it is detectable by pages (a common bot-detection check) and makes them slower.

The browser core decides everything else: nothing is saved without the user clicking Save, private
tabs never offer to save, a saved login is only filled into the origin it was saved for, and only
after the user picks it. Passwords are encrypted with DPAPI (current Windows user) in SQLite, and the
UI receives one only through `password_reveal`, after Windows Hello.

## Consequences

- Page scripts still cannot reach native code: the binding exists only in a world they cannot see
  (checked in the app: `typeof __vsPw` and `typeof __vsFill` are `undefined` in the page,
  `tests/passwords/login.html`).
- A page controls its own DOM, so it can make the script report any values for its own origin. The
  worst case is a "Save password?" bar the user can dismiss; it cannot read other sites' logins.
- A filled password is readable by the page it was filled into, as in every browser.
- Cross-site login iframes (out-of-process frames) are not covered; that would need a DevTools
  session per frame target.
- The private-world mechanism is WebView2/Chromium-specific. macOS and Linux adapters need their own
  equivalent (WKWebView content worlds, WebKitGTK script worlds).
