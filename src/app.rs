use eframe::egui;
use eframe::egui::CornerRadius;
use rusqlite::Connection;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

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

fn pipe_through_agent(bin_path: &str, content: &str) -> Option<String> {
    let output = std::process::Command::new(bin_path)
        .arg("--prompt")
        .arg(format!("Summarize this thought concisely: {}", content))
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
    let ps_script = format!(
        "Add-Type -AssemblyName System.Speech; \
         $rec = New-Object System.Speech.Recognition.SpeechRecognitionEngine; \
         $rec.SetInputToDefaultAudioDevice(); \
         Write-Host 'Listening...'; \
         $result = $rec.Recognize(); \
         if ($result -ne $null) {{ $result.Text }} else {{ '' }}"
    );
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &ps_script])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let text = stdout.trim().to_string();
    if !text.is_empty() {
        std::fs::write(&path, text.as_bytes()).ok()?;
        Some(path.to_string_lossy().to_string())
    } else if output.status.success() {
        std::fs::write(&path, b"[voice recording]").ok()?;
        Some(path.to_string_lossy().to_string())
    } else {
        None
    }
}

fn speak_text(text: &str) {
    let _ = std::process::Command::new("vox")
        .arg(text)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
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
    pub url_input: String,
    pub view: View,
    pub status_message: String,
    pub quiet_start: String,
    pub quiet_end: String,
    pub pending_count: i64,
    pub last_screenshot_path: Option<String>,
    pub screenshot_attached: bool,
    pub selected_agent_idx: usize,
    pub dashboard_filter: String,
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
            url_input: String::new(),
            view: View::Capture,
            status_message: String::new(),
            quiet_start,
            quiet_end,
            pending_count,
            last_screenshot_path: None,
            screenshot_attached: false,
            selected_agent_idx: 0,
            dashboard_filter: "all".into(),
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
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Thought").size(13.0).color(MUTED));
        ui.add_sized([ui.available_width(), 100.0], egui::TextEdit::multiline(&mut self.text_input)
            .hint_text("What's on your mind?")
            .desired_rows(5).frame(true));

        ui.add_space(6.0);
        ui.label(egui::RichText::new("URL (optional)").size(13.0).color(MUTED));
        ui.add_sized([ui.available_width(), 28.0], egui::TextEdit::singleline(&mut self.url_input)
            .hint_text("https://...").frame(true));

        ui.add_space(10.0);
        egui::Frame { fill: CARD, corner_radius: CornerRadius::same(8), inner_margin: egui::Margin::symmetric(10, 8), ..Default::default() }
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let cam = if self.screenshot_attached { "[CAM]" } else { "Screenshot" };
                    if ui.button(cam).clicked() {
                        let data_dir = db::get_data_dir();
                        if let Some(path) = take_screenshot(&data_dir) {
                            self.last_screenshot_path = Some(path);
                            self.screenshot_attached = true;
                            self.status_message = "Screenshot attached".into();
                        } else {
                            self.status_message = "Screenshot failed".into();
                        }
                    }
                    if !self.text_input.trim().is_empty() && ui.button("Speak").clicked() {
                        speak_text(self.text_input.trim());
                    }
                    if ui.button("Voice").clicked() {
                        let data_dir = db::get_data_dir();
                        self.status_message = "Recording...".into();
                        let tx = self.cmd_tx.clone();
                        std::thread::spawn(move || {
                            let text = record_voice(&data_dir);
                            let msg = match text {
                                Some(p) => format!("Voice saved: {}", p),
                                None => "Voice recording failed.".into(),
                            };
                            let _ = tx.send(AppCommand::StatusMessage(msg));
                        });
                    }
                });
            });

        let detected: Vec<&agents::AgentInfo> = self.agents.iter().filter(|a| a.detected).collect();
        if !detected.is_empty() {
            ui.add_space(8.0);
            egui::Frame { fill: CARD, corner_radius: CornerRadius::same(8), inner_margin: egui::Margin::symmetric(10, 8), ..Default::default() }
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("AI Agent:").size(13.0).color(MUTED));
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
                });
        }

        ui.add_space(12.0);
        if ui.add_sized([ui.available_width(), 40.0], egui::Button::new(
            egui::RichText::new("Capture").size(15.0).color(egui::Color32::WHITE)
        ).fill(ACCENT).corner_radius(CornerRadius::same(8))).clicked() {
            self.do_capture();
        }

        if !self.status_message.is_empty() {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(&self.status_message).size(12.0).color(DIM));
        }
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
                    ui.label(egui::RichText::new("Press Ctrl+Shift+P to capture").size(12.0).color(DIM));
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

                egui::Frame { fill: CARD, corner_radius: CornerRadius::same(8), stroke: egui::Stroke::new(1.0_f32, egui::Color32::from_rgba_premultiplied(255, 255, 255, 6)), inner_margin: egui::Margin::symmetric(10, 8), ..Default::default() }
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(type_label).size(11.0).color(DIM));
                            ui.label(egui::RichText::new(status_label).size(11.0).color(DIM));
                            ui.add(egui::Label::new(egui::RichText::new(&preview).size(12.0)).sense(egui::Sense::click()));
                            if ui.add(egui::Button::new(egui::RichText::new("SPK").size(11.0).color(ACCENT)).fill(SURFACE).corner_radius(CornerRadius::same(4))).clicked() {
                                speak_text(&capture.content);
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
        ui.label(egui::RichText::new("Agents").size(14.0).color(ACCENT).strong());
        ui.add_space(4.0);
        for agent in &self.agents {
            if agent.detected {
                ui.label(egui::RichText::new(format!("[OK] {} {}", agent.name, agent.version.as_deref().unwrap_or(""))).size(12.0).color(TEXT));
            } else {
                ui.label(egui::RichText::new(format!("[--] {} (not found)", agent.name)).size(12.0).color(DIM));
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

    fn do_capture(&mut self) {
        let mut captured = false;
        let (agent_bin, agent_name) = {
            let detected: Vec<&agents::AgentInfo> = self.agents.iter().filter(|a| a.detected).collect();
            if self.selected_agent_idx > 0 && self.selected_agent_idx <= detected.len() {
                let agent = detected[self.selected_agent_idx - 1];
                (agent.path.clone(), Some(agent.name.clone()))
            } else { (None, None) }
        };

        if !self.text_input.is_empty() {
            let content = self.text_input.trim().to_string();
            let id = if let Ok(conn) = self.conn.lock() { db::insert_capture(&conn, &content, "text").ok() } else { None };
            if let (Some(bin), Some(name), Some(cap_id)) = (agent_bin.clone(), agent_name.clone(), id) {
                let conn = self.conn.clone();
                std::thread::spawn(move || {
                    if let Some(summary) = pipe_through_agent(&bin, &content) {
                        if let Ok(db) = conn.lock() { db::update_ai_summary(&db, cap_id, &summary, &name).ok(); }
                    }
                });
            }
            self.text_input.clear();
            captured = true;
        }

        if !self.url_input.is_empty() {
            let content = self.url_input.trim().to_string();
            let id = if let Ok(conn) = self.conn.lock() { db::insert_capture(&conn, &content, "url").ok() } else { None };
            if let (Some(bin), Some(name), Some(cap_id)) = (agent_bin.clone(), agent_name.clone(), id) {
                let conn = self.conn.clone();
                std::thread::spawn(move || {
                    if let Some(summary) = pipe_through_agent(&bin, &content) {
                        if let Ok(db) = conn.lock() { db::update_ai_summary(&db, cap_id, &summary, &name).ok(); }
                    }
                });
            }
            self.url_input.clear();
            captured = true;
        }

        if self.screenshot_attached {
            if let Some(ref path) = self.last_screenshot_path {
                if let Ok(conn) = self.conn.lock() { db::insert_capture(&conn, path, "screenshot").ok(); }
            }
            self.screenshot_attached = false;
            self.last_screenshot_path = None;
            captured = true;
        }

        if captured {
            self.status_message = "Captured!".into();
            self.refresh_captures();
        } else {
            self.status_message = "Type something or attach a screenshot.".into();
        }
    }

    pub fn refresh_captures(&mut self) {
        if let Ok(conn) = self.conn.lock() {
            self.captures = db::get_all_captures(&conn).unwrap_or_default();
            self.pending_count = db::get_pending_count(&conn).unwrap_or(0);
        }
    }
}
