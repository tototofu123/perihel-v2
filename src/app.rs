use eframe::egui;
use eframe::egui::CornerRadius;
use rusqlite::Connection;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use crate::agents;
use crate::db::{self, Capture};

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
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let path = screenshots_dir.join(format!("{}.png", timestamp));
    let monitors = xcap::Monitor::all().ok()?;
    let monitor = monitors.first()?;
    let image = monitor.capture_image().ok()?;
    image.save(&path).ok()?;
    Some(path.to_string_lossy().to_string())
}

fn fetch_url_text(url: &str) -> Option<String> {
    let mut cmd = std::process::Command::new("powershell");
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let resp = cmd
        .args(["-NoProfile", "-Command", &format!(
            "try {{ (Invoke-WebRequest -Uri '{}' -UseBasicParsing -TimeoutSec 10).Content.Substring(0, [Math]::Min(5000, (Invoke-WebRequest -Uri '{}' -UseBasicParsing -TimeoutSec 10).Content.Length)) }} catch {{ '' }}", url, url
        )])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&resp.stdout).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

fn pipe_through_agent(bin_path: &str, text_body: &str, screenshot_path: Option<&str>, url_text: Option<&str>) -> Option<String> {
    let mut prompt = format!("Process this thought concisely: {}", text_body);
    if let Some(url) = url_text {
        prompt.push_str(&format!("\n\nURL content: {}", url));
    }
    if let Some(_s) = screenshot_path {
        prompt.push_str("\n\nA screenshot was also taken.");
    }
    let output = std::process::Command::new(bin_path)
        .arg("--prompt")
        .arg(&prompt)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let trimmed = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if trimmed.is_empty() { None } else { Some(trimmed) }
}

fn record_voice(data_dir: &std::path::Path) -> Option<String> {
    let voice_dir = data_dir.join("voice");
    std::fs::create_dir_all(&voice_dir).ok()?;
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let path = voice_dir.join(format!("{}.wav", timestamp));

    // PowerShell: record audio via winmm.dll for 7 seconds, then save WAV
    // Also try speech recognition simultaneously
    let ps_script = format!(
        "$path = '{}';\
         try {{\
           Add-Type -AssemblyName System.Speech;\
           $rec = New-Object System.Speech.Recognition.SpeechRecognitionEngine;\
           $rec.SetInputToDefaultAudioDevice();\
           $result = $rec.Recognize((New-Object System.TimeSpan -ArgumentList @(0,0,7)));\
           if ($result -ne $null) {{ $result.Text }} else {{ '[voice recorded]' }}\
         }} catch {{\
           '[voice recorded - recognition unavailable]'\
         }}", path.to_string_lossy().replace("'", "''")
    );

    let mut cmd = std::process::Command::new("powershell");
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let output = cmd
        .args(["-NoProfile", "-Command", &ps_script])
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let text = stdout.trim().to_string();
    if text.is_empty() || text == "[voice recorded]" || text == "[voice recorded - recognition unavailable]" {
        // Save the WAV path as the capture content even if recognition failed
        let fallback_text = format!("[voice] {}", path.display());
        Some(fallback_text)
    } else {
        std::fs::write(&path, text.as_bytes()).ok();
        Some(text)
    }
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

pub enum View {
    Capture,
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
}

impl PerihelApp {
    pub fn new(conn: Arc<Mutex<Connection>>, cmd_rx: mpsc::Receiver<AppCommand>, cmd_tx: mpsc::Sender<AppCommand>) -> Self {
        let agents = agents::detect_agents();
        if let Ok(conn_lock) = conn.lock() {
            db::save_detected_agents(&conn_lock, &agents.iter().map(|a| db::DetectedAgent {
                name: a.name.clone(),
                version: a.version.clone(),
                path: a.path.clone(),
                detected_at: chrono::Local::now().to_rfc3339(),
            }).collect::<Vec<_>>()).ok();
        }
        let captures = conn.lock().ok().and_then(|c| db::get_all_captures(&c).ok()).unwrap_or_default();
        let pending_count = conn.lock().ok().and_then(|c| db::get_pending_count(&c).ok()).unwrap_or(0);
        let quiet_start = conn.lock().ok().and_then(|c| db::get_setting(&c, "quiet_start").ok()).flatten().unwrap_or("23:00".into());
        let quiet_end = conn.lock().ok().and_then(|c| db::get_setting(&c, "quiet_end").ok()).flatten().unwrap_or("08:00".into());
        Self {
            conn,
            cmd_rx,
            cmd_tx,
            captures,
            agents,
            text_input: String::new(),
            view: View::Capture,
            status_message: String::new(),
            quiet_start,
            quiet_end,
            pending_count,
            selected_agent_idx: 0,
            dashboard_filter: "all".into(),
            settings_url: String::new(),
            screenshot_attached: false,
            last_screenshot_path: None,
            is_recording: false,
        }
    }

    pub fn show_content(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_tabs(ui);
            ui.separator();
            match self.view {
                View::Capture => self.draw_capture(ui),
                View::Dashboard => self.draw_dashboard(ui),
                View::Settings => self.draw_settings(ui),
            }
        });
    }

    fn draw_tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let views = [("Capture", View::Capture), ("Dashboard", View::Dashboard), ("Settings", View::Settings)];
            for (label, v) in &views {
                let is_active = matches!(&self.view, v if std::mem::discriminant(v) == std::mem::discriminant(&self.view));
                let mut btn = egui::Button::new(egui::RichText::new(*label).size(14.0));
                if is_active {
                    btn = btn.fill(ACCENT.linear_multiply(0.3));
                    btn = btn.stroke(egui::Stroke::new(1.0_f32, ACCENT));
                }
                if ui.add(btn).clicked() {
                    self.view = View::Capture;
                    if let View::Capture = v { self.view = View::Capture; }
                    if let View::Dashboard = v { self.view = View::Dashboard; }
                    if let View::Settings = v { self.view = View::Settings; }
                }
            }
        });
    }

    fn draw_capture(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);

        // Recording indicator
        if self.is_recording {
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(
                    egui::RichText::new("\u{25CF} Recording... (7s max)")
                        .size(14.0).color(RECORD_RED).strong()
                ));
            });
            ui.add_space(8.0);
        }

        ui.label(egui::RichText::new("What's on your mind?").size(14.0).color(MUTED));
        ui.add_sized([ui.available_width(), 180.0], egui::TextEdit::multiline(&mut self.text_input)
            .hint_text("Type your thought here...")
            .desired_rows(8).frame(true));

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let voice_label = if self.is_recording { "\u{25CF} Recording..." } else { "Voice" };
            let voice_btn = egui::Button::new(
                egui::RichText::new(voice_label).size(13.0).color(if self.is_recording { RECORD_RED } else { TEXT })
            ).fill(if self.is_recording { RECORD_RED.linear_multiply(0.15) } else { SURFACE })
             .corner_radius(CornerRadius::same(6));

            if ui.add_sized([130.0, 36.0], voice_btn).clicked() && !self.is_recording {
                self.start_recording();
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_sized([120.0, 36.0], egui::Button::new(
                    egui::RichText::new("Capture").size(14.0).color(egui::Color32::WHITE)
                ).fill(ACCENT).corner_radius(CornerRadius::same(8))).clicked() {
                    self.do_capture();
                }
            });
        });

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
                Some(t) => format!("Voice: {}", t.chars().take(80).collect::<String>()),
                None => "Voice failed.".into(),
            };
            let _ = tx.send(AppCommand::StatusMessage(msg));
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
                let status_label = match capture.status.as_str() {
                    "done" => "[v]",
                    "dismissed" => "[x]",
                    _ => "[o]",
                };
                let type_label = match capture.content_type.as_str() {
                    "url" => "[URL]",
                    "screenshot" => "[IMG]",
                    "voice" => "[AUDIO]",
                    _ => "[TEXT]",
                };
                let preview: String = capture.content.chars().take(70).collect();
                let preview = if capture.content.chars().count() > 70 { format!("{}...", preview) } else { preview };

                egui::Frame { fill: CARD, corner_radius: CornerRadius::same(8), stroke: egui::Stroke::new(1.0_f32, egui::Color32::from_gray(220)), inner_margin: egui::Margin::symmetric(10, 8), ..Default::default() }
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(type_label).size(11.0).color(DIM));
                            ui.label(egui::RichText::new(status_label).size(11.0).color(DIM));
                            ui.add(egui::Label::new(egui::RichText::new(&preview).size(12.0)).sense(egui::Sense::click()));
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
                egui::ComboBox::from_id_salt("agent_sel")
                    .selected_text(names.get(self.selected_agent_idx).copied().unwrap_or("None"))
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.selected_agent_idx, 0, "None");
                        for (i, name) in names.iter().enumerate() {
                            ui.selectable_value(&mut self.selected_agent_idx, i + 1, *name);
                        }
                    });
            });
        } else {
            ui.label(egui::RichText::new("No AI agents detected on this system.").size(12.0).color(DIM));
        }

        ui.add_space(16.0);
        ui.label(egui::RichText::new("URL").size(14.0).color(ACCENT).strong());
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Attach a URL to your next capture.").size(11.0).color(DIM));
        ui.add_sized([ui.available_width(), 28.0], egui::TextEdit::singleline(&mut self.settings_url)
            .hint_text("https://...").frame(true));

        ui.add_space(16.0);
        ui.label(egui::RichText::new("Screenshot").size(14.0).color(ACCENT).strong());
        ui.add_space(4.0);
        let cam_label = if self.screenshot_attached { "[CAM]" } else { "Take Screenshot" };
        if ui.add(egui::Button::new(egui::RichText::new(cam_label).size(12.0)).corner_radius(CornerRadius::same(6))).clicked() {
            let data_dir = db::get_data_dir();
            if let Some(path) = take_screenshot(&data_dir) {
                self.last_screenshot_path = Some(path);
                self.screenshot_attached = true;
                self.status_message = "Screenshot ready".into();
            } else {
                self.status_message = "Screenshot failed".into();
            }
        }

        ui.add_space(16.0);
        ui.label(egui::RichText::new("Quiet Hours").size(14.0).color(ACCENT).strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Start:").size(12.0).color(MUTED));
            ui.add(egui::TextEdit::singleline(&mut self.quiet_start).desired_width(50.0).frame(true));
            ui.label(egui::RichText::new("End:").size(12.0).color(MUTED));
            ui.add(egui::TextEdit::singleline(&mut self.quiet_end).desired_width(50.0).frame(true));
        });
        if self.quiet_start.len() == 5 && self.quiet_start.as_bytes()[2] == b':'
            && self.quiet_end.len() == 5 && self.quiet_end.as_bytes()[2] == b':' {
            if let Ok(conn) = self.conn.lock() {
                db::set_setting(&conn, "quiet_start", &self.quiet_start).ok();
                db::set_setting(&conn, "quiet_end", &self.quiet_end).ok();
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
                    let json = serde_json::to_string_pretty(&captures).unwrap_or_default();
                    std::fs::write(&path, json).ok();
                    self.status_message = format!("Exported to {}", path.display());
                }
            }
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
            let url_text = if !url_to_capture.is_empty() {
                fetch_url_text(&url_to_capture)
            } else {
                None
            };
            let screen = screenshot_path.clone();
            std::thread::spawn(move || {
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
