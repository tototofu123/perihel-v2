mod agents;
mod app;
mod db;
mod scheduler;

use app::PerihelApp;
use eframe::egui;
use global_hotkey::{GlobalHotKeyManager, GlobalHotKeyEvent, hotkey::{HotKey, Modifiers, Code}};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use tray_icon::{TrayIcon, TrayIconBuilder, menu::{Menu, MenuEvent, MenuItem}};

pub enum AppCommand {
    ShowWindow,
    Quit,
}

fn main() -> eframe::Result {
    let conn = db::init_db().expect("Failed to initialize database");
    let conn = Arc::new(Mutex::new(conn));

    let (cmd_tx, cmd_rx) = mpsc::channel::<AppCommand>();

    let app = PerihelApp::new(conn.clone(), cmd_rx);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([400.0, 600.0])
            .with_visible(false),
        ..Default::default()
    };

    eframe::run_native(
        "Perihel",
        options,
        Box::new(|cc| {
            let tray = setup_tray(cc, cmd_tx.clone());
            let _hotkey = setup_hotkey(cmd_tx.clone());
            Ok(Box::new(PerihelRunner { app, _tray: tray }))
        }),
    )
}

struct PerihelRunner {
    app: PerihelApp,
    _tray: TrayIcon,
}

impl eframe::App for PerihelRunner {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(cmd) = self.app.cmd_rx.try_recv() {
            match cmd {
                AppCommand::ShowWindow => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                }
                AppCommand::Quit => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }

        if self.app.dashboard_open {
            self.app.show_dashboard(ctx);
        }
        if self.app.settings_open {
            self.app.show_settings(ctx);
        }
        if self.app.show_capture {
            self.app.show_capture_popup(ctx);
        }
    }
}

fn setup_tray(_cc: &eframe::CreationContext<'_>, cmd_tx: mpsc::Sender<AppCommand>) -> TrayIcon {
    let tray_menu = Menu::new();
    let capture_item = MenuItem::with_id("capture", "New Capture", true, None);
    let dashboard_item = MenuItem::with_id("dashboard", "Dashboard", true, None);
    let settings_item = MenuItem::with_id("settings", "Settings", true, None);
    let quit_item = MenuItem::with_id("quit", "Quit", true, None);

    tray_menu.append(&capture_item).ok();
    tray_menu.append(&dashboard_item).ok();
    tray_menu.append(&settings_item).ok();
    tray_menu.append(&quit_item).ok();

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_tooltip("Perihel")
        .build()
        .expect("Failed to create tray icon");

    std::thread::spawn(move || {
        let menu_receiver = MenuEvent::receiver();
        loop {
            if let Ok(event) = menu_receiver.recv() {
                if event.id() == "capture" || event.id() == "dashboard" || event.id() == "settings" {
                    cmd_tx.send(AppCommand::ShowWindow).ok();
                } else if event.id() == "quit" {
                    cmd_tx.send(AppCommand::Quit).ok();
                }
            }
        }
    });

    tray
}

fn setup_hotkey(cmd_tx: mpsc::Sender<AppCommand>) -> GlobalHotKeyManager {
    let manager = GlobalHotKeyManager::new().expect("Failed to create hotkey manager");
    let hotkey = HotKey::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyP);
    manager.register(hotkey).expect("Failed to register hotkey");

    std::thread::spawn(move || {
        let receiver = GlobalHotKeyEvent::receiver();
        loop {
            if let Ok(_event) = receiver.recv() {
                cmd_tx.send(AppCommand::ShowWindow).ok();
            }
        }
    });

    manager
}
