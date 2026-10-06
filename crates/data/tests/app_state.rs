use std::fs;
use std::path::PathBuf;

use data::app_state::AppStateFile;
use domain::session::{Session, SessionStore};

#[test]
fn round_trip_creates_parent_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let store = AppStateFile::new(dir.path().join("roundtrip/state.json"));
    let session = Session {
        collections: vec![
            PathBuf::from("/home/me/api"),
            PathBuf::from("/home/me/other"),
        ],
        tabs: vec![PathBuf::from("/home/me/api/users/list.yaml")],
        active_tab: Some(0),
        environment: Some("dev".into()),
    };
    store.save(&session);
    assert_eq!(store.load(), session);
}

#[test]
fn old_single_collection_file_migrates() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    fs::write(
        &path,
        r#"{"last_collection":"/home/me/api","last_environment":"dev"}"#,
    )
    .unwrap();
    let store = AppStateFile::new(path.clone());
    assert_eq!(
        store.load(),
        Session {
            collections: vec![PathBuf::from("/home/me/api")],
            tabs: vec![],
            active_tab: None,
            environment: Some("dev".into()),
        }
    );
    store.save(&store.load());
    let written = fs::read_to_string(&path).unwrap();
    assert!(!written.contains("last_collection"), "{written}");
}

#[test]
fn missing_or_broken_file_is_empty_session() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    assert_eq!(AppStateFile::new(path.clone()).load(), Session::default());
    fs::write(&path, "{not json").unwrap();
    assert_eq!(AppStateFile::new(path).load(), Session::default());
}
