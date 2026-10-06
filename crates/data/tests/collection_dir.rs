use std::fs;
use std::path::Path;

use data::collection_dir::CollectionDir;
use domain::AppError;
use domain::collection::{CollectionStore, Node, Protocol};
use domain::http::{BodyKind, KeyValue, Method, Request};
use serde_yaml::Value;

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

fn collection() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    write(
        dir.path(),
        ".apiark/apiark.yaml",
        "name: demo\nversion: 1\n",
    );
    dir
}

fn names(nodes: &[Node]) -> Vec<&str> {
    nodes.iter().map(Node::name).collect()
}

fn protocol_of(yaml: &str) -> Protocol {
    let dir = collection();
    write(dir.path(), "r.yaml", yaml);
    match &CollectionDir.load(dir.path()).expect("load").children[..] {
        [Node::Request { protocol, .. }] => *protocol,
        other => panic!("expected one request, got {other:?}"),
    }
}

fn reload(file: &Path) -> Value {
    serde_yaml::from_str(&fs::read_to_string(file).expect("read")).expect("yaml")
}

const OK: &str = "name: Ok\nmethod: GET\nurl: http://x\n";

#[test]
fn tree_order_folders_first_then_folder_order_then_name() {
    let dir = collection();
    let r = dir.path();
    write(r, "_folder.yaml", "order:\n  - zeta\n  - b\n");
    write(r, "a.yaml", "name: A\nmethod: GET\nurl: http://x\n");
    write(r, "b.yaml", "name: B\nmethod: GET\nurl: http://x\n");
    write(r, "zeta.yaml", "name: Z\nmethod: GET\nurl: http://x\n");
    write(r, "users/list.yaml", OK);
    write(r, "orders/get.yaml", OK);
    let c = CollectionDir.load(r).unwrap();
    assert_eq!(c.name, "demo");
    assert_eq!(names(&c.children), ["orders", "users", "Z", "B", "A"]);
}

#[test]
fn hidden_broken_and_foreign_files_are_skipped() {
    let dir = collection();
    let r = dir.path();
    write(r, ".hidden.yaml", OK);
    write(r, "broken.yaml", "name: [unclosed\n");
    write(
        r,
        "bad-method.yaml",
        "name: X\nmethod: FETCH\nurl: http://x\n",
    );
    write(r, "notes.txt", "hi");
    write(r, "ok.yaml", OK);
    assert_eq!(names(&CollectionDir.load(r).unwrap().children), ["Ok"]);
}

#[test]
fn missing_config_is_not_a_collection() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        CollectionDir.load(dir.path()),
        Err(AppError::NotACollection(_))
    ));
}

// parity: apiark models/collection.rs tests
#[test]
fn graphql_detected_from_body() {
    let yaml = "name: Test\nmethod: POST\nurl: https://example.com/graphql\nbody:\n  type: json\n  content: '{\"query\": \"{ users { id } }\"}'\n";
    assert_eq!(protocol_of(yaml), Protocol::Graphql);
}

#[test]
fn json_without_query_is_http() {
    let yaml = "name: Test\nmethod: POST\nurl: https://example.com/api\nbody:\n  type: json\n  content: '{\"key\": \"value\"}'\n";
    assert_eq!(protocol_of(yaml), Protocol::Http);
}

#[test]
fn graphql_detected_from_multiline_body() {
    let yaml = "name: Test\nmethod: POST\nurl: https://countries.trevorblades.com/graphql\nheaders:\n  Content-Type: application/json\nbody:\n  type: json\n  content: |-\n    {\n      \"query\": \"query { countries { name } }\"\n    }\n";
    assert_eq!(protocol_of(yaml), Protocol::Graphql);
}

#[test]
fn get_is_never_graphql() {
    assert_eq!(
        protocol_of("name: Test\nmethod: GET\nurl: https://example.com/api\n"),
        Protocol::Http
    );
}

#[test]
fn explicit_protocol_wins_except_http() {
    assert_eq!(
        protocol_of("name: S\nmethod: GET\nurl: ws://x\nprotocol: websocket\n"),
        Protocol::WebSocket
    );
    let http_with_query = "name: Q\nmethod: POST\nurl: http://x\nprotocol: http\nbody:\n  type: json\n  content: '{\"query\": \"q\"}'\n";
    assert_eq!(protocol_of(http_with_query), Protocol::Graphql);
}

#[test]
fn read_request_keeps_file_order() {
    let dir = collection();
    write(
        dir.path(),
        "r.yaml",
        "name: R\nmethod: POST\nurl: '{{baseUrl}}/post'\nheaders:\n  X-B: '2'\n  X-A: '1'\nparams:\n  page: '2'\nbody:\n  type: json\n  content: '{\"a\":1}'\n",
    );
    let req = CollectionDir
        .read_request(&dir.path().join("r.yaml"))
        .unwrap();
    assert_eq!(
        req,
        Request {
            method: Method::Post,
            url: "{{baseUrl}}/post".into(),
            params: vec![KeyValue::new("page", "2")],
            headers: vec![KeyValue::new("X-B", "2"), KeyValue::new("X-A", "1")],
            body_kind: BodyKind::Json,
            body: r#"{"a":1}"#.into(),
        }
    );
}

#[test]
fn scalar_values_read_as_text() {
    let dir = collection();
    let file = dir.path().join("r.yaml");
    write(
        dir.path(),
        "r.yaml",
        "name: R\nmethod: GET\nurl: http://x\nparams:\n  page: 2\n  flag: true\nheaders:\n  X-Empty:\n",
    );
    let req = CollectionDir.read_request(&file).unwrap();
    assert_eq!(
        req.params,
        vec![KeyValue::new("page", "2"), KeyValue::new("flag", "true")]
    );
    assert_eq!(req.headers, vec![KeyValue::new("X-Empty", "")]);
    CollectionDir.save_request(&file, &req).unwrap();
    assert_eq!(CollectionDir.read_request(&file).unwrap(), req);
}

#[test]
fn null_headers_and_params_are_empty() {
    let dir = collection();
    write(
        dir.path(),
        "r.yaml",
        "name: R\nmethod: GET\nurl: http://x\nheaders:\nparams:\n",
    );
    let req = CollectionDir
        .read_request(&dir.path().join("r.yaml"))
        .unwrap();
    assert!(req.headers.is_empty() && req.params.is_empty());
}

#[test]
fn merge_conflict_is_refused_on_read_and_save() {
    let dir = collection();
    let file = dir.path().join("r.yaml");
    let text = "<<<<<<< HEAD\nname: A\n=======\nname: B\n>>>>>>> branch\n";
    write(dir.path(), "r.yaml", text);
    assert!(matches!(
        CollectionDir.read_request(&file),
        Err(AppError::MergeConflict(_))
    ));
    assert!(matches!(
        CollectionDir.save_request(&file, &Request::default()),
        Err(AppError::MergeConflict(_))
    ));
    assert_eq!(fs::read_to_string(&file).unwrap(), text);
}

#[test]
fn save_keeps_unknown_keys() {
    let dir = collection();
    let file = dir.path().join("r.yaml");
    write(
        dir.path(),
        "r.yaml",
        "name: R\nmethod: GET\nurl: http://a\ndescription: keep me\nauth:\n  type: bearer\n  token: '{{t}}'\ntests: |\n  ok\nheaders:\n  X-Old: '1'\n",
    );
    let mut req = CollectionDir.read_request(&file).unwrap();
    req.method = Method::Put;
    req.url = "http://b".into();
    req.headers = vec![KeyValue::new("X-New", "2")];
    CollectionDir.save_request(&file, &req).unwrap();

    let doc = reload(&file);
    assert_eq!(doc["name"], "R");
    assert_eq!(doc["description"], "keep me");
    assert_eq!(doc["auth"]["token"], "{{t}}");
    assert_eq!(doc["tests"], "ok\n");
    assert_eq!(doc["method"], "PUT");
    assert_eq!(doc["url"], "http://b");
    assert_eq!(doc["headers"]["X-New"], "2");
    assert!(doc["headers"].get("X-Old").is_none());
    assert_eq!(CollectionDir.read_request(&file).unwrap(), req);
}

#[test]
fn save_drops_disabled_rows_empty_maps_and_none_body() {
    let dir = collection();
    let file = dir.path().join("r.yaml");
    write(
        dir.path(),
        "r.yaml",
        "name: R\nmethod: POST\nurl: http://a\nparams:\n  page: '1'\nbody:\n  type: json\n  content: '{}'\n",
    );
    let mut req = CollectionDir.read_request(&file).unwrap();
    req.params[0].enabled = false;
    req.body_kind = BodyKind::None;
    CollectionDir.save_request(&file, &req).unwrap();
    let doc = reload(&file);
    assert!(doc.get("params").is_none());
    assert!(doc.get("body").is_none());
}

#[test]
fn save_leaves_unsupported_body_untouched() {
    let dir = collection();
    let file = dir.path().join("u.yaml");
    write(
        dir.path(),
        "u.yaml",
        "name: U\nmethod: POST\nurl: http://a\nbody:\n  type: form-data\n  content:\n    - key: a\n      value: '1'\n",
    );
    let mut req = CollectionDir.read_request(&file).unwrap();
    assert_eq!(req.body_kind, BodyKind::Unsupported("form-data".into()));
    req.url = "http://b".into();
    CollectionDir.save_request(&file, &req).unwrap();
    let doc = reload(&file);
    assert_eq!(doc["url"], "http://b");
    assert_eq!(doc["body"]["type"], "form-data");
    assert_eq!(doc["body"]["content"][0]["value"], "1");
}

#[test]
fn save_leaves_no_tmp_file() {
    let dir = collection();
    let file = dir.path().join("r.yaml");
    write(dir.path(), "r.yaml", OK);
    let req = CollectionDir.read_request(&file).unwrap();
    CollectionDir.save_request(&file, &req).unwrap();
    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
fn save_to_missing_file_fails_cleanly() {
    let dir = collection();
    let file = dir.path().join("gone.yaml");
    assert!(matches!(
        CollectionDir.save_request(&file, &Request::default()),
        Err(AppError::Storage(_))
    ));
    assert!(!file.exists());
    assert!(!dir.path().join("gone.yaml.tmp").exists());
}

#[test]
fn save_writes_edited_json_body() {
    let dir = collection();
    let file = dir.path().join("j.yaml");
    write(
        dir.path(),
        "j.yaml",
        "name: J\nmethod: POST\nurl: http://a\nbody:\n  type: json\n  content: '{\"a\":1}'\n",
    );
    let mut req = CollectionDir.read_request(&file).unwrap();
    req.body = "{\"b\": 2}".into();
    CollectionDir.save_request(&file, &req).unwrap();
    let doc = reload(&file);
    assert_eq!(doc["body"]["type"], "json");
    assert_eq!(doc["body"]["content"], "{\"b\": 2}");
    assert_eq!(CollectionDir.read_request(&file).unwrap(), req);
}
