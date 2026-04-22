use std::path::PathBuf;
use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;

pub async fn transcribe_local(
    app: &AppHandle,
    model_size: &str,
    model_path: &PathBuf,
    audio_path: &PathBuf,
) -> Result<String, String> {
    if !model_path.exists() {
        return Err("Whisper model not found. Please download a model first.".to_string());
    }

    validate_model_file(model_size, model_path)?;

    println!("[Typr] Running whisper.cpp sidecar with model {:?}", model_path);

    let sidecar = app
        .shell()
        .sidecar("whisper-cpp")
        .map_err(|e| {
            format!(
                "Local whisper sidecar is missing in this checkout. Use the Cloud engine for now, or add the whisper-cpp sidecar binary under src-tauri/binaries. Original error: {}",
                e
            )
        })?;

    let threads = std::thread::available_parallelism()
        .map(|count| count.get().clamp(1, 8))
        .unwrap_or(4)
        .to_string();

    let output = sidecar
        .args([
            "-m",
            model_path.to_str().unwrap(),
            "-f",
            audio_path.to_str().unwrap(),
            "-t",
            threads.as_str(),
            "--no-gpu",
            "--no-prints",
            "--no-timestamps",
            "-l",
            "en",
        ])
        .output()
        .await
        .map_err(|e| format!("Failed to run whisper.cpp: {}", e))?;

    if output.status.code() != Some(0) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stderr.contains("failed to initialize whisper context") {
            return Err(format!(
                "The local '{}' Whisper model looks incomplete or corrupted. Please re-download it from the app and try again.",
                model_size
            ));
        }
        return Err(format!(
            "whisper.cpp failed (status {:?}). stdout: {} stderr: {}",
            output.status.code(),
            stdout.trim(),
            stderr.trim()
        ));
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    println!("[Typr] Whisper output: {}", text);
    Ok(text)
}

pub fn model_filename(model_size: &str) -> String {
    format!("ggml-{}.bin", model_size)
}

pub fn model_download_url(model_size: &str) -> String {
    format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{}.bin",
        model_size
    )
}

pub fn model_min_bytes(model_size: &str) -> u64 {
    match model_size {
        "small" => 300 * 1024 * 1024,
        "medium" => 1_000 * 1024 * 1024,
        _ => 1,
    }
}

pub fn validate_model_file(model_size: &str, model_path: &PathBuf) -> Result<(), String> {
    let metadata = std::fs::metadata(model_path).map_err(|e| e.to_string())?;
    let min_bytes = model_min_bytes(model_size);
    if metadata.len() < min_bytes {
        return Err(format!(
            "The local '{}' Whisper model is incomplete. Please download it again.",
            model_size
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_filename() {
        assert_eq!(model_filename("small"), "ggml-small.bin");
        assert_eq!(model_filename("medium"), "ggml-medium.bin");
    }

    #[test]
    fn test_model_download_url() {
        assert_eq!(
            model_download_url("small"),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin"
        );
    }

    #[test]
    fn test_model_min_bytes() {
        assert!(model_min_bytes("small") > 300_000_000);
        assert!(model_min_bytes("medium") > 1_000_000_000);
    }
}
