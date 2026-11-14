//! Saved logins in the browser core: offering to save one after the user logs in, and typing a
//! saved one into the page. Pages report forms through the engine (see `platform/win/passwords.rs`).

use super::{url, Prompt, Window};
use crate::errors::{AppError, AppResult};
use crate::passwords::{self, SaveAction, Secret};
use crate::platform::TabId;
use crate::storage::Login;
use tauri::Emitter;

/// A submitted login, held until the page navigates or the form goes away, so a login the site
/// rejected (form still there) is not offered for saving.
pub(super) struct PendingLogin {
    origin: String,
    username: String,
    password: Secret,
}

impl Window {
    /// The page has a password field: count the logins saved for it (shows the key button).
    pub(super) fn login_form_found(&mut self, tab: TabId, origin: &str) {
        let Some(t) = self.tabs.get(tab) else { return };
        if t.internal.is_some() || url::origin_of(&t.url).as_deref() != Some(origin) {
            return;
        }
        let saved = self
            .services
            .db
            .logins_for_origin(origin)
            .map(|l| l.len() as u32)
            .unwrap_or(0);
        if let Some(t) = self.tabs.get_mut(tab) {
            t.saved_logins = saved;
        }
        self.mark_dirty();
    }

    pub(super) fn login_submitted(
        &mut self,
        tab: TabId,
        origin: String,
        username: String,
        password: Secret,
    ) {
        // Private tabs never save anything.
        let private = self.tabs.get(tab).is_none_or(|t| t.private);
        if private || !self.services.settings().offer_to_save_passwords {
            return;
        }
        let web_origin = url::origin_of(&origin).is_some_and(|o| o == origin);
        if !web_origin || !passwords::valid_login(&username, &password.0) {
            return;
        }
        self.pending_logins.insert(
            tab,
            PendingLogin {
                origin,
                username,
                password,
            },
        );
    }

    /// Offers to save the tab's pending login, if any: called once the page navigates on or the
    /// submitted form disappears.
    pub(super) fn offer_pending_login(&mut self, tab: TabId) {
        let Some(p) = self.pending_logins.remove(&tab) else {
            return;
        };
        let db = &self.services.db;
        if db.login_never_has(&p.origin).unwrap_or(true) {
            return;
        }
        let saved: Vec<_> = db
            .login_secrets_for_origin(&p.origin)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(id, user, blob)| Some((id, user, passwords::unprotect(&blob).ok()?)))
            .collect();
        let update = match passwords::save_action(&saved, &p.username, &p.password.0) {
            SaveAction::Known(id) => {
                let _ = db.login_used(id);
                return;
            }
            SaveAction::Update => true,
            SaveAction::Save => false,
        };
        // One offer per tab: `push_prompt` replaces an unanswered one.
        self.push_prompt(|id| Prompt::SavePassword {
            id,
            tab_id: tab,
            origin: p.origin,
            username: p.username,
            update,
            password: p.password,
        });
    }

    /// The answer to a "Save password?" prompt: save it, never ask for this site, or neither.
    pub(super) fn answer_save_login(
        &mut self,
        origin: &str,
        username: &str,
        password: &Secret,
        save: bool,
        never: bool,
    ) -> AppResult<()> {
        if save {
            let blob = passwords::protect(&password.0)?;
            self.services.db.login_save(origin, username, &blob)?;
        } else if never {
            self.services.db.login_never_set(origin, true)?;
        }
        let _ = self.app.emit("passwords:changed", ());
        Ok(())
    }

    pub fn logins_for_tab(&self, tab: TabId) -> AppResult<Vec<Login>> {
        let t = self
            .tabs
            .get(tab)
            .ok_or_else(|| AppError::NotFound("tab".into()))?;
        match url::origin_of(&t.url) {
            Some(origin) if t.internal.is_none() => self.services.db.logins_for_origin(&origin),
            _ => Ok(Vec::new()),
        }
    }

    /// Types a saved login into the tab's page. `done` gets whether a form was really filled;
    /// only then does the login count as used.
    pub fn fill_login(
        &mut self,
        tab: TabId,
        id: i64,
        done: Box<dyn FnOnce(AppResult<()>)>,
    ) -> AppResult<()> {
        let t = self
            .tabs
            .get(tab)
            .ok_or_else(|| AppError::NotFound("tab".into()))?;
        let (private, page_origin) = (t.private, url::origin_of(&t.url));
        let (origin, username, blob) = self
            .services
            .db
            .login_secret(id)?
            .ok_or_else(|| AppError::NotFound("login".into()))?;
        // A saved login is only ever typed into the site it was saved for.
        if t.internal.is_some() || page_origin.as_deref() != Some(origin.as_str()) {
            return Err(AppError::InvalidInput(
                "this login belongs to another site".into(),
            ));
        }
        let password = Secret(passwords::unprotect(&blob)?);
        let services = self.services.clone();
        self.engine.fill_login(
            tab,
            &origin,
            &username,
            &password.0,
            Box::new(move |filled| {
                let result = filled.and_then(|ok| {
                    ok.then_some(())
                        .ok_or_else(|| AppError::NotFound("login form on this page".into()))
                });
                if result.is_ok() && !private {
                    let _ = services.db.login_used(id);
                }
                done(result);
            }),
        );
        self.engine.focus_view(tab);
        Ok(())
    }
}
