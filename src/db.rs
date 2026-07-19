use rusqlite::{Connection, Result, params};
use serde::{Deserialize, Serialize};
use chrono::Local;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum CaptureStatus {
    Pending,
    Done,
    Dismissed,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum ContentType {
    Text,
    Url,
    Screenshot,
    Voice,
}

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
    let conn = Connection::open(&db_path)?;

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
        INSERT OR IGNORE INTO settings (key, value) VALUES ('quiet_start', '23:00');
        INSERT OR IGNORE INTO settings (key, value) VALUES ('quiet_end', '08:00');
        INSERT OR IGNORE INTO settings (key, value) VALUES ('hotkey', 'Ctrl+Shift+P');
    ")?;

    Ok(conn)
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

pub fn get_pending_captures(conn: &Connection) -> Result<Vec<Capture>> {
    let mut stmt = conn.prepare(
        "SELECT id, content, content_type, status, created_at, scheduled_at, ai_summary, agent_used
         FROM captures ORDER BY created_at DESC"
    )?;
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

pub fn get_unscheduled_pending(conn: &Connection) -> Result<Vec<Capture>> {
    let mut stmt = conn.prepare(
        "SELECT id, content, content_type, status, created_at, scheduled_at, ai_summary, agent_used
         FROM captures WHERE status = 'pending' AND scheduled_at IS NULL"
    )?;
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

pub fn get_detected_agents(conn: &Connection) -> Result<Vec<DetectedAgent>> {
    let mut stmt = conn.prepare(
        "SELECT name, version, path, detected_at FROM detected_agents ORDER BY name"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(DetectedAgent {
            name: row.get(0)?,
            version: row.get(1)?,
            path: row.get(2)?,
            detected_at: row.get(3)?,
        })
    })?;
    let mut agents = Vec::new();
    for row in rows {
        agents.push(row?);
    }
    Ok(agents)
}

pub fn get_all_captures(conn: &Connection) -> Result<Vec<Capture>> {
    let mut stmt = conn.prepare(
        "SELECT id, content, content_type, status, created_at, scheduled_at, ai_summary, agent_used
         FROM captures ORDER BY created_at DESC"
    )?;
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

pub fn get_pending_count(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM captures WHERE status = 'pending'",
        [],
        |row| row.get(0),
    )
}
