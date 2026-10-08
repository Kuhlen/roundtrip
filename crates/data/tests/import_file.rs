use std::fs;
use std::path::{Path, PathBuf};

use data::import_file::read;
use domain::AppError;
use domain::import::{ImportFormat, ImportedCollection};

fn file(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, text).expect("write");
    path
}

fn import(name: &str, text: &str) -> Result<ImportedCollection, AppError> {
    let dir = tempfile::tempdir().expect("tempdir");
    read(&file(dir.path(), name, text))
}

fn import_error(name: &str, text: &str) -> String {
    match import(name, text) {
        Err(AppError::Import(msg)) => msg,
        other => panic!("expected an import error, got {other:?}"),
    }
}

#[test]
fn postman_json_and_yaml_are_detected() {
    let json = import("c.json", r#"{"info":{"name":"J"},"item":[]}"#).unwrap();
    assert_eq!(
        (json.name.as_str(), &json.format),
        ("J", &ImportFormat::Postman)
    );
    let yaml = import("c.yaml", "info:\n  name: Y\nitem: []\n").unwrap();
    assert_eq!(yaml.name, "Y");
}

#[test]
fn bom_is_ignored() {
    let c = import("c.json", "\u{feff}{\"info\":{\"name\":\"B\"},\"item\":[]}").unwrap();
    assert_eq!(c.name, "B");
}

#[test]
fn unsupported_versions_say_why() {
    assert!(import_error("s.yaml", "swagger: \"2.0\"\n").contains("Swagger 2.0"));
    assert!(import_error("s.yaml", "openapi: 4.0.0\n").contains("only 3.x"));
    assert!(
        import_error("d.yaml", "type: spec.insomnia.rest/5.0\nname: D\n")
            .contains("design documents")
    );
}

#[test]
fn unknown_format_lists_supported_ones() {
    assert!(import_error("x.json", r#"{"foo":1}"#).contains("Supported"));
    // plain text is a YAML string, still not a collection
    assert!(import_error("x.json", "not json").contains("Supported"));
}

#[test]
fn missing_file_is_a_storage_error() {
    assert!(matches!(
        read(Path::new("/nope/x.json")),
        Err(AppError::Storage(_))
    ));
}

#[test]
fn huge_file_is_refused_before_reading() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.json");
    let f = fs::File::create(&path).unwrap();
    // sparse: no 50 MB written to disk
    f.set_len(50 * 1024 * 1024 + 1).unwrap();
    assert!(matches!(read(&path), Err(AppError::Import(_))));
}
