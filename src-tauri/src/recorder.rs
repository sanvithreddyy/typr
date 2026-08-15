use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::menu::MenuItem;
use tauri::{AppHandle, Emitter};

use crate::audio::AudioRecorder;
use crate::cleanup::cleanup_text;
use crate::paste::{capture_paste_target, paste_text};
use crate::settings::Settings;
use crate::transcribe_local;
use crate::transcribe_groq;
use crate::transcribe_openrouter;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum RecordingState {
    Ready,
    Recording,
    Transcribing,
}

/// Which input device started a recording. Used to enforce the rule that
/// only the source that started a recording is allowed to stop it — so a
/// keyboard press doesn't cancel a mouse-button recording mid-flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerSource {
    Keyboard,
    Mouse,
}

fn status_text(state: &RecordingState) -> (&'static str, &'static str) {
    match state {
        RecordingState::Ready => ("Status: Ready", "Typr — Ready"),
        RecordingState::Recording => ("Status: Recording…", "Typr — Recording"),
        RecordingState::Transcribing => ("Status: Transcribing…", "Typr — Transcribing"),
    }
}

pub struct Recorder {
    state: Arc<Mutex<RecordingState>>,
    audio_recorder: Arc<Mutex<AudioRecorder>>,
    active_trigger: Arc<Mutex<Option<TriggerSource>>>,
    paste_target_pid: Arc<Mutex<Option<i32>>>,
    tray_status_item: Arc<Mutex<Option<MenuItem<tauri::Wry>>>>,
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(RecordingState::Ready)),
            audio_recorder: Arc::new(Mutex::new(AudioRecorder::new())),
            active_trigger: Arc::new(Mutex::new(None)),
            paste_target_pid: Arc::new(Mutex::new(None)),
            tray_status_item: Arc::new(Mutex::new(None)),
        }
    }

    pub fn set_tray_status_item(&self, item: MenuItem<tauri::Wry>) {
        *self.tray_status_item.lock().unwrap() = Some(item);
    }

    pub fn get_state(&self) -> RecordingState {
        self.state.lock().unwrap().clone()
    }

    /// Source that started the in-progress recording, or None if idle.
    pub fn active_trigger(&self) -> Option<TriggerSource> {
        *self.active_trigger.lock().unwrap()
    }

    fn set_state(&self, app: &AppHandle, next: RecordingState) {
        *self.state.lock().unwrap() = next.clone();
        let _ = app.emit("recording-state", next.clone());
        self.update_tray_status(app, &next);
    }

    fn update_tray_status(&self, app: &AppHandle, state: &RecordingState) {
        let (menu_text, tooltip) = status_text(state);
        if let Some(item) = self.tray_status_item.lock().unwrap().as_ref() {
            let _ = item.set_text(menu_text);
        }
        if let Some(tray) = app.tray_by_id("typr-tray") {
            let _ = tray.set_tooltip(Some(tooltip));
        }
    }

    pub fn start_recording(
        &self,
        app: &AppHandle,
        mic_name: &str,
        source: TriggerSource,
    ) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        if *state != RecordingState::Ready {
            return Err("Already recording or transcribing".to_string());
        }

        let paste_target_pid = capture_paste_target();
        let mut recorder = self.audio_recorder.lock().unwrap();
        recorder.start(mic_name)?;

        *state = RecordingState::Recording;
        *self.active_trigger.lock().unwrap() = Some(source);
        *self.paste_target_pid.lock().unwrap() = paste_target_pid;
        println!("[Typr] Captured paste target PID {:?}", paste_target_pid);
        let _ = app.emit("recording-state", RecordingState::Recording);
        self.update_tray_status(app, &RecordingState::Recording);
        Ok(())
    }

    pub async fn stop_and_transcribe(
        &self,
        app: &AppHandle,
        settings: &Settings,
        app_dir: &PathBuf,
        source: TriggerSource,
    ) -> Result<String, String> {
        {
            let state = self.state.lock().unwrap();
            if *state != RecordingState::Recording {
                return Err("Not currently recording".to_string());
            }
            let trigger = self.active_trigger.lock().unwrap();
            if *trigger != Some(source) {
                return Err(
                    "Recording was started by a different hotkey; use the same one to stop it"
                        .to_string(),
                );
            }
        }
        self.set_state(app, RecordingState::Transcribing);

        let temp_path = app_dir.join("temp_recording.wav");
        let result = async {
            {
                let mut recorder = self.audio_recorder.lock().unwrap();
                recorder.stop_and_save(&temp_path)?;
            }

            let raw_text = match settings.engine.as_str() {
                "local" => {
                    let model_path =
                        app_dir.join(transcribe_local::model_filename(&settings.whisper_model));
                    transcribe_local::transcribe_local(
                        app,
                        &settings.whisper_model,
                        &model_path,
                        &temp_path,
                    ).await?
                }
                "groq" => transcribe_groq::transcribe_groq(&settings.groq_api_key, &temp_path).await?,
                "openrouter" => {
                    transcribe_openrouter::transcribe_openrouter(
                        &settings.openrouter_api_key,
                        &settings.openrouter_model,
                        &temp_path,
                    ).await?
                }
                _ => return Err(format!("Unknown engine: {}", settings.engine)),
            };

            let cleaned = cleanup_text(&raw_text);
            if !cleaned.is_empty() {
                if let Err(e) = crate::history::add(app_dir, &cleaned, &settings.engine) {
                    eprintln!("[Typr] Failed to save history entry: {}", e);
                }
                let target_pid = *self.paste_target_pid.lock().unwrap();
                paste_text(&cleaned, target_pid)?;
            }

            Ok(cleaned)
        }
        .await;

        let _ = std::fs::remove_file(&temp_path);
        *self.active_trigger.lock().unwrap() = None;
        *self.paste_target_pid.lock().unwrap() = None;
        self.set_state(app, RecordingState::Ready);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state_is_ready() {
        let recorder = Recorder::new();
        assert_eq!(recorder.get_state(), RecordingState::Ready);
    }

    #[test]
    fn test_tray_status_text() {
        assert_eq!(status_text(&RecordingState::Ready).0, "Status: Ready");
        assert_eq!(
            status_text(&RecordingState::Recording).0,
            "Status: Recording…"
        );
        assert_eq!(
            status_text(&RecordingState::Transcribing).0,
            "Status: Transcribing…"
        );
    }
}
