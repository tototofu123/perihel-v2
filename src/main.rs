//! perihel-transcribe — Perihel ecosystem, low-memory on-device dictation
//! Hold Right Ctrl → talk → release → text injected into focused window.
//! Memory target: <120 MB RSS (66 MB model + 18 MB base)

mod core;
mod formatting;
mod support;
mod transcription;
mod ui;

use anyhow::Result;
use log::info;
use support::settings::Settings;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    info!("perihel-transcribe 0.1.0 — low-memory local STT");

    let settings = Settings::load_or_default();
    info!("hotkey: {:?}, model: {:?}", settings.hotkey, settings.model);

    // Ensure model exists or download (sherpa int8)
    let model_path = support::paths::model_dir();
    if !model_path.exists() {
        info!("model not found at {:?} — will download parakeet int8 on first dictation", model_path);
    }

    // Wire controller like murmur: HotkeyMonitor → DictationController → AudioCapture → Engine → Formatter → Injector
    let (controller, hud_handle) = core::controller::DictationController::new(settings).await?;

    // Tray + HUD run on main thread (egui needs it)
    // Controller runs on tokio background
    ui::tray::run_tray(controller, hud_handle).await?;

    Ok(())
}
