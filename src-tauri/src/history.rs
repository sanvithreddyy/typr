use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Transcription history, persisted locally in the app config dir.
/// On Windows the file is encrypted with DPAPI (CryptProtectData), which
/// ties it to the current Windows user account — other accounts (or anyone
/// copying the file to another machine) cannot read it. Nothing is ever
/// sent anywhere; the file only exists on this device.
const HISTORY_FILE: &str = "history.dat";
const MAX_ENTRIES: usize = 1000;

static FILE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: u64,
    #[serde(rename = "timestampMs")]
    pub timestamp_ms: u64,
    pub text: String,
    pub engine: String,
}

pub fn load(app_dir: &Path) -> Vec<HistoryEntry> {
    let _guard = FILE_LOCK.lock().unwrap();
    load_unlocked(app_dir)
}

pub fn add(app_dir: &Path, text: &str, engine: &str) -> Result<(), String> {
    let _guard = FILE_LOCK.lock().unwrap();
    let mut entries = load_unlocked(app_dir);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    // Entries are newest-first; keep ids strictly increasing even if two
    // recordings land in the same millisecond.
    let id = entries.first().map(|e| e.id + 1).unwrap_or(0).max(now);
    entries.insert(
        0,
        HistoryEntry {
            id,
            timestamp_ms: now,
            text: text.to_string(),
            engine: engine.to_string(),
        },
    );
    entries.truncate(MAX_ENTRIES);
    save_unlocked(app_dir, &entries)
}

pub fn delete(app_dir: &Path, id: u64) -> Result<(), String> {
    let _guard = FILE_LOCK.lock().unwrap();
    let mut entries = load_unlocked(app_dir);
    entries.retain(|e| e.id != id);
    save_unlocked(app_dir, &entries)
}

pub fn clear(app_dir: &Path) -> Result<(), String> {
    let _guard = FILE_LOCK.lock().unwrap();
    let path = app_dir.join(HISTORY_FILE);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn load_unlocked(app_dir: &Path) -> Vec<HistoryEntry> {
    let path = app_dir.join(HISTORY_FILE);
    let raw = match std::fs::read(&path) {
        Ok(raw) => raw,
        Err(_) => return Vec::new(),
    };
    let plain = match decrypt(&raw) {
        Ok(plain) => plain,
        Err(e) => {
            eprintln!("[Typr] Failed to decrypt history file: {}", e);
            return Vec::new();
        }
    };
    serde_json::from_slice(&plain).unwrap_or_default()
}

fn save_unlocked(app_dir: &Path, entries: &[HistoryEntry]) -> Result<(), String> {
    std::fs::create_dir_all(app_dir).map_err(|e| e.to_string())?;
    let plain = serde_json::to_vec(entries).map_err(|e| e.to_string())?;
    let raw = encrypt(&plain)?;
    std::fs::write(app_dir.join(HISTORY_FILE), raw).map_err(|e| e.to_string())
}

#[cfg(windows)]
fn encrypt(plain: &[u8]) -> Result<Vec<u8>, String> {
    use windows::core::PCWSTR;
    use windows::Win32::Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB};
    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: plain.len() as u32,
            pbData: plain.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        CryptProtectData(&input, PCWSTR::null(), None, None, None, 0, &mut output)
            .map_err(|e| format!("DPAPI encrypt failed: {}", e))?;
        Ok(take_blob(output))
    }
}

#[cfg(windows)]
fn decrypt(raw: &[u8]) -> Result<Vec<u8>, String> {
    use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};
    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: raw.len() as u32,
            pbData: raw.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        CryptUnprotectData(&input, None, None, None, None, 0, &mut output)
            .map_err(|e| format!("DPAPI decrypt failed: {}", e))?;
        Ok(take_blob(output))
    }
}

/// Copy a DPAPI output blob into a Vec and free the LocalAlloc'd buffer.
#[cfg(windows)]
unsafe fn take_blob(
    blob: windows::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB,
) -> Vec<u8> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    let bytes = std::slice::from_raw_parts(blob.pbData, blob.cbData as usize).to_vec();
    let _ = LocalFree(HLOCAL(blob.pbData as *mut core::ffi::c_void));
    bytes
}

#[cfg(not(windows))]
fn encrypt(plain: &[u8]) -> Result<Vec<u8>, String> {
    Ok(plain.to_vec())
}

#[cfg(not(windows))]
fn decrypt(raw: &[u8]) -> Result<Vec<u8>, String> {
    Ok(raw.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip() {
        let data = b"hello history";
        let enc = encrypt(data).unwrap();
        let dec = decrypt(&enc).unwrap();
        assert_eq!(dec, data);
    }

    #[test]
    fn test_add_load_delete_clear() {
        let dir = std::env::temp_dir().join(format!(
            "typr-history-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);

        add(&dir, "first entry", "groq").unwrap();
        add(&dir, "second entry", "openrouter").unwrap();

        let entries = load(&dir);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "second entry");
        assert_eq!(entries[1].text, "first entry");
        assert!(entries[0].id > entries[1].id);

        delete(&dir, entries[1].id).unwrap();
        assert_eq!(load(&dir).len(), 1);

        clear(&dir).unwrap();
        assert!(load(&dir).is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
