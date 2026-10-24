//! System tray for the dictation sidecar.

use anyhow::Result;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::TrayIconBuilder;

pub struct Tray {
    _icon: TrayIconBuilderGuard,
}

/// Keep the tray icon alive for the process lifetime.
struct TrayIconBuilderGuard {
    _inner: tray_icon::TrayIcon,
}

impl Tray {
    pub fn new() -> Result<Self> {
        let menu = Menu::new();
        menu.append(&MenuItem::with_id("data", "Open data folder", true, None))?;
        menu.append(&MenuItem::with_id("sep", "", false, None))?;
        menu.append(&MenuItem::with_id("quit", "Quit Perihel Transcribe", true, None))?;

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Perihel Transcribe — hold Right Ctrl to dictate")
            .build()?;

        Ok(Self { _icon: TrayIconBuilderGuard { _inner: icon } })
    }
}

/// Polled from the egui update loop (tray-icon menu events need a pump).
pub fn poll_events() {
    let rx = MenuEvent::receiver();
    while let Ok(event) = rx.try_recv() {
        match event.id().0.as_str() {
            "data" => {
                let dir = perihel_core::app_data_dir();
                std::fs::create_dir_all(&dir).ok();
                #[cfg(windows)]
                let _ = std::process::Command::new("explorer").arg(dir).spawn();
                #[cfg(not(windows))]
                let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
            }
            "quit" => std::process::exit(0),
            _ => {}
        }
    }
}
