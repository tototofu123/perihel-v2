#[test]
fn test_db_init_and_capture_roundtrip() {
    let conn = perihel::db::init_db().expect("init_db failed");
    let id = perihel::db::insert_capture(&conn, "hello world", "text").expect("insert failed");
    assert!(id > 0);

    let all = perihel::db::get_all_captures(&conn).expect("get_all failed");
    assert!(!all.is_empty());

    let pending = perihel::db::get_pending_captures(&conn).expect("get_pending failed");
    assert!(!pending.is_empty());

    let count = perihel::db::get_pending_count(&conn).expect("get_count failed");
    assert!(count > 0);

    perihel::db::update_status(&conn, id, "done").expect("update failed");
    let done = perihel::db::get_pending_count(&conn).expect("get_count failed");
    assert_eq!(done, 0);

    perihel::db::update_ai_summary(&conn, id, "test summary", "codex").expect("ai_summary failed");
    let all2 = perihel::db::get_all_captures(&conn).expect("get_all failed");
    let cap = all2.iter().find(|c| c.id == id).unwrap();
    assert_eq!(cap.ai_summary.as_deref(), Some("test summary"));
    assert_eq!(cap.agent_used.as_deref(), Some("codex"));
}

#[test]
fn test_screenshot_capture() {
    let monitors = xcap::Monitor::all().expect("no monitors");
    assert!(!monitors.is_empty(), "at least one monitor");
    let img = monitors[0].capture_image().expect("capture failed");
    assert!(img.width() > 0);
    assert!(img.height() > 0);
}

#[test]
fn test_settings_roundtrip() {
    let conn = perihel::db::init_db().expect("init_db failed");
    perihel::db::set_setting(&conn, "test_key", "test_val").expect("set failed");
    let val = perihel::db::get_setting(&conn, "test_key").expect("get failed");
    assert_eq!(val, Some("test_val".into()));
}

#[test]
fn test_agent_detection() {
    let agents = perihel::agents::detect_agents();
    // Should always return entries (even if not detected)
    assert!(!agents.is_empty());
    // Each agent should have a name and id
    for a in &agents {
        assert!(!a.id.is_empty());
        assert!(!a.name.is_empty());
    }
}
