# 0004 — No Edge-branded or Edge-specific surfaces

Status: accepted (2026-11-12, requested by the project owner)

## Context

WebView2 is the Edge engine. By default it shows Edge error pages (with the Microsoft Edge logo),
identifies as Edge (`Edg/` user-agent token, "Microsoft Edge" client-hint brands), opens Edge
DevTools and Edge print preview, and offers Edge context-menu features. The owner asked to remove
"any use of Edge". Replacing the engine with a bundled Chromium (CEF) was considered and declined:
+150–250 MB install, self-managed Chromium security updates, unstable Tauri support, full engine
rewrite.

## Decision

Keep the system WebView2 runtime and remove every Edge surface reachable from the UI or visible to
websites. The list and its verification live in `docs/security.md` ("No Edge surfaces").

## Consequences

- Load failures are rendered by the shell (`TabError::LoadFailed`); certificate errors cannot be
  bypassed.
- Page DevTools are unavailable.
- Workers can still read the original client-hint brands; WebView2 offers no API to change them.
- The runtime and its Windows-governed telemetry remain; documented, not hidden.
