//! Site permission model. Decisions are stored per origin in SQLite (`storage::permissions`);
//! the engine never persists its own decisions.

use crate::errors::{AppError, AppResult};
use crate::storage::Db;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum PermissionKind {
    Camera,
    Microphone,
    Location,
    Notifications,
    /// A page opening windows without a user gesture.
    Popups,
    /// A page starting several downloads without a user gesture.
    AutomaticDownloads,
}

impl PermissionKind {
    pub const ALL: [PermissionKind; 6] = [
        PermissionKind::Camera,
        PermissionKind::Microphone,
        PermissionKind::Location,
        PermissionKind::Notifications,
        PermissionKind::Popups,
        PermissionKind::AutomaticDownloads,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            PermissionKind::Camera => "camera",
            PermissionKind::Microphone => "microphone",
            PermissionKind::Location => "location",
            PermissionKind::Notifications => "notifications",
            PermissionKind::Popups => "popups",
            PermissionKind::AutomaticDownloads => "automaticDownloads",
        }
    }

    pub fn parse(s: &str) -> AppResult<Self> {
        Self::ALL
            .into_iter()
            .find(|k| k.as_str() == s)
            .ok_or_else(|| AppError::InvalidInput(format!("unknown permission '{s}'")))
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Decision {
    Allow,
    Deny,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Decision::Allow => "allow",
            Decision::Deny => "deny",
        }
    }
}

/// Stored decision for `origin`, if the user made one.
pub fn stored(db: &Db, origin: &str, kind: PermissionKind) -> AppResult<Option<Decision>> {
    Ok(match db.permission_get(origin, kind.as_str())?.as_deref() {
        Some("allow") => Some(Decision::Allow),
        Some("deny") => Some(Decision::Deny),
        _ => None,
    })
}

pub fn remember(
    db: &Db,
    origin: &str,
    kind: PermissionKind,
    decision: Option<Decision>,
) -> AppResult<()> {
    if crate::browser::url::origin_of(origin).as_deref() != Some(origin) {
        return Err(AppError::InvalidInput(
            "permissions are stored per http(s) origin".into(),
        ));
    }
    db.permission_set(origin, kind.as_str(), decision.map(Decision::as_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remember_round_trip_and_origin_validation() {
        let db = Db::open_in_memory().unwrap();
        let o = "https://meet.example";
        assert_eq!(stored(&db, o, PermissionKind::Camera).unwrap(), None);
        remember(&db, o, PermissionKind::Camera, Some(Decision::Allow)).unwrap();
        assert_eq!(
            stored(&db, o, PermissionKind::Camera).unwrap(),
            Some(Decision::Allow)
        );
        assert_eq!(stored(&db, o, PermissionKind::Microphone).unwrap(), None);
        remember(&db, o, PermissionKind::Camera, None).unwrap();
        assert_eq!(stored(&db, o, PermissionKind::Camera).unwrap(), None);
        assert!(remember(
            &db,
            "https://meet.example/path",
            PermissionKind::Camera,
            None
        )
        .is_err());
        assert!(remember(&db, "file:///", PermissionKind::Camera, None).is_err());
    }

    #[test]
    fn kinds_round_trip_through_strings() {
        for k in PermissionKind::ALL {
            assert_eq!(PermissionKind::parse(k.as_str()).unwrap(), k);
        }
        assert!(PermissionKind::parse("usb").is_err());
    }
}
