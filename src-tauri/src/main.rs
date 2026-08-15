#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Mutex;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use typr_lib::audio;
use typr_lib::downloader;
use typr_lib::history;
use typr_lib::hotkey::{self, Hotkey};
#[cfg(windows)]
use typr_lib::mouse_hook::{self, ButtonState};
use typr_lib::recorder::{Recorder, RecordingState, TriggerSource};
use typr_lib::settings::Settings;
use typr_lib::transcribe_local;

struct AppState {
    recorder: Recorder,
    settings: Mutex<Settings>,
    app_dir: PathBuf,
}

fn get_app_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.typr.app")
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn save_settings(
    app: tauri::AppHandle,
    state: State<AppState>,
    settings: Settings,
) -> Result<(), String> {
    let (old_kb, old_mouse) = {
        let s = state.settings.lock().unwrap();
        (s.keyboard_hotkey.clone(), s.mouse_hotkey.clone())
    };
    let new_kb = settings.keyboard_hotkey.clone();
    let new_mouse = settings.mouse_hotkey.clone();
    settings.save(&state.app_dir)?;
    *state.settings.lock().unwrap() = settings;

    if old_kb != new_kb || old_mouse != new_mouse {
        register_hotkeys(&app, &new_kb, &new_mouse)?;
        println!(
            "[Typr] Hotkeys changed: kb {:?} -> {:?}, mouse {:?} -> {:?}",
            old_kb, new_kb, old_mouse, new_mouse
        );
    }
    Ok(())
}

#[tauri::command]
fn list_microphones() -> Vec<audio::MicDevice> {
    audio::list_microphones()
}

#[tauri::command]
fn get_history(state: State<AppState>) -> Vec<history::HistoryEntry> {
    history::load(&state.app_dir)
}

#[tauri::command]
fn delete_history_entry(state: State<AppState>, id: u64) -> Result<(), String> {
    history::delete(&state.app_dir, id)
}

#[tauri::command]
fn clear_history(state: State<AppState>) -> Result<(), String> {
    history::clear(&state.app_dir)
}

#[tauri::command]
fn get_recording_state(state: State<AppState>) -> RecordingState {
    state.recorder.get_state()
}

#[tauri::command]
fn check_model_downloaded(state: State<AppState>, model_size: String) -> bool {
    let model_file = transcribe_local::model_filename(&model_size);
    let model_path = state.app_dir.join(&model_file);
    model_path.exists() && transcribe_local::validate_model_file(&model_size, &model_path).is_ok()
}

#[tauri::command]
async fn download_model(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    model_size: String,
) -> Result<(), String> {
    let url = transcribe_local::model_download_url(&model_size);
    let model_file = transcribe_local::model_filename(&model_size);
    let dest = state.app_dir.join(&model_file);
    downloader::download_model(app, &url, &dest).await
}

#[tauri::command]
async fn toggle_recording(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    // The settings UI's manual Toggle command isn't tied to a physical hotkey,
    // so it uses whichever source is currently active (or defaults to Keyboard
    // for the start case).
    let source = state.recorder.active_trigger().unwrap_or(TriggerSource::Keyboard);
    do_toggle_recording(&app, &state, source).await
}

/// Open the TLS connection to the transcription API while the user is still
/// speaking, so the request after they stop doesn't pay the handshake cost.
fn prewarm_engine(engine: &str) {
    match engine {
        "openrouter" => typr_lib::http::prewarm("https://openrouter.ai/api/v1/models"),
        "groq" => typr_lib::http::prewarm("https://api.groq.com/openai/v1/models"),
        _ => {}
    }
}

/// Shared logic for toggle recording, used by both the Tauri command and hotkey handler.
async fn do_toggle_recording(
    app: &tauri::AppHandle,
    state: &AppState,
    source: TriggerSource,
) -> Result<String, String> {
    let current_state = state.recorder.get_state();
    match current_state {
        RecordingState::Ready => {
            let (mic, engine) = {
                let s = state.settings.lock().unwrap();
                (s.microphone.clone(), s.engine.clone())
            };
            prewarm_engine(&engine);
            state.recorder.start_recording(app, &mic, source)?;
            Ok("recording".to_string())
        }
        RecordingState::Recording => {
            let settings = state.settings.lock().unwrap().clone();
            let result = state
                .recorder
                .stop_and_transcribe(app, &settings, &state.app_dir, source)
                .await?;
            Ok(result)
        }
        RecordingState::Transcribing => Err("Currently transcribing, please wait".to_string()),
    }
}

/// Shared dispatch for hotkey events (whether keyboard or mouse).
fn handle_hotkey_event(handle: tauri::AppHandle, source: TriggerSource, pressed: bool) {
    let state = handle.state::<AppState>();
    let mode = state.settings.lock().unwrap().recording_mode.clone();

    if pressed {
        match mode.as_str() {
            "toggle" => {
                let h = handle.clone();
                tauri::async_runtime::spawn(async move {
                    let s = h.state::<AppState>();
                    match do_toggle_recording(&h, s.inner(), source).await {
                        Ok(result) => println!("[Typr] Toggle result: {}", result),
                        Err(e) => {
                            eprintln!("[Typr] Toggle error: {}", e);
                            let _ = h.emit("typr-error", e);
                        }
                    }
                });
            }
            "push-to-talk" => {
                if state.recorder.get_state() == RecordingState::Ready {
                    let (mic, engine) = {
                        let s = state.settings.lock().unwrap();
                        (s.microphone.clone(), s.engine.clone())
                    };
                    prewarm_engine(&engine);
                    if let Err(e) = state.recorder.start_recording(&handle, &mic, source) {
                        eprintln!("[Typr] PTT start error: {}", e);
                        let _ = handle.emit("typr-error", e);
                    }
                }
            }
            _ => {}
        }
    } else if mode == "push-to-talk" {
        let h = handle.clone();
        tauri::async_runtime::spawn(async move {
            let s = h.state::<AppState>();
            if s.recorder.get_state() == RecordingState::Recording {
                let settings = s.settings.lock().unwrap().clone();
                if let Err(e) = s
                    .recorder
                    .stop_and_transcribe(&h, &settings, &s.app_dir, source)
                    .await
                {
                    eprintln!("[Typr] PTT transcription error: {}", e);
                    let _ = h.emit("typr-error", e);
                }
            }
        });
    }
}

/// Register both hotkey slots. Either may be empty (= disabled). Always
/// fully tears down previous bindings first so this is idempotent.
fn register_hotkeys(
    app: &tauri::AppHandle,
    keyboard_hotkey: &str,
    mouse_hotkey: &str,
) -> Result<(), String> {
    if let Err(e) = app.global_shortcut().unregister_all() {
        eprintln!("[Typr] unregister_all warning: {}", e);
    }
    #[cfg(windows)]
    mouse_hook::clear_binding();

    if !keyboard_hotkey.trim().is_empty() {
        match hotkey::parse(keyboard_hotkey) {
            Hotkey::Keyboard(s) => {
                let handle_clone = app.clone();
                app.global_shortcut()
                    .on_shortcut(s.as_str(), move |_app, _shortcut, event| {
                        handle_hotkey_event(
                            handle_clone.clone(),
                            TriggerSource::Keyboard,
                            event.state == ShortcutState::Pressed,
                        );
                    })
                    .map_err(|e| format!("Failed to register keyboard shortcut: {}", e))?;
                println!("[Typr] Keyboard shortcut registered: {}", s);
            }
            #[cfg(windows)]
            Hotkey::Mouse { .. } => {
                eprintln!(
                    "[Typr] Ignoring keyboard slot value '{}' — it parses as a mouse hotkey",
                    keyboard_hotkey
                );
            }
        }
    }

    #[cfg(windows)]
    if !mouse_hotkey.trim().is_empty() {
        match hotkey::parse(mouse_hotkey) {
            Hotkey::Mouse { button, modifiers } => {
                mouse_hook::set_binding(button, modifiers);
                println!(
                    "[Typr] Mouse hotkey set: button={:?} modifiers={:?}",
                    button, modifiers
                );
            }
            Hotkey::Keyboard(_) => {
                eprintln!(
                    "[Typr] Ignoring mouse slot value '{}' — it parses as a keyboard hotkey",
                    mouse_hotkey
                );
            }
        }
    }
    #[cfg(not(windows))]
    let _ = mouse_hotkey;

    Ok(())
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn main() {
    let app_dir = get_app_dir();
    let settings = Settings::load(&app_dir);
    let initial_kb = settings.keyboard_hotkey.clone();
    let initial_mouse = settings.mouse_hotkey.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            recorder: Recorder::new(),
            settings: Mutex::new(settings),
            app_dir,
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            list_microphones,
            get_history,
            delete_history_entry,
            clear_history,
            get_recording_state,
            check_model_downloaded,
            download_model,
            toggle_recording,
        ])
        .on_window_event(|window, event| {
            // Closing the settings window hides it to the system tray so the
            // hotkeys keep working in the background. Quitting for real is
            // done via the tray menu.
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(move |app| {
            // Menu bar: clicking shows live status and app actions.
            let status_item =
                MenuItem::with_id(app, "status", "Status: Ready", false, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;
            let open_item = MenuItem::with_id(app, "open", "Open Typr", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit Typr", true, None::<&str>)?;
            let tray_menu =
                Menu::with_items(app, &[&status_item, &separator, &open_item, &quit_item])?;

            let mut tray = TrayIconBuilder::with_id("typr-tray")
                .menu(&tray_menu)
                .show_menu_on_left_click(true)
                .tooltip("Typr — Ready")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            app.state::<AppState>()
                .recorder
                .set_tray_status_item(status_item);

            // Install the Win32 mouse hook + spawn a thread that forwards
            // matched mouse events into the same hotkey dispatch path the
            // keyboard hotkey uses.
            #[cfg(windows)]
            {
                let receiver = mouse_hook::install();
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    for event in receiver {
                        let pressed = matches!(event.state, ButtonState::Pressed);
                        println!(
                            "[Typr] Mouse hotkey event: button={:?} pressed={}",
                            event.button, pressed
                        );
                        handle_hotkey_event(handle.clone(), TriggerSource::Mouse, pressed);
                    }
                });
            }

            println!(
                "[Typr] Registering hotkeys: keyboard={:?} mouse={:?}",
                initial_kb, initial_mouse
            );
            if let Err(e) = register_hotkeys(app.handle(), &initial_kb, &initial_mouse) {
                eprintln!("[Typr] ERROR: Failed to register hotkeys: {}", e);
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
