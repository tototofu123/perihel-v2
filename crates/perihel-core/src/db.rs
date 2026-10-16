use rusqlite::{Connection, Result, params};
use serde::{Deserialize, Serialize};
use chrono::Local;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Capture {
    pub id: i64,
    pub content: String,
    pub content_type: String,
    pub status: String,
    pub created_at: String,
    pub scheduled_at: Option<String>,
    pub ai_summary: Option<String>,
    pub agent_used: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DetectedAgent {
    pub name: String,
    pub version: Option<String>,
    pub path: Option<String>,
    pub detected_at: String,
}

pub fn init_db() -> Result<Connection> {
    let data_dir = get_data_dir();
    std::fs::create_dir_all(&data_dir).ok();
    let db_path = data_dir.join("perihel.db");
    let mut conn = Connection::open(&db_path)?;
    init_schema(&mut conn)?;
    Ok(conn)
}

/// Create the schema + default settings on any connection (main path or tests).
pub fn init_schema(conn: &mut Connection) -> Result<()> {
    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS captures (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            content TEXT NOT NULL,
            content_type TEXT NOT NULL DEFAULT 'text',
            status TEXT NOT NULL DEFAULT 'pending',
            created_at TEXT NOT NULL,
            scheduled_at TEXT,
            ai_summary TEXT,
            agent_used TEXT
        );
        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS detected_agents (
            name TEXT PRIMARY KEY,
            version TEXT,
            path TEXT,
            detected_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS reminders (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            text TEXT NOT NULL,
            due_at TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            created_at TEXT NOT NULL,
            fired_at TEXT
        );
        CREATE TABLE IF NOT EXISTS tool_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            input TEXT NOT NULL,
            calls TEXT NOT NULL,
            answer TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        INSERT OR IGNORE INTO settings (key, value) VALUES ('quiet_start', '23:00');
        INSERT OR IGNORE INTO settings (key, value) VALUES ('quiet_end', '08:00');
        INSERT OR IGNORE INTO settings (key, value) VALUES ('hotkey', 'Ctrl+Shift+P');
    ")?;

    Ok(())
}

pub fn get_data_dir() -> std::path::PathBuf {
    let base = dirs::data_local_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join("perihel")
}

pub fn insert_capture(conn: &Connection, content: &str, content_type: &str) -> Result<i64> {
    let now = Local::now().to_rfc3339();
    conn.execute(
        "INSERT INTO captures (content, content_type, status, created_at) VALUES (?1, ?2, 'pending', ?3)",
        params![content, content_type, now],
    )?;
    Ok(conn.last_insert_rowid())
}

fn map_capture_rows(stmt: &mut rusqlite::Statement<'_>) -> Result<Vec<Capture>> {
    let rows = stmt.query_map([], |row| {
        Ok(Capture {
            id: row.get(0)?,
            content: row.get(1)?,
            content_type: row.get(2)?,
            status: row.get(3)?,
            created_at: row.get(4)?,
            scheduled_at: row.get(5)?,
            ai_summary: row.get(6)?,
            agent_used: row.get(7)?,
        })
    })?;
    let mut captures = Vec::new();
    for row in rows {
        captures.push(row?);
    }
    Ok(captures)
}

pub fn get_pending_captures(conn: &Connection) -> Result<Vec<Capture>> {
    let mut stmt = conn.prepare(
        "SELECT id, content, content_type, status, created_at, scheduled_at, ai_summary, agent_used
         FROM captures WHERE status = 'pending' AND scheduled_at IS NOT NULL AND scheduled_at != ''
         ORDER BY created_at DESC"
    )?;
    map_capture_rows(&mut stmt)
}

pub fn get_unscheduled_pending(conn: &Connection) -> Result<Vec<Capture>> {
    let mut stmt = conn.prepare(
        "SELECT id, content, content_type, status, created_at, scheduled_at, ai_summary, agent_used
         FROM captures WHERE status = 'pending' AND scheduled_at IS NULL"
    )?;
    map_capture_rows(&mut stmt)
}

pub fn update_status(conn: &Connection, id: i64, status: &str) -> Result<()> {
    conn.execute(
        "UPDATE captures SET status = ?1 WHERE id = ?2",
        params![status, id],
    )?;
    Ok(())
}

pub fn schedule_capture(conn: &Connection, id: i64, scheduled_at: &str) -> Result<()> {
    conn.execute(
        "UPDATE captures SET scheduled_at = ?1 WHERE id = ?2",
        params![scheduled_at, id],
    )?;
    Ok(())
}

pub fn update_ai_summary(conn: &Connection, id: i64, summary: &str, agent: &str) -> Result<()> {
    conn.execute(
        "UPDATE captures SET ai_summary = ?1, agent_used = ?2 WHERE id = ?3",
        params![summary, agent, id],
    )?;
    Ok(())
}

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
    let mut rows = stmt.query_map(params![key], |row| row.get::<_, String>(0))?;
    match rows.next() {
        Some(Ok(val)) => Ok(Some(val)),
        _ => Ok(None),
    }
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
        params![key, value],
    )?;
    Ok(())
}

pub fn save_detected_agents(conn: &Connection, agents: &[DetectedAgent]) -> Result<()> {
    conn.execute("DELETE FROM detected_agents", [])?;
    let now = Local::now().to_rfc3339();
    for agent in agents {
        conn.execute(
            "INSERT INTO detected_agents (name, version, path, detected_at) VALUES (?1, ?2, ?3, ?4)",
            params![agent.name, agent.version, agent.path, now],
        )?;
    }
    Ok(())
}

pub fn get_all_captures(conn: &Connection) -> Result<Vec<Capture>> {
    let mut stmt = conn.prepare(
        "SELECT id, content, content_type, status, created_at, scheduled_at, ai_summary, agent_used
         FROM captures ORDER BY created_at DESC"
    )?;
    map_capture_rows(&mut stmt)
}

pub fn get_pending_count(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM captures WHERE status = 'pending'",
        [],
        |row| row.get(0),
    )
}

// ---------- reminders (2.0) ----------

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Reminder {
    pub id: i64,
    pub text: String,
    pub due_at: String,
    pub status: String,
    pub created_at: String,
}

pub fn insert_reminder(conn: &Connection, text: &str, due_at: &str) -> Result<i64> {
    let now = Local::now().to_rfc3339();
    conn.execute(
        "INSERT INTO reminders (text, due_at, status, created_at) VALUES (?1, ?2, 'pending', ?3)",
        params![text, due_at, now],
    )?;
    Ok(conn.last_insert_rowid())
}

fn map_reminder_rows<P: rusqlite::Params>(
    stmt: &mut rusqlite::Statement<'_>,
    p: P,
) -> Result<Vec<Reminder>> {
    let rows = stmt.query_map(p, |row| {
        Ok(Reminder {
            id: row.get(0)?,
            text: row.get(1)?,
            due_at: row.get(2)?,
            status: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?;
    rows.collect()
}

/// Reminders whose time has come (and not yet fired).
pub fn due_reminders(conn: &Connection) -> Result<Vec<Reminder>> {
    let now = Local::now().to_rfc3339();
    let mut stmt = conn.prepare(
        "SELECT id, text, due_at, status, created_at FROM reminders
         WHERE status = 'pending' AND due_at <= ?1 ORDER BY due_at ASC"
    )?;
    map_reminder_rows(&mut stmt, params![now])
}

/// All reminders still pending, soonest first (for reminder.list).
pub fn get_pending_reminders(conn: &Connection) -> Result<Vec<Reminder>> {
    let mut stmt = conn.prepare(
        "SELECT id, text, due_at, status, created_at FROM reminders
         WHERE status = 'pending' ORDER BY due_at ASC LIMIT 20"
    )?;
    map_reminder_rows(&mut stmt, [])
}

pub fn fire_reminder(conn: &Connection, id: i64) -> Result<()> {
    let now = Local::now().to_rfc3339();
    conn.execute(
        "UPDATE reminders SET status = 'fired', fired_at = ?2 WHERE id = ?1",
        params![id, now],
    )?;
    Ok(())
}

pub fn complete_reminder(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "UPDATE reminders SET status = 'done' WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

// ---------- captures search (2.0) ----------

pub fn search_captures(conn: &Connection, query: &str, limit: i64) -> Result<Vec<Capture>> {
    let pattern = format!("%{}%", query.replace('%', ""));
    let mut stmt = conn.prepare(
        "SELECT id, content, content_type, status, created_at, scheduled_at, ai_summary, agent_used
         FROM captures WHERE content LIKE ?1 ORDER BY created_at DESC LIMIT ?2"
    )?;
    let rows = stmt.query_map(params![pattern, limit], |row| {
        Ok(Capture {
            id: row.get(0)?,
            content: row.get(1)?,
            content_type: row.get(2)?,
            status: row.get(3)?,
            created_at: row.get(4)?,
            scheduled_at: row.get(5)?,
            ai_summary: row.get(6)?,
            agent_used: row.get(7)?,
        })
    })?;
    rows.collect()
}

// ---------- tool log (2.0) ----------

pub fn log_tool_call(conn: &Connection, input: &str, calls: &str, answer: &str) -> Result<()> {
    let now = Local::now().to_rfc3339();
    conn.execute(
        "INSERT INTO tool_log (input, calls, answer, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![input, calls, answer, now],
    )?;
    Ok(())
}

pub fn get_recent_tool_log(conn: &Connection, limit: i64) -> Result<Vec<(i64, String, String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, input, calls, answer FROM tool_log ORDER BY id DESC LIMIT ?1"
    )?;
    let rows = stmt.query_map(params![limit], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    })?;
    rows.collect()
}
