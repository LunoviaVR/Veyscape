# Performance

Only measured numbers are recorded here. Add a row (with environment and method) whenever you measure.

## Environment

Windows 11 Home 10.0.26300, WebView2 runtime 157.0.4341.46, AMD Radeon RX 9060 XT, Rust 1.99,
measured 2026-11-12.

## Results

| Metric | Value | Build | Method |
|---|---|---|---|
| Executable size | 8.0 MB | release (`opt-level = "s"`, LTO, stripped) | file size of `target/release/veyscape.exe` |
| Launch → main window visible | 119 ms first run, 31–32 ms next runs | release | `Start-Process` until `IsWindowVisible(MainWindowHandle)`, 10 ms polling, 3 runs; restored session of 3 tabs |
| Memory, all processes | 370 MB private / 692 MB working set (9 processes) | debug | shell + overlay + 1 loaded web page (+2 tabs not yet loaded), sum of `PrivateMemorySize64` over `veyscape.exe` and `msedgewebview2.exe` processes using the app's data folder |
| — of which `veyscape.exe` | 30 MB private | debug | includes the compiled ad-block engine with URLhaus enabled |
| Compiled ad-block engine | 23 MB on disk with EasyList + EasyPrivacy + URLhaus | — | size of `filters/engine.dat`; URLhaus is now off by default for this reason |
| Blocking effect | 29 requests blocked on cnn.com front page | debug | shield popup counter |
| Ad-block check per request (UI thread) | ~0.3 ms average, 2.3 ms worst; cnn.com: 82 requests in 26 ms, no UI-thread stall over 60 ms while loading | dev build with optimized dependencies | temporary timers around `should_block` plus a probe posting to the UI thread every 50 ms; cnn.com then youtube.com |
| Scroll freeze on Google search while loading | ~440 ms freeze in 9 of 10 fresh searches in Chrome (clean profile) and about 1 in 4 in Veyscape; 0 of 12 in Veyscape after the fix | dev build | frame-by-frame screen capture (~80 fps) while sending wheel notches every 100 ms, time-aligned with in-page frame, wheel and scroll probes |
| — same, before optimizing dependencies | 20–110 ms per request; 85 requests took 1.8 s and the UI thread stalled up to 1.5 s (pages and all windows froze) | dev build, unoptimized dependencies | same |

"Window visible" means the shell has rendered and reported its layout; it does not include loading
the restored web page.

Every request waits for the UI thread (WebView2 raises `WebResourceRequested` there), so anything
slow on that thread slows every page load. This is why `Cargo.toml` builds dependencies optimized
even in dev builds (`[profile.dev.package."*"]`). The first dev build after that change recompiles
dependencies, which took about 2 minutes here.

**Google search scroll freeze.** Google's results page preloads a hidden YouTube player
(`youtube.com/embed/…`, used for hover previews). About half a second after it starts, the page
stops getting frames and wheel input for ~440 ms; then the scroll catches up in one jump. The page's
main thread, the app's UI thread and the request filter were idle or ruled out (the freeze also
happened with interception off). Chrome with a clean profile froze the same way, and making that one
host unreachable removed it there too, so it is Chromium behaviour rather than Veyscape's. Veyscape
answers that player request with an empty response on Google search pages only
(`browser::url::is_search_preview_player`); video results still open normally. A 50–150 ms hitch when
the page first appears remains; Chrome has it as well.

## Not yet measured

Tab creation and switching latency, memory growth per tab, per-request cost in the release build
(`opt-level = "s"`), idle CPU, responsiveness during large downloads, release-build memory. Measure these before claiming
numbers.
