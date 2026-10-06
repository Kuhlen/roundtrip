use std::fs;
use std::path::Path;

use data::collection_dir::CollectionDir;
use domain::AppError;
use domain::auth::{ApiKeyPlace, Auth};
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
            auth: None,
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

fn auth_of(yaml_auth: &str) -> Option<Auth> {
    let dir = collection();
    write(
        dir.path(),
        "r.yaml",
        &format!("name: R\nmethod: GET\nurl: http://x\n{yaml_auth}"),
    );
    CollectionDir
        .read_request(&dir.path().join("r.yaml"))
        .expect("read")
        .auth
}

#[test]
fn reads_each_auth_type() {
    assert_eq!(
        auth_of("auth:\n  type: bearer\n  token: '{{t}}'\n"),
        Some(Auth::Bearer {
            token: "{{t}}".into()
        })
    );
    assert_eq!(
        auth_of("auth:\n  type: basic\n  username: u\n  password: p\n"),
        Some(Auth::Basic {
            username: "u".into(),
            password: "p".into()
        })
    );
    assert_eq!(
        auth_of("auth:\n  type: api-key\n  key: k\n  value: v\n  addTo: query\n"),
        Some(Auth::ApiKey {
            key: "k".into(),
            value: "v".into(),
            place: ApiKeyPlace::Query
        })
    );
    assert_eq!(
        auth_of("auth:\n  type: api-key\n  key: k\n  value: v\n"),
        Some(Auth::ApiKey {
            key: "k".into(),
            value: "v".into(),
            place: ApiKeyPlace::Header
        })
    );
    assert_eq!(auth_of("auth:\n  type: none\n"), None);
    assert_eq!(auth_of(""), None);
    assert_eq!(
        auth_of("auth:\n  type: oauth2\n  clientId: c\n"),
        Some(Auth::Unsupported("oauth2".into()))
    );
    // broken block: never overwritten
    assert_eq!(
        auth_of("auth: just-a-string\n"),
        Some(Auth::Unsupported("unknown".into()))
    );
}

#[test]
fn collection_auth_comes_from_defaults() {
    let dir = collection();
    write(
        dir.path(),
        ".apiark/apiark.yaml",
        "name: demo\nversion: 1\ndefaults:\n  auth:\n    type: bearer\n    token: abc\n  sendCookies: true\n",
    );
    assert_eq!(
        CollectionDir.load(dir.path()).unwrap().auth,
        Some(Auth::Bearer {
            token: "abc".into()
        })
    );
}

#[test]
fn save_writes_and_removes_auth() {
    let dir = collection();
    let file = dir.path().join("r.yaml");
    write(dir.path(), "r.yaml", OK);
    let mut req = CollectionDir.read_request(&file).unwrap();
    req.auth = Some(Auth::ApiKey {
        key: "X-Key".into(),
        value: "{{k}}".into(),
        place: ApiKeyPlace::Header,
    });
    CollectionDir.save_request(&file, &req).unwrap();
    let doc = reload(&file);
    assert_eq!(doc["auth"]["type"], "api-key");
    assert_eq!(doc["auth"]["key"], "X-Key");
    assert_eq!(doc["auth"]["value"], "{{k}}");
    assert_eq!(doc["auth"]["addTo"], "header");
    req.auth = None;
    CollectionDir.save_request(&file, &req).unwrap();
    assert!(reload(&file).get("auth").is_none());
}

#[test]
fn save_keeps_unsupported_auth() {
    let dir = collection();
    let file = dir.path().join("r.yaml");
    write(
        dir.path(),
        "r.yaml",
        "name: R\nmethod: GET\nurl: http://x\nauth:\n  type: oauth2\n  clientId: c\n  usePkce: true\n",
    );
    let before = reload(&file)["auth"].clone();
    let mut req = CollectionDir.read_request(&file).unwrap();
    req.url = "http://y".into();
    CollectionDir.save_request(&file, &req).unwrap();
    assert_eq!(reload(&file)["auth"], before);
}

#[test]
fn save_collection_auth_keeps_other_keys() {
    let dir = collection();
    let config = dir.path().join(".apiark/apiark.yaml");
    write(
        dir.path(),
        ".apiark/apiark.yaml",
        "name: demo\nversion: 1\ndefaults:\n  sendCookies: true\n",
    );
    let auth = Auth::Bearer {
        token: "{{t}}".into(),
    };
    CollectionDir
        .save_collection_auth(dir.path(), Some(&auth))
        .unwrap();
    let doc = reload(&config);
    assert_eq!(doc["name"], "demo");
    assert_eq!(doc["defaults"]["sendCookies"], true);
    assert_eq!(doc["defaults"]["auth"]["token"], "{{t}}");
    CollectionDir
        .save_collection_auth(dir.path(), None)
        .unwrap();
    let doc = reload(&config);
    assert!(doc["defaults"].get("auth").is_none());
    assert_eq!(doc["defaults"]["sendCookies"], true);
}

#[test]
fn removing_auth_does_not_create_defaults() {
    let dir = collection();
    CollectionDir
        .save_collection_auth(dir.path(), None)
        .unwrap();
    assert!(
        reload(&dir.path().join(".apiark/apiark.yaml"))
            .get("defaults")
            .is_none()
    );
}

#[test]
fn create_request_writes_a_slug_file() {
    let dir = collection();
    let r = dir.path();
    let file = CollectionDir
        .create_request(r, " List  All Users ")
        .unwrap();
    assert_eq!(file, r.join("list-all-users.yaml"));
    let doc = reload(&file);
    assert_eq!(doc["name"], "List  All Users");
    assert_eq!(doc["method"], "GET");
    assert_eq!(doc["url"], "");
    assert_eq!(
        names(&CollectionDir.load(r).unwrap().children),
        ["List  All Users"]
    );
    assert!(matches!(
        CollectionDir.create_request(r, "list all users"),
        Err(AppError::AlreadyExists(n)) if n == "list-all-users.yaml"
    ));
}

#[test]
fn create_folder_uses_the_clean_name() {
    let dir = collection();
    let r = dir.path();
    assert_eq!(
        CollectionDir.create_folder(r, "a/b").unwrap(),
        r.join("a-b")
    );
    assert!(r.join("a-b").is_dir());
    assert!(!r.join("a-b/_folder.yaml").exists());
    assert!(matches!(
        CollectionDir.create_folder(r, "a-b"),
        Err(AppError::AlreadyExists(_))
    ));
}

#[test]
fn names_that_would_hide_are_invalid() {
    let dir = collection();
    let r = dir.path();
    for bad in [".env", "   ", "_folder"] {
        assert!(
            matches!(
                CollectionDir.create_request(r, bad),
                Err(AppError::InvalidName(_))
            ),
            "{bad:?}"
        );
    }
    assert!(matches!(
        CollectionDir.create_folder(r, ".git"),
        Err(AppError::InvalidName(_))
    ));
}

#[test]
fn rename_request_rewrites_name_file_and_order() {
    let dir = collection();
    let r = dir.path();
    write(r, "_folder.yaml", "order:\n  - b\n  - old-one\n");
    write(
        r,
        "old-one.yaml",
        "name: Old one\nmethod: POST\nurl: http://x\ndescription: keep\n",
    );
    let new = CollectionDir
        .rename(&r.join("old-one.yaml"), "New One")
        .unwrap();
    assert_eq!(new, r.join("new-one.yaml"));
    assert!(!r.join("old-one.yaml").exists());
    let doc = reload(&new);
    assert_eq!(doc["name"], "New One");
    assert_eq!(doc["description"], "keep");
    let order = reload(&r.join("_folder.yaml"));
    assert_eq!(order["order"][1], "new-one");
}

#[test]
fn case_only_rename_keeps_the_file() {
    let dir = collection();
    let r = dir.path();
    write(r, "ping.yaml", "name: ping\nmethod: GET\nurl: http://x\n");
    let new = CollectionDir.rename(&r.join("ping.yaml"), "PING").unwrap();
    assert_eq!(new, r.join("ping.yaml"));
    assert_eq!(reload(&new)["name"], "PING");
}

#[test]
fn rename_folder_moves_children_and_refuses_collisions() {
    let dir = collection();
    let r = dir.path();
    write(r, "users/list.yaml", OK);
    write(r, "orders/get.yaml", OK);
    let new = CollectionDir.rename(&r.join("users"), "People").unwrap();
    assert_eq!(new, r.join("People"));
    assert!(r.join("People/list.yaml").exists());
    assert!(matches!(
        CollectionDir.rename(&r.join("orders"), "People"),
        Err(AppError::AlreadyExists(n)) if n == "People"
    ));
    assert!(r.join("orders/get.yaml").exists());
}

#[test]
fn delete_removes_a_file_and_a_full_folder() {
    let dir = collection();
    let r = dir.path();
    write(r, "a.yaml", OK);
    write(r, "users/list.yaml", OK);
    CollectionDir.delete(&r.join("a.yaml")).unwrap();
    CollectionDir.delete(&r.join("users")).unwrap();
    assert!(CollectionDir.load(r).unwrap().children.is_empty());
}

#[test]
fn rename_with_a_conflicted_file_changes_nothing() {
    let dir = collection();
    let r = dir.path();
    write(
        r,
        "old.yaml",
        "<<<<<<< HEAD\nname: a\n=======\nname: b\n>>>>>>> x\n",
    );
    assert!(matches!(
        CollectionDir.rename(&r.join("old.yaml"), "New"),
        Err(AppError::MergeConflict(_))
    ));
    assert!(r.join("old.yaml").exists());
    assert!(!r.join("new.yaml").exists());
}
