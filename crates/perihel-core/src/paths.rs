//! Data layout for the whole Perihel ecosystem.
//!
//! - `%LOCALAPPDATA%/perihel`         — runtime data (db, screenshots, voice, exports)
//! - `%APPDATA%/perihel`              — editable config (settings.json, dictionary.json)
//! - models: repo `models/` if running from a checkout, else `%APPDATA%/perihel/models`

use std::path::PathBuf;

pub fn app_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("perihel")
}

pub fn app_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("perihel")
}

/// Repo root when running from a source checkout (`target/{debug,release}/<bin>.exe`
/// sits three levels below the workspace root). Returns None for installed binaries.
pub fn find_repo_dir() -> Option<PathBuf> {
    if let Ok(env_path) = std::env::var("PERIHEL_REPO") {
        let p = PathBuf::from(env_path);
        if p.join("Cargo.toml").exists() {
            return Some(p);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?.to_path_buf();
    for _ in 0..4 {
        if dir.join("Cargo.toml").exists() && dir.join("grammar").exists() {
            return Some(dir);
        }
        dir = dir.parent()?.to_path_buf();
    }
    None
}

/// Where model weights live. Preference: `$PERIHEL_MODELS`, repo `models/`, config dir.
pub fn models_dir() -> PathBuf {
    if let Ok(p) = std::env::var("PERIHEL_MODELS") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Some(repo) = find_repo_dir() {
        let m = repo.join("models");
        if m.exists() {
            return m;
        }
    }
    app_config_dir().join("models")
}

pub fn llm_models_dir() -> PathBuf {
    models_dir().join("llm")
}

pub fn stt_models_dir() -> PathBuf {
    models_dir().join("stt")
}

pub fn vision_models_dir() -> PathBuf {
    models_dir().join("vision")
}

pub fn stt_bin_dir() -> PathBuf {
    models_dir().join("bin")
}
