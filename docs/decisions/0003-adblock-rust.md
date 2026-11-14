# 0003 — Ad blocking with adblock-rust, network-level via WebResourceRequested

Status: accepted (2026-11-12)

## Context

Requirements: built in (no extension), network-level blocking before requests load, EasyList-style
syntax, cosmetic filtering, per-site exceptions, offline operation, low overhead. No proxy, no MITM.

## Decision

- Engine: Brave's `adblock` crate 0.13.5 (MPL-2.0), built with `default-features = false` +
  `embedded-domain-resolver`, `full-regex-handling` so `Engine` is `Send + Sync` and can be compiled on
  a background thread (the default `single-thread` feature makes it `!Send`).
- Hook: WebView2 `WebResourceRequested` with filter `*`, all resource contexts and all request source
  kinds; blocked requests get a synthesized 403 response, so they never hit the network.
- Lists: EasyList + EasyPrivacy on by default; URLhaus, Peter Lowe, Fanboy Cookie/Annoyance optional.
  Downloaded from their maintainers (no bundled copies, no Veyscape server), refreshed after 4 days.
- Cache: compiled engine serialized to `engine.dat` with a fingerprint; normal startups deserialize.
- Cosmetic: hostname-specific selectors at `ContentLoading`, generic class/id selectors after
  `DOMContentLoaded`; one CSS rule per selector so a bad selector cannot void the rest. Scriptlets and
  procedural filters are not applied.

## Consequences

- First launch has no lists until the download finishes (seconds); the UI says so honestly.
- Each request takes a round trip to the UI thread; the matching itself is microseconds.
- URLhaus is off by default: with it the compiled engine grew to ~23 MB.
- `engine.dat` is trusted local data (adblock-rust skips FlatBuffer verification for same-version
  files); anyone able to rewrite it already controls the user account.
