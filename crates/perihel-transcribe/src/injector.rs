//! TextInjector — SendInput via enigo. The foreground HWND is captured on hotkey
//! press (before the HUD could take focus) purely for logging/verification; the
//! injection itself goes to whatever currently has keyboard focus, which is the
//! same window because the HUD never activates.

use anyhow::Result;
use enigo::{Enigo, Keyboard, Settings};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Listening,
    Transcribing,
}

pub struct TextInjector {
    enigo: Enigo,
}

impl TextInjector {
    pub fn new() -> Result<Self> {
        let enigo = Enigo::new(&Settings::default())?;
        Ok(Self { enigo })
    }

    pub fn inject(&mut self, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        self.enigo.text(text)?;
        Ok(())
    }
}
