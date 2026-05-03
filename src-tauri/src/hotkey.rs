// Hotkey string parsing. Strings combine modifiers and a key joined by '+',
// e.g. "Ctrl+Shift+Space" or "Ctrl+XButton1". A mouse token routes to the
// Win32 mouse hook; everything else routes to tauri-plugin-global-shortcut
// which has its own parser.

#[cfg(windows)]
use crate::mouse_hook::{Modifiers, MouseButton};

#[cfg(windows)]
#[derive(Debug)]
pub enum Hotkey {
    Keyboard(String),
    Mouse {
        button: MouseButton,
        modifiers: Modifiers,
    },
}

#[cfg(not(windows))]
#[derive(Debug)]
pub enum Hotkey {
    Keyboard(String),
}

/// True if the hotkey string names any mouse button. Cross-platform — used
/// during settings migration to route a legacy single hotkey string into the
/// correct slot, even on non-Windows targets where `parse` would always
/// return `Keyboard`.
pub fn is_mouse_hotkey(s: &str) -> bool {
    s.split('+').any(|tok| {
        matches!(
            tok.trim().to_lowercase().as_str(),
            "xbutton1"
                | "mouse4"
                | "mousebutton4"
                | "back"
                | "xbutton2"
                | "mouse5"
                | "mousebutton5"
                | "forward"
                | "middlemouse"
                | "mousemiddle"
                | "mouse3"
                | "middle"
        )
    })
}

pub fn parse(s: &str) -> Hotkey {
    #[cfg(windows)]
    {
        let mut mouse_button: Option<MouseButton> = None;
        let mut modifiers = Modifiers::default();

        for raw in s.split('+') {
            match raw.trim().to_lowercase().as_str() {
                "ctrl" | "control" | "cmdorctrl" | "cmd" | "command" => modifiers.ctrl = true,
                "shift" => modifiers.shift = true,
                "alt" | "option" => modifiers.alt = true,
                "win" | "super" | "meta" => modifiers.win = true,
                "xbutton1" | "mouse4" | "mousebutton4" | "back" => {
                    mouse_button = Some(MouseButton::XButton1)
                }
                "xbutton2" | "mouse5" | "mousebutton5" | "forward" => {
                    mouse_button = Some(MouseButton::XButton2)
                }
                "middlemouse" | "mousemiddle" | "mouse3" | "middle" => {
                    mouse_button = Some(MouseButton::Middle)
                }
                _ => {}
            }
        }

        if let Some(button) = mouse_button {
            return Hotkey::Mouse { button, modifiers };
        }
    }

    Hotkey::Keyboard(s.to_string())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn keyboard_only_passthrough() {
        match parse("Ctrl+Shift+Space") {
            Hotkey::Keyboard(s) => assert_eq!(s, "Ctrl+Shift+Space"),
            _ => panic!("expected keyboard"),
        }
    }

    #[test]
    fn cmdorctrl_passes_through_unchanged() {
        match parse("CmdOrCtrl+Shift+Space") {
            Hotkey::Keyboard(s) => assert_eq!(s, "CmdOrCtrl+Shift+Space"),
            _ => panic!("expected keyboard"),
        }
    }

    #[test]
    fn xbutton1_alone() {
        match parse("XButton1") {
            Hotkey::Mouse { button, modifiers } => {
                assert_eq!(button, MouseButton::XButton1);
                assert_eq!(modifiers, Modifiers::default());
            }
            _ => panic!("expected mouse"),
        }
    }

    #[test]
    fn ctrl_shift_xbutton2() {
        match parse("Ctrl+Shift+XButton2") {
            Hotkey::Mouse { button, modifiers } => {
                assert_eq!(button, MouseButton::XButton2);
                assert!(modifiers.ctrl);
                assert!(modifiers.shift);
                assert!(!modifiers.alt);
                assert!(!modifiers.win);
            }
            _ => panic!("expected mouse"),
        }
    }

    #[test]
    fn middle_mouse_with_alt() {
        match parse("Alt+MiddleMouse") {
            Hotkey::Mouse { button, modifiers } => {
                assert_eq!(button, MouseButton::Middle);
                assert!(modifiers.alt);
            }
            _ => panic!("expected mouse"),
        }
    }

    #[test]
    fn aliases_and_case_insensitive() {
        match parse("ctrl+mouse4") {
            Hotkey::Mouse { button, modifiers } => {
                assert_eq!(button, MouseButton::XButton1);
                assert!(modifiers.ctrl);
            }
            _ => panic!("expected mouse"),
        }
    }
}
