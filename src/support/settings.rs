use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Settings {
    pub hotkey: String, // RightCtrl, RightAlt, F8
    pub model: String,  // parakeet-tdt-0.6b-int8
    pub hud_position: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self { hotkey: "RightCtrl".into(), model: "parakeet-tdt-0.6b-int8".into(), hud_position: "bottom-center".into() }
    }
}

impl Settings {
    pub fn load_or_default() -> Self {
        let path = crate::support::paths::settings_path();
        fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }
    pub fn save(&self) -> anyhow::Result<()> {
        let path = crate::support::paths::settings_path();
        if let Some(p) = path.parent() { fs::create_dir_all(p)?; }
        fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}
