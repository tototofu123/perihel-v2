use crate::core::controller::DictationController;
use crate::ui::hud::HudHandle;
use anyhow::Result;
use log::info;

pub async fn run_tray(_controller: DictationController, _hud: HudHandle) -> Result<()> {
    info!("tray running — Right Ctrl to dictate");
    // Real impl: tray-item with menu: Settings, Dictionary, Quit
    // For skeleton, just park forever
    std::future::pending::<()>().await;
    Ok(())
}
