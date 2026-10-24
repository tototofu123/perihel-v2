//! Global settings for the Perihel ecosystem (shared by app + transcribe).
//!
//! Stored as `%APPDATA%/perihel/settings.json` — plain JSON, editable outside the UI.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::paths::app_config_dir;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    // --- dictation (perihel-transcribe) ---
    pub stt_engine: String, // "whisper-tiny" (default, light) | "parakeet" (quality)
    pub dictation_hotkey: String,

    // --- copilot (LLM tool calling) ---
    pub llm_model: String, // file name under models/llm, e.g. "perihel-tools-q4_k_m.gguf"
    pub llm_context_window: u32,
    pub llm_backend: String,       // "local" (llama-server sidecar) | "remote" (OpenAI-compatible API)
    pub llm_remote_url: String,    // e.g. "https://integrate.api.nvidia.com/v1"
    pub llm_remote_model: String,  // e.g. "meta/llama-3.2-11b-vision-instruct"
    pub llm_remote_api_key: String, // empty = read $NVIDIA_NIM_API_KEY
    pub tts_enabled: bool,
    pub tts_voice: String,

    // --- sync (2.1) ---
    pub sync_enabled: bool,   // default false — 2.0 behaviour when off
    pub server_url: String,   // e.g. "https://203.0.113.10:8443"
    pub device_token: String, // bearer token from the server's token file
    // TLS trusts the OS cert store; the server's self-signed cert is imported
    // once per machine via `certutil -user -addstore Root cert.pem`.

    // --- resource guard ---
    pub low_power_mode: bool,
    pub pause_below_free_ram_mb: u32,

    // --- capture ---
    pub quiet_start: String,
    pub quiet_end: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            stt_engine: "whisper-tiny".into(),
            dictation_hotkey: "RightCtrl".into(),
            llm_model: String::new(), // resolved against models/llm at runtime
            llm_context_window: 4096,
            llm_backend: "local".into(),
            llm_remote_url: String::new(),
            llm_remote_model: String::new(),
            llm_remote_api_key: String::new(),
            tts_enabled: true,
            tts_voice: String::new(), // OS default
            low_power_mode: true,
            pause_below_free_ram_mb: 1500,
            quiet_start: "23:00".into(),
            quiet_end: "08:00".into(),
            sync_enabled: false,
            server_url: String::new(),
            device_token: String::new(),
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        app_config_dir().join("settings.json")
    }

    pub fn load_or_default() -> Self {
        fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path();
        if let Some(p) = path.parent() {
            fs::create_dir_all(p)?;
        }
        fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}
