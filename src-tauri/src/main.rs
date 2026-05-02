#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use typr_lib::audio;
use typr_lib::downloader;
use typr_lib::hotkey::{self, Hotkey};
#[cfg(windows)]
use typr_lib::mouse_hook::{self, ButtonState};
use typr_lib::recorder::{Recorder, RecordingState};
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
    let old_hotkey = state.settings.lock().unwrap().hotkey.clone();
    let new_hotkey = settings.hotkey.clone();
    settings.save(&state.app_dir)?;
    *state.settings.lock().unwrap() = settings;

    if old_hotkey != new_hotkey {
        register_hotkey(&app, &new_hotkey)?;
        println!("[Typr] Hotkey changed: {} -> {}", old_hotkey, new_hotkey);
    }
    Ok(())
}

#[tauri::command]
fn list_microphones() -> Vec<audio::MicDevice> {
    audio::list_microphones()
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
    do_toggle_recording(&app, &state).await
}

/// Shared logic for toggle recording, used by both the Tauri command and hotkey handler.
async fn do_toggle_recording(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<String, String> {
    let current_state = state.recorder.get_state();
    match current_state {
        RecordingState::Ready => {
            let mic = state.settings.lock().unwrap().microphone.clone();
            state.recorder.start_recording(app, &mic)?;
            Ok("recording".to_string())
        }
        RecordingState::Recording => {
            let settings = state.settings.lock().unwrap().clone();
            let result = state
                .recorder
                .stop_and_transcribe(app, &settings, &state.app_dir)
                .await?;
            Ok(result)
        }
        RecordingState::Transcribing => Err("Currently transcribing, please wait".to_string()),
    }
}

/// Shared dispatch for hotkey events (whether keyboard or mouse).
fn handle_hotkey_event(handle: tauri::AppHandle, pressed: bool) {
    let state = handle.state::<AppState>();
    let mode = state.settings.lock().unwrap().recording_mode.clone();

    if pressed {
        match mode.as_str() {
            "toggle" => {
                let h = handle.clone();
                tauri::async_runtime::spawn(async move {
                    let s = h.state::<AppState>();
                    match do_toggle_recording(&h, s.inner()).await {
                        Ok(result) => println!("[Typr] Toggle result: {}", result),
                        Err(e) => eprintln!("[Typr] Toggle error: {}", e),
                    }
                });
            }
            "push-to-talk" => {
                if state.recorder.get_state() == RecordingState::Ready {
                    let mic = state.settings.lock().unwrap().microphone.clone();
                    if let Err(e) = state.recorder.start_recording(&handle, &mic) {
                        eprintln!("[Typr] PTT start error: {}", e);
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
                    .stop_and_transcribe(&h, &settings, &s.app_dir)
                    .await
                {
                    eprintln!("[Typr] PTT transcription error: {}", e);
                }
            }
        });
    }
}

/// Register the given hotkey string. Replaces any previously registered hotkey.
fn register_hotkey(app: &tauri::AppHandle, hotkey_str: &str) -> Result<(), String> {
    if let Err(e) = app.global_shortcut().unregister_all() {
        eprintln!("[Typr] unregister_all warning: {}", e);
    }
    #[cfg(windows)]
    mouse_hook::clear_binding();

    match hotkey::parse(hotkey_str) {
        Hotkey::Keyboard(s) => {
            let handle_clone = app.clone();
            app.global_shortcut()
                .on_shortcut(s.as_str(), move |_app, _shortcut, event| {
                    handle_hotkey_event(handle_clone.clone(), event.state == ShortcutState::Pressed);
                })
                .map_err(|e| format!("Failed to register keyboard shortcut: {}", e))?;
            println!("[Typr] Keyboard shortcut registered: {}", s);
            Ok(())
        }
        #[cfg(windows)]
        Hotkey::Mouse { button, modifiers } => {
            mouse_hook::set_binding(button, modifiers);
            println!(
                "[Typr] Mouse hotkey set: button={:?} modifiers={:?}",
                button, modifiers
            );
            Ok(())
        }
    }
}

fn main() {
    let app_dir = get_app_dir();
    let settings = Settings::load(&app_dir);
    let initial_hotkey = settings.hotkey.clone();

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
            get_recording_state,
            check_model_downloaded,
            download_model,
            toggle_recording,
        ])
        .setup(move |app| {
            // Create the overlay window (small mic icon, top-right, always on top)
            let monitor = app.primary_monitor().ok().flatten();
            let (x, y) = if let Some(m) = monitor {
                let size = m.size();
                let scale = m.scale_factor();
                let logical_w = size.width as f64 / scale;
                ((logical_w - 60.0) as i32, 10_i32)
            } else {
                (1380, 10)
            };

            let overlay = WebviewWindowBuilder::new(
                app,
                "overlay",
                WebviewUrl::App("src/overlay.html".into()),
            )
            .title("")
            .inner_size(50.0, 50.0)
            .position(x as f64, y as f64)
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .shadow(false)
            .build();

            match overlay {
                Ok(_) => println!("[Typr] Overlay window created"),
                Err(e) => eprintln!("[Typr] Failed to create overlay: {}", e),
            }

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
                        handle_hotkey_event(handle.clone(), pressed);
                    }
                });
            }

            println!("[Typr] Registering global hotkey: {}", initial_hotkey);
            if let Err(e) = register_hotkey(app.handle(), &initial_hotkey) {
                eprintln!("[Typr] ERROR: Failed to register hotkey: {}", e);
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
