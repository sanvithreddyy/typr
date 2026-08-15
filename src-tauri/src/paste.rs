#[cfg(target_os = "macos")]
pub fn capture_paste_target() -> Option<i32> {
    use objc2_app_kit::NSWorkspace;

    NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map(|app| app.processIdentifier())
}

#[cfg(not(target_os = "macos"))]
pub fn capture_paste_target() -> Option<i32> {
    None
}

pub fn paste_text(text: &str, target_pid: Option<i32>) -> Result<(), String> {
    // Set clipboard (arboard is thread-safe)
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard.set_text(text).map_err(|e| e.to_string())?;

    // Small delay to ensure clipboard is set
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Use macOS System Events to activate the app that owned focus when
    // recording began and invoke its standard Paste shortcut. This remains
    // reliable while transcription runs asynchronously in the background.
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;

        let script = match target_pid {
            Some(pid) => format!(
                "tell application \"System Events\"\nset targetProcess to first application process whose unix id is {}\nset frontmost of targetProcess to true\ndelay 0.15\ntell targetProcess to click menu item \"Paste\" of menu \"Edit\" of menu bar 1\nend tell",
                pid
            ),
            None => {
                "tell application \"System Events\" to keystroke \"v\" using command down"
                    .to_string()
            }
        };
        let output = Command::new("/usr/bin/osascript")
            .args(["-e", &script])
            .output()
            .map_err(|e| format!("Failed to run the macOS paste shortcut: {}", e))?;

        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(format!(
                "Typr copied the transcript, but macOS blocked its paste shortcut. Allow Typr under System Settings → Privacy & Security → Accessibility and Automation, then try again. {}",
                detail
            ));
        }

        println!("[Typr] Ran macOS paste shortcut for PID {:?}", target_pid);
    }

    #[cfg(target_os = "windows")]
    {
        // Use Shift+Insert (Windows' original paste shortcut) instead of
        // Ctrl+V. Terminals like Windows Terminal handle Shift+Insert as
        // plain-text paste at the terminal layer, before any TUI app
        // (Claude Code, vim, etc.) gets a chance to intercept Ctrl+V for
        // image paste or other custom handling. Shift+Insert is also
        // honored everywhere else Ctrl+V is — Notepad, VS Code, browsers,
        // Office, etc.
        use enigo::{Direction, Enigo, Key, Keyboard, Settings};
        let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
        enigo.key(Key::Shift, Direction::Press).map_err(|e| e.to_string())?;
        enigo.key(Key::Insert, Direction::Click).map_err(|e| e.to_string())?;
        enigo.key(Key::Shift, Direction::Release).map_err(|e| e.to_string())?;
    }

    Ok(())
}
