use eframe::egui;
use rusqlite::Connection;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use crate::agents;
use crate::db::{self, Capture};

use super::AppCommand;

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
    let mut child = std::process::Command::new(bin_path)
        .arg("--prompt")
        .arg(format!("Summarize this thought concisely in 1-2 sentences: {}", content))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    let _ = child.wait().ok()?;
    let output = child.stdout.take()?;
    let mut buf = String::new();
    use std::io::Read;
    std::io::BufReader::new(output).read_to_string(&mut buf).ok()?;
    let trimmed = buf.trim().to_string();
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

        let captures = db::get_all_captures(&conn.lock().unwrap()).unwrap_or_default();
        let pending_count = db::get_pending_count(&conn.lock().unwrap()).unwrap_or(0);
        let quiet_start = db::get_setting(&conn.lock().unwrap(), "quiet_start").unwrap_or(Some("23:00".into())).unwrap_or("23:00".into());
        let quiet_end = db::get_setting(&conn.lock().unwrap(), "quiet_end").unwrap_or(Some("08:00".into())).unwrap_or("08:00".into());

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
        egui::Window::new("Capture")
            .id(egui::Id::new("capture_popup"))
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("What do you want to remember?");
                ui.add_sized(
                    [300.0, 60.0],
                    egui::TextEdit::multiline(&mut self.text_input)
                        .hint_text("Type your thought..."),
                );

                ui.add_space(4.0);
                ui.label("URL (optional)");
                ui.add_sized(
                    [300.0, 20.0],
                    egui::TextEdit::singleline(&mut self.url_input)
                        .hint_text("https://..."),
                );

                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    if ui.button("📷 Screenshot").clicked() {
                        let data_dir = db::get_data_dir();
                        if let Some(path) = take_screenshot(&data_dir) {
                            self.last_screenshot_path = Some(path);
                            self.screenshot_attached = true;
                            self.status_message = "Screenshot attached!".into();
                        } else {
                            self.status_message = "Screenshot failed.".into();
                        }
                    }
                    if self.screenshot_attached {
                        ui.label("✅");
                    }
                    if ui.button("🎤 Voice").clicked() {
                        let data_dir = db::get_data_dir();
                        self.status_message = "Recording...".into();
                        // Record in a thread so we don't block the UI
                        let tx = self.cmd_tx.clone();
                        std::thread::spawn(move || {
                            let text = record_voice(&data_dir);
                            let msg = match text {
                                Some(path) => format!("Voice saved: {}", path),
                                None => "Voice recording failed.".into(),
                            };
                            let _ = tx.send(AppCommand::StatusMessage(msg));
                        });
                    }
                });

                ui.add_space(4.0);

                let detected_agents: Vec<&agents::AgentInfo> = self.agents.iter().filter(|a| a.detected).collect();
                if !detected_agents.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label("Process with:");
                        let agent_names: Vec<&str> = detected_agents.iter().map(|a| a.name.as_str()).collect();
                        egui::ComboBox::from_id_salt("agent_selector")
                            .selected_text(agent_names.get(self.selected_agent_idx).copied().unwrap_or("None"))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.selected_agent_idx, 0, "None");
                                for (i, name) in agent_names.iter().enumerate() {
                                    ui.selectable_value(&mut self.selected_agent_idx, i + 1, *name);
                                }
                            });
                    });
                    ui.add_space(4.0);
                }

                if ui.button("Capture").clicked() {
                    self.do_capture();
                }

                if !self.status_message.is_empty() {
                    ui.label(&self.status_message);
                }
            });
    }

    pub fn show_dashboard(&mut self, ctx: &egui::Context) {
        egui::Window::new("Dashboard")
            .id(egui::Id::new("dashboard"))
            .default_size([500.0, 400.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.dashboard_filter, "all".into(), "All");
                    ui.selectable_value(&mut self.dashboard_filter, "pending".into(), "Pending");
                    ui.selectable_value(&mut self.dashboard_filter, "done".into(), "Done");
                    ui.selectable_value(&mut self.dashboard_filter, "dismissed".into(), "Dismissed");
                });
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let filtered: Vec<Capture> = if self.dashboard_filter == "all" {
                        self.captures.clone()
                    } else {
                        self.captures.iter().filter(|c| c.status == self.dashboard_filter).cloned().collect()
                    };

                    if filtered.is_empty() {
                        ui.label("No captures yet. Press Ctrl+Shift+P to capture something.");
                        return;
                    }

                    ui.label(format!("{} capture(s)", filtered.len()));
                    ui.add_space(4.0);

                    for capture in &filtered {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                let type_icon = match capture.content_type.as_str() {
                                    "url" => "🔗",
                                    "screenshot" => "📷",
                                    "voice" => "🎤",
                                    _ => "📝",
                                };
                                ui.label(type_icon);
                                let status_label = match capture.status.as_str() {
                                    "done" => "✓",
                                    "dismissed" => "✗",
                                    _ => "○",
                                };
                                ui.label(status_label);
                                let preview = if capture.content.len() > 60 {
                                    format!("{}...", &capture.content[..60])
                                } else {
                                    capture.content.clone()
                                };
                                ui.label(&preview);
                                if capture.status == "pending" {
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if ui.button("Done").clicked() {
                                            if let Ok(conn) = self.conn.lock() {
                                                db::update_status(&conn, capture.id, "done").ok();
                                            }
                                            self.refresh_captures();
                                        }
                                        if ui.button("Dismiss").clicked() {
                                            if let Ok(conn) = self.conn.lock() {
                                                db::update_status(&conn, capture.id, "dismissed").ok();
                                            }
                                            self.refresh_captures();
                                        }
                                    });
                                }
                            });
                            if let Some(ref agent) = capture.agent_used {
                                ui.label(format!("via {}", agent));
                            }
                            if let Some(ref summary) = capture.ai_summary {
                                ui.label(format!("AI: {}", summary));
                            }
                        });
                        ui.add_space(2.0);
                    }
                });
            });
    }

    pub fn show_settings(&mut self, ctx: &egui::Context) {
        egui::Window::new("Settings")
            .id(egui::Id::new("settings"))
            .default_size([350.0, 300.0])
            .show(ctx, |ui| {
                ui.heading("Detected Agents");
                for agent in &self.agents {
                    if agent.detected {
                        ui.label(format!("✓ {} {}", agent.name, agent.version.as_deref().unwrap_or("")));
                    } else {
                        ui.label(format!("✗ {} (not found)", agent.name));
                    }
                }

                ui.add_space(10.0);
                ui.heading("Quiet Hours");
                ui.horizontal(|ui| {
                    ui.label("Start:");
                    if ui.text_edit_singleline(&mut self.quiet_start).lost_focus() {
                        if let Ok(conn) = self.conn.lock() {
                            db::set_setting(&conn, "quiet_start", &self.quiet_start).ok();
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("End:");
                    if ui.text_edit_singleline(&mut self.quiet_end).lost_focus() {
                        if let Ok(conn) = self.conn.lock() {
                            db::set_setting(&conn, "quiet_end", &self.quiet_end).ok();
                        }
                    }
                });

                ui.add_space(10.0);
                ui.heading("Export");
                if ui.button("Export captures as JSON").clicked() {
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
        let mut last_id = None;
        let captured_text = self.text_input.clone();

        let (agent_bin, agent_name) = {
            let detected: Vec<&agents::AgentInfo> = self.agents.iter().filter(|a| a.detected).collect();
            if self.selected_agent_idx > 0 && self.selected_agent_idx <= detected.len() {
                let agent = detected[self.selected_agent_idx - 1];
                (agent.path.clone(), Some(agent.name.clone()))
            } else {
                (None, None)
            }
        };

        if !self.text_input.is_empty() {
            if let Ok(conn) = self.conn.lock() {
                last_id = db::insert_capture(&conn, self.text_input.trim(), "text").ok();
            }
            self.text_input.clear();
            captured = true;
        }

        if !self.url_input.is_empty() {
            if let Ok(conn) = self.conn.lock() {
                last_id = db::insert_capture(&conn, self.url_input.trim(), "url").ok();
            }
            self.url_input.clear();
            captured = true;
        }

        if self.screenshot_attached {
            if let Some(ref path) = self.last_screenshot_path {
                if let Ok(conn) = self.conn.lock() {
                    last_id = db::insert_capture(&conn, path, "screenshot").ok();
                }
            }
            self.screenshot_attached = false;
            self.last_screenshot_path = None;
            captured = true;
        }

        if captured {
            self.status_message = "Captured!".into();
            self.refresh_captures();

            if let (Some(bin), Some(name), Some(id)) = (agent_bin, agent_name, last_id) {
                let conn = self.conn.clone();
                std::thread::spawn(move || {
                    let summary = pipe_through_agent(&bin, &captured_text);
                    if let Some(summary) = summary {
                        if let Ok(db) = conn.lock() {
                            db::update_ai_summary(&db, id, &summary, &name).ok();
                        }
                    }
                });
            }
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
