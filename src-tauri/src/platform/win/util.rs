use super::Shared;
use crate::config::{Theme, TrackingPrevention};
use crate::errors::{AppError, AppResult};
use crate::platform::ProfilePrefs;
use base64::Engine as _;
use std::cell::Cell;
use std::rc::Rc;
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use windows::core::{Interface, HSTRING, PWSTR};
use windows::Win32::Foundation::E_POINTER;
use windows::Win32::System::Com::IStream;

/// Reads a COM string out-parameter getter into an owned String.
pub fn get_string(f: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>) -> String {
    let mut p = PWSTR::null();
    match f(&mut p) {
        Ok(()) => take_pwstr(p),
        Err(_) => String::new(),
    }
}

/// Per-view settings for untrusted content. No host objects, no web messages: pages get no
/// channel to native code at all.
pub fn configure_settings(core: &ICoreWebView2) -> AppResult<()> {
    unsafe {
        let s = core.Settings()?;
        s.SetIsWebMessageEnabled(false)?;
        s.SetAreHostObjectsAllowed(false)?;
        s.SetIsStatusBarEnabled(true)?;
        // WebView2 DevTools are Edge DevTools (Edge branding, Microsoft panels); off by design.
        s.SetAreDevToolsEnabled(false)?;
        // The engine's error pages are Edge-branded; failures are shown by the shell instead
        // (blank view + EngineEvent::LoadFailed). Certificate errors then have no bypass at all.
        s.SetIsBuiltInErrorPageEnabled(false)?;
        s.SetAreDefaultContextMenusEnabled(true)?;
        s.SetIsZoomControlEnabled(true)?;
        if let Ok(s2) = s.cast::<ICoreWebView2Settings2>() {
            let ua = get_string(|p| s2.UserAgent(p));
            let plain = strip_edge_token(&ua);
            if plain != ua {
                s2.SetUserAgent(&HSTRING::from(plain))?;
            }
        }
        if let Ok(s4) = s.cast::<ICoreWebView2Settings4>() {
            s4.SetIsPasswordAutosaveEnabled(false)?;
            s4.SetIsGeneralAutofillEnabled(false)?;
        }
        core.AddScriptToExecuteOnDocumentCreated(
            &HSTRING::from(HIDE_ENGINE_SCRIPT),
            &AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(|_, _| Ok(()))),
        )?;
    }
    Ok(())
}

/// Runs before page scripts in every top-level document:
/// - deletes the inert `chrome.webview` object (web messages are disabled; it only reveals WebView2);
/// - drops "Microsoft Edge" brands from `navigator.userAgentData` (the HTTP `Sec-CH-UA` headers are
///   rewritten separately in the request handler). Workers still see the engine's original brands.
const HIDE_ENGINE_SCRIPT: &str = r#"(()=>{try{if(window.chrome&&window.chrome.webview){delete window.chrome.webview}}catch(e){}
try{const p=NavigatorUAData.prototype,f=l=>l.filter(b=>!/Edge/.test(b.brand)),d=Object.getOwnPropertyDescriptor(p,'brands');
Object.defineProperty(p,'brands',{get(){return f(d.get.call(this))},configurable:true,enumerable:true});
const g=p.getHighEntropyValues;p.getHighEntropyValues=function(h){return g.call(this,h).then(v=>{if(v.brands)v.brands=f(v.brands);if(v.fullVersionList)v.fullVersionList=f(v.fullVersionList);return v})};
const j=p.toJSON;if(j)p.toJSON=function(){const v=j.call(this);if(v.brands)v.brands=f(v.brands);return v}}catch(e){}})()"#;

/// `"Chromium";v="157", "Microsoft Edge";v="157", ...` → the same list without Edge entries.
pub fn strip_edge_brands(header: &str) -> String {
    header
        .split(',')
        .map(str::trim)
        .filter(|b| !b.contains("Edge"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Removes the ` Edg/<version>` product token so sites see a plain Chromium user agent
/// (no Edge branding; also blends in with the far larger Chrome population).
pub fn strip_edge_token(ua: &str) -> String {
    ua.split(' ')
        .filter(|t| !t.starts_with("Edg/"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn apply_profile_prefs(core: &ICoreWebView2, prefs: &ProfilePrefs) {
    let result: windows::core::Result<()> = (|| unsafe {
        let profile = core.cast::<ICoreWebView2_13>()?.Profile()?;
        profile.SetPreferredColorScheme(match prefs.theme {
            Theme::System => COREWEBVIEW2_PREFERRED_COLOR_SCHEME_AUTO,
            Theme::Light => COREWEBVIEW2_PREFERRED_COLOR_SCHEME_LIGHT,
            Theme::Dark => COREWEBVIEW2_PREFERRED_COLOR_SCHEME_DARK,
        })?;
        if let Some(dir) = &prefs.download_dir {
            profile.SetDefaultDownloadFolderPath(&HSTRING::from(dir.as_str()))?;
        }
        if let Ok(p3) = profile.cast::<ICoreWebView2Profile3>() {
            p3.SetPreferredTrackingPreventionLevel(match prefs.tracking_prevention {
                TrackingPrevention::Off => COREWEBVIEW2_TRACKING_PREVENTION_LEVEL_NONE,
                TrackingPrevention::Basic => COREWEBVIEW2_TRACKING_PREVENTION_LEVEL_BASIC,
                TrackingPrevention::Balanced => COREWEBVIEW2_TRACKING_PREVENTION_LEVEL_BALANCED,
                TrackingPrevention::Strict => COREWEBVIEW2_TRACKING_PREVENTION_LEVEL_STRICT,
            })?;
        }
        if let Ok(p6) = profile.cast::<ICoreWebView2Profile6>() {
            p6.SetIsPasswordAutosaveEnabled(false)?;
            p6.SetIsGeneralAutofillEnabled(false)?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        log::warn!("applying profile preferences failed: {}", e.message());
    }
}

/// Clears `jobs` (data kinds, and a start time in Unix seconds for a ranged job) from the profile;
/// `done` runs exactly once, after all of them finished, with the first error if any.
pub fn clear_profile_data(
    core: &ICoreWebView2,
    jobs: Vec<(COREWEBVIEW2_BROWSING_DATA_KINDS, Option<f64>)>,
    done: Box<dyn FnOnce(AppResult<()>)>,
) {
    // Shared by the error paths and the completion handlers: callers (exit with "clear cookies on
    // exit") wait for `done`, so it must run exactly once whichever way this ends.
    let done = Rc::new(Cell::new(Some(done)));
    let left = Rc::new(Cell::new(jobs.len()));
    let failure = Rc::new(Cell::new(None::<AppError>));
    if jobs.is_empty() {
        return done.take().map_or((), |d| d(Ok(())));
    }
    let finish: Rc<dyn Fn(AppResult<()>)> = Rc::new(move |r| {
        if let Err(e) = r {
            log::error!("clearing browsing data failed: {e}");
            let first = failure.take();
            failure.set(first.or(Some(e)));
        }
        left.set(left.get() - 1);
        if left.get() == 0 {
            if let Some(done) = done.take() {
                done(failure.take().map_or(Ok(()), Err));
            }
        }
    });
    for (kinds, since) in jobs {
        let result: windows::core::Result<()> = (|| unsafe {
            let profile: ICoreWebView2Profile2 =
                core.cast::<ICoreWebView2_13>()?.Profile()?.cast()?;
            let on_done = finish.clone();
            let handler = ClearBrowsingDataCompletedHandler::create(Box::new(move |r| {
                on_done(r.map_err(AppError::from));
                Ok(())
            }));
            match since {
                Some(start) => {
                    // Open-ended: everything from `start` on.
                    profile.ClearBrowsingDataInTimeRange(kinds, start, start + 1e10, &handler)
                }
                None => profile.ClearBrowsingData(kinds, &handler),
            }
        })();
        if let Err(e) = result {
            finish(Err(e.into()));
        }
    }
}

/// Creates a hidden, non-private view, runs `f` exactly once with it (or with the error that
/// prevented it), then closes the view once `f` returns.
pub fn with_temporary_view(
    shared: &Rc<Shared>,
    f: impl FnOnce(windows::core::Result<ICoreWebView2>) + 'static,
) {
    let f = Rc::new(Cell::new(Some(f)));
    let pending = f.clone();
    let created: windows::core::Result<()> = (|| unsafe {
        let env10: ICoreWebView2Environment10 = shared.env.cast()?;
        let options = env10.CreateCoreWebView2ControllerOptions()?;
        options.SetProfileName(&HSTRING::from("Browsing"))?;
        let handler =
            CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |r, controller| {
                let Some(f) = pending.take() else {
                    return Ok(());
                };
                let controller = match r.and_then(|()| controller.ok_or_else(|| E_POINTER.into())) {
                    Ok(c) => c,
                    Err(e) => {
                        f(Err(e));
                        return Ok(());
                    }
                };
                f(controller
                    .SetIsVisible(false)
                    .and_then(|()| controller.CoreWebView2()));
                // Profile operations keep their own reference; closing the view does not cancel them.
                controller.Close()
            }));
        env10.CreateCoreWebView2ControllerWithOptions(shared.host, &options, &handler)
    })();
    if let Err(e) = created {
        log::error!("creating a temporary view failed: {}", e.message());
        if let Some(f) = f.take() {
            f(Err(e));
        }
    }
}

/// Reads up to `limit` bytes from a COM stream.
pub fn read_stream(stream: &IStream, limit: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        let mut read = 0u32;
        let hr = unsafe { stream.Read(buf.as_mut_ptr().cast(), buf.len() as u32, Some(&mut read)) };
        if hr.is_err() || read == 0 {
            break;
        }
        out.extend_from_slice(&buf[..read as usize]);
        if out.len() > limit {
            return Vec::new();
        }
    }
    out
}

pub fn png_data_url(bytes: &[u8]) -> String {
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn edge_token_is_removed() {
        let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/157.0.0.0 Safari/537.36 Edg/157.0.4341.46";
        assert_eq!(
            super::strip_edge_token(ua),
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/157.0.0.0 Safari/537.36"
        );
        assert_eq!(
            super::strip_edge_token("Chrome/1 Safari/2"),
            "Chrome/1 Safari/2"
        );
    }

    #[test]
    fn edge_brands_are_removed_from_client_hints() {
        let h = r#""Chromium";v="157", "Microsoft Edge WebView2";v="157", "Not-A.Brand";v="24", "Microsoft Edge";v="157""#;
        assert_eq!(
            super::strip_edge_brands(h),
            r#""Chromium";v="157", "Not-A.Brand";v="24""#
        );
    }
}
