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

    // sync columns (added after 2.0): stable cross-device identity + change clock
    ensure_column(conn, "captures", "uuid", "TEXT")?;
    ensure_column(conn, "captures", "updated_at", "TEXT")?;
    Ok(())
}

/// SQLite has no `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` — check first.
pub fn ensure_column(conn: &Connection, table: &str, column: &str, decl: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let exists = stmt.query_map([], |row| row.get::<_, String>(1))?
        .any(|c| c.map(|c| c == column).unwrap_or(false));
    if !exists {
        conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"), [])?;
    }
    Ok(())
}

fn now_rfc3339() -> String {
    Local::now().to_rfc3339()
}

pub fn get_data_dir() -> std::path::PathBuf {
    let base = dirs::data_local_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join("perihel")
}

pub fn insert_capture(conn: &Connection, content: &str, content_type: &str) -> Result<i64> {
    let now = Local::now().to_rfc3339();
    conn.execute(
        "INSERT INTO captures (content, content_type, status, created_at, updated_at) VALUES (?1, ?2, 'pending', ?3, ?3)",
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
        "UPDATE captures SET status = ?1, updated_at = ?2 WHERE id = ?3",
        params![status, now_rfc3339(), id],
    )?;
    Ok(())
}

pub fn schedule_capture(conn: &Connection, id: i64, scheduled_at: &str) -> Result<()> {
    conn.execute(
        "UPDATE captures SET scheduled_at = ?1, updated_at = ?2 WHERE id = ?3",
        params![scheduled_at, now_rfc3339(), id],
    )?;
    Ok(())
}

pub fn update_ai_summary(conn: &Connection, id: i64, summary: &str, agent: &str) -> Result<()> {
    conn.execute(
        "UPDATE captures SET ai_summary = ?1, agent_used = ?2, updated_at = ?3 WHERE id = ?4",
        params![summary, agent, now_rfc3339(), id],
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

// ---------- sync (2.1) ----------

/// One capture as it travels over the wire — uuid is the cross-device identity,
/// updated_at is the last-write-wins clock.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SyncCapture {
    pub uuid: String,
    pub content: String,
    pub content_type: String,
    pub status: String,
    pub created_at: String,
    pub scheduled_at: Option<String>,
    pub ai_summary: Option<String>,
    pub agent_used: Option<String>,
    pub updated_at: String,
}

/// Give every capture a stable identity on first sync.
pub fn stamp_missing_uuids(conn: &Connection, uuid_for: impl Fn(i64) -> String) -> Result<()> {
    let mut stmt = conn.prepare("SELECT id FROM captures WHERE uuid IS NULL")?;
    let ids: Vec<i64> = stmt.query_map([], |row| row.get(0))?.flatten().collect();
    for id in ids {
        conn.execute(
            "UPDATE captures SET uuid = ?1, updated_at = COALESCE(updated_at, ?2) WHERE id = ?3",
            params![uuid_for(id), now_rfc3339(), id],
        )?;
    }
    Ok(())
}

/// Every local capture, for the push phase (full-state reconciliation is fine
/// at personal scale; the server keeps whichever copy is newer).
pub fn get_sync_captures(conn: &Connection) -> Result<Vec<SyncCapture>> {
    let mut stmt = conn.prepare(
        "SELECT uuid, content, content_type, status, created_at, scheduled_at, ai_summary, agent_used, updated_at
         FROM captures WHERE uuid IS NOT NULL")?;
    let rows = stmt.query_map([], |row| {
        Ok(SyncCapture {
            uuid: row.get(0)?,
            content: row.get(1)?,
            content_type: row.get(2)?,
            status: row.get(3)?,
            created_at: row.get(4)?,
            scheduled_at: row.get(5)?,
            ai_summary: row.get(6)?,
            agent_used: row.get(7)?,
            updated_at: row.get::<_, Option<String>>(8)?.unwrap_or_else(now_rfc3339),
        })
    })?;
    rows.collect()
}

/// Merge one remote capture into the local DB — last write wins on updated_at.
/// Returns true if the local row was changed.
pub fn apply_remote_capture(conn: &Connection, cap: &SyncCapture) -> Result<bool> {
    let existing: Option<(i64, Option<String>)> = conn
        .query_row(
            "SELECT id, updated_at FROM captures WHERE uuid = ?1",
            params![cap.uuid],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;

    match existing {
        Some((id, local_updated)) => {
            if local_updated.as_deref().unwrap_or("") >= cap.updated_at.as_str() {
                return Ok(false);
            }
            conn.execute(
                "UPDATE captures SET content = ?1, content_type = ?2, status = ?3,
                 scheduled_at = ?4, ai_summary = ?5, agent_used = ?6, updated_at = ?7
                 WHERE id = ?8",
                params![cap.content, cap.content_type, cap.status, cap.scheduled_at,
                        cap.ai_summary, cap.agent_used, cap.updated_at, id],
            )?;
            Ok(true)
        }
        None => {
            conn.execute(
                "INSERT INTO captures (uuid, content, content_type, status, created_at,
                 scheduled_at, ai_summary, agent_used, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![cap.uuid, cap.content, cap.content_type, cap.status, cap.created_at,
                        cap.scheduled_at, cap.ai_summary, cap.agent_used, cap.updated_at],
            )?;
            Ok(true)
        }
    }
}
