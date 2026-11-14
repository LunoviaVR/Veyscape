//! Keyboard shortcuts. The table lives in `shared/shortcuts.json` and is the single source of
//! truth for both the React shell (key events while the UI has focus) and the native engine
//! (key events while a web page has focus). Rust only maps keys to action names; the shell
//! executes every action, so behaviour is identical regardless of where focus is.

use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Deserialize, Debug)]
struct Binding {
    action: String,
    code: String,
    ctrl: bool,
    shift: bool,
    alt: bool,
}

fn table() -> &'static [(Binding, &'static str)] {
    static TABLE: OnceLock<Vec<(Binding, &'static str)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let bindings: Vec<Binding> =
            serde_json::from_str(include_str!("../../../shared/shortcuts.json"))
                .expect("shared/shortcuts.json is valid (checked by tests)");
        bindings
            .into_iter()
            .map(|b| {
                // Leaked once at startup so actions can be `&'static str` in engine events.
                let action: &'static str = Box::leak(b.action.clone().into_boxed_str());
                (b, action)
            })
            .collect()
    })
}

/// Returns the action bound to a `KeyboardEvent.code`-style key with these modifiers.
pub fn lookup(code: &str, ctrl: bool, shift: bool, alt: bool) -> Option<&'static str> {
    table()
        .iter()
        .find(|(b, _)| b.code == code && b.ctrl == ctrl && b.shift == shift && b.alt == alt)
        .map(|(_, action)| *action)
}

/// Windows virtual-key code → `KeyboardEvent.code` for the keys used in the table.
pub fn code_from_virtual_key(vk: u32) -> Option<String> {
    Some(match vk {
        0x41..=0x5A => format!("Key{}", char::from_u32(vk)?),
        0x30..=0x39 => format!("Digit{}", vk - 0x30),
        0x60..=0x69 => format!("Numpad{}", vk - 0x60),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        0x09 => "Tab".into(),
        0x1B => "Escape".into(),
        0x21 => "PageUp".into(),
        0x24 => "Home".into(),
        0x22 => "PageDown".into(),
        0x25 => "ArrowLeft".into(),
        0x27 => "ArrowRight".into(),
        0x6B => "NumpadAdd".into(),
        0x6D => "NumpadSubtract".into(),
        0xBB => "Equal".into(),
        0xBD => "Minus".into(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_parses_and_has_no_conflicting_bindings() {
        let t = table();
        assert!(t.len() > 30);
        for (i, (a, _)) in t.iter().enumerate() {
            for (b, _) in &t[i + 1..] {
                let same =
                    a.code == b.code && a.ctrl == b.ctrl && a.shift == b.shift && a.alt == b.alt;
                assert!(!same, "duplicate binding for {}", a.code);
            }
        }
    }

    #[test]
    fn required_shortcuts_exist() {
        assert_eq!(lookup("KeyT", true, false, false), Some("newTab"));
        assert_eq!(lookup("KeyT", true, true, false), Some("reopenClosedTab"));
        assert_eq!(lookup("Tab", true, true, false), Some("previousTab"));
        assert_eq!(lookup("Digit9", true, false, false), Some("selectLastTab"));
        assert_eq!(lookup("KeyT", false, false, false), None);
    }

    #[test]
    fn virtual_keys_map_to_codes() {
        assert_eq!(code_from_virtual_key(0x54).as_deref(), Some("KeyT"));
        assert_eq!(code_from_virtual_key(0x39).as_deref(), Some("Digit9"));
        assert_eq!(code_from_virtual_key(0x74).as_deref(), Some("F5"));
        assert_eq!(code_from_virtual_key(0xBB).as_deref(), Some("Equal"));
        assert_eq!(code_from_virtual_key(0x10), None);
    }
}
