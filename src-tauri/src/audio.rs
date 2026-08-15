use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound::{WavSpec, WavWriter};
use std::sync::{Arc, Mutex};
use std::path::PathBuf;

#[derive(Debug, Clone, serde::Serialize)]
pub struct MicDevice {
    pub name: String,
    pub is_default: bool,
}

pub fn list_microphones() -> Vec<MicDevice> {
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_default();

    let mut devices = Vec::new();
    if let Ok(input_devices) = host.input_devices() {
        for device in input_devices {
            if let Ok(name) = device.name() {
                devices.push(MicDevice {
                    is_default: name == default_name,
                    name,
                });
            }
        }
    }
    devices
}

/// Wrapper to make cpal::Stream usable across threads.
/// SAFETY: cpal::Stream on macOS (CoreAudio) is thread-safe in practice;
/// we only access it behind a Mutex to start/stop recording.
struct SendStream(#[allow(dead_code)] cpal::Stream);
unsafe impl Send for SendStream {}
unsafe impl Sync for SendStream {}

pub struct AudioRecorder {
    samples: Arc<Mutex<Vec<f32>>>,
    stream: Option<SendStream>,
    source_sample_rate: u32,
    source_channels: u16,
    device_name: String,
    stream_error: Arc<Mutex<Option<String>>>,
}

/// Extra guidance appended to capture errors when the device looks like a
/// Bluetooth headset, whose microphone only becomes live a second or two
/// after the stream opens (Windows has to switch it into hands-free mode).
fn bluetooth_hint(device_name: &str) -> &'static str {
    let n = device_name.to_lowercase();
    if n.contains("airpods")
        || n.contains("headset")
        || n.contains("hands-free")
        || n.contains("buds")
        || n.contains("bluetooth")
    {
        " Bluetooth headsets take a moment to switch their microphone on — wait for the indicator to turn red before speaking."
    } else {
        ""
    }
}

impl AudioRecorder {
    pub fn new() -> Self {
        Self {
            samples: Arc::new(Mutex::new(Vec::new())),
            stream: None,
            source_sample_rate: 48000,
            source_channels: 1,
            device_name: String::new(),
            stream_error: Arc::new(Mutex::new(None)),
        }
    }

    pub fn start(&mut self, mic_name: &str) -> Result<(), String> {
        // Clear any leftover samples from previous recording
        self.samples.lock().unwrap().clear();
        *self.stream_error.lock().unwrap() = None;

        let host = cpal::default_host();

        let device = if mic_name == "default" {
            host.default_input_device()
                .ok_or("No default input device found")?
        } else {
            host.input_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.name().map(|n| n == mic_name).unwrap_or(false))
                .ok_or(format!("Microphone '{}' not found", mic_name))?
        };

        self.device_name = device.name().unwrap_or_else(|_| mic_name.to_string());

        // Use the device's default config instead of forcing 16kHz
        let default_config = device
            .default_input_config()
            .map_err(|e| format!("Failed to get default input config: {}", e))?;

        let sample_rate = default_config.sample_rate().0;
        let channels = default_config.channels();

        println!("[Typr] Mic config: {}Hz, {} channels", sample_rate, channels);

        self.source_sample_rate = sample_rate;
        self.source_channels = channels;

        let config = cpal::StreamConfig {
            channels,
            sample_rate: cpal::SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let samples = self.samples.clone();
        let stream = device
            .build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let mut buf = samples.lock().unwrap();
                    buf.extend_from_slice(data);
                },
                {
                    let error_slot = self.stream_error.clone();
                    move |err| {
                        eprintln!("[Typr] Audio stream error: {}", err);
                        *error_slot.lock().unwrap() = Some(err.to_string());
                    }
                },
                None,
            )
            .map_err(|e| e.to_string())?;

        stream.play().map_err(|e| e.to_string())?;
        self.stream = Some(SendStream(stream));

        // Wait for the device to actually deliver audio before reporting the
        // recording as started. Bluetooth headsets (AirPods etc.) need 1-2s
        // to switch into hands-free mode after the stream opens; returning
        // early would show "Recording" while the mic is still dead. The
        // the app reports Recording only after this returns, so that state
        // means the microphone stream is live.
        let wait_start = std::time::Instant::now();
        loop {
            if !self.samples.lock().unwrap().is_empty() {
                println!(
                    "[Typr] Mic '{}' live after {}ms",
                    self.device_name,
                    wait_start.elapsed().as_millis()
                );
                break;
            }
            if let Some(err) = self.stream_error.lock().unwrap().clone() {
                self.stream = None;
                return Err(format!(
                    "Microphone '{}' failed to start: {}.{}",
                    self.device_name,
                    err,
                    bluetooth_hint(&self.device_name)
                ));
            }
            if wait_start.elapsed() > std::time::Duration::from_millis(2500) {
                eprintln!(
                    "[Typr] Warning: no audio from '{}' after 2.5s — continuing anyway",
                    self.device_name
                );
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        println!("[Typr] Audio recording started");
        Ok(())
    }

    pub fn stop_and_save(&mut self, output_path: &PathBuf) -> Result<PathBuf, String> {
        self.stream = None; // Drop stops the stream
        println!("[Typr] Audio recording stopped");

        let samples = self.samples.lock().unwrap();
        if samples.is_empty() {
            if let Some(err) = self.stream_error.lock().unwrap().clone() {
                return Err(format!(
                    "Microphone '{}' stream error: {}.{}",
                    self.device_name,
                    err,
                    bluetooth_hint(&self.device_name)
                ));
            }
            return Err(format!(
                "No audio captured from '{}'.{} Hold the hotkey while speaking and verify the microphone works in Windows sound settings.",
                self.device_name,
                bluetooth_hint(&self.device_name)
            ));
        }

        println!("[Typr] Captured {} raw samples", samples.len());

        // Convert to mono if multi-channel
        let mono: Vec<f32> = if self.source_channels > 1 {
            samples
                .chunks(self.source_channels as usize)
                .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
                .collect()
        } else {
            samples.clone()
        };

        // Silence gate: Whisper hallucinates text ("The quick brown fox...",
        // "Thank you.", etc.) when given silent or near-silent audio, so bail
        // out before transcription if there's no speech-level signal.
        let peak = mono.iter().fold(0.0_f32, |acc, s| acc.max(s.abs()));
        let rms = (mono.iter().map(|s| s * s).sum::<f32>() / mono.len() as f32).sqrt();
        let duration_seconds = mono.len() as f32 / self.source_sample_rate as f32;
        println!("[Typr] Audio level: peak={:.4}, rms={:.4}", peak, rms);
        if peak < 0.02 && rms < 0.005 {
            return Err(format!(
                "No speech detected from '{}' ({:.1}s captured, peak {:.3}, RMS {:.3}). Keep the hotkey held while speaking, or switch Recording Mode to Toggle.{}",
                self.device_name,
                duration_seconds,
                peak,
                rms,
                bluetooth_hint(&self.device_name),
            ));
        }

        // Downsample to 16kHz for whisper.cpp
        let resampled = resample(&mono, self.source_sample_rate, 16000);
        println!("[Typr] Resampled to {} samples at 16kHz", resampled.len());

        let spec = WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let mut writer = WavWriter::create(output_path, spec).map_err(|e| e.to_string())?;
        for &sample in resampled.iter() {
            let amplitude = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            writer.write_sample(amplitude).map_err(|e| e.to_string())?;
        }
        writer.finalize().map_err(|e| e.to_string())?;

        drop(samples);
        self.samples.lock().unwrap().clear();

        println!("[Typr] WAV saved to {:?}", output_path);
        Ok(output_path.clone())
    }
}

/// Simple linear interpolation resampler
fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate {
        return samples.to_vec();
    }

    let ratio = from_rate as f64 / to_rate as f64;
    let output_len = (samples.len() as f64 / ratio) as usize;
    let mut output = Vec::with_capacity(output_len);

    for i in 0..output_len {
        let src_idx = i as f64 * ratio;
        let idx = src_idx as usize;
        let frac = src_idx - idx as f64;

        let sample = if idx + 1 < samples.len() {
            samples[idx] as f64 * (1.0 - frac) + samples[idx + 1] as f64 * frac
        } else {
            samples[idx.min(samples.len() - 1)] as f64
        };

        output.push(sample as f32);
    }

    output
}
