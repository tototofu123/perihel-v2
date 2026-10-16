//! HotkeyMonitor — global push-to-talk. Observed, never swallowed (AltGr-safe).
//! global-hotkey delivers Pressed/Released for the registered key.

use anyhow::Result;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use std::sync::mpsc;

#[derive(Debug, Clone, Copy)]
pub enum HotkeyEvent {
    Pressed,
    Released,
}

pub struct HotkeyMonitor {
    rx: mpsc::Receiver<HotkeyEvent>,
}

impl HotkeyMonitor {
    pub fn new(hotkey_str: &str) -> Result<Self> {
        // The manager must outlive everything; global_hotkey's manager is not
        // Send, so we leak it deliberately — one registration per process.
        let manager = GlobalHotKeyManager::new()?;
        let hk = match hotkey_str {
            "RightAlt" => HotKey::new(Some(Modifiers::empty()), Code::AltRight),
            "F8" => HotKey::new(None, Code::F8),
            // Right Ctrl default — never interferes with AltGr layouts
            _ => HotKey::new(Some(Modifiers::empty()), Code::ControlRight),
        };
        manager.register(hk)?;
        std::mem::forget(manager);
        log::info!("push-to-talk hotkey registered: {hk:?}");

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            loop {
                match GlobalHotKeyEvent::receiver().recv() {
                    Ok(evt) => {
                        let ev = if evt.state == HotKeyState::Pressed {
                            HotkeyEvent::Pressed
                        } else {
                            HotkeyEvent::Released
                        };
                        if tx.send(ev).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self { rx })
    }

    pub fn next_event(&self) -> Option<HotkeyEvent> {
        self.rx.recv().ok()
    }
}
