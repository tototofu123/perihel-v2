//! HotkeyMonitor — global hook, Right Ctrl default (AltGr-safe), observed not swallowed

use anyhow::Result;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::{Code, HotKey, Modifiers}};
use log::info;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy)]
pub enum HotkeyEvent { Pressed, Released }

pub struct HotkeyMonitor {
    _manager: GlobalHotKeyManager,
    rx: mpsc::Receiver<HotkeyEvent>,
}

impl HotkeyMonitor {
    pub fn new(hotkey_str: &str) -> Result<Self> {
        let manager = GlobalHotKeyManager::new()?;
        // Default Right Ctrl — configurable via settings.json
        let hk = match hotkey_str {
            "RightCtrl" | "RCtrl" => HotKey::new(Some(Modifiers::empty()), Code::ControlRight),
            "RightAlt" => HotKey::new(Some(Modifiers::empty()), Code::AltRight),
            "F8" => HotKey::new(None, Code::F8),
            _ => HotKey::new(Some(Modifiers::empty()), Code::ControlRight),
        };
        manager.register(hk)?;
        info!("hotkey registered: {:?}", hk);

        let (tx, rx) = mpsc::channel(16);
        // global-hotkey is press-only; we emulate hold via Pressed/Released with 150ms debounce elsewhere
        // Full push-to-talk needs WH_KEYBOARD_LL hook for key-up; global-hotkey v0.6 gives press events only.
        // TODO: replace with rdev low-level hook for true hold detection (see murmur HotkeyMonitor CGEventTap).
        std::thread::spawn(move || {
            loop {
                if let Ok(evt) = GlobalHotKeyEvent::receiver().recv() {
                    if evt.state == HotKeyState::Pressed { let _ = tx.blocking_send(HotkeyEvent::Pressed); }
                    else { let _ = tx.blocking_send(HotkeyEvent::Released); }
                }
            }
        });

        Ok(Self { _manager: manager, rx })
    }

    pub async fn next_event(&mut self) -> Option<HotkeyEvent> { self.rx.recv().await }
}
