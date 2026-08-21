//! HUDOverlay — non-activating floating panel (340x76, bottom-center)
//! WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_TOOLWINDOW

use anyhow::Result;

#[derive(Clone)]
pub struct HudHandle;

impl HudHandle {
    pub fn new() -> Self { Self }
    pub fn show_listening(&self, rms: f32) { let _ = rms; }
    pub fn hide(&self) {}
    pub fn set_transcript(&self, _t: &str) {}
}

pub struct HudWindow;

impl HudWindow {
    pub fn new() -> Result<Self> { Ok(Self) }
    // Real impl with eframe: transparent, undecorated, always_on_top, 340x76
    // WindowAttributes: with_decorations(false).with_transparent(true).with_always_on_top(true).with_resizable(false)
    // + WS_EX_NOACTIVATE via windows crate after creation
}
