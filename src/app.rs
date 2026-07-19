use eframe::egui;
use eframe::egui::CornerRadius;
use rusqlite::Connection;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use crate::agents;
use crate::db::{self, Capture};

use super::AppCommand;

const ACCENT: egui::Color32 = egui::Color32::from_rgb(0, 200, 220);
const DONE_GREEN: egui::Color32 = egui::Color32::from_rgb(74, 222, 128);

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

pub fn apply_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let cr6 = CornerRadius::same(6);
    style.visuals = egui::Visuals {
        dark_mode: true,
        override_text_color: Some(egui::Color32::from_gray(210)),
        window_corner_radius: CornerRadius::same(10),
        window_shadow: egui::Shadow {
            offset: [0, 4].into(),
            blur: 16,
            spread: 0,
            color: egui::Color32::from_black_alpha(60),
        },
        window_fill: egui::Color32::from_rgb(20, 20, 30),
        window_stroke: egui::Stroke::new(1.0_f32, egui::Color32::from_rgba_premultiplied(255, 255, 255, 10)),
        panel_fill: egui::Color32::from_rgb(16, 16, 24),
        faint_bg_color: egui::Color32::from_rgb(26, 26, 38),
        extreme_bg_color: egui::Color32::from_rgb(12, 12, 18),
        code_bg_color: egui::Color32::from_rgb(26, 26, 38),
        hyperlink_color: ACCENT,
        selection: egui::style::Selection {
            bg_fill: ACCENT.linear_multiply(0.3),
            stroke: egui::Stroke::new(1.0_f32, ACCENT),
        },
        widgets: egui::style::Widgets {
            noninteractive: egui::style::WidgetVisuals {
                bg_fill: egui::Color32::from_rgb(26, 26, 38),
                weak_bg_fill: egui::Color32::from_rgb(20, 20, 32),
                bg_stroke: egui::Stroke::new(1.0_f32, egui::Color32::from_rgba_premultiplied(255, 255, 255, 6)),
                corner_radius: cr6,
                fg_stroke: egui::Stroke::new(1.0_f32, egui::Color32::from_gray(160)),
                expansion: 0.0,
            },
            inactive: egui::style::WidgetVisuals {
                bg_fill: egui::Color32::from_rgb(30, 30, 44),
                weak_bg_fill: egui::Color32::from_rgb(26, 26, 38),
                bg_stroke: egui::Stroke::new(1.0_f32, egui::Color32::from_rgba_premultiplied(255, 255, 255, 12)),
                corner_radius: cr6,
                fg_stroke: egui::Stroke::new(1.0_f32, egui::Color32::from_gray(180)),
                expansion: 0.0,
            },
            hovered: egui::style::WidgetVisuals {
                bg_fill: egui::Color32::from_rgb(38, 38, 56),
                weak_bg_fill: egui::Color32::from_rgb(34, 34, 50),
                bg_stroke: egui::Stroke::new(1.0_f32, ACCENT.linear_multiply(0.5)),
                corner_radius: cr6,
                fg_stroke: egui::Stroke::new(1.5_f32, egui::Color32::from_gray(220)),
                expansion: 1.0,
            },
            active: egui::style::WidgetVisuals {
                bg_fill: ACCENT.linear_multiply(0.2),
                weak_bg_fill: ACCENT.linear_multiply(0.15),
                bg_stroke: egui::Stroke::new(1.0_f32, ACCENT),
                corner_radius: cr6,
                fg_stroke: egui::Stroke::new(2.0_f32, egui::Color32::from_gray(240)),
                expansion: 1.0,
            },
            open: egui::style::WidgetVisuals {
                bg_fill: egui::Color32::from_rgb(34, 34, 50),
                weak_bg_fill: egui::Color32::from_rgb(30, 30, 44),
                bg_stroke: egui::Stroke::new(1.0_f32, ACCENT.linear_multiply(0.3)),
                corner_radius: cr6,
                fg_stroke: egui::Stroke::new(1.5_f32, egui::Color32::from_gray(200)),
                expansion: 0.0,
            },
        },
        ..Default::default()
    };
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(16.0, 6.0);
    style.spacing.indent = 16.0;
    ctx.set_style(style);
}

pub struct PerihelApp {
    pub conn: Arc<Mutex<Connection>>,
    pub cmd_rx: mpsc::Receiver<AppCommand>,
    pub cmd_tx: mpsc::Sender<AppCommand>,
    pub captures: Vec<Capture>,
    pub agents: Vec<agents::AgentInfo>,
    pub text_input: String,
    pub url_input: String,
    pub show_capture: bool,
    pub dashboard_open: bool,
    pub settings_open: bool,
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
            cmd_rx: cmd_rx,
            cmd_tx,
            captures,
            agents,
            text_input: String::new(),
            url_input: String::new(),
            show_capture: false,
            dashboard_open: false,
            settings_open: false,
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

    pub fn show_capture_popup(&mut self, ctx: &egui::Context) {
        egui::Window::new("")
            .id(egui::Id::new("capture_popup"))
            .resizable(false)
            .collapsible(false)
            .title_bar(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .fixed_size([340.0, 310.0])
            .frame(egui::Frame {
                fill: ctx.style().visuals.window_fill(),
                corner_radius: CornerRadius::same(12),
                shadow: ctx.style().visuals.window_shadow,
                stroke: ctx.style().visuals.window_stroke,
                inner_margin: egui::Margin::symmetric(20, 16),
                ..Default::default()
            })
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("New Capture").size(18.0).color(ACCENT).strong());
                });
                ui.add_space(12.0);

                ui.label(egui::RichText::new("Thought").size(12.0).color(egui::Color32::from_gray(140)));
                ui.add_sized([300.0, 80.0], egui::TextEdit::multiline(&mut self.text_input)
                    .hint_text("What's on your mind?")
                    .desired_rows(4).frame(true));

                ui.add_space(6.0);
                ui.label(egui::RichText::new("URL (optional)").size(12.0).color(egui::Color32::from_gray(140)));
                ui.add_sized([300.0, 24.0], egui::TextEdit::singleline(&mut self.url_input)
                    .hint_text("https://...").frame(true));

                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    let cam = if self.screenshot_attached { "Screenshot \u{2713}" } else { "Screenshot" };
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

                let detected_agents: Vec<&agents::AgentInfo> = self.agents.iter().filter(|a| a.detected).collect();
                if !detected_agents.is_empty() {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Agent:").size(12.0).color(egui::Color32::from_gray(140)));
                        let agent_names: Vec<&str> = detected_agents.iter().map(|a| a.name.as_str()).collect();
                        egui::ComboBox::from_id_salt("agent_selector")
                            .selected_text(agent_names.get(self.selected_agent_idx).copied().unwrap_or("None"))
                            .width(180.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.selected_agent_idx, 0, "None");
                                for (i, name) in agent_names.iter().enumerate() {
                                    ui.selectable_value(&mut self.selected_agent_idx, i + 1, *name);
                                }
                            });
                    });
                    ui.add_space(6.0);
                }

                ui.add_space(6.0);
                if ui.add_sized([300.0, 36.0], egui::Button::new(
                    egui::RichText::new("Capture").size(14.0).color(egui::Color32::from_gray(240))
                ).fill(ACCENT).corner_radius(CornerRadius::same(8))).clicked() {
                    self.do_capture();
                }

                if !self.status_message.is_empty() {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(&self.status_message).size(11.0).color(egui::Color32::from_gray(150)));
                }
            });
    }

    pub fn show_dashboard(&mut self, ctx: &egui::Context) {
        egui::Window::new("Dashboard")
            .id(egui::Id::new("dashboard"))
            .default_size([520.0, 440.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (key, label) in &[("all", "All"), ("pending", "Pending"), ("done", "Done"), ("dismissed", "Dismissed")] {
                        let selected = self.dashboard_filter == *key;
                        let mut btn = egui::Button::new(egui::RichText::new(*label).size(12.0))
                            .corner_radius(CornerRadius::same(6));
                        if selected {
                            btn = btn.fill(ACCENT.linear_multiply(0.25));
                            btn = btn.stroke(egui::Stroke::new(1.0_f32, ACCENT));
                        }
                        if ui.add(btn).clicked() {
                            self.dashboard_filter = key.to_string();
                        }
                    }
                });

                let filtered: Vec<Capture> = if self.dashboard_filter == "all" {
                    self.captures.clone()
                } else {
                    self.captures.iter().filter(|c| c.status == self.dashboard_filter).cloned().collect()
                };

                ui.add_space(4.0);
                ui.label(egui::RichText::new(format!("{} capture{}", filtered.len(),
                    if filtered.len() == 1 { "" } else { "s" })).size(11.0).color(egui::Color32::from_gray(130)));
                ui.separator();

                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if filtered.is_empty() {
                        ui.add_space(40.0);
                        ui.vertical_centered(|ui| {
                            ui.label(egui::RichText::new("No captures yet").size(14.0).color(egui::Color32::from_gray(120)));
                            ui.label(egui::RichText::new("Press Ctrl+Shift+P to capture").size(12.0).color(egui::Color32::from_gray(100)));
                        });
                        return;
                    }

                    for capture in &filtered {
                        let (status_color, status_icon) = match capture.status.as_str() {
                            "done" => (DONE_GREEN, "\u{2713}"),
                            "dismissed" => (egui::Color32::from_gray(100), "\u{2717}"),
                            _ => (ACCENT, "\u{25CB}"),
                        };
                        let type_icon = match capture.content_type.as_str() {
                            "url" => "\u{1F517}",
                            "screenshot" => "\u{1F4F7}",
                            "voice" => "\u{1F3A4}",
                            _ => "\u{1F4DD}",
                        };
                        let preview: String = capture.content.chars().take(60).collect();
                        let preview = if capture.content.chars().count() > 60 {
                            format!("{}...", preview)
                        } else {
                            preview
                        };

                        egui::Frame {
                            fill: ctx.style().visuals.faint_bg_color,
                            corner_radius: CornerRadius::same(8),
                            stroke: egui::Stroke::new(1.0_f32, egui::Color32::from_rgba_premultiplied(255, 255, 255, 5)),
                            inner_margin: egui::Margin::symmetric(10, 8),
                            ..Default::default()
                        }.show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(type_icon);
                                ui.label(status_icon);
                                ui.add(egui::Label::new(egui::RichText::new(&preview).size(12.0)).sense(egui::Sense::click()));
                                if capture.status == "pending" {
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if ui.button("\u{2715}").clicked() {
                                            if let Ok(conn) = self.conn.lock() {
                                                db::update_status(&conn, capture.id, "dismissed").ok();
                                            }
                                            self.refresh_captures();
                                        }
                                        if ui.add(egui::Button::new(
                                            egui::RichText::new("\u{2713}").size(12.0).color(DONE_GREEN)
                                        ).fill(DONE_GREEN.linear_multiply(0.2)).corner_radius(CornerRadius::same(4))).clicked() {
                                            if let Ok(conn) = self.conn.lock() {
                                                db::update_status(&conn, capture.id, "done").ok();
                                            }
                                            self.refresh_captures();
                                        }
                                    });
                                }
                            });
                            if let Some(ref agent) = capture.agent_used {
                                ui.add_space(2.0);
                                ui.label(egui::RichText::new(format!("via {}", agent)).size(10.0).color(egui::Color32::from_gray(130)));
                            }
                            if let Some(ref summary) = capture.ai_summary {
                                ui.add_space(2.0);
                                ui.label(egui::RichText::new(summary).size(11.0).color(egui::Color32::from_gray(150)).italics());
                            }
                        });
                        ui.add_space(4.0);
                    }
                });
            });
    }

    pub fn show_settings(&mut self, ctx: &egui::Context) {
        egui::Window::new("Settings")
            .id(egui::Id::new("settings"))
            .default_size([380.0, 360.0])
            .show(ctx, |ui| {
                ui.label(egui::RichText::new("Agents").size(14.0).color(ACCENT).strong());
                ui.add_space(4.0);
                for agent in &self.agents {
                    if agent.detected {
                        ui.label(egui::RichText::new(format!("\u{2713} {} {}", agent.name, agent.version.as_deref().unwrap_or("")))
                            .size(12.0).color(egui::Color32::from_gray(190)));
                    } else {
                        ui.label(egui::RichText::new(format!("\u{2717} {} (not found)", agent.name))
                            .size(12.0).color(egui::Color32::from_gray(110)));
                    }
                }

                ui.add_space(16.0);
                ui.label(egui::RichText::new("Quiet Hours").size(14.0).color(ACCENT).strong());
                ui.add_space(4.0);
                let muted = egui::Color32::from_gray(140);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Start:").size(12.0).color(muted));
                    ui.add(egui::TextEdit::singleline(&mut self.quiet_start).desired_width(50.0).frame(true));
                    ui.label(egui::RichText::new("End:").size(12.0).color(muted));
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
                if ui.add(egui::Button::new(egui::RichText::new("Export as JSON").size(12.0))
                    .corner_radius(CornerRadius::same(6))).clicked() {
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
            });
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
