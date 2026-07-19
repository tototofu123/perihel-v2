use rusqlite::Connection;
use std::sync::{Arc, Mutex};

pub struct Scheduler {
    conn: Arc<Mutex<Connection>>,
}

impl Scheduler {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn start(&self) {
        let conn = self.conn.clone();
        std::thread::spawn(move || {
            loop {
                if let Ok(db) = conn.lock() {
                    if let Ok(captures) = super::db::get_unscheduled_pending(&db) {
                        for cap in captures {
                            let now = chrono::Local::now();
                            let offset_hours = rand::random::<u64>() % 4 + 1;
                            let scheduled = now + chrono::Duration::hours(offset_hours as i64);
                            super::db::schedule_capture(&db, cap.id, &scheduled.to_rfc3339()).ok();
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_secs(300));
            }
        });
    }
}
