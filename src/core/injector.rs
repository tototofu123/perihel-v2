//! TextInjector — SendInput + clipboard fallback, captures HWND before HUD steals focus

use anyhow::Result;
use enigo::{Enigo, Keyboard, Settings};
use log::info;
use windows::Win32::{Foundation::HWND, UI::WindowsAndMessaging::GetForegroundWindow};

pub struct TextInjector {
    enigo: Enigo,
    target: Option<HWND>,
}

impl TextInjector {
    pub fn new() -> Self {
        let enigo = Enigo::new(&Settings::default()).expect("enigo init");
        Self { enigo, target: None }
    }

    /// Call on hotkey down, BEFORE showing HUD — preserves injection target.
    pub fn capture_foreground(&mut self) -> Option<HWND> {
        let hwnd = unsafe { GetForegroundWindow() };
        self.target = Some(hwnd);
        info!("captured foreground HWND {:?}", hwnd);
        Some(hwnd)
    }

    pub fn inject(&mut self, text: &str) -> Result<()> {
        if text.is_empty() { return Ok(()); }
        // Skeleton: use SendInput for all lengths to keep low RAM (no clipboard dep)
        // Long-text clipboard fallback can be re-enabled with arboard later
        self.enigo.text(text)?;
        Ok(())
    }
}
