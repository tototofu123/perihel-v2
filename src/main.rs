mod agents;
mod app;
mod db;
mod scheduler;

use app::PerihelApp;
use eframe::egui;
use global_hotkey::{GlobalHotKeyManager, GlobalHotKeyEvent, hotkey::{HotKey, Modifiers, Code}};
use scheduler::Scheduler;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use tray_icon::{TrayIcon, TrayIconBuilder, menu::{Menu, MenuEvent, MenuItem}};

pub enum AppCommand {
    ShowWindow,
    HideWindow,
    Quit,
    ShowCapture,
    ShowDashboard,
    ShowSettings,
    StatusMessage(String),
}

fn main() -> eframe::Result {
    let conn = db::init_db().expect("Failed to initialize database");
    let conn = Arc::new(Mutex::new(conn));

    let (cmd_tx, cmd_rx) = mpsc::channel::<AppCommand>();

    let app = PerihelApp::new(conn.clone(), cmd_rx, cmd_tx.clone());

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([400.0, 600.0])
            .with_visible(false),
        ..Default::default()
    };

    let _scheduler = {
        let s = Scheduler::new(conn.clone(), cmd_tx.clone());
        s.start();
        s
    };

    eframe::run_native(
        "Perihel",
        options,
        Box::new(|cc| {
            let tray = setup_tray(cc, cmd_tx.clone());
            let _hotkey = setup_hotkey(cmd_tx.clone());
            Ok(Box::new(PerihelRunner { app, _tray: tray, was_focused: false }))
        }),
    )
}

struct PerihelRunner {
    app: PerihelApp,
    _tray: TrayIcon,
    was_focused: bool,
}

impl eframe::App for PerihelRunner {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        app::apply_style(ctx);
        while let Ok(cmd) = self.app.cmd_rx.try_recv() {
            match cmd {
                AppCommand::ShowWindow | AppCommand::ShowCapture => {
                    self.app.show_capture = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                }
                AppCommand::HideWindow => {
                    self.app.show_capture = false;
                    self.app.dashboard_open = false;
                    self.app.settings_open = false;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                }
                AppCommand::ShowDashboard => {
                    self.app.dashboard_open = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                }
                AppCommand::ShowSettings => {
                    self.app.settings_open = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                }
                AppCommand::Quit => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                AppCommand::StatusMessage(msg) => {
                    self.app.status_message = msg;
                }
            }
        }

        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if !focused && self.was_focused {
            // Auto-hide all popups when focus is lost
            self.app.show_capture = false;
            self.app.dashboard_open = false;
            self.app.settings_open = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        self.was_focused = focused;

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
    let separator_item = MenuItem::with_id("sep", "", false, None);
    let exit_item = MenuItem::with_id("exit", "Exit", true, None);

    tray_menu.append(&capture_item).ok();
    tray_menu.append(&dashboard_item).ok();
    tray_menu.append(&settings_item).ok();
    tray_menu.append(&separator_item).ok();
    tray_menu.append(&exit_item).ok();

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_tooltip("Perihel")
        .build()
        .expect("Failed to create tray icon");

    std::thread::spawn(move || {
        let menu_receiver = MenuEvent::receiver();
        loop {
            if let Ok(event) = menu_receiver.recv() {
                let id = event.id().0.clone();
                match id.as_str() {
                    "capture" => cmd_tx.send(AppCommand::ShowCapture).ok(),
                    "dashboard" => cmd_tx.send(AppCommand::ShowDashboard).ok(),
                    "settings" => cmd_tx.send(AppCommand::ShowSettings).ok(),
                    "exit" => cmd_tx.send(AppCommand::Quit).ok(),
                    _ => None,
                };
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
