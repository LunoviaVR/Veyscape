# Security model

Every web page is untrusted. The browser UI and the Rust backend are trusted. The design goal is that
a malicious page can do nothing a page in a hardened mainstream browser could not.

## Trust boundaries

| Component | Trust | Can reach |
|---|---|---|
| Rust backend | trusted | OS, SQLite, files under app data, WebView2 APIs |
| Shell webviews (`shell-<n>`, one per window; React UI, `tauri.localhost`) | trusted | Allow-listed Tauri commands + event listen/emit |
| Overlay webviews (`overlay-<n>`; same bundle, `?view=overlay`) | trusted | Same capability as the shell |
| Web content views (one per tab) | **untrusted** | The network, through the engine; nothing else |
| WebView2 runtime (Microsoft) | platform | Chromium sandbox, site isolation, TLS |

## How web content is isolated

1. **No IPC channel exists.** Tabs are raw WebView2 controllers created by our adapter, not Tauri
   webviews. Tauri injects `window.isTauri`, `__TAURI_INTERNALS__` and the invoke key into every
   webview it creates; tab views get none of this and no custom-protocol handlers (`ipc:`, `tauri:`).
   Web messages are disabled (`IsWebMessageEnabled = false`), host objects are disallowed, and the
   inert `window.chrome.webview` object is deleted before page scripts run.
2. **Capability ACL.** `capabilities/shell.json` grants permissions only to the webview labels
   `shell-*` and `overlay-*` (each browser window's UI; only Rust creates webviews with these
   labels), for the local app origin only (no `remote` entry, no window-wide grant). An
   app manifest in `build.rs` puts every custom command under this ACL.
3. **Caller check.** `lib.rs` wraps the invoke handler and rejects any webview label other than
   `shell-<n>`/`overlay-<n>` before a command runs. Commands then act only on the caller's window.
4. **Separate storage.** Tabs use the `Browsing` WebView2 profile; the UI uses the default profile.
   Clearing browsing data never touches UI storage and vice versa.
5. **Navigation policy** (`browser/url.rs`). Tabs may load `http`, `https`, `blob` and `about:blank`.
   `file:`, `javascript:`, `data:` (top level), `veyscape:`, `edge:`/`chrome:`, `view-source:`,
   `search-ms:`, `ms-*`, `microsoft-edge*` and other abuse-prone schemes are cancelled. Internal
   pages exist only in the shell, so a page cannot navigate a tab into privileged UI; a pop-up to a
   non-web address starts its tab at `about:blank`, never at an internal page.
6. **External protocols** (`mailto:`, `zoommtg:` ...) are cancelled in the engine and shown as an
   infobar; nothing launches without a click. Forbidden schemes are never offered. A tab's app-link
   and pop-up bars are dropped when it moves to another site, and a page repeating a request
   replaces its bar instead of stacking new ones.
7. **Popups** become ordinary unprivileged tabs. Non-user-initiated popups are blocked unless the user
   allowed popups for that origin. Pages can never open a Tauri window.
8. **Permissions** (camera, microphone, location, notifications, automatic downloads) are decided by
   the backend: stored decision or an infobar prompt; the engine's own persistence is disabled
   (`SavesInProfile = false`). WebView2 reports a request from an embedded
   frame (allowed via `allow=`) as coming from the page shown in the tab (checked in the app,
   2026-11-13), so decisions are per top-level site, as in Chrome. Other permission kinds (clipboard read, sensors, MIDI SysEx, local
   fonts, window management, file system) are denied. Pending prompts are denied when the tab closes
   or navigates to another origin.
9. **Downloads** go to the profile download folder with engine-chosen unique names; the default engine
   UI is suppressed. Nothing is opened automatically; opening executables/scripts requires explicit
   confirmation, and Windows still applies Mark-of-the-Web/SmartScreen at launch.

10. **Password manager** (ADR 0006). The only code of ours that runs inside pages with a way back to
    the backend is the password script, in a private JavaScript world created per document through
    the host-only DevTools protocol. Page scripts cannot see or call it; messages from any other
    context are dropped. Its reports (a password field, a submitted login) only lead to a "Save
    password?" bar; nothing is stored without a click.

Verified behaviour (Windows 11, WebView2 157, `tests/isolation/index.html` served over http):
no `__TAURI_INTERNALS__`, no `isTauri`, no `__TAURI__`, no `chrome.webview`, `http://ipc.localhost`
unreachable, ad script and tracking pixel blocked before loading.

## No Edge surfaces

The engine is the WebView2 runtime (Chromium, shipped by Microsoft as part of Edge). Everything
Edge-specific is removed or replaced (decision of 2026-11-12, see `decisions/0004-no-edge-surfaces.md`):

| Edge surface | What Veyscape does |
|---|---|
| Edge error pages ("Hmmm… can't reach this page") | Disabled (`IsBuiltInErrorPageEnabled = false`); the shell shows its own screens via `EngineEvent::LoadFailed`. Certificate errors therefore have no click-through at all. |
| `Edg/<version>` in the User-Agent | Removed; sites see a plain Chromium UA. |
| "Microsoft Edge" in client hints | Removed from `Sec-CH-UA*` request headers (request handler) and from `navigator.userAgentData` (document-start script). Workers still see the original brands. |
| `window.chrome.webview` | Deleted at document start. |
| Edge DevTools | Disabled for web content. |
| Edge print preview | Print uses the Windows system dialog; the context-menu Print entry is removed. |
| Context menu (More tools, Share, web capture, QR, Read aloud, "Open link in new window") | Removed; links get Veyscape's "Open link in new tab / private tab". |
| Engine menu on the browser UI (Back, Refresh, Save as, Print, Inspect) | Suppressed; text fields and selections keep Undo/Cut/Copy/Paste/Select all only. |
| Download, find and autofill UI; password saving | Suppressed/disabled; Veyscape UI used instead. |
| SmartScreen, mini menu, PDF mini menu | Disabled via browser arguments. |

What remains: the runtime itself (security updates come from Microsoft via Windows), its telemetry
(governed by Windows diagnostic settings), the engine's tracking-prevention lists, and the WebView2
PDF viewer's look.

Verified with `tests/isolation/index.html` and a header-echo server: UA has no `Edg/`, `Sec-CH-UA`
and JS brands are `Chromium, Not-A.Brand`, DNS and certificate failures show Veyscape pages.

## Private tabs

A **private window** (Ctrl+Shift+P, "New private window") holds only private tabs: every tab opened
in it is private however it was opened, a normal tab cannot be dragged into it (the strip refuses
the drag and `Browser::drop_tab` checks again), a tab torn off it opens another private window, and
private windows are never saved in the session. Its tab strip is tinted with a "Private window"
badge and its title is "Veyscape (private)". Private tabs can still be opened in normal windows.

Private tabs run in WebView2's InPrivate mode of the `Browsing` profile. Cookies, cache, storage,
service workers and HSTS state live in memory, are shared by all private tabs in all windows, and are
discarded by WebView2 when the last private tab closes. Veyscape itself keeps nothing on disk for them:

| Data | Private tabs |
|---|---|
| History, titles, top sites | Never recorded |
| Saved session, reopen-closed-tab | Never included |
| Site permissions | Asked every time; "Remember" is hidden and stored decisions are not applied |
| Ad-block on/off for a site | Memory only, private tabs only; dropped when the last private tab closes (`AdBlockEngine::set_private_site_exception`) |
| Bookmark icons | Never learned from a private page |
| Download list | Never saved; finished entries leave the list when the last private tab closes (files stay on disk) |
| Tracking prevention | Always Strict (the InPrivate profile has its own level, so normal tabs keep the user's choice) |
| Passwords | Never offered for saving; saved logins can still be filled (read only). The engine's own autofill is off for every tab |

Things a private tab does not hide: the network (sites, the ISP and DNS still see requests), files
the user downloads, and bookmarks the user adds on purpose. Sending a Global Privacy Control signal
only from private tabs was considered and left out, because it would let sites detect private tabs.

## Password manager

| Aspect | Behaviour |
|---|---|
| Storage | `logins` table; each password encrypted with DPAPI for the current Windows user (`CryptProtectData`, app-specific entropy). Another Windows account, or the database copied to another machine, cannot decrypt it |
| Saving | Only after the user clicks Save on the infobar. Offered after the page navigates on or the submitted form goes away, so a rejected login (form still there) is usually not offered. Never in private tabs, never for sites on the "Never" list, off entirely with the setting |
| Filling | Only when the user picks a login in the key popup, only into the origin it was saved for (checked in Rust against the tab's URL, and again by the fill script against the document actually
shown, so a page still on screen during a navigation gets nothing), only in the top frame. A fill
that finds no form reports an error and does not count as a use. Values travel as JSON string literals |
| Showing, copying, editing | `password_reveal` / `password_update` ask Windows Hello first (`UserConsentVerifier`, over the browser window); one confirmation lasts a minute. Without Windows Hello set up there is nothing to ask and they proceed (Chrome asks for the Windows password instead; not implemented) |
| UI exposure | Lists, prompts and the key popup carry usernames and origins only. In memory, pending passwords are a `Secret` whose `Debug` output is redacted |
| Network | None. No breach checks, no sync |

## Trusted UI hardening

- CSP (production): `default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:;
  connect-src ipc: http://ipc.localhost; object-src 'none'; base-uri 'none'; frame-src 'none'`.
  The UI never loads remote resources; favicons arrive from the engine as PNG data URLs. Bookmark
  icons are copies of those (or of icons embedded in an imported file), decoded with size limits and
  stored only for bookmarked sites; Veyscape never requests a favicon itself.
- `freezePrototype: true`, `withGlobalTauri: false`.
- Least privilege: only `core:event` listen/emit and a few window-control permissions besides the app
  commands. No fs, shell, http or opener permissions are exposed to JavaScript (the opener plugin is
  used from Rust only). The single-instance plugin has no JavaScript API: a second launch only asks
  the running process to open a new-tab window; its command-line arguments are ignored.
- Inputs are validated in Rust: URLs (`web_url`, `internal_page`), settings (`Settings::validate`),
  permission origins, filter-list URLs (https only), custom rules size, bookmark titles, address length.

## Network and TLS

- TLS validation is never disabled. WebView2's certificate error pages are left in place.
- HTTPS-only mode upgrades top-level `http://` navigations (except localhost/LAN names and loopback,
  private, link-local or unique-local IP addresses; public IPs are upgraded) and shows an
  interstitial before any downgrade; continuing is a per-host, per-tab user decision.
- Filter lists are fetched with `ureq` + rustls defaults (certificate verification on), HTTPS only
  (redirects too), with a 5-minute limit; lists over 16 MB are rejected rather than cut short, and
  anything that looks like HTML is rejected. Cosmetic rules that could act as CSS at-rules (`@`, `;`,
  braces) are dropped before they reach a page.
- No proxy, no MITM, no traffic leaves the machine except page loads and filter-list downloads.

## Known limitations and residual risk

- **SmartScreen is off** (same default as wry: `--disable-features=msSmartScreenProtection`), because
  it sends visited URLs to Microsoft. Malicious-site protection relies on filter lists (URLhaus is
  available, off by default for memory reasons). Revisit if a privacy-preserving option appears.
- **WebView2 runtime telemetry** follows Windows diagnostic-data settings; Veyscape cannot disable it.
- **Cosmetic filtering** runs injected CSS in the page's main world; a page can detect or remove it.
  Network blocking is unaffected.
- **Iframe cosmetic filtering** is not implemented (network blocking does cover iframe requests).
- **Password manager**: cross-site login iframes are not covered; a site that rejects a login by
  reloading the whole page still gets the offer; with no Windows Hello set up, saved passwords can be
  shown without a prompt; anyone using the unlocked Windows account can fill saved logins.
- **Third-party cookies** have no dedicated WebView2 switch; protection comes from the engine's
  tracking prevention level and blocklists.
- **`unstable` Tauri feature** is enabled for the overlay child webview (see ADR 0002).
- Single shared browser process with the UI (separate profiles, separate renderer processes).

## Reporting

Security issues: open a private advisory on the GitHub repository.
