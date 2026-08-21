use std::path::PathBuf;

pub fn app_data_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("perihel-transcribe")
}
pub fn model_dir() -> PathBuf { app_data_dir().join("models") }
pub fn dictionary_path() -> PathBuf { app_data_dir().join("dictionary.json") }
pub fn settings_path() -> PathBuf { app_data_dir().join("settings.json") }
pub fn log_path() -> PathBuf { app_data_dir().join("app.log") }
