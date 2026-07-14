pub mod settings;
pub mod audio;
pub mod http;
pub mod history;
pub mod transcribe_local;
pub mod transcribe_groq;
pub mod transcribe_openrouter;
pub mod cleanup;
pub mod paste;
pub mod recorder;
pub mod downloader;
pub mod hotkey;
#[cfg(windows)]
pub mod mouse_hook;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_shell::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
