//! Integration tests — exercise the shared db layer exactly the way both
//! binaries use it (real SQLite file, real schema).

use perihel_core::db;
use rusqlite::Connection;

fn test_conn() -> Connection {
    let dir = std::env::temp_dir().join("perihel-integration-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("test_{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut conn = Connection::open(&path).unwrap();
    db::init_schema(&mut conn).unwrap();
    conn
}

#[test]
fn settings_roundtrip() {
    let conn = test_conn();
    db::set_setting(&conn, "test_key", "test_val").expect("set failed");
    let val = db::get_setting(&conn, "test_key").expect("get failed");
    assert_eq!(val, Some("test_val".to_string()));
}

#[test]
fn capture_lifecycle() {
    let conn = test_conn();
    let id = db::insert_capture(&conn, "test thought", "text").unwrap();
    assert!(id > 0);

    // unscheduled pending → gets scheduled → becomes due → done
    let unscheduled = db::get_unscheduled_pending(&conn).unwrap();
    assert!(unscheduled.iter().any(|c| c.id == id));

    db::schedule_capture(&conn, id, "2020-01-01T00:00:00Z").unwrap();
    let due = db::get_pending_captures(&conn).unwrap();
    assert!(due.iter().any(|c| c.id == id));

    db::update_status(&conn, id, "done").unwrap();
    let all = db::get_all_captures(&conn).unwrap();
    assert_eq!(all.iter().find(|c| c.id == id).unwrap().status, "done");
}

#[test]
fn reminder_lifecycle() {
    let conn = test_conn();
    let id = db::insert_reminder(&conn, "ask Prof. Chan", "2020-01-01T00:00:00Z").unwrap();

    let due = db::due_reminders(&conn).unwrap();
    assert!(due.iter().any(|r| r.id == id));

    db::fire_reminder(&conn, id).unwrap();
    assert!(db::due_reminders(&conn).unwrap().is_empty());
    assert_eq!(db::get_pending_reminders(&conn).unwrap().len(), 0);
}

#[test]
fn search_and_tool_log() {
    let conn = test_conn();
    db::insert_capture(&conn, "climbing session notes", "text").unwrap();
    db::insert_capture(&conn, "grocery list", "text").unwrap();

    let hits = db::search_captures(&conn, "climbing", 5).unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].content.contains("climbing"));

    db::log_tool_call(&conn, "what did I note?", "search.captures: 1 hit", "You noted: climbing session notes").unwrap();
    let log = db::get_recent_tool_log(&conn, 10).unwrap();
    assert_eq!(log.len(), 1);
}
