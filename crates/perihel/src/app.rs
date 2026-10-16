use eframe::egui;
use eframe::egui::CornerRadius;
use rusqlite::Connection;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use crate::agents;
use crate::copilot;
use perihel_core::db::{self, Capture};
use crate::llm::LlamaSidecar;
use crate::vision::VisionSidecar;

use super::AppCommand;

const ACCENT: egui::Color32 = egui::Color32::from_rgb(0, 140, 200);
const BG: egui::Color32 = egui::Color32::from_rgb(240, 240, 245);
const CARD: egui::Color32 = egui::Color32::from_rgb(255, 255, 255);
const TEXT: egui::Color32 = egui::Color32::from_gray(30);
const MUTED: egui::Color32 = egui::Color32::from_gray(100);
const DIM: egui::Color32 = egui::Color32::from_gray(120);
const DONE_GREEN: egui::Color32 = egui::Color32::from_rgb(0, 150, 80);
const SURFACE: egui::Color32 = egui::Color32::from_rgb(230, 230, 238);
const RECORD_RED: egui::Color32 = egui::Color32::from_rgb(220, 50, 50);

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

fn take_screenshot(data_dir: &std::path::Path) -> Option<String> {
    let screenshots_dir = data_dir.join("screenshots");
    std::fs::create_dir_all(&screenshots_dir).ok()?;
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S_%3f").to_string();
    let path = screenshots_dir.join(format!("{}.png", timestamp));
    let monitors = xcap::Monitor::all().ok()?;
    let monitor = monitors.first()?;
    let image = monitor.capture_image().ok()?;
    image.save(&path).ok()?;
    Some(path.to_string_lossy().to_string())
}

#[cfg(windows)]
fn fetch_url_text(url: &str) -> Option<String> {
    let safe_url = url.replace('\'', "''");
    let mut cmd = std::process::Command::new("powershell");
    cmd.creation_flags(CREATE_NO_WINDOW);
    let resp = cmd
        .args(["-NoProfile", "-Command", &format!(
            "try {{ `$r = Invoke-WebRequest -Uri '{}' -UseBasicParsing -TimeoutSec 10; `$c = `$r.Content; if (`$c.Length -gt 5000) {{ `$c.Substring(0, 5000) }} else {{ `$c }} }} catch {{ '' }}", safe_url
        )])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&resp.stdout).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

#[cfg(not(windows))]
fn fetch_url_text(_url: &str) -> Option<String> {
    None
}

fn pipe_through_agent(bin_path: &str, text_body: &str, screenshot_path: Option<&str>, url_text: Option<&str>) -> Option<String> {
    let mut prompt = format!("Process this thought concisely: {}", text_body);
    if let Some(url) = url_text {
        prompt.push_str(&format!("\n\nURL content: {}", url));
    }
    if screenshot_path.is_some() {
        prompt.push_str("\n\nA screenshot was also taken.");
    }
    let mut cmd = std::process::Command::new(bin_path);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.arg("--prompt").arg(&prompt)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let output = cmd.output().ok()?;
    let trimmed = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if trimmed.is_empty() { None } else { Some(trimmed) }
}

#[cfg(windows)]
fn record_voice(data_dir: &std::path::Path) -> Option<String> {
    let voice_dir = data_dir.join("voice");
    std::fs::create_dir_all(&voice_dir).ok()?;
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S_%3f").to_string();
    let txt_path = voice_dir.join(format!("{}.txt", timestamp));

    let ps_script = format!(
        "Add-Type -AssemblyName System.Speech;\
         `$rec = New-Object System.Speech.Recognition.SpeechRecognitionEngine;\
         `$rec.SetInputToDefaultAudioDevice();\
         `$grammar = New-Object System.Speech.Recognition.DictationGrammar;\
         `$rec.LoadGrammar(`$grammar);\
         `$result = `$rec.Recognize((New-Object System.TimeSpan -ArgumentList @(0,0,15)));\
         if (`$result -ne `$null) {{ [System.IO.File]::WriteAllText('{}', `$result.Text, [System.Text.Encoding]::UTF8); `$result.Text }} else {{ '' }}",
        txt_path.to_string_lossy().replace('\'', "''")
    );

    let mut cmd = std::process::Command::new("powershell");
    cmd.creation_flags(CREATE_NO_WINDOW);
    let output = cmd
        .args(["-NoProfile", "-Command", &ps_script])
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let text = stdout.trim().to_string();
    if text.is_empty() {
        Some(String::new())
    } else {
        Some(text)
    }
}

#[cfg(not(windows))]
fn record_voice(_data_dir: &std::path::Path) -> Option<String> {
    None
}

pub fn apply_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let cr = CornerRadius::same(6);
    style.visuals = egui::Visuals {
        dark_mode: false,
        override_text_color: Some(TEXT),
        window_fill: BG,
        panel_fill: BG,
        faint_bg_color: CARD,
        extreme_bg_color: SURFACE,
        hyperlink_color: ACCENT,
        selection: egui::style::Selection {
            bg_fill: ACCENT.linear_multiply(0.2),
            stroke: egui::Stroke::new(1.0_f32, ACCENT),
        },
        widgets: egui::style::Widgets {
            noninteractive: egui::style::WidgetVisuals {
                bg_fill: CARD,
                weak_bg_fill: SURFACE,
                bg_stroke: egui::Stroke::new(1.0_f32, egui::Color32::from_gray(200)),
                corner_radius: cr,
                fg_stroke: egui::Stroke::new(1.0_f32, MUTED),
                expansion: 0.0,
            },
            inactive: egui::style::WidgetVisuals {
                bg_fill: CARD,
                weak_bg_fill: SURFACE,
                bg_stroke: egui::Stroke::new(1.0_f32, egui::Color32::from_gray(210)),
                corner_radius: cr,
                fg_stroke: egui::Stroke::new(1.0_f32, TEXT),
                expansion: 0.0,
            },
            hovered: egui::style::WidgetVisuals {
                bg_fill: egui::Color32::from_rgb(220, 225, 240),
                weak_bg_fill: egui::Color32::from_rgb(215, 220, 235),
                bg_stroke: egui::Stroke::new(1.0_f32, ACCENT),
                corner_radius: cr,
                fg_stroke: egui::Stroke::new(1.5_f32, TEXT),
                expansion: 1.0,
            },
            active: egui::style::WidgetVisuals {
                bg_fill: ACCENT.linear_multiply(0.15),
                weak_bg_fill: ACCENT.linear_multiply(0.1),
                bg_stroke: egui::Stroke::new(1.0_f32, ACCENT),
                corner_radius: cr,
                fg_stroke: egui::Stroke::new(2.0_f32, TEXT),
                expansion: 1.0,
            },
            open: egui::style::WidgetVisuals {
                bg_fill: CARD,
                weak_bg_fill: SURFACE,
                bg_stroke: egui::Stroke::new(1.0_f32, ACCENT.linear_multiply(0.3)),
                corner_radius: cr,
                fg_stroke: egui::Stroke::new(1.5_f32, TEXT),
                expansion: 0.0,
            },
        },
        ..Default::default()
    };
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(14.0, 5.0);
    ctx.set_style(style);
}

#[derive(Clone, Copy, PartialEq)]
pub enum View {
    Capture,
    Copilot,
    Dashboard,
    Settings,
}

pub struct PerihelApp {
    pub conn: Arc<Mutex<Connection>>,
    pub cmd_rx: mpsc::Receiver<AppCommand>,
    pub cmd_tx: mpsc::Sender<AppCommand>,
    pub captures: Vec<Capture>,
    pub agents: Vec<agents::AgentInfo>,
    pub text_input: String,
    pub view: View,
    pub status_message: String,
    pub quiet_start: String,
    pub quiet_end: String,
    pub pending_count: i64,
    pub selected_agent_idx: usize,
    pub dashboard_filter: String,
    pub settings_url: String,
    pub screenshot_attached: bool,
    pub last_screenshot_path: Option<String>,
    pub is_recording: bool,
    pub expanded_id: Option<i64>,
    quiet_dirty: bool,
    // --- 2.0: copilot ---
    pub llm: Arc<Mutex<LlamaSidecar>>,
    pub vision: Arc<Mutex<VisionSidecar>>,
    pub guard: perihel_core::ResourceGuard,
    pub copilot_input: String,
    pub copilot_log: Vec<(String, String)>,
    pub copilot_busy: bool,
    // --- 2.0: settings mirror ---
    pub settings: perihel_core::Settings,
    pub model_choices: Vec<String>,
}

impl PerihelApp {
    pub fn new(
        conn: Arc<Mutex<Connection>>,
        cmd_rx: mpsc::Receiver<AppCommand>,
        cmd_tx: mpsc::Sender<AppCommand>,
        llm: Arc<Mutex<LlamaSidecar>>,
        vision: Arc<Mutex<VisionSidecar>>,
        guard: perihel_core::ResourceGuard,
    ) -> Self {
        let agents = agents::detect_agents();
        if let Ok(conn_lock) = conn.lock() {
            db::save_detected_agents(&conn_lock, &agents.iter().map(|a| db::DetectedAgent {
                name: a.name.clone(),
                version: a.version.clone(),
                path: a.path.clone(),
                detected_at: chrono::Local::now().to_rfc3339(),
            }).collect::<Vec<_>>()).ok();
        }
        let (captures, pending_count, quiet_start, quiet_end, agent_idx) = conn.lock().ok().map(|c| {
            let caps = db::get_all_captures(&c).unwrap_or_default();
            let pc = db::get_pending_count(&c).unwrap_or(0);
            let qs = db::get_setting(&c, "quiet_start").ok().flatten().unwrap_or("23:00".into());
            let qe = db::get_setting(&c, "quiet_end").ok().flatten().unwrap_or("08:00".into());
            let ai = db::get_setting(&c, "selected_agent").ok().flatten().and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
            (caps, pc, qs, qe, ai)
        }).unwrap_or_default();

        let settings = perihel_core::Settings::load_or_default();
        let model_choices = crate::llm::list_models();

        Self {
            conn,
            cmd_rx,
            cmd_tx,
            captures,
            agents,
            text_input: String::new(),
            view: View::Copilot,
            status_message: String::new(),
            quiet_start,
            quiet_end,
            pending_count,
            selected_agent_idx: agent_idx,
            dashboard_filter: "all".into(),
            settings_url: String::new(),
            screenshot_attached: false,
            last_screenshot_path: None,
            is_recording: false,
            expanded_id: None,
            quiet_dirty: false,
            llm,
            vision,
            guard,
            copilot_input: String::new(),
            copilot_log: Vec::new(),
            copilot_busy: false,
            settings,
            model_choices,
        }
    }

    pub fn show_content(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_tabs(ui);
            ui.separator();
            match self.view {
                View::Capture => self.draw_capture(ui),
                View::Copilot => self.draw_copilot(ui),
                View::Dashboard => self.draw_dashboard(ui),
                View::Settings => self.draw_settings(ui),
            }
        });
    }

    fn draw_tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (label, v) in &[("Copilot", View::Copilot), ("Capture", View::Capture), ("Dashboard", View::Dashboard), ("Settings", View::Settings)] {
                let is_active = self.view == *v;
                let mut btn = egui::Button::new(egui::RichText::new(*label).size(14.0));
                if is_active {
                    btn = btn.fill(ACCENT.linear_multiply(0.3));
                    btn = btn.stroke(egui::Stroke::new(1.0_f32, ACCENT));
                }
                if ui.add(btn).clicked() {
                    self.view = *v;
                }
            }
        });
    }

    // ---------- Copilot (2.0) ----------

    fn draw_copilot(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);

        // status line: model + engine state
        ui.horizontal(|ui| {
            let (label, color) = {
                let llm = self.llm.lock().unwrap();
                if llm.is_running() {
                    (format!("● {}", llm.model_label()), DONE_GREEN)
                } else {
                    ("● copilot asleep (starts on first message)".to_string(), DIM)
                }
            };
            ui.label(egui::RichText::new(label).size(11.0).color(color));
        });
        ui.add_space(4.0);

        // conversation log
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(ui.available_height() - 74.0)
            .show(ui, |ui| {
                if self.copilot_log.is_empty() {
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new("Ask anything, or try:").size(13.0).color(MUTED));
                        ui.label(egui::RichText::new("\"remind me to ask Prof. Chan at 15:00\"").size(12.0).color(DIM).italics());
                        ui.label(egui::RichText::new("\"remember: thePerihel repo uses sherpa-onnx now\"").size(12.0).color(DIM).italics());
                        ui.label(egui::RichText::new("\"what did I capture about climbing?\"").size(12.0).color(DIM).italics());
                        ui.label(egui::RichText::new("Ctrl+Shift+K · hold Right Ctrl anywhere to dictate").size(11.0).color(DIM));
                    });
                }
                for (who, text) in &self.copilot_log {
                    let (name, color) = match who.as_str() {
                        "you" => ("you", MUTED),
                        "tool" => ("tool", ACCENT),
                        _ => ("perihel", TEXT),
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new(format!("{name}:")).size(12.0).color(color).strong());
                        ui.label(egui::RichText::new(text).size(12.5).color(TEXT));
                    });
                    ui.add_space(2.0);
                }
                if self.copilot_busy {
                    ui.label(egui::RichText::new("thinking…").size(12.0).color(DIM).italics());
                }
            });

        ui.add_space(4.0);

        // input row
        ui.horizontal(|ui| {
            let hint = if self.copilot_busy { "working…" } else { "Ask, capture, or remind…" };
            let enter = egui::TextEdit::singleline(&mut self.copilot_input)
                .hint_text(hint)
                .desired_width(ui.available_width() - 96.0)
                .show(ui)
                .response
                .lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if self.copilot_busy {
                ui.add_enabled_ui(false, |ui| {
                    ui.add_sized([88.0, 30.0], egui::Button::new("…")).clicked();
                });
            } else if ui.add_sized([88.0, 30.0], egui::Button::new(
                egui::RichText::new("Send").size(13.0).color(egui::Color32::WHITE)
            ).fill(ACCENT).corner_radius(CornerRadius::same(8))).clicked() || (enter && !self.copilot_input.trim().is_empty()) {
                self.run_copilot();
            }
        });

        if !self.status_message.is_empty() {
            ui.label(egui::RichText::new(&self.status_message).size(11.0).color(DIM));
        }
    }

    fn run_copilot(&mut self) {
        let input = self.copilot_input.trim().to_string();
        if input.is_empty() || self.copilot_busy {
            return;
        }
        self.copilot_input.clear();
        self.copilot_busy = true;
        self.status_message.clear();

        let conn = self.conn.clone();
        let llm = self.llm.clone();
        let vision = self.vision.clone();
        let data_dir = db::get_data_dir();
        let guard = perihel_core::ResourceGuard::conservative();
        let tts_enabled = self.settings.tts_enabled;
        let tts_voice = self.settings.tts_voice.clone();
        let tx = self.cmd_tx.clone();

        std::thread::spawn(move || {
            let outcome = copilot::run_copilot(&llm, &vision, &conn, data_dir, &guard, &input, tts_enabled, tts_voice);
            tx.send(AppCommand::CopilotDone { input, answer: outcome.answer, calls: outcome.calls }).ok();
        });
    }

    /// Called every frame from the runner: reclaim RAM from idle sidecars.
    pub fn tick_sidecars(&mut self) {
        let mut llm = self.llm.lock().unwrap();
        llm.stop_if_idle(std::time::Duration::from_secs(300));
        let mut vision = self.vision.lock().unwrap();
        vision.stop_if_idle(std::time::Duration::from_secs(120));
    }

    pub fn shutdown_sidecars(&mut self) {
        self.llm.lock().unwrap().stop();
        self.vision.lock().unwrap().stop();
    }

    fn draw_capture(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);

        if self.is_recording {
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(
                    egui::RichText::new("Recording... (15s max)")
                        .size(14.0).color(RECORD_RED).strong()
                ));
            });
            ui.add_space(8.0);
        }

        ui.label(egui::RichText::new("What's on your mind?").size(14.0).color(MUTED));
        ui.add_sized([ui.available_width(), 140.0], egui::TextEdit::multiline(&mut self.text_input)
            .hint_text("Type your thought here...")
            .desired_rows(6).frame(true));

        ui.add_space(8.0);
        ui.add_sized([ui.available_width(), 28.0], egui::TextEdit::singleline(&mut self.settings_url)
            .hint_text("https://... (optional URL to attach)").frame(true));

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let voice_label = if self.is_recording { "Recording..." } else { "Voice" };
            let voice_btn = egui::Button::new(
                egui::RichText::new(voice_label).size(13.0).color(if self.is_recording { RECORD_RED } else { TEXT })
            ).fill(if self.is_recording { RECORD_RED.linear_multiply(0.15) } else { SURFACE })
             .corner_radius(CornerRadius::same(6));

            ui.add_enabled_ui(!self.is_recording, |ui| {
                if ui.add_sized([130.0, 36.0], voice_btn).clicked() {
                    self.start_recording();
                }
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(!self.is_recording, |ui| {
                    if ui.add_sized([120.0, 36.0], egui::Button::new(
                        egui::RichText::new("Capture").size(14.0).color(egui::Color32::WHITE)
                    ).fill(ACCENT).corner_radius(CornerRadius::same(8))).clicked() {
                        self.do_capture();
                    }
                });
            });
        });

        if self.screenshot_attached {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("[CAM] Screenshot attached").size(11.0).color(DIM));
                if ui.button("X").clicked() {
                    self.screenshot_attached = false;
                    self.last_screenshot_path = None;
                }
            });
        }

        if !self.status_message.is_empty() {
            ui.add_space(8.0);
            ui.label(egui::RichText::new(&self.status_message).size(12.0).color(DIM));
        }
    }

    fn start_recording(&mut self) {
        self.is_recording = true;
        self.status_message = "Recording...".into();
        let data_dir = db::get_data_dir();
        let tx = self.cmd_tx.clone();

        std::thread::spawn(move || {
            let text = record_voice(&data_dir);
            let msg = match text {
                Some(t) => {
                    if t.is_empty() {
                        "Voice: no speech detected".into()
                    } else {
                        format!("Transcribed: {}", t.chars().take(80).collect::<String>())
                    }
                }
                None => "Voice unavailable on this platform.".into(),
            };
            let _ = tx.send(AppCommand::RecordingComplete(msg));
        });
    }

    fn draw_dashboard(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for (key, label) in &[("all", "All"), ("pending", "Pending"), ("done", "Done"), ("dismissed", "Dismissed")] {
                let selected = self.dashboard_filter == *key;
                let mut btn = egui::Button::new(egui::RichText::new(*label).size(12.0)).corner_radius(CornerRadius::same(6));
                if selected { btn = btn.fill(ACCENT.linear_multiply(0.3)).stroke(egui::Stroke::new(1.0_f32, ACCENT)); }
                if ui.add(btn).clicked() { self.dashboard_filter = key.to_string(); }
            }
        });

        let filtered: Vec<Capture> = if self.dashboard_filter == "all" {
            self.captures.clone()
        } else {
            self.captures.iter().filter(|c| c.status == self.dashboard_filter).cloned().collect()
        };

        ui.add_space(4.0);
        ui.label(egui::RichText::new(format!("{} capture{}", filtered.len(), if filtered.len() == 1 { "" } else { "s" })).size(12.0).color(DIM));
        ui.separator();

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if filtered.is_empty() {
                ui.add_space(60.0);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("No captures yet").size(15.0).color(DIM));
                    ui.label(egui::RichText::new("Type a thought and submit it").size(12.0).color(DIM));
                });
                return;
            }

            for capture in &filtered {
                let is_expanded = self.expanded_id == Some(capture.id);
                let status_color = match capture.status.as_str() {
                    "done" => DONE_GREEN,
                    "dismissed" => egui::Color32::from_gray(160),
                    _ => ACCENT,
                };
                let status_dot = match capture.status.as_str() {
                    "done" => "\u{25CF}",
                    "dismissed" => "\u{25CB}",
                    _ => "\u{25CF}",
                };
                let type_color = match capture.content_type.as_str() {
                    "url" => egui::Color32::from_rgb(100, 140, 200),
                    "screenshot" => egui::Color32::from_rgb(180, 120, 80),
                    "voice" => egui::Color32::from_rgb(180, 80, 120),
                    _ => egui::Color32::from_gray(140),
                };
                let type_badge = match capture.content_type.as_str() {
                    "url" => "URL",
                    "screenshot" => "IMG",
                    "voice" => "AUDIO",
                    _ => "TEXT",
                };
                let display_content = if is_expanded {
                    capture.content.clone()
                } else {
                    let preview: String = capture.content.chars().take(70).collect();
                    if capture.content.chars().count() > 70 { format!("{}...", preview) } else { preview }
                };

                egui::Frame { fill: CARD, corner_radius: CornerRadius::same(8), stroke: egui::Stroke::new(1.0_f32, if is_expanded { ACCENT.linear_multiply(0.5) } else { egui::Color32::from_gray(220) }), inner_margin: egui::Margin::symmetric(10, 8), ..Default::default() }
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(status_dot).size(14.0).color(status_color));
                            egui::Frame { fill: type_color.linear_multiply(0.2), corner_radius: CornerRadius::same(4), inner_margin: egui::Margin::symmetric(5, 2), ..Default::default() }
                                .show(ui, |ui| {
                                    ui.label(egui::RichText::new(type_badge).size(10.0).color(type_color).strong());
                                });
                            ui.add(egui::Label::new(egui::RichText::new(&display_content).size(12.0)).sense(egui::Sense::click()));
                            if ui.rect_contains_pointer(ui.min_rect()) && ui.input(|i| i.pointer.any_click()) {
                                self.expanded_id = if is_expanded { None } else { Some(capture.id) };
                            }
                            if capture.status == "pending" {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.button("X").clicked() {
                                        if let Ok(conn) = self.conn.lock() { db::update_status(&conn, capture.id, "dismissed").ok(); }
                                        self.refresh_captures();
                                    }
                                    if ui.add(egui::Button::new(egui::RichText::new("OK").size(11.0).color(DONE_GREEN)).fill(DONE_GREEN.linear_multiply(0.2)).corner_radius(CornerRadius::same(4))).clicked() {
                                        if let Ok(conn) = self.conn.lock() { db::update_status(&conn, capture.id, "done").ok(); }
                                        self.refresh_captures();
                                    }
                                });
                            }
                        });
                        if let Some(ref agent) = capture.agent_used {
                            ui.label(egui::RichText::new(format!("via {}", agent)).size(10.0).color(DIM));
                        }
                        if let Some(ref summary) = capture.ai_summary {
                            ui.label(egui::RichText::new(summary).size(11.0).color(MUTED).italics());
                        }
                    });
                ui.add_space(4.0);
            }
        });
    }

    fn draw_settings(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);

        ui.label(egui::RichText::new("AI Agent").size(14.0).color(ACCENT).strong());
        ui.add_space(4.0);
        let detected: Vec<&agents::AgentInfo> = self.agents.iter().filter(|a| a.detected).collect();
        if !detected.is_empty() {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Process with:").size(12.0).color(MUTED));
                let names: Vec<&str> = detected.iter().map(|a| a.name.as_str()).collect();
                let display_text = if self.selected_agent_idx == 0 {
                    "None".to_string()
                } else {
                    names.get(self.selected_agent_idx - 1).copied().unwrap_or("None").to_string()
                };
                egui::ComboBox::from_id_salt("agent_sel")
                    .selected_text(&display_text)
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        let prev = self.selected_agent_idx;
                        ui.selectable_value(&mut self.selected_agent_idx, 0, "None");
                        for (i, name) in names.iter().enumerate() {
                            ui.selectable_value(&mut self.selected_agent_idx, i + 1, *name);
                        }
                        if self.selected_agent_idx != prev {
                            if let Ok(conn) = self.conn.lock() {
                                db::set_setting(&conn, "selected_agent", &self.selected_agent_idx.to_string()).ok();
                            }
                        }
                    });
            });
        } else {
            ui.label(egui::RichText::new("No AI agents detected on this system.").size(12.0).color(DIM));
        }

        ui.add_space(16.0);
        ui.label(egui::RichText::new("Screenshot").size(14.0).color(ACCENT).strong());
        ui.add_space(4.0);
        if ui.add(egui::Button::new(egui::RichText::new("Take Screenshot").size(12.0)).corner_radius(CornerRadius::same(6))).clicked() {
            let data_dir = db::get_data_dir();
            if let Some(path) = take_screenshot(&data_dir) {
                self.last_screenshot_path = Some(path);
                self.screenshot_attached = true;
                self.status_message = "Screenshot ready".into();
            } else {
                self.status_message = "Screenshot failed".into();
            }
        }
        if self.screenshot_attached {
            ui.label(egui::RichText::new("Screenshot attached to next capture.").size(11.0).color(DONE_GREEN));
        }

        ui.add_space(16.0);
        ui.label(egui::RichText::new("Quiet Hours").size(14.0).color(ACCENT).strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Start:").size(12.0).color(MUTED));
            let prev_start = self.quiet_start.clone();
            ui.add(egui::TextEdit::singleline(&mut self.quiet_start).desired_width(50.0).frame(true));
            if self.quiet_start != prev_start { self.quiet_dirty = true; }

            ui.label(egui::RichText::new("End:").size(12.0).color(MUTED));
            let prev_end = self.quiet_end.clone();
            ui.add(egui::TextEdit::singleline(&mut self.quiet_end).desired_width(50.0).frame(true));
            if self.quiet_end != prev_end { self.quiet_dirty = true; }
        });

        if self.quiet_dirty {
            let valid = self.quiet_start.len() == 5 && self.quiet_end.len() == 5
                && self.quiet_start.as_bytes()[2] == b':'
                && self.quiet_end.as_bytes()[2] == b':'
                && self.quiet_start.chars().all(|c| c.is_ascii_digit() || c == ':')
                && self.quiet_end.chars().all(|c| c.is_ascii_digit() || c == ':')
                && self.quiet_start[..2].parse::<u8>().ok().is_some_and(|h| h < 24)
                && self.quiet_start[3..].parse::<u8>().ok().is_some_and(|m| m < 60)
                && self.quiet_end[..2].parse::<u8>().ok().is_some_and(|h| h < 24)
                && self.quiet_end[3..].parse::<u8>().ok().is_some_and(|m| m < 60);
            if valid {
                if let Ok(conn) = self.conn.lock() {
                    db::set_setting(&conn, "quiet_start", &self.quiet_start).ok();
                    db::set_setting(&conn, "quiet_end", &self.quiet_end).ok();
                }
                self.quiet_dirty = false;
            }
        }

        ui.add_space(16.0);
        ui.label(egui::RichText::new("Data").size(14.0).color(ACCENT).strong());
        ui.add_space(4.0);
        if ui.add(egui::Button::new(egui::RichText::new("Export as JSON").size(12.0)).corner_radius(CornerRadius::same(6))).clicked() {
            let data_dir = db::get_data_dir();
            let path = data_dir.join("perihel_export.json");
            if let Ok(conn) = self.conn.lock() {
                if let Ok(captures) = db::get_all_captures(&conn) {
                    match serde_json::to_string_pretty(&captures) {
                        Ok(json) => {
                            std::fs::write(&path, json).ok();
                            self.status_message = format!("Exported to {}", path.display());
                        }
                        Err(e) => self.status_message = format!("Export failed: {}", e),
                    }
                }
            }
        }

        // ---------- 2.0: Copilot / models ----------
        ui.add_space(16.0);
        ui.label(egui::RichText::new("Copilot Model").size(14.0).color(ACCENT).strong());
        ui.add_space(4.0);
        if self.model_choices.is_empty() {
            ui.label(egui::RichText::new("No .gguf in models/llm — run scripts/download-models.ps1").size(11.0).color(DIM));
        } else {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Model:").size(12.0).color(MUTED));
                egui::ComboBox::from_id_salt("llm_model")
                    .selected_text(if self.settings.llm_model.is_empty() { "auto (smallest perihel-*)".to_string() } else { self.settings.llm_model.clone() })
                    .width(230.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.settings.llm_model, String::new(), "auto (smallest perihel-*)");
                        for m in self.model_choices.clone() {
                            ui.selectable_value(&mut self.settings.llm_model, m.clone(), m);
                        }
                    });
            });
            if ui.button("Apply + save").clicked() {
                if let Ok(mut llm) = self.llm.lock() {
                    llm.preferred_model = self.settings.llm_model.clone();
                    llm.stop(); // reload on next use
                }
                self.settings.save().ok();
                self.status_message = "Model preference saved.".into();
            }
        }
        let (engine_state, vision_state) = {
            let l = self.llm.lock().unwrap();
            let v = self.vision.lock().unwrap();
            (l.is_running(), v.is_running())
        };
        ui.label(egui::RichText::new(format!(
            "llm: {} · vision: {} · free RAM: {} MB",
            if engine_state { "running" } else { "asleep" },
            if vision_state { if self.vision.lock().unwrap().available() { "running" } else { "no model" } } else { "asleep" },
            perihel_core::ram_free_mb(),
        )).size(11.0).color(DIM));

        // ---------- 2.0: Voice ----------
        ui.add_space(16.0);
        ui.label(egui::RichText::new("Voice").size(14.0).color(ACCENT).strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.settings.tts_enabled, "Speak replies & reminders (TTS)");
            if ui.button("Test voice").clicked() && self.settings.tts_enabled {
                crate::tts::speak("Perihel voice check.", &self.settings.tts_voice).ok();
            }
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Dictation engine:").size(12.0).color(MUTED));
            egui::ComboBox::from_id_salt("stt_engine")
                .selected_text(self.settings.stt_engine.clone())
                .width(160.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.settings.stt_engine, "sense-voice".into(), "sense-voice (zh+en)");
                    ui.selectable_value(&mut self.settings.stt_engine, "whisper-tiny".into(), "whisper-tiny (en, light)");
                    ui.selectable_value(&mut self.settings.stt_engine, "parakeet".into(), "parakeet (en, best)");
                });
        });
        if ui.button("Save settings").clicked() {
            self.settings.save().ok();
            self.status_message = "Settings saved.".into();
        }
    }

    fn get_agent_bin_and_name(&self) -> (Option<String>, Option<String>) {
        let detected: Vec<&agents::AgentInfo> = self.agents.iter().filter(|a| a.detected).collect();
        if self.selected_agent_idx > 0 && self.selected_agent_idx <= detected.len() {
            let agent = detected[self.selected_agent_idx - 1];
            (agent.path.clone(), Some(agent.name.clone()))
        } else {
            (None, None)
        }
    }

    fn do_capture(&mut self) {
        let text = self.text_input.trim().to_string();
        if text.is_empty() {
            self.status_message = "Type something first.".into();
            return;
        }

        let url_to_capture = self.settings_url.trim().to_string();
        let screenshot_path = self.last_screenshot_path.take();
        let has_screenshot = screenshot_path.is_some();
        let (agent_bin, agent_name) = self.get_agent_bin_and_name();

        let id = if let Ok(conn) = self.conn.lock() {
            db::insert_capture(&conn, &text, "text").ok()
        } else { None };

        if !url_to_capture.is_empty() {
            if let Ok(conn) = self.conn.lock() {
                db::insert_capture(&conn, &url_to_capture, "url").ok();
            }
        }

        if has_screenshot {
            if let Some(ref path) = screenshot_path {
                if let Ok(conn) = self.conn.lock() {
                    db::insert_capture(&conn, path, "screenshot").ok();
                }
            }
        }

        if let (Some(bin), Some(name), Some(cap_id)) = (agent_bin, agent_name, id) {
            let conn = self.conn.clone();
            let text_for_agent = text.clone();
            let screen = screenshot_path.clone();
            std::thread::spawn(move || {
                let url_text = if !url_to_capture.is_empty() {
                    fetch_url_text(&url_to_capture)
                } else {
                    None
                };
                let summary = pipe_through_agent(&bin, &text_for_agent, screen.as_deref(), url_text.as_deref());
                if let Some(s) = summary {
                    if let Ok(db) = conn.lock() { db::update_ai_summary(&db, cap_id, &s, &name).ok(); }
                }
            });
        }

        self.text_input.clear();
        self.screenshot_attached = false;
        self.settings_url.clear();
        self.status_message = "Captured!".into();
        self.refresh_captures();
    }

    pub fn refresh_captures(&mut self) {
        if let Ok(conn) = self.conn.lock() {
            self.captures = db::get_all_captures(&conn).unwrap_or_default();
            self.pending_count = db::get_pending_count(&conn).unwrap_or(0);
        }
    }
}
