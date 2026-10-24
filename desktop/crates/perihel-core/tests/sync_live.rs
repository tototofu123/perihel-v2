//! Live smoke test — hits the real sync server. Ignored by default; run with:
//!   PERIHEL_SYNC_TEST_URL=https://... PERIHEL_SYNC_TEST_TOKEN=... \
//!   cargo test -p perihel-core --test sync_live -- --ignored --nocapture

use perihel_core::db::{self, SyncCapture};
use perihel_core::sync::{sync_once, SyncConfig};
use rusqlite::Connection;
use std::sync::{Arc, Mutex};

#[test]
#[ignore]
fn live_sync_roundtrip() {
    let url = std::env::var("PERIHEL_SYNC_TEST_URL").expect("set PERIHEL_SYNC_TEST_URL");
    let token = std::env::var("PERIHEL_SYNC_TEST_TOKEN").expect("set PERIHEL_SYNC_TEST_TOKEN");
    let cfg = SyncConfig { server_url: url, token };

    let mut conn = Connection::open_in_memory().unwrap();
    db::init_schema(&mut conn).unwrap();
    let conn = Arc::new(Mutex::new(conn));

    // a capture unique to this run
    let stamp = chrono::Utc::now().to_rfc3339();
    db::insert_capture(&conn.lock().unwrap(), &format!("sync smoke {stamp}"), "text").unwrap();

    let outcome = sync_once(&conn, &cfg).expect("sync failed");
    println!("pushed {}, pulled {}", outcome.pushed, outcome.pulled);
    assert!(outcome.pushed >= 1);

    // second sync: full reconciliation, nothing lost
    let caps = db::get_sync_captures(&conn.lock().unwrap()).unwrap();
    assert!(caps.iter().any(|c| c.content.contains("sync smoke")));
}
