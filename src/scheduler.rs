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

    pub fn start(&self) {
        let conn = self.conn.clone();
        let cmd_tx = self.cmd_tx.clone();
        std::thread::spawn(move || {
            loop {
                if let Ok(db) = conn.lock() {
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
                                        Notification::new()
                                            .summary("Perihel")
                                            .body(&preview)
                                            .icon("perihel")
                                            .timeout(0)
                                            .show()
                                            .ok();
                                        cmd_tx.send(AppCommand::ShowWindow).ok();
                                    }
                                }
                            }
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_secs(60));
            }
        });
    }
}
