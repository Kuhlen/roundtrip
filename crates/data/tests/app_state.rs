use std::fs;
use std::path::PathBuf;

use data::app_state::AppStateFile;
use domain::session::{Session, SessionStore};

#[test]
fn round_trip_creates_parent_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let store = AppStateFile::new(dir.path().join("roundtrip/state.json"));
    let session = Session {
        last_collection: Some(PathBuf::from("/home/me/api")),
        last_environment: Some("dev".into()),
    };
    store.save(&session);
    assert_eq!(store.load(), session);
}

#[test]
fn missing_or_broken_file_is_empty_session() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    assert_eq!(AppStateFile::new(path.clone()).load(), Session::default());
    fs::write(&path, "{not json").unwrap();
    assert_eq!(AppStateFile::new(path).load(), Session::default());
}
