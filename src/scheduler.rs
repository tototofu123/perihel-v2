use notify_rust::Notification;
use rusqlite::Connection;
use std::sync::{Arc, Mutex};
use std::sync::mpsc;

use crate::AppCommand;

pub struct Scheduler {
    conn: Arc<Mutex<Connection>>,
    cmd_tx: mpsc::Sender<AppCommand>,
}

impl Scheduler {
    pub fn new(conn: Arc<Mutex<Connection>>, cmd_tx: mpsc::Sender<AppCommand>) -> Self {
        Self { conn, cmd_tx }
    }

    fn is_quiet_hours(db: &Connection) -> bool {
        let start = crate::db::get_setting(db, "quiet_start").ok().flatten().unwrap_or("23:00".into());
        let end = crate::db::get_setting(db, "quiet_end").ok().flatten().unwrap_or("08:00".into());
        let now = chrono::Local::now();
        let now_min = now.format("%H:%M").to_string();

        if start < end {
            now_min >= start && now_min < end
        } else {
            now_min >= start || now_min < end
        }
    }

    fn get_agent_text(cap: &crate::db::Capture) -> Option<String> {
        if let Some(ref agent) = cap.agent_used {
            if !agent.is_empty() {
                return Some(agent.clone());
            }
        }
        None
    }

    pub fn start(&self) {
        let conn = self.conn.clone();
        let cmd_tx = self.cmd_tx.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
                if let Ok(db) = conn.lock() {
                    if Self::is_quiet_hours(&db) {
                        continue;
                    }

                    if let Ok(captures) = crate::db::get_unscheduled_pending(&db) {
                        for cap in captures {
                            let now = chrono::Local::now();
                            let offset_hours = (rand::random::<u64>() % 4 + 1) as i64;
                            let scheduled = now + chrono::Duration::hours(offset_hours);
                            crate::db::schedule_capture(&db, cap.id, &scheduled.to_rfc3339()).ok();
                        }
                    }

                    if let Ok(captures) = crate::db::get_pending_captures(&db) {
                        let now = chrono::Local::now();
                        for cap in captures {
                            if let Some(ref scheduled_at) = cap.scheduled_at {
                                if let Ok(scheduled_time) = chrono::DateTime::parse_from_rfc3339(scheduled_at) {
                                    let scheduled_local = scheduled_time.with_timezone(&chrono::Local);
                                    if scheduled_local <= now {
                                        let preview = if cap.content.len() > 120 {
                                            format!("{}...", &cap.content[..120])
                                        } else {
                                            cap.content.clone()
                                        };

                                        let agent_info = Self::get_agent_text(&cap)
                                            .map(|a| format!(" (via {})", a))
                                            .unwrap_or_default();

                                        Notification::new()
                                            .summary(&format!("Perihel{}", agent_info))
                                            .body(&preview)
                                            .icon("perihel")
                                            .timeout(0)
                                            .show()
                                            .ok();
                                        cmd_tx.send(AppCommand::ShowWindow).ok();

                                        // Exponential backoff: reschedule further out
                                        let backoff_hours = (rand::random::<u64>() % 4 + 4) as i64;
                                        let next = chrono::Local::now() + chrono::Duration::hours(backoff_hours);
                                        crate::db::schedule_capture(&db, cap.id, &next.to_rfc3339()).ok();
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });
    }
}