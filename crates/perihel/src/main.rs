#![windows_subsystem = "windows"]

mod agents;
mod app;
mod copilot;
mod llm;
mod scheduler;
mod tools;
mod tts;
mod vision;

use app::PerihelApp;
use eframe::egui;
use global_hotkey::{GlobalHotKeyManager, GlobalHotKeyEvent, hotkey::{HotKey, Modifiers, Code}};
use llm::LlamaSidecar;
use scheduler::Scheduler;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use tray_icon::{TrayIcon, TrayIconBuilder, menu::{Menu, MenuEvent, MenuItem}};
use vision::VisionSidecar;

pub enum AppCommand {
    ShowWindow,
    Quit,
    ShowCapture,
    ShowDashboard,
    ShowSettings,
    ShowCopilot,
    RecordingComplete(String),
    CopilotDone { input: String, answer: String, calls: Vec<(String, String)> },
}

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    // Perihel is a companion widget, never a foreground workload: below-normal
    // priority for the whole process so the OS scheduler always favors the
    // user's real work under CPU contention.
    perihel_core::set_priority_idle();

    let conn = perihel_core::db::init_db().expect("Failed to initialize database");
    let conn = Arc::new(Mutex::new(conn));

    let settings = perihel_core::Settings::load_or_default();

    let (cmd_tx, cmd_rx) = mpsc::channel::<AppCommand>();

    let guard = perihel_core::ResourceGuard::conservative();
    let llm = Arc::new(Mutex::new(LlamaSidecar::new(8642, settings.llm_context_window, settings.llm_model.clone())));
    let vision = Arc::new(Mutex::new(VisionSidecar::new(8643)));

    let app = PerihelApp::new(conn.clone(), cmd_rx, cmd_tx.clone(), llm.clone(), vision.clone(), guard);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([420.0, 580.0])
            .with_visible(false),
        ..Default::default()
    };

    let _scheduler = {
        let s = Scheduler::new(conn.clone(), cmd_tx.clone(), settings.tts_enabled, settings.tts_voice.clone());
        s.start();
        s
    };

    eframe::run_native(
        "Perihel",
        options,
        Box::new(|cc| {
            let tray = setup_tray(cc, cmd_tx.clone());
            let hotkey_manager = setup_hotkey(cmd_tx.clone());
            Ok(Box::new(PerihelRunner { app, _tray: tray, _hotkey_manager: hotkey_manager, was_focused: false, blur_timer: 0 }))
        }),
    )
}

struct PerihelRunner {
    app: PerihelApp,
    _tray: TrayIcon,
    _hotkey_manager: GlobalHotKeyManager,
    was_focused: bool,
    blur_timer: u32,
}

impl eframe::App for PerihelRunner {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        app::apply_style(ctx);
        while let Ok(cmd) = self.app.cmd_rx.try_recv() {
            match cmd {
                AppCommand::ShowWindow | AppCommand::ShowCapture => {
                    self.app.view = app::View::Capture;
                    self.blur_timer = 0;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                }
                AppCommand::ShowCopilot => {
                    self.app.view = app::View::Copilot;
                    self.blur_timer = 0;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                }
                AppCommand::ShowDashboard => {
                    self.app.view = app::View::Dashboard;
                    self.blur_timer = 0;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                }
                AppCommand::ShowSettings => {
                    self.app.view = app::View::Settings;
                    self.blur_timer = 0;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                }
                AppCommand::Quit => {
                    self.app.shutdown_sidecars();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                AppCommand::RecordingComplete(msg) => {
                    self.app.is_recording = false;
                    if msg.starts_with("Transcribed: ") {
                        let text = msg.trim_start_matches("Transcribed: ");
                        if !self.app.text_input.is_empty() {
                            self.app.text_input.push('\n');
                        }
                        self.app.text_input.push_str(text);
                        self.app.status_message = "Voice transcribed to text above.".into();
                    } else {
                        self.app.status_message = msg;
                    }
                }
                AppCommand::CopilotDone { input, answer, calls } => {
                    self.app.copilot_busy = false;
                    self.app.copilot_log.push(("you".into(), input));
                    for (tool, result) in &calls {
                        self.app.copilot_log.push(("tool".into(), format!("{tool} → {result}")));
                    }
                    self.app.copilot_log.push(("perihel".into(), answer));
                }
            }
        }

        // free RAM when sidecars have been idle a while
        self.app.tick_sidecars();

        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(false));
        if !focused && self.was_focused {
            self.blur_timer += 1;
            if self.blur_timer > 30 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                self.blur_timer = 0;
            }
        } else {
            self.blur_timer = 0;
        }
        self.was_focused = focused;

        self.app.show_content(ctx);
    }
}

fn setup_tray(_cc: &eframe::CreationContext<'_>, cmd_tx: mpsc::Sender<AppCommand>) -> TrayIcon {
    let tray_menu = Menu::new();
    let capture_item = MenuItem::with_id("capture", "New Capture", true, None);
    let copilot_item = MenuItem::with_id("copilot", "Copilot", true, None);
    let dashboard_item = MenuItem::with_id("dashboard", "Dashboard", true, None);
    let settings_item = MenuItem::with_id("settings", "Settings", true, None);
    let separator_item = MenuItem::with_id("sep", "", false, None);
    let exit_item = MenuItem::with_id("exit", "Exit", true, None);

    tray_menu.append(&capture_item).ok();
    tray_menu.append(&copilot_item).ok();
    tray_menu.append(&dashboard_item).ok();
    tray_menu.append(&settings_item).ok();
    tray_menu.append(&separator_item).ok();
    tray_menu.append(&exit_item).ok();

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_tooltip("Perihel 2.0")
        .build()
        .expect("Failed to create tray icon");

    std::thread::spawn(move || {
        let menu_receiver = MenuEvent::receiver();
        loop {
            if let Ok(event) = menu_receiver.recv() {
                let id = event.id().0.clone();
                match id.as_str() {
                    "capture" => cmd_tx.send(AppCommand::ShowCapture).ok(),
                    "copilot" => cmd_tx.send(AppCommand::ShowCopilot).ok(),
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
    let capture = HotKey::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyP);
    let copilot = HotKey::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyK);
    manager.register(capture).expect("Failed to register capture hotkey");
    manager.register(copilot).expect("Failed to register copilot hotkey");
    let copilot_id = copilot.id();

    std::thread::spawn(move || {
        let receiver = GlobalHotKeyEvent::receiver();
        loop {
            if let Ok(event) = receiver.recv() {
                let cmd = if event.id == copilot_id { AppCommand::ShowCopilot } else { AppCommand::ShowWindow };
                cmd_tx.send(cmd).ok();
            }
        }
    });

    manager
}
