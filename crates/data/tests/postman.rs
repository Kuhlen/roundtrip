use std::fs;
use std::path::PathBuf;

use data::postman::read;
use domain::AppError;
use domain::auth::{ApiKeyPlace, Auth};
use domain::collection::Protocol;
use domain::http::{Body, FormField, KeyValue, Method, TextKind};
use domain::import::{ImportItem, ImportWarning, ImportedCollection, ImportedRequest};
use serde_json::json;

fn import(doc: serde_json::Value) -> Result<ImportedCollection, AppError> {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("c.json");
    fs::write(&path, doc.to_string()).expect("write");
    read(&path)
}

fn collection(items: serde_json::Value) -> serde_json::Value {
    json!({
        "info": { "name": "Shop", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json" },
        "item": items
    })
}

fn only_request(c: &ImportedCollection) -> &ImportedRequest {
    match &c.items[0] {
        ImportItem::Request(r) => r,
        other => panic!("not a request: {other:?}"),
    }
}

#[test]
fn nested_folders_keep_order() {
    let c = import(collection(json!([
        { "name": "Users", "item": [
            { "name": "B", "request": { "method": "GET", "url": "https://s.test/b" } },
            { "name": "A", "request": { "method": "GET", "url": "https://s.test/a" } }
        ]},
        { "name": "Health", "request": { "method": "GET", "url": "https://s.test/h" } }
    ])))
    .unwrap();
    assert_eq!(c.name, "Shop");
    let ImportItem::Folder { name, items } = &c.items[0] else {
        panic!("first item is not a folder");
    };
    assert_eq!(name, "Users");
    let names: Vec<&str> = items
        .iter()
        .map(|i| match i {
            ImportItem::Request(r) => r.name.as_str(),
            ImportItem::Folder { name, .. } => name.as_str(),
        })
        .collect();
    assert_eq!(names, ["B", "A"]);
    assert_eq!(c.counts(), (1, 3, 0));
}

#[test]
fn url_object_query_and_path_variables() {
    let c = import(collection(json!([{ "name": "Get", "request": {
        "method": "GET",
        "url": {
            "raw": "{{baseUrl}}/users/:id?page=2&off=1",
            "host": ["{{baseUrl}}"], "path": ["users", ":id"],
            "query": [{ "key": "page", "value": "2" }, { "key": "off", "value": "1", "disabled": true }],
            "variable": [{ "key": "id", "value": "7" }]
        },
        "header": [{ "key": "X-A", "value": "1" }, { "key": "X-Off", "value": "0", "disabled": true }]
    }}])))
    .unwrap();
    let r = &only_request(&c).request;
    assert_eq!(r.url, "{{baseUrl}}/users/:id");
    assert_eq!(r.params, vec![KeyValue::new("page", "2")]);
    assert_eq!(r.headers, vec![KeyValue::new("X-A", "1")]);
    assert!(c.warnings.contains(&ImportWarning::PathVariables));
    assert!(c.warnings.contains(&ImportWarning::DisabledDropped));
}

#[test]
fn url_rebuilt_from_parts_when_raw_is_missing() {
    let c = import(collection(json!([{ "name": "Get", "request": {
        "method": "GET",
        "url": { "protocol": "https", "host": ["api", "shop", "test"], "path": ["v1", "items"] }
    }}])))
    .unwrap();
    assert_eq!(
        only_request(&c).request.url,
        "https://api.shop.test/v1/items"
    );
}

#[test]
fn bodies() {
    let body = |b: serde_json::Value| {
        let c = import(collection(json!([{ "name": "P", "request": {
            "method": "POST", "url": "https://s.test", "body": b
        }}])))
        .unwrap();
        let r = only_request(&c).clone();
        (r.request.body, r.protocol, c.warnings)
    };
    assert_eq!(
        body(json!({ "mode": "raw", "raw": "{\"a\":1}", "options": { "raw": { "language": "json" } } })).0,
        Body::Text { kind: TextKind::Json, text: "{\"a\":1}".into() }
    );
    assert_eq!(
        body(json!({ "mode": "raw", "raw": "<a/>", "options": { "raw": { "language": "xml" } } }))
            .0,
        Body::Text {
            kind: TextKind::Xml,
            text: "<a/>".into()
        }
    );
    assert_eq!(
        body(json!({ "mode": "raw", "raw": "hi" })).0,
        Body::Text {
            kind: TextKind::Raw,
            text: "hi".into()
        }
    );
    assert_eq!(
        body(json!({ "mode": "urlencoded", "urlencoded": [{ "key": "a", "value": "1" }] })).0,
        Body::Urlencoded(vec![KeyValue::new("a", "1")])
    );
    assert_eq!(
        body(json!({ "mode": "formdata", "formdata": [
            { "key": "name", "value": "Ayu", "type": "text" },
            { "key": "avatar", "src": "/tmp/a.png", "type": "file" },
            { "key": "off", "value": "x", "type": "text", "disabled": true }
        ] }))
        .0,
        Body::FormData(vec![
            FormField::text("name", "Ayu"),
            FormField::file("avatar", "/tmp/a.png"),
            FormField {
                enabled: false,
                ..FormField::text("off", "x")
            },
        ])
    );
    assert_eq!(
        body(json!({ "mode": "file", "file": { "src": "/tmp/b.bin" } })).0,
        Body::Binary("/tmp/b.bin".into())
    );
    let (gql, protocol, _) =
        body(json!({ "mode": "graphql", "graphql": { "query": "{ me }", "variables": "" } }));
    assert_eq!(protocol, Protocol::Graphql);
    assert!(
        matches!(gql, Body::Text { kind: TextKind::Json, ref text } if text.contains("\"query\""))
    );
    let (none, _, warnings) = body(json!({ "mode": "nonsense" }));
    assert_eq!(none, Body::None);
    assert!(warnings.contains(&ImportWarning::UnsupportedBody("nonsense".into())));
}

#[test]
fn auth_levels() {
    let c = import(json!({
        "info": { "name": "A" },
        "auth": { "type": "bearer", "bearer": [{ "key": "token", "value": "{{token}}" }] },
        "item": [
            { "name": "Basic", "request": { "method": "GET", "url": "https://a.test",
              "auth": { "type": "basic", "basic": [{ "key": "username", "value": "u" }, { "key": "password", "value": "p" }] } } },
            { "name": "Key", "request": { "method": "GET", "url": "https://a.test",
              "auth": { "type": "apikey", "apikey": [{ "key": "key", "value": "X-Key" }, { "key": "value", "value": "k" }, { "key": "in", "value": "query" }] } } },
            { "name": "None", "request": { "method": "GET", "url": "https://a.test", "auth": { "type": "noauth" } } },
            { "name": "OAuth", "request": { "method": "GET", "url": "https://a.test", "auth": { "type": "oauth2" } } },
            { "name": "F", "auth": { "type": "basic" }, "item": [] }
        ]
    }))
    .unwrap();
    assert_eq!(
        c.auth,
        Some(Auth::Bearer {
            token: "{{token}}".into()
        })
    );
    let auths: Vec<Option<Auth>> = c
        .items
        .iter()
        .filter_map(|i| match i {
            ImportItem::Request(r) => Some(r.request.auth.clone()),
            ImportItem::Folder { .. } => None,
        })
        .collect();
    assert_eq!(
        auths,
        vec![
            Some(Auth::Basic {
                username: "u".into(),
                password: "p".into()
            }),
            Some(Auth::ApiKey {
                key: "X-Key".into(),
                value: "k".into(),
                place: ApiKeyPlace::Query
            }),
            None,
            None,
        ]
    );
    assert!(c.warnings.contains(&ImportWarning::NoAuthInherits));
    assert!(
        c.warnings
            .contains(&ImportWarning::UnsupportedAuth("oauth2".into()))
    );
    assert!(c.warnings.contains(&ImportWarning::FolderAuth));
}

#[test]
fn v2_0_object_auth_and_string_url() {
    let c = import(collection(json!([
        { "name": "Old", "request": { "method": "GET", "url": "https://o.test/x?y=1",
          "auth": { "type": "bearer", "bearer": { "token": "abc" } } } },
        { "name": "Bare", "request": "https://o.test/bare" }
    ])))
    .unwrap();
    let ImportItem::Request(old) = &c.items[0] else {
        panic!("not a request")
    };
    assert_eq!(old.request.url, "https://o.test/x?y=1");
    assert_eq!(
        old.request.auth,
        Some(Auth::Bearer {
            token: "abc".into()
        })
    );
    let ImportItem::Request(bare) = &c.items[1] else {
        panic!("not a request")
    };
    assert_eq!(bare.request.method, Method::Get);
    assert_eq!(bare.request.url, "https://o.test/bare");
}

#[test]
fn variables_scripts_description_and_unknown_method() {
    let c = import(json!({
        "info": { "name": "V" },
        "variable": [{ "key": "baseUrl", "value": "https://v.test" }, { "key": "off", "value": "1", "disabled": true }],
        "item": [
            { "name": "S", "event": [
                { "listen": "prerequest", "script": { "exec": ["const a = 1;", "pm.environment.set('a', a);"] } },
                { "listen": "test", "script": { "exec": "pm.test('ok')" } }
              ],
              "request": { "method": "GET", "url": "https://v.test", "description": { "content": "Hello" } } },
            { "name": "Q", "request": { "method": "PROPFIND", "url": "https://v.test" } }
        ]
    }))
    .unwrap();
    assert_eq!(
        c.environments,
        vec![(
            "Collection Variables".to_owned(),
            vec![("baseUrl".to_owned(), "https://v.test".to_owned())]
        )]
    );
    assert_eq!(c.items.len(), 1);
    let r = only_request(&c);
    assert_eq!(
        r.pre_request_script.as_deref(),
        Some("const a = 1;\npm.environment.set('a', a);")
    );
    assert_eq!(r.tests.as_deref(), Some("pm.test('ok')"));
    assert_eq!(r.description.as_deref(), Some("Hello"));
    assert!(c.warnings.contains(&ImportWarning::Scripts));
    assert!(
        c.warnings
            .contains(&ImportWarning::UnknownMethod("PROPFIND".into()))
    );
}

#[test]
fn warnings_are_one_per_occurrence() {
    let item = json!({ "name": "O", "request": { "method": "GET", "url": "https://o.test", "auth": { "type": "oauth2" } } });
    let c = import(collection(json!([item.clone(), item]))).unwrap();
    let n = c
        .warnings
        .iter()
        .filter(|w| **w == ImportWarning::UnsupportedAuth("oauth2".into()))
        .count();
    assert_eq!(n, 2);
}

#[test]
fn not_postman_is_an_error() {
    assert!(matches!(
        import(json!({ "openapi": "3.0.0" })),
        Err(AppError::Import(_))
    ));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.json");
    fs::write(&path, "not json").unwrap();
    assert!(matches!(read(&path), Err(AppError::Import(_))));
    assert!(matches!(
        read(&PathBuf::from("/nope/x.json")),
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

#[test]
fn root_and_folder_scripts_warn_once_each() {
    let script = json!([{ "listen": "prerequest", "script": { "exec": ["x()"] } }]);
    let mut doc = collection(json!([
        { "name": "F", "event": script.clone(), "item": [
            { "name": "R", "request": { "method": "GET", "url": "https://s.test" } }
        ]}
    ]));
    doc["event"] = script;
    let c = import(doc).unwrap();
    let n = c
        .warnings
        .iter()
        .filter(|w| **w == ImportWarning::CollectionScriptsDropped)
        .count();
    assert_eq!(n, 2);
}
