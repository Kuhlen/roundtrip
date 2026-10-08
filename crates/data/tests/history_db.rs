use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use data::history_db::HistoryDb;
use domain::auth::Auth;
use domain::history::{HistoryEntry, HistoryStore};
use domain::http::{Body, KeyValue, Method, Request, TextKind};
use tempfile::tempdir;

fn entry(method: Method, url: &str, name: Option<&str>, secs_ago: u64) -> HistoryEntry {
    HistoryEntry {
        id: 0,
        method,
        url: url.into(),
        status: Some(200),
        status_text: Some("OK".into()),
        time_ms: Some(84),
        size_bytes: Some(21),
        at: SystemTime::now() - Duration::from_secs(secs_ago),
        collection: Some(PathBuf::from("/c")),
        name: name.map(Into::into),
    }
}

fn db(dir: &Path) -> HistoryDb {
    HistoryDb::open(&dir.join("history.db")).expect("open")
}

fn urls(entries: &[HistoryEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.url.as_str()).collect()
}

#[test]
fn recent_is_newest_first_and_limited() {
    let dir = tempdir().unwrap();
    let h = db(dir.path());
    for (i, url) in ["https://a", "https://b", "https://c"].iter().enumerate() {
        h.record(
            &entry(Method::Get, url, None, 30 - i as u64),
            &Request::default(),
        )
        .unwrap();
    }
    assert_eq!(
        urls(&h.recent(50).unwrap()),
        ["https://c", "https://b", "https://a"]
    );
    assert_eq!(urls(&h.recent(2).unwrap()), ["https://c", "https://b"]);
    let first = &h.recent(50).unwrap()[0];
    assert_eq!(first.status, Some(200));
    assert_eq!(first.time_ms, Some(84));
    assert_eq!(first.collection, Some(PathBuf::from("/c")));
}

#[test]
fn search_matches_url_method_and_name() {
    let dir = tempdir().unwrap();
    let h = db(dir.path());
    h.record(
        &entry(Method::Get, "https://x/users", None, 3),
        &Request::default(),
    )
    .unwrap();
    h.record(
        &entry(Method::Post, "https://x/orders", Some("Create order"), 2),
        &Request::default(),
    )
    .unwrap();
    h.record(
        &entry(Method::Delete, "https://x/items", Some("Wipe"), 1),
        &Request::default(),
    )
    .unwrap();
    assert_eq!(urls(&h.search("USERS", 50).unwrap()), ["https://x/users"]);
    assert_eq!(urls(&h.search("post", 50).unwrap()), ["https://x/orders"]);
    assert_eq!(urls(&h.search("wipe", 50).unwrap()), ["https://x/items"]);
    assert_eq!(h.search("https://x", 2).unwrap().len(), 2);
}

#[test]
fn search_treats_percent_and_underscore_literally() {
    let dir = tempdir().unwrap();
    let h = db(dir.path());
    h.record(
        &entry(Method::Get, "https://x/?user_id=1", None, 2),
        &Request::default(),
    )
    .unwrap();
    h.record(
        &entry(Method::Get, "https://x/?userXid=1", None, 1),
        &Request::default(),
    )
    .unwrap();
    assert_eq!(
        urls(&h.search("user_id", 50).unwrap()),
        ["https://x/?user_id=1"]
    );
    assert!(h.search("50%", 50).unwrap().is_empty());
}

#[test]
fn request_round_trips() {
    let dir = tempdir().unwrap();
    let h = db(dir.path());
    let request = Request {
        method: Method::Post,
        url: "{{baseUrl}}/post".into(),
        params: vec![KeyValue::new("b", "2"), KeyValue::new("a", "1")],
        headers: vec![KeyValue::new("X-Trace", "1")],
        body: Body::Text {
            kind: TextKind::Json,
            text: r#"{"name":"Ayu"}"#.into(),
        },
        auth: Some(Auth::Bearer {
            token: "{{token}}".into(),
        }),
    };
    h.record(&entry(Method::Post, "https://x/post", None, 1), &request)
        .unwrap();
    let id = h.recent(1).unwrap()[0].id;
    assert_eq!(h.request(id).unwrap(), request);
    assert!(h.request(id + 100).is_err());
}

#[test]
fn error_entry_keeps_empty_status() {
    let dir = tempdir().unwrap();
    let h = db(dir.path());
    let failed = HistoryEntry {
        status: None,
        status_text: None,
        time_ms: None,
        size_bytes: None,
        ..entry(Method::Get, "https://bad-host", None, 1)
    };
    h.record(&failed, &Request::default()).unwrap();
    let got = &h.recent(1).unwrap()[0];
    assert_eq!(
        (got.status, got.time_ms, got.size_bytes),
        (None, None, None)
    );
}

#[test]
fn clear_empties() {
    let dir = tempdir().unwrap();
    let h = db(dir.path());
    h.record(
        &entry(Method::Get, "https://a", None, 1),
        &Request::default(),
    )
    .unwrap();
    h.clear().unwrap();
    assert!(h.recent(50).unwrap().is_empty());
}

#[test]
fn garbage_file_is_set_aside() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("history.db"), vec![b'x'; 4096]).unwrap();
    let h = db(dir.path());
    assert!(h.recent(50).unwrap().is_empty());
    let aside = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("history.db.corrupt.")
        });
    assert!(aside);
}

#[test]
fn two_instances_share_the_file() {
    let dir = tempdir().unwrap();
    let first = db(dir.path());
    let second = db(dir.path());
    first
        .record(
            &entry(Method::Get, "https://a", None, 2),
            &Request::default(),
        )
        .unwrap();
    second
        .record(
            &entry(Method::Get, "https://b", None, 1),
            &Request::default(),
        )
        .unwrap();
    assert_eq!(urls(&first.recent(50).unwrap()), ["https://b", "https://a"]);
    let names: Vec<String> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(!names.iter().any(|n| n.contains("corrupt")), "{names:?}");
}

#[test]
fn locked_file_is_never_set_aside() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("history.db");
    {
        let h = db(dir.path());
        h.record(
            &entry(Method::Get, "https://a", None, 1),
            &Request::default(),
        )
        .unwrap();
    }
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch(
        "BEGIN EXCLUSIVE; INSERT INTO history (method, url, timestamp, request_yaml) \
         VALUES ('GET', 'https://held', '2026-01-01T00:00:00.000Z', '');",
    )
    .unwrap();
    let _ = HistoryDb::open(&path);
    let names: Vec<String> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(!names.iter().any(|n| n.contains("corrupt")), "{names:?}");
    raw.execute_batch("ROLLBACK").unwrap();
    drop(raw);
    assert_eq!(urls(&db(dir.path()).recent(50).unwrap()), ["https://a"]);
}
