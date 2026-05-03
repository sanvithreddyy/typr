use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::hotkey;

pub const DEFAULT_OPENROUTER_MODEL: &str = "google/gemini-3.1-flash-lite-preview";
const LEGACY_OPENROUTER_MODEL: &str = "openai/gpt-audio-mini";
const DEFAULT_KEYBOARD_HOTKEY: &str = "CmdOrCtrl+Shift+Space";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub microphone: String,
    pub engine: String,
    #[serde(rename = "whisperModel")]
    pub whisper_model: String,
    #[serde(rename = "groqApiKey")]
    pub groq_api_key: String,
    #[serde(rename = "openRouterApiKey")]
    pub openrouter_api_key: String,
    #[serde(rename = "openRouterModel")]
    pub openrouter_model: String,
    #[serde(rename = "recordingMode")]
    pub recording_mode: String,
    #[serde(rename = "keyboardHotkey")]
    pub keyboard_hotkey: String,
    #[serde(rename = "mouseHotkey")]
    pub mouse_hotkey: String,
    /// Legacy single-hotkey field. Read for migration on load, never written
    /// back. Presence in JSON signals an old config that needs routing into
    /// the keyboard/mouse slots.
    #[serde(default, skip_serializing)]
    pub hotkey: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            microphone: "default".to_string(),
            engine: "local".to_string(),
            whisper_model: "small".to_string(),
            groq_api_key: String::new(),
            openrouter_api_key: String::new(),
            openrouter_model: DEFAULT_OPENROUTER_MODEL.to_string(),
            recording_mode: "toggle".to_string(),
            keyboard_hotkey: DEFAULT_KEYBOARD_HOTKEY.to_string(),
            mouse_hotkey: String::new(),
            hotkey: String::new(),
        }
    }
}

impl Settings {
    pub fn config_path(app_dir: &PathBuf) -> PathBuf {
        app_dir.join("config.json")
    }

    pub fn normalized(mut self) -> Self {
        if self.engine == "cloud" {
            self.engine = "groq".to_string();
        }

        if self.engine != "local" && self.engine != "groq" && self.engine != "openrouter" {
            self.engine = "local".to_string();
        }

        if self.openrouter_model.trim().is_empty()
            || self.openrouter_model == LEGACY_OPENROUTER_MODEL
        {
            self.openrouter_model = DEFAULT_OPENROUTER_MODEL.to_string();
        }

        // Migrate the legacy single `hotkey` field. The user's explicit choice
        // wins: route it to whichever slot matches and leave the other empty,
        // even if the new fields had picked up defaults via serde(default).
        if !self.hotkey.is_empty() {
            let legacy = std::mem::take(&mut self.hotkey);
            self.keyboard_hotkey.clear();
            self.mouse_hotkey.clear();
            if hotkey::is_mouse_hotkey(&legacy) {
                self.mouse_hotkey = legacy;
            } else {
                self.keyboard_hotkey = legacy;
            }
        }

        self
    }

    pub fn load(app_dir: &PathBuf) -> Self {
        let path = Self::config_path(app_dir);
        match fs::read_to_string(&path) {
            Ok(contents) => serde_json::from_str::<Self>(&contents)
                .unwrap_or_default()
                .normalized(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, app_dir: &PathBuf) -> Result<(), String> {
        let path = Self::config_path(app_dir);
        fs::create_dir_all(app_dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(&path, json).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;

    #[test]
    fn test_default_settings() {
        let settings = Settings::default();
        assert_eq!(settings.microphone, "default");
        assert_eq!(settings.engine, "local");
        assert_eq!(settings.whisper_model, "small");
        assert_eq!(settings.groq_api_key, "");
        assert_eq!(settings.openrouter_api_key, "");
        assert_eq!(settings.openrouter_model, DEFAULT_OPENROUTER_MODEL);
        assert_eq!(settings.recording_mode, "toggle");
        assert_eq!(settings.keyboard_hotkey, DEFAULT_KEYBOARD_HOTKEY);
        assert_eq!(settings.mouse_hotkey, "");
        assert_eq!(settings.hotkey, "");
    }

    #[test]
    fn test_save_and_load() {
        let dir = temp_dir().join("typr_test_settings");
        let _ = fs::remove_dir_all(&dir);

        let mut settings = Settings::default();
        settings.engine = "groq".to_string();
        settings.groq_api_key = "test-key-123".to_string();
        settings.openrouter_model = DEFAULT_OPENROUTER_MODEL.to_string();

        settings.save(&dir).unwrap();
        let loaded = Settings::load(&dir);

        assert_eq!(loaded.engine, "groq");
        assert_eq!(loaded.groq_api_key, "test-key-123");
        assert_eq!(loaded.openrouter_model, DEFAULT_OPENROUTER_MODEL);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_old_cloud_config_is_migrated() {
        let dir = temp_dir().join("typr_test_migrate_cloud");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("config.json"),
            r#"{
                "microphone": "default",
                "engine": "cloud",
                "whisperModel": "small",
                "groqApiKey": "groq-test",
                "recordingMode": "toggle",
                "hotkey": "CmdOrCtrl+Shift+Space"
            }"#,
        ).unwrap();

        let settings = Settings::load(&dir);
        assert_eq!(settings.engine, "groq");
        assert_eq!(settings.groq_api_key, "groq-test");
        assert_eq!(settings.openrouter_model, DEFAULT_OPENROUTER_MODEL);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_missing_file_returns_default() {
        let dir = temp_dir().join("typr_test_missing");
        let _ = fs::remove_dir_all(&dir);
        let settings = Settings::load(&dir);
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn test_load_corrupt_json_returns_default() {
        let dir = temp_dir().join("typr_test_corrupt");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.json"), "not json").unwrap();

        let settings = Settings::load(&dir);
        assert_eq!(settings, Settings::default());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_legacy_mouse_hotkey_migrates_to_mouse_slot() {
        let dir = temp_dir().join("typr_test_legacy_hotkey_mouse");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("config.json"),
            r#"{
                "microphone": "default",
                "engine": "local",
                "whisperModel": "small",
                "recordingMode": "toggle",
                "hotkey": "XButton2"
            }"#,
        )
        .unwrap();

        let settings = Settings::load(&dir);
        assert_eq!(settings.mouse_hotkey, "XButton2");
        assert_eq!(settings.keyboard_hotkey, "");
        assert_eq!(settings.hotkey, "");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_legacy_keyboard_hotkey_migrates_to_keyboard_slot() {
        let dir = temp_dir().join("typr_test_legacy_hotkey_keyboard");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("config.json"),
            r#"{
                "microphone": "default",
                "engine": "local",
                "whisperModel": "small",
                "recordingMode": "toggle",
                "hotkey": "CmdOrCtrl+Shift+Space"
            }"#,
        )
        .unwrap();

        let settings = Settings::load(&dir);
        assert_eq!(settings.keyboard_hotkey, "CmdOrCtrl+Shift+Space");
        assert_eq!(settings.mouse_hotkey, "");
        assert_eq!(settings.hotkey, "");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_save_does_not_write_legacy_hotkey_field() {
        let dir = temp_dir().join("typr_test_no_legacy_field_on_save");
        let _ = fs::remove_dir_all(&dir);
        let mut settings = Settings::default();
        settings.hotkey = "CmdOrCtrl+Shift+Space".to_string();
        settings.save(&dir).unwrap();
        let raw = fs::read_to_string(dir.join("config.json")).unwrap();
        assert!(!raw.contains("\"hotkey\""), "hotkey field leaked into save output: {}", raw);
        assert!(raw.contains("keyboardHotkey"));
        assert!(raw.contains("mouseHotkey"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_dual_hotkey_roundtrip() {
        let dir = temp_dir().join("typr_test_dual_hotkey_roundtrip");
        let _ = fs::remove_dir_all(&dir);
        let mut settings = Settings::default();
        settings.keyboard_hotkey = "Ctrl+Shift+R".to_string();
        settings.mouse_hotkey = "XButton2".to_string();
        settings.save(&dir).unwrap();
        let loaded = Settings::load(&dir);
        assert_eq!(loaded.keyboard_hotkey, "Ctrl+Shift+R");
        assert_eq!(loaded.mouse_hotkey, "XButton2");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_legacy_openrouter_model_is_migrated() {
        let dir = temp_dir().join("typr_test_legacy_openrouter_model");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("config.json"),
            r#"{
                "microphone": "default",
                "engine": "openrouter",
                "whisperModel": "small",
                "openRouterApiKey": "or-test",
                "openRouterModel": "openai/gpt-audio-mini",
                "recordingMode": "toggle",
                "hotkey": "CmdOrCtrl+Shift+Space"
            }"#,
        )
        .unwrap();

        let settings = Settings::load(&dir);
        assert_eq!(settings.engine, "openrouter");
        assert_eq!(settings.openrouter_api_key, "or-test");
        assert_eq!(settings.openrouter_model, DEFAULT_OPENROUTER_MODEL);

        let _ = fs::remove_dir_all(&dir);
    }
}
