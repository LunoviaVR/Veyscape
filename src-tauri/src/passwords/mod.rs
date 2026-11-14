//! Saved logins: what a submitted login means for the store, encryption at rest (the Windows
//! user's DPAPI key) and Windows Hello before a saved password is shown or edited.
//! How logins reach the core from pages: `platform/win/passwords.rs`; see docs/security.md.

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::{protect, unprotect, verify_user};

#[cfg(not(windows))]
mod unsupported {
    use crate::errors::{AppError, AppResult};
    pub fn protect(_: &str) -> AppResult<Vec<u8>> {
        Err(AppError::Unsupported("password storage"))
    }
    pub fn unprotect(_: &[u8]) -> AppResult<String> {
        Err(AppError::Unsupported("password storage"))
    }
    pub fn verify_user(_: &tauri::Window, _: &str) -> AppResult<bool> {
        Err(AppError::Unsupported("password storage"))
    }
}
#[cfg(not(windows))]
pub use unsupported::{protect, unprotect, verify_user};

/// Bounds for values read from pages (a page controls its own form fields).
pub const MAX_USERNAME: usize = 256;
pub const MAX_PASSWORD: usize = 1024;

/// A password in memory. Its `Debug` output is redacted so it cannot end up in a log line.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(pub String);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(…)")
    }
}

/// True if a username/password pair may be stored.
pub fn valid_login(username: &str, password: &str) -> bool {
    username.len() <= MAX_USERNAME && !password.is_empty() && password.len() <= MAX_PASSWORD
}

#[derive(Debug, PartialEq, Eq)]
pub enum SaveAction {
    /// New username for this site: offer to save.
    Save,
    /// Known username with a different password: offer to update it.
    Update,
    /// Exactly this login is already saved.
    Known(i64),
}

/// `saved`: (id, username, password) of the logins stored for the same origin.
pub fn save_action(saved: &[(i64, String, String)], username: &str, password: &str) -> SaveAction {
    match saved.iter().find(|(_, u, _)| u == username) {
        Some((id, _, p)) if p == password => SaveAction::Known(*id),
        Some(_) => SaveAction::Update,
        None => SaveAction::Save,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_update_or_known() {
        let saved = vec![(7, "ann".to_string(), "pw1".to_string())];
        assert_eq!(save_action(&saved, "ann", "pw1"), SaveAction::Known(7));
        assert_eq!(save_action(&saved, "ann", "pw2"), SaveAction::Update);
        assert_eq!(save_action(&saved, "bob", "pw1"), SaveAction::Save);
        assert_eq!(save_action(&[], "", "pw"), SaveAction::Save);
    }

    #[test]
    fn login_bounds() {
        assert!(valid_login("", "x"));
        assert!(!valid_login("ann", ""));
        assert!(!valid_login(&"a".repeat(MAX_USERNAME + 1), "x"));
        assert!(!valid_login("ann", &"x".repeat(MAX_PASSWORD + 1)));
    }

    #[test]
    fn secret_is_not_printed() {
        assert_eq!(format!("{:?}", Secret("hunter2".into())), "Secret(…)");
    }

    #[cfg(windows)]
    #[test]
    fn encryption_round_trip() {
        let blob = protect("p@ss wörd").unwrap();
        assert!(!blob.windows(4).any(|w| w == b"p@ss"));
        assert_eq!(unprotect(&blob).unwrap(), "p@ss wörd");
        assert!(unprotect(b"not a dpapi blob").is_err());
    }
}
