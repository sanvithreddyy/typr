// Windows-only low-level mouse hook for binding extra mouse buttons (Middle,
// XButton1, XButton2) as global hotkeys. The hook runs on a dedicated thread
// with its own message pump. The OS calls hook_proc synchronously for every
// mouse event, so the work done there must stay tiny — match button + check
// modifiers + push to a channel.

#![cfg(windows)]

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::OnceLock;
use std::thread;

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage, MSG,
    MSLLHOOKSTRUCT, WH_MOUSE_LL, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum MouseButton {
    Middle = 1,
    XButton1 = 2,
    XButton2 = 3,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ButtonState {
    Pressed,
    Released,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub win: bool,
}

impl Modifiers {
    fn as_bits(&self) -> u32 {
        (self.ctrl as u32)
            | ((self.shift as u32) << 1)
            | ((self.alt as u32) << 2)
            | ((self.win as u32) << 3)
    }

    fn from_bits(b: u32) -> Self {
        Self {
            ctrl: b & 1 != 0,
            shift: b & 2 != 0,
            alt: b & 4 != 0,
            win: b & 8 != 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MouseEvent {
    pub button: MouseButton,
    pub state: ButtonState,
}

// Global state read by the hook callback. 0 in REGISTERED_BUTTON disables matching.
static REGISTERED_BUTTON: AtomicU32 = AtomicU32::new(0);
static REGISTERED_MODIFIERS: AtomicU32 = AtomicU32::new(0);
// Tracks whether the bound button is currently held down via a matched press.
// Lets us still send Released events even if modifiers were lifted during the hold.
static IS_HELD: AtomicBool = AtomicBool::new(false);
static EVENT_SENDER: OnceLock<Sender<MouseEvent>> = OnceLock::new();

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    let bound_code = REGISTERED_BUTTON.load(Ordering::Relaxed);
    if bound_code == 0 {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    let msg = wparam.0 as u32;
    let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);

    let event_button = match msg {
        WM_MBUTTONDOWN | WM_MBUTTONUP => Some(MouseButton::Middle),
        WM_XBUTTONDOWN | WM_XBUTTONUP => match (info.mouseData >> 16) as u16 {
            1 => Some(MouseButton::XButton1),
            2 => Some(MouseButton::XButton2),
            _ => None,
        },
        _ => None,
    };

    let Some(eb) = event_button else {
        return CallNextHookEx(None, code, wparam, lparam);
    };

    if eb as u32 != bound_code {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    let is_down = matches!(msg, WM_MBUTTONDOWN | WM_XBUTTONDOWN);

    if is_down {
        // Check modifiers only on the press. If they don't match, the click
        // is "just a click" — pass it through so the back/forward/middle
        // button still does its normal job.
        let bound_mods = Modifiers::from_bits(REGISTERED_MODIFIERS.load(Ordering::Relaxed));
        if current_modifiers() != bound_mods {
            return CallNextHookEx(None, code, wparam, lparam);
        }
        IS_HELD.store(true, Ordering::Relaxed);
        if let Some(sender) = EVENT_SENDER.get() {
            let _ = sender.send(MouseEvent {
                button: eb,
                state: ButtonState::Pressed,
            });
        }
        return LRESULT(1);
    }

    // Released — only fire if we're tracking a held press, regardless of
    // current modifier state.
    if IS_HELD.swap(false, Ordering::Relaxed) {
        if let Some(sender) = EVENT_SENDER.get() {
            let _ = sender.send(MouseEvent {
                button: eb,
                state: ButtonState::Released,
            });
        }
        return LRESULT(1);
    }

    CallNextHookEx(None, code, wparam, lparam)
}

fn current_modifiers() -> Modifiers {
    unsafe {
        let pressed = |vk: u16| (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0;
        Modifiers {
            ctrl: pressed(VK_CONTROL.0),
            shift: pressed(VK_SHIFT.0),
            alt: pressed(VK_MENU.0),
            win: pressed(VK_LWIN.0) || pressed(VK_RWIN.0),
        }
    }
}

/// Install the global mouse hook. Returns a receiver that yields events
/// matching whatever is currently bound via `set_binding`. Call once at
/// startup.
pub fn install() -> Receiver<MouseEvent> {
    let (tx, rx) = mpsc::channel();
    let _ = EVENT_SENDER.set(tx);

    thread::spawn(|| unsafe {
        let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(hook_proc), None, 0);
        match hook {
            Ok(_h) => {
                println!("[Typr] WH_MOUSE_LL installed");
                let mut msg = MSG::default();
                loop {
                    let result = GetMessageW(&mut msg, None, 0, 0);
                    if result.0 <= 0 {
                        break;
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                println!("[Typr] Mouse hook message loop exited");
            }
            Err(e) => eprintln!("[Typr] Failed to install mouse hook: {}", e),
        }
    });

    rx
}

pub fn set_binding(button: MouseButton, modifiers: Modifiers) {
    REGISTERED_MODIFIERS.store(modifiers.as_bits(), Ordering::Relaxed);
    IS_HELD.store(false, Ordering::Relaxed);
    // Set button last so the hook never reads stale modifiers for a new button.
    REGISTERED_BUTTON.store(button as u32, Ordering::Relaxed);
}

pub fn clear_binding() {
    REGISTERED_BUTTON.store(0, Ordering::Relaxed);
    REGISTERED_MODIFIERS.store(0, Ordering::Relaxed);
    IS_HELD.store(false, Ordering::Relaxed);
}
