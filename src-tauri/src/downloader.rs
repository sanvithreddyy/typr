use std::io::Write;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};

use crate::transcribe_local;

#[derive(Clone, serde::Serialize)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: u64,
    pub percent: f64,
}

pub async fn download_model(
    app: AppHandle,
    url: &str,
    dest: &PathBuf,
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Download failed with status: {}", response.status()));
    }

    let total = response.content_length().unwrap_or(0);

    // Ensure parent directory exists
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let temp_dest = dest.with_extension("bin.partial");
    let mut file = std::fs::File::create(&temp_dest).map_err(|e| e.to_string())?;
    let mut downloaded: u64 = 0;

    let mut stream = response.bytes_stream();
    use futures_util::StreamExt;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download stream error: {}", e))?;
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        downloaded += chunk.len() as u64;

        let percent = if total > 0 {
            (downloaded as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        let _ = app.emit("download-progress", DownloadProgress {
            downloaded,
            total,
            percent,
        });
    }

    if total > 0 && downloaded != total {
        let _ = std::fs::remove_file(&temp_dest);
        return Err(format!(
            "Model download was incomplete (downloaded {} of {} bytes). Please try again.",
            downloaded, total
        ));
    }

    file.flush().map_err(|e| e.to_string())?;
    drop(file);

    let model_size = dest
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            if name.contains("small") {
                "small"
            } else if name.contains("medium") {
                "medium"
            } else {
                ""
            }
        })
        .unwrap_or("");

    if !model_size.is_empty() {
        if let Err(err) = transcribe_local::validate_model_file(model_size, &temp_dest) {
            let _ = std::fs::remove_file(&temp_dest);
            return Err(err);
        }
    }

    if dest.exists() {
        std::fs::remove_file(dest).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&temp_dest, dest).map_err(|e| e.to_string())?;
    Ok(())
}
