pub fn paste_text(text: &str) -> Result<(), String> {
    // Set clipboard (arboard is thread-safe)
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard.set_text(text).map_err(|e| e.to_string())?;

    // Small delay to ensure clipboard is set
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Simulate Cmd+V via osascript (works from any thread, unlike enigo which
    // calls TSMGetInputSourceProperty requiring the main thread)
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("osascript")
            .args(["-e", r#"tell application "System Events" to keystroke "v" using command down"#])
            .output()
            .map_err(|e| format!("Failed to simulate paste: {}", e))?;
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
