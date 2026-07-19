use eframe::egui;
use rusqlite::Connection;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use crate::agents;
use crate::db::{self, Capture};

use super::AppCommand;

pub struct PerihelApp {
    pub conn: Arc<Mutex<Connection>>,
    pub cmd_rx: mpsc::Receiver<AppCommand>,
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
}

impl PerihelApp {
    pub fn new(conn: Arc<Mutex<Connection>>, cmd_rx: mpsc::Receiver<AppCommand>) -> Self {
        let agents = agents::detect_agents();
        if let Ok(conn_lock) = conn.lock() {
            db::save_detected_agents(&conn_lock, &agents.iter().map(|a| db::DetectedAgent {
                name: a.name.clone(),
                version: a.version.clone(),
                path: a.path.clone(),
                detected_at: chrono::Local::now().to_rfc3339(),
            }).collect::<Vec<_>>()).ok();
        }

        let captures = db::get_pending_captures(&conn.lock().unwrap()).unwrap_or_default();
        let pending_count = db::get_pending_count(&conn.lock().unwrap()).unwrap_or(0);
        let quiet_start = db::get_setting(&conn.lock().unwrap(), "quiet_start").unwrap_or(Some("23:00".into())).unwrap_or("23:00".into());
        let quiet_end = db::get_setting(&conn.lock().unwrap(), "quiet_end").unwrap_or(Some("08:00".into())).unwrap_or("08:00".into());

        Self {
            conn,
            cmd_rx,
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
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if self.captures.is_empty() {
                        ui.label("No captures yet. Press Ctrl+Shift+P to capture something.");
                        return;
                    }
                    for capture in self.captures.clone() {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                let type_icon = match capture.content_type.as_str() {
                                    "url" => "🔗",
                                    "screenshot" => "📷",
                                    "voice" => "🎤",
                                    _ => "📝",
                                };
                                ui.label(type_icon);
                                let preview = if capture.content.len() > 60 {
                                    format!("{}...", &capture.content[..60])
                                } else {
                                    capture.content.clone()
                                };
                                ui.label(&preview);
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
                            });
                            if let Some(summary) = &capture.ai_summary {
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
            });
    }

    fn do_capture(&mut self) {
        let mut captured = false;

        if !self.text_input.is_empty() {
            if let Ok(conn) = self.conn.lock() {
                db::insert_capture(&conn, self.text_input.trim(), "text").ok();
            }
            self.text_input.clear();
            captured = true;
        }

        if !self.url_input.is_empty() {
            if let Ok(conn) = self.conn.lock() {
                db::insert_capture(&conn, self.url_input.trim(), "url").ok();
            }
            self.url_input.clear();
            captured = true;
        }

        if captured {
            self.status_message = "Captured!".into();
            self.refresh_captures();
        } else {
            self.status_message = "Type something first.".into();
        }
    }

    pub fn refresh_captures(&mut self) {
        if let Ok(conn) = self.conn.lock() {
            self.captures = db::get_pending_captures(&conn).unwrap_or_default();
            self.pending_count = db::get_pending_count(&conn).unwrap_or(0);
        }
    }
}
