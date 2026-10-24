//! perihel-core — shared foundations for the Perihel ecosystem.
//!
//! Used by both the main `perihel` app (capture, copilot, reminders) and the
//! `perihel-transcribe` dictation sidecar, so dictionary corrections and the
//! data layout stay identical across the ecosystem.

pub mod db;
pub mod dictionary;
pub mod formatting;
pub mod paths;
pub mod resource;
pub mod settings;
pub mod sync;
pub mod tools;

pub use db::{Capture, DetectedAgent, Reminder};
pub use dictionary::{Correction, Dictionary, DictionaryCorrector};
pub use formatting::{format_text, RuleBasedFormatter};
pub use paths::{app_config_dir, app_data_dir, find_repo_dir, llm_models_dir, models_dir, stt_bin_dir, stt_models_dir, vision_models_dir};
pub use resource::{cpu_load_percent, ram_free_mb, set_priority_idle, Pressure, ResourceGuard};
pub use settings::Settings;
pub use tools::{system_prompt, ToolCall};


