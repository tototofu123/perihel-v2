//! STT sidecar — wraps prebuilt `sherpa-onnx-offline.exe` (16 MB, k2-fsa release).
//!
//! Engine lives in an external process: model weights (226 MB SenseVoice / 98 MB
//! whisper tiny / 628 MB Parakeet) load only for the ~1 s a dictation takes and
//! are gone when the process exits. Nothing stays resident.

use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    SenseVoice,
    WhisperTiny,
    Parakeet,
}

impl Engine {
    pub fn from_name(name: &str) -> Engine {
        match name {
            "whisper-tiny" | "whisper" => Engine::WhisperTiny,
            "parakeet" => Engine::Parakeet,
            _ => Engine::SenseVoice,
        }
    }

    pub fn folder(&self) -> &'static str {
        match self {
            Engine::SenseVoice => "sensevoice-int8",
            Engine::WhisperTiny => "whisper-tiny-en",
            Engine::Parakeet => "parakeet-int8",
        }
    }
}

pub fn bin_dir() -> PathBuf {
    perihel_core::stt_bin_dir().join("sherpa-onnx-v1.13.8-win-x64-shared-MD-MinSizeRel/bin")
}

pub fn offline_exe() -> PathBuf {
    bin_dir().join("sherpa-onnx-offline.exe")
}

/// Verify the sidecar binary and the chosen engine's model files exist.
pub fn check_models(engine_name: &str) -> Result<(), String> {
    let engine = Engine::from_name(engine_name);
    if !offline_exe().exists() {
        return Err("sherpa-onnx-offline.exe not found under models/bin/".into());
    }
    let dir = perihel_core::stt_models_dir().join(engine.folder());
    let missing: Vec<&str> = match engine {
        Engine::SenseVoice => {
            ["model.int8.onnx", "tokens.txt"]
                .iter()
                .filter(|f| !dir.join(f).exists())
                .cloned()
                .collect()
        }
        Engine::WhisperTiny => {
            ["tiny.en-encoder.int8.onnx", "tiny.en-decoder.int8.onnx", "tiny.en-tokens.txt"]
                .iter()
                .filter(|f| !dir.join(f).exists())
                .cloned()
                .collect()
        }
        Engine::Parakeet => {
            ["encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx", "tokens.txt"]
                .iter()
                .filter(|f| !dir.join(f).exists())
                .cloned()
                .collect()
        }
    };
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "engine '{}' missing model files in {}: {}",
            engine_name,
            dir.display(),
            missing.join(", ")
        ))
    }
}

/// Transcribe a 16 kHz mono i16 wav via the sidecar. Blocks ~0.5–3 s.
pub fn transcribe_wav(wav: &Path, engine_name: &str) -> Result<String> {
    let engine = Engine::from_name(engine_name);
    let exe = offline_exe();
    if !exe.exists() {
        bail!("sherpa-onnx-offline.exe missing — run scripts/download-models.ps1");
    }
    let model_dir = perihel_core::stt_models_dir().join(engine.folder());
    let model_dir = model_dir.to_string_lossy().to_string();

    let mut cmd = Command::new(&exe);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.arg("--num-threads=2");

    match engine {
        Engine::SenseVoice => {
            cmd.arg(format!("--sense-voice-model={model_dir}/model.int8.onnx"));
            cmd.arg(format!("--tokens={model_dir}/tokens.txt"));
            cmd.arg("--sense-voice-language=auto");
        }
        Engine::WhisperTiny => {
            cmd.arg(format!("--whisper-encoder={model_dir}/tiny.en-encoder.int8.onnx"));
            cmd.arg(format!("--whisper-decoder={model_dir}/tiny.en-decoder.int8.onnx"));
            cmd.arg(format!("--tokens={model_dir}/tiny.en-tokens.txt"));
            cmd.arg("--whisper-tail-paddings=50");
        }
        Engine::Parakeet => {
            cmd.arg(format!("--nemo-encoder={model_dir}/encoder.int8.onnx"));
            cmd.arg(format!("--nemo-decoder={model_dir}/decoder.int8.onnx"));
            cmd.arg(format!("--nemo-joiner={model_dir}/joiner.int8.onnx"));
            cmd.arg(format!("--tokens={model_dir}/tokens.txt"));
        }
    }
    cmd.arg(wav);

    let out = cmd.output().context("failed to run sherpa-onnx-offline")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("STT sidecar failed: {}", err.lines().last().unwrap_or("unknown error"));
    }
    parse_stdout(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| anyhow!("sidecar produced no text (engine {})", engine_name))
}

/// sherpa-onnx-offline prints the wav path, then a JSON result object per file.
fn parse_stdout(stdout: &str) -> Option<String> {
    for line in stdout.lines() {
        let line = line.trim();
        if line.starts_with('{') {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                if let Some(text) = v.get("text").and_then(|t| t.as_str()) {
                    return Some(text.trim().to_string());
                }
            }
        }
    }
    // older builds: "text: hello world"
    stdout.lines().find_map(|l| {
        l.trim()
            .strip_prefix("text: ")
            .map(|t| t.trim().trim_matches('"').to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_result_line() {
        let s = "C:\\tmp\\a.wav\n{\"text\":\" Hello world.\",\"timestamps\":[1],\"tokens\":[]}\n";
        assert_eq!(parse_stdout(s).unwrap(), "Hello world.");
    }

    #[test]
    fn parses_legacy_text_line() {
        assert_eq!(parse_stdout("a.wav\ntext: hi there\n").unwrap(), "hi there");
    }

    #[test]
    fn none_on_empty() {
        assert!(parse_stdout("a.wav\nno result here\n").is_none());
    }
}
