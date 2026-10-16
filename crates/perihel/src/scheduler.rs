use notify_rust::Notification;
use rusqlite::Connection;
use std::sync::{Arc, Mutex};
use std::sync::mpsc;

use crate::AppCommand;

pub struct Scheduler {
    conn: Arc<Mutex<Connection>>,
    cmd_tx: mpsc::Sender<AppCommand>,
    tts_enabled: bool,
    tts_voice: String,
}

impl Scheduler {
    pub fn new(conn: Arc<Mutex<Connection>>, cmd_tx: mpsc::Sender<AppCommand>, tts_enabled: bool, tts_voice: String) -> Self {
        Self { conn, cmd_tx, tts_enabled, tts_voice }
    }

    fn is_quiet_hours(db: &Connection) -> bool {
        let start_str = perihel_core::db::get_setting(db, "quiet_start").ok().flatten().unwrap_or("23:00".into());
        let end_str = perihel_core::db::get_setting(db, "quiet_end").ok().flatten().unwrap_or("08:00".into());
        let start = chrono::NaiveTime::parse_from_str(&start_str, "%H:%M").unwrap_or(chrono::NaiveTime::from_hms_opt(23, 0, 0).unwrap());
        let end = chrono::NaiveTime::parse_from_str(&end_str, "%H:%M").unwrap_or(chrono::NaiveTime::from_hms_opt(8, 0, 0).unwrap());
        let now = chrono::Local::now().time();

        if start == end {
            false
        } else if start < end {
            now >= start && now < end
        } else {
            now >= start || now < end
        }
    }

    pub fn start(&self) {
        let conn = self.conn.clone();
        let cmd_tx = self.cmd_tx.clone();
        let tts_enabled = self.tts_enabled;
        let tts_voice = self.tts_voice.clone();
        std::thread::spawn(move || {
            let mut tick: u32 = 0;
            loop {
                std::thread::sleep(std::time::Duration::from_secs(20));
                tick = tick.wrapping_add(1);
                if let Ok(db) = conn.lock() {
                    if Self::is_quiet_hours(&db) {
                        continue;
                    }

                    // --- reminders due (every 20 s) ---
                    if let Ok(due) = perihel_core::db::due_reminders(&db) {
                        for r in due {
                            let body = format!("[Reminder] {}", r.text);
                            Notification::new()
                                .summary("Perihel")
                                .body(&body)
                                .timeout(0)
                                .show()
                                .ok();
                            if tts_enabled {
                                crate::tts::speak(&format!("Reminder: {}", r.text), &tts_voice).ok();
                            }
                            perihel_core::db::fire_reminder(&db, r.id).ok();
                            cmd_tx.send(AppCommand::ShowWindow).ok();
                        }
                    }

                    // --- capture resurfacing (every 60 s = every 3rd tick) ---
                    if tick % 3 == 0 {
                        if let Ok(captures) = perihel_core::db::get_unscheduled_pending(&db) {
                            for cap in captures {
                                let now = chrono::Local::now();
                                let offset_hours = rand::random::<i64>().abs() % 4 + 1;
                                let scheduled = now + chrono::Duration::hours(offset_hours);
                                perihel_core::db::schedule_capture(&db, cap.id, &scheduled.to_rfc3339()).ok();
                            }
                        }

                        if let Ok(captures) = perihel_core::db::get_pending_captures(&db) {
                            let now = chrono::Local::now();
                            for cap in captures {
                                if let Some(ref scheduled_at) = cap.scheduled_at {
                                    if let Ok(scheduled_time) = chrono::DateTime::parse_from_rfc3339(scheduled_at) {
                                        let scheduled_local = scheduled_time.with_timezone(&chrono::Local);
                                        if scheduled_local <= now {
                                            let preview = if cap.content.chars().count() > 120 {
                                                format!("{}...", cap.content.chars().take(120).collect::<String>())
                                            } else {
                                                cap.content.clone()
                                            };

                                            Notification::new()
                                                .summary("Perihel")
                                                .body(&preview)
                                                .timeout(0)
                                                .show()
                                                .ok();
                                            cmd_tx.send(AppCommand::ShowWindow).ok();

                                            // Clear scheduled_at so it won't trigger again
                                            perihel_core::db::schedule_capture(&db, cap.id, "").ok();
                                        }
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
