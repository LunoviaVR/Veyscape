//! Password manager bridge: how login forms on a page reach the browser core, and how saved
//! logins are typed back in, without giving the page any channel to native code.
//!
//! Per document, through the host-only DevTools protocol (never exposed to the page):
//! 1. `Page.createIsolatedWorld` makes a private JavaScript world for the main frame. It shares
//!    the DOM with the page but none of its globals, so page scripts cannot see or call into it.
//! 2. `Runtime.addBinding` puts `__vsPw` into that world only. Calls arrive here as
//!    `Runtime.bindingCalled` and are accepted only from the context created in step 1.
//! 3. `Runtime.evaluate` runs [`SCRIPT`] there; filling evaluates `__vsFill(...)` there too.
//!
//! The `Runtime` domain is never enabled: pages can detect that, and it slows them down.
//! Out-of-process iframes (cross-site login frames) are not covered.

use super::util::get_string;
use super::{Shared, ViewState};
use crate::errors::{AppError, AppResult};
use crate::passwords::{Secret, MAX_PASSWORD, MAX_USERNAME};
use crate::platform::{EngineEvent, TabId};
use std::rc::Rc;
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use windows::core::{Interface, HSTRING};

const WORLD: &str = "veyscape-passwords";
const BINDING: &str = "__vsPw";

/// Runs in the private world of each top-level http(s) document. Reports a password field once
/// (`form`), every submission (`submit`), and a submitted form going away without navigating
/// (`done`). Defines `__vsFill(origin, username, password)` for filling; it fills nothing in a
/// document of another origin (this world's `location` cannot be faked by the page).
const SCRIPT: &str = r#"(()=>{
if(window.top!==window||window.__vsFill)return;
const send=m=>{try{__vsPw(JSON.stringify(m))}catch(e){}};
const vis=e=>e.isConnected&&!!(e.offsetWidth||e.offsetHeight||e.getClientRects().length);
const pws=s=>[...s.querySelectorAll('input[type=password]')];
const texty=e=>/^(text|email|tel)$/.test(e.type);
const tagged=e=>/\b(username|email)\b/.test(e.autocomplete||'');
const scopeOf=e=>(e&&e.closest&&e.closest('form'))||document;
const userFor=(scope,pw,filled)=>{const all=[...scope.querySelectorAll('input')],ok=e=>filled?!!e.value:vis(e);
 const t=all.find(e=>tagged(e)&&ok(e));if(t)return t;
 for(let i=all.indexOf(pw)-1;i>=0;i--){const e=all[i];if(texty(e)&&vis(e)&&(!filled||e.value))return e}return null};
let last='',poll=0;
const capture=el=>{const scope=scopeOf(el),filled=pws(scope).filter(e=>e.value);if(!filled.length)return;
 const pw=filled[filled.length-1],u=userFor(scope,filled[0],true),user=u?u.value.trim():'';
 if(user+'\n'+pw.value===last)return;last=user+'\n'+pw.value;
 send({t:'submit',o:location.origin,u:user,p:pw.value});
 clearInterval(poll);const t0=Date.now();
 poll=setInterval(()=>{if(!vis(pw)){clearInterval(poll);send({t:'done'})}else if(Date.now()-t0>15000)clearInterval(poll)},400)};
document.addEventListener('submit',e=>capture(e.target),true);
document.addEventListener('keydown',e=>{if(e.key==='Enter'&&e.target instanceof HTMLInputElement)capture(e.target)},true);
document.addEventListener('click',e=>{const b=e.target instanceof Element&&e.target.closest('button,input[type=submit],input[type=button],[role=button]');if(b)capture(b)},true);
let found=false,timer=0;
const look=()=>{if(found||!pws(document).length)return;found=true;obs.disconnect();send({t:'form',o:location.origin})};
const obs=new MutationObserver(()=>{if(!timer)timer=setTimeout(()=>{timer=0;look()},300)});
obs.observe(document.documentElement,{childList:true,subtree:true});look();
const set=(e,v)=>{if(!e||e.readOnly||e.disabled)return false;e.focus();e.value=v;
 e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));return true};
window.__vsFill=(o,u,p)=>{if(location.origin!==o)return false;const a=document.activeElement;
 const pw=(a instanceof HTMLInputElement&&a.type==='password'?a:null)||pws(scopeOf(a)).find(vis)||pws(document).find(vis);
 let uf=pw?userFor(scopeOf(pw),pw,false):null;
 if(!pw&&!uf)uf=(a instanceof HTMLInputElement&&texty(a)?a:null)||[...document.querySelectorAll('input')].find(e=>tagged(e)&&vis(e))||null;
 const wroteU=!!u&&set(uf,u),wroteP=set(pw,p);return wroteU||wroteP};
})()"#;

/// Calls a DevTools protocol method; `then` gets the parsed result on success.
fn cdp(
    core: &ICoreWebView2,
    method: &'static str,
    params: serde_json::Value,
    then: impl FnOnce(serde_json::Value) + 'static,
) {
    cdp_result(core, method, params, |v| {
        if let Some(v) = v {
            then(v)
        }
    });
}

/// Like [`cdp`], but `done` runs exactly once, with `None` if the call failed.
fn cdp_result(
    core: &ICoreWebView2,
    method: &'static str,
    params: serde_json::Value,
    done: impl FnOnce(Option<serde_json::Value>) + 'static,
) {
    // Shared with the completion handler: a call that fails to start never completes.
    let done = Rc::new(std::cell::Cell::new(Some(done)));
    let pending = done.clone();
    let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |r, json| {
        if r.is_err() {
            log::debug!("{method} failed");
        }
        if let Some(f) = pending.take() {
            f(r.ok().and_then(|_| serde_json::from_str(&json).ok()));
        }
        Ok(())
    }));
    let started = unsafe {
        core.CallDevToolsProtocolMethod(
            &HSTRING::from(method),
            &HSTRING::from(params.to_string()),
            &handler,
        )
    };
    if started.is_err() {
        if let Some(f) = done.take() {
            f(None);
        }
    }
}

/// Sets up the private world and script for the current main-frame document.
fn attach(core: ICoreWebView2, state: Rc<ViewState>) {
    state.login_context.set(0);
    let c = core.clone();
    cdp(
        &core,
        "Page.getFrameTree",
        serde_json::json!({}),
        move |v| {
            let Some(frame) = v["frameTree"]["frame"]["id"].as_str() else {
                return;
            };
            let world = serde_json::json!({ "frameId": frame, "worldName": WORLD });
            let c2 = c.clone();
            cdp(&c, "Page.createIsolatedWorld", world, move |v| {
                let Some(ctx) = v["executionContextId"].as_i64() else {
                    return;
                };
                state.login_context.set(ctx);
                let c3 = c2.clone();
                let binding = serde_json::json!({ "name": BINDING, "executionContextName": WORLD });
                cdp(&c2, "Runtime.addBinding", binding, move |_| {
                    let run = serde_json::json!({ "expression": SCRIPT, "contextId": ctx });
                    cdp(&c3, "Runtime.evaluate", run, |_| {});
                });
            });
        },
    );
}

#[derive(serde::Deserialize)]
struct BindingCalled {
    name: String,
    payload: String,
    #[serde(rename = "executionContextId")]
    context: i64,
}

#[derive(serde::Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
enum Message {
    Form { o: String },
    Submit { o: String, u: String, p: String },
    Done,
}

/// Turns one binding call into an engine event. Anything not from our own world is dropped.
fn event_for(tab: TabId, expected_context: i64, json: &str) -> Option<EngineEvent> {
    let call: BindingCalled = serde_json::from_str(json).ok()?;
    if call.name != BINDING || call.context == 0 || call.context != expected_context {
        return None;
    }
    if call.payload.len() > 4 * (MAX_USERNAME + MAX_PASSWORD) {
        return None;
    }
    Some(match serde_json::from_str(&call.payload).ok()? {
        Message::Form { o } => EngineEvent::LoginFormFound { tab, origin: o },
        Message::Submit { o, u, p } => EngineEvent::LoginSubmitted {
            tab,
            origin: o,
            username: u,
            password: Secret(p),
        },
        Message::Done => EngineEvent::LoginSucceeded { tab },
    })
}

pub fn register(shared: &Rc<Shared>, core: &ICoreWebView2, state: &Rc<ViewState>) -> AppResult<()> {
    let mut token = 0i64;
    unsafe {
        let receiver =
            core.GetDevToolsProtocolEventReceiver(&HSTRING::from("Runtime.bindingCalled"))?;
        let (s, st) = (shared.clone(), state.clone());
        receiver.add_DevToolsProtocolEventReceived(
            &DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let json = get_string(|p| args.ParameterObjectAsJson(p));
                if let Some(event) = event_for(st.tab, st.login_context.get(), &json) {
                    s.emit(event);
                }
                Ok(())
            })),
            &mut token,
        )?;
        let c2 = core.cast::<ICoreWebView2_2>()?;
        let st = state.clone();
        c2.add_DOMContentLoaded(
            &DOMContentLoadedEventHandler::create(Box::new(move |sender, _| {
                let web = st.page_url.borrow().starts_with("http");
                if let (Some(core), true) = (sender, web) {
                    attach(core, st.clone());
                }
                Ok(())
            })),
            &mut token,
        )?;
    }
    Ok(())
}

/// Fills the page's login form with a saved login if the document is still on `origin`. `done`
/// gets whether anything was filled.
pub fn fill(
    core: &ICoreWebView2,
    state: &ViewState,
    origin: &str,
    username: &str,
    password: &str,
    done: Box<dyn FnOnce(AppResult<bool>)>,
) {
    let ctx = state.login_context.get();
    if ctx == 0 {
        return done(Err(AppError::NotFound("login form on this page".into())));
    }
    // JSON string literals: the values cannot break out of the expression.
    let literals = (|| {
        Ok::<_, serde_json::Error>(format!(
            "__vsFill({}, {}, {})",
            serde_json::to_string(origin)?,
            serde_json::to_string(username)?,
            serde_json::to_string(password)?
        ))
    })();
    let expression = match literals {
        Ok(e) => e,
        Err(e) => return done(Err(AppError::Engine(e.to_string()))),
    };
    let params = serde_json::json!({ "expression": expression, "contextId": ctx });
    cdp_result(core, "Runtime.evaluate", params, move |v| {
        done(match v {
            Some(v) => Ok(fill_succeeded(&v)),
            None => Err(AppError::Engine("filling the login failed".into())),
        })
    });
}

/// `Runtime.evaluate`'s answer for `__vsFill(...)`: true only if it returned true.
fn fill_succeeded(result: &serde_json::Value) -> bool {
    result["result"]["value"].as_bool() == Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(ctx: i64, payload: serde_json::Value) -> String {
        serde_json::json!({ "name": BINDING, "executionContextId": ctx, "payload": payload.to_string() })
            .to_string()
    }

    #[test]
    fn fill_reports_only_a_real_fill() {
        let r = |v: serde_json::Value| serde_json::json!({ "result": v });
        assert!(fill_succeeded(&r(
            serde_json::json!({ "type": "boolean", "value": true })
        )));
        assert!(!fill_succeeded(&r(
            serde_json::json!({ "type": "boolean", "value": false })
        )));
        assert!(!fill_succeeded(&serde_json::json!({
            "result": { "type": "object" },
            "exceptionDetails": { "text": "Uncaught" }
        })));
    }

    #[test]
    fn accepts_only_our_context() {
        let submit =
            serde_json::json!({ "t": "submit", "o": "https://a.example", "u": "ann", "p": "pw" });
        match event_for(1, 5, &call(5, submit.clone())) {
            Some(EngineEvent::LoginSubmitted {
                username, password, ..
            }) => {
                assert_eq!(username, "ann");
                assert_eq!(password.0, "pw");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(event_for(1, 5, &call(6, submit.clone())).is_none());
        assert!(event_for(1, 0, &call(0, submit)).is_none());
        let done = serde_json::json!({ "t": "done" });
        assert!(matches!(
            event_for(1, 5, &call(5, done)),
            Some(EngineEvent::LoginSucceeded { tab: 1 })
        ));
        assert!(event_for(1, 5, &call(5, serde_json::json!({ "t": "other" }))).is_none());
        let wrong_name =
            serde_json::json!({ "name": "x", "executionContextId": 5, "payload": "{}" });
        assert!(event_for(1, 5, &wrong_name.to_string()).is_none());
    }
}
