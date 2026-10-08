use std::fs;

use data::import_file::read;
use domain::auth::{ApiKeyPlace, Auth};
use domain::collection::Protocol;
use domain::http::{Body, FormField, KeyValue, Method, TextKind};
use domain::import::{
    ImportFormat, ImportItem, ImportWarning, ImportedCollection, ImportedRequest,
};
use serde_json::{Value, json};

fn import_json(doc: &Value) -> ImportedCollection {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("insomnia.json");
    fs::write(&path, doc.to_string()).expect("write");
    read(&path).expect("import")
}

fn v4(resources: Value) -> Value {
    json!({ "_type": "export", "__export_format": 4, "resources": resources })
}

fn ws() -> Value {
    json!({ "_id": "wrk_1", "_type": "workspace", "name": "Shop", "parentId": null })
}

fn req(id: &str, parent: &str, name: &str, sort: f64, extra: Value) -> Value {
    let mut r = json!({
        "_id": id, "_type": "request", "parentId": parent, "name": name,
        "method": "GET", "url": "https://shop.test", "metaSortKey": sort
    });
    r.as_object_mut()
        .expect("object")
        .extend(extra.as_object().expect("object").clone());
    r
}

fn names(items: &[ImportItem]) -> Vec<String> {
    items
        .iter()
        .map(|i| match i {
            ImportItem::Folder { name, .. } => format!("{name}/"),
            ImportItem::Request(r) => r.name.clone(),
        })
        .collect()
}

fn named<'a>(items: &'a [ImportItem], name: &str) -> &'a ImportedRequest {
    fn walk<'a>(items: &'a [ImportItem], name: &str) -> Option<&'a ImportedRequest> {
        items.iter().find_map(|i| match i {
            ImportItem::Request(r) if r.name == name => Some(r),
            ImportItem::Folder { items, .. } => walk(items, name),
            ImportItem::Request(_) => None,
        })
    }
    walk(items, name).unwrap_or_else(|| panic!("no request {name:?}"))
}

#[test]
fn v4_tree_follows_meta_sort_key() {
    let c = import_json(&v4(json!([
        ws(),
        { "_id": "wrk_2", "_type": "workspace", "name": "Other", "parentId": null },
        req("req_b", "fld_1", "B", 2.0, json!({})),
        { "_id": "fld_1", "_type": "request_group", "parentId": "wrk_1", "name": "Users", "metaSortKey": -1 },
        req("req_a", "fld_1", "A", 1.0, json!({})),
        req("req_top", "wrk_1", "Top", -5.0, json!({})),
        req("req_other", "wrk_2", "Elsewhere", 0.0, json!({})),
    ])));
    assert_eq!(c.name, "Shop");
    assert_eq!(c.format, ImportFormat::Insomnia);
    assert_eq!(names(&c.items), ["Top", "Users/"]);
    let ImportItem::Folder { items, .. } = &c.items[1] else {
        panic!("not a folder");
    };
    assert_eq!(names(items), ["A", "B"]);
    assert!(c.warnings.contains(&ImportWarning::OtherWorkspaces(1)));
}

#[test]
fn v4_yaml_export_imports() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("insomnia.yaml");
    fs::write(
        &path,
        "_type: export\n__export_format: 4\nresources:\n  - _id: wrk_1\n    _type: workspace\n    name: Y\n",
    )
    .unwrap();
    assert_eq!(read(&path).unwrap().name, "Y");
}

#[test]
fn requests_map_rows_and_bodies() {
    let c = import_json(&v4(json!([
        ws(),
        req(
            "r1",
            "wrk_1",
            "Query",
            1.0,
            json!({
                "url": "https://shop.test/items?page=2",
                "parameters": [{ "name": "sort", "value": "asc" }, { "name": "off", "value": "1", "disabled": true }],
                "headers": [{ "name": "X-A", "value": "1" }],
                "description": "Lists items"
            })
        ),
        req(
            "r2",
            "wrk_1",
            "Json",
            2.0,
            json!({ "method": "post",
            "body": { "mimeType": "application/json", "text": "{\"a\":1}" } })
        ),
        req(
            "r3",
            "wrk_1",
            "Form",
            3.0,
            json!({ "method": "POST",
            "body": { "mimeType": "application/x-www-form-urlencoded",
                      "params": [{ "name": "u", "value": "ann" }] } })
        ),
        req(
            "r4",
            "wrk_1",
            "Upload",
            4.0,
            json!({ "method": "POST",
            "body": { "mimeType": "multipart/form-data", "params": [
                { "name": "note", "value": "hi" },
                { "name": "pic", "type": "file", "fileName": "/tmp/a.png", "disabled": true }
            ] } })
        ),
        req(
            "r5",
            "wrk_1",
            "Blob",
            5.0,
            json!({ "method": "PUT",
            "body": { "mimeType": "application/octet-stream", "fileName": "/tmp/b.bin" } })
        ),
        req(
            "r6",
            "wrk_1",
            "Gql",
            6.0,
            json!({ "method": "POST",
            "body": { "mimeType": "application/graphql",
                      "text": "{\"query\":\"{ me { id } }\",\"variables\":{\"a\":1}}" } })
        ),
        req("r7", "wrk_1", "Odd", 7.0, json!({ "method": "PURGE" })),
    ])));
    let q = &named(&c.items, "Query");
    assert_eq!(q.request.url, "https://shop.test/items");
    assert_eq!(
        q.request.params,
        vec![KeyValue::new("page", "2"), KeyValue::new("sort", "asc")]
    );
    assert_eq!(q.request.headers, vec![KeyValue::new("X-A", "1")]);
    assert_eq!(q.description.as_deref(), Some("Lists items"));
    assert!(c.warnings.contains(&ImportWarning::DisabledDropped));
    let json = &named(&c.items, "Json").request;
    assert_eq!(json.method, Method::Post);
    assert_eq!(
        json.body,
        Body::Text {
            kind: TextKind::Json,
            text: "{\"a\":1}".into()
        }
    );
    assert_eq!(
        named(&c.items, "Form").request.body,
        Body::Urlencoded(vec![KeyValue::new("u", "ann")])
    );
    assert_eq!(
        named(&c.items, "Upload").request.body,
        Body::FormData(vec![
            FormField::text("note", "hi"),
            FormField {
                enabled: false,
                ..FormField::file("pic", "/tmp/a.png")
            },
        ])
    );
    assert_eq!(
        named(&c.items, "Blob").request.body,
        Body::Binary("/tmp/b.bin".into())
    );
    let gql = named(&c.items, "Gql");
    assert_eq!(gql.protocol, Protocol::Graphql);
    let Body::Text {
        kind: TextKind::Json,
        text,
    } = &gql.request.body
    else {
        panic!("not JSON");
    };
    let v: Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        v,
        json!({ "query": "{ me { id } }", "variables": { "a": 1 } })
    );
    assert!(
        c.warnings
            .contains(&ImportWarning::UnknownMethod("PURGE".into()))
    );
    assert_eq!(c.counts().1, 6);
}

#[test]
fn auth_kinds() {
    let c = import_json(&v4(json!([
        ws(),
        req(
            "r1",
            "wrk_1",
            "Bearer",
            1.0,
            json!({ "authentication": { "type": "bearer", "token": "t" } })
        ),
        req(
            "r2",
            "wrk_1",
            "Token",
            2.0,
            json!({ "authentication": { "type": "bearer", "token": "t", "prefix": "Token" } })
        ),
        req(
            "r3",
            "wrk_1",
            "Basic",
            3.0,
            json!({ "authentication": { "type": "basic", "username": "u", "password": "p" } })
        ),
        req(
            "r4",
            "wrk_1",
            "Key",
            4.0,
            json!({ "authentication": { "type": "apikey", "key": "k", "value": "v", "addTo": "queryParams" } })
        ),
        req(
            "r5",
            "wrk_1",
            "Off",
            5.0,
            json!({ "authentication": { "type": "basic", "username": "u", "disabled": true } })
        ),
        req(
            "r6",
            "wrk_1",
            "OAuth",
            6.0,
            json!({ "authentication": { "type": "oauth2" } })
        ),
    ])));
    assert_eq!(
        named(&c.items, "Bearer").request.auth,
        Some(Auth::Bearer { token: "t".into() })
    );
    let token = &named(&c.items, "Token").request;
    assert_eq!(token.auth, None);
    assert_eq!(
        token.headers,
        vec![KeyValue::new("Authorization", "Token t")]
    );
    assert_eq!(
        named(&c.items, "Basic").request.auth,
        Some(Auth::Basic {
            username: "u".into(),
            password: "p".into()
        })
    );
    assert_eq!(
        named(&c.items, "Key").request.auth,
        Some(Auth::ApiKey {
            key: "k".into(),
            value: "v".into(),
            place: ApiKeyPlace::Query
        })
    );
    assert_eq!(named(&c.items, "Off").request.auth, None);
    assert_eq!(named(&c.items, "OAuth").request.auth, None);
    assert!(
        c.warnings
            .contains(&ImportWarning::UnsupportedAuth("oauth2".into()))
    );
}

#[test]
fn environments_merge_base_under_each_sub() {
    let c = import_json(&v4(json!([
        ws(),
        { "_id": "env_base", "_type": "environment", "parentId": "wrk_1", "name": "Base Environment",
          "data": { "host": "base.test", "api": { "version": 2 }, "tags": ["a"] } },
        { "_id": "env_prod", "_type": "environment", "parentId": "env_base", "name": "Prod",
          "metaSortKey": 2, "data": { "host": "prod.test" } },
        { "_id": "env_dev", "_type": "environment", "parentId": "env_base", "name": "Dev",
          "metaSortKey": 1, "data": { "token": "d" } },
    ])));
    let base = vec![
        ("host".to_owned(), "base.test".to_owned()),
        ("api.version".into(), "2".into()),
        ("tags".into(), "[\"a\"]".into()),
    ];
    let mut dev = base.clone();
    dev.push(("token".into(), "d".into()));
    let mut prod = base.clone();
    prod[0].1 = "prod.test".into();
    assert_eq!(
        c.environments,
        vec![("Dev".into(), dev), ("Prod".into(), prod)]
    );

    let only_base = import_json(&v4(json!([
        ws(),
        { "_id": "env_base", "_type": "environment", "parentId": "wrk_1", "name": "Base Environment",
          "data": { "host": "base.test" } },
    ])));
    assert_eq!(
        only_base.environments,
        vec![("Base".into(), vec![("host".into(), "base.test".into())])]
    );
}

#[test]
fn templates_are_rewritten() {
    let c = import_json(&v4(json!([
        ws(),
        req("r1", "wrk_1", "T", 1.0, json!({
            "url": "{{ _.api.host }}/users/{{_.id}}?x={{ plain }}",
            "headers": [{ "name": "X-Req", "value": "{% response 'body', 'req_1', '$.id' %}" }]
        })),
        { "_id": "env_base", "_type": "environment", "parentId": "wrk_1", "name": "Base",
          "data": { "url": "{{ _.host }}/v1" } },
    ])));
    let r = &named(&c.items, "T").request;
    assert_eq!(r.url, "{{api.host}}/users/{{id}}");
    assert_eq!(r.params, vec![KeyValue::new("x", "{{ plain }}")]);
    assert_eq!(
        c.environments[0].1,
        vec![("url".into(), "{{host}}/v1".into())]
    );
    assert!(c.warnings.contains(&ImportWarning::TemplateTags));
}

#[test]
fn skipped_kinds_folder_settings_and_scripts() {
    let c = import_json(&v4(json!([
        ws(),
        { "_id": "fld_1", "_type": "request_group", "parentId": "wrk_1", "name": "F",
          "authentication": { "type": "bearer", "token": "x" }, "environment": { "a": "1" } },
        { "_id": "g1", "_type": "grpc_request", "parentId": "wrk_1", "name": "G" },
        { "_id": "w1", "_type": "websocket_request", "parentId": "wrk_1", "name": "W" },
        req("r1", "fld_1", "S", 1.0, json!({
            "preRequestScript": "insomnia.environment.set('a', 1)",
            "afterResponseScript": "insomnia.test('ok', () => {})"
        })),
    ])));
    for w in [
        ImportWarning::FolderAuth,
        ImportWarning::FolderVariables,
        ImportWarning::UnsupportedRequest("gRPC".into()),
        ImportWarning::UnsupportedRequest("WebSocket".into()),
        ImportWarning::Scripts,
    ] {
        assert!(c.warnings.contains(&w), "missing {w:?}");
    }
    let s = named(&c.items, "S");
    assert_eq!(
        s.pre_request_script.as_deref(),
        Some("insomnia.environment.set('a', 1)")
    );
    assert_eq!(s.tests.as_deref(), Some("insomnia.test('ok', () => {})"));
    assert_eq!(c.counts(), (1, 1, 0));
}

fn import_yaml(yaml: &str) -> ImportedCollection {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("insomnia.yaml");
    fs::write(&path, yaml).expect("write");
    read(&path).expect("import")
}

#[test]
fn v5_tree_envs_and_skips() {
    let c = import_yaml(
        r#"
type: collection.insomnia.rest/5.0
name: Shop v5
meta: { id: wrk_1 }
collection:
  - name: Users
    meta: { id: fld_1, sortKey: -1 }
    authentication: { type: basic, username: u }
    children:
      - name: B
        meta: { id: req_b, sortKey: 2 }
        method: GET
        url: "{{ _.host }}/b"
      - name: A
        meta: { id: req_a, sortKey: 1 }
        method: POST
        url: https://shop.test/a
        body: { mimeType: application/json, text: '{"x":1}' }
        scripts: { preRequest: "", afterResponse: "insomnia.test('ok')" }
  - name: Top
    meta: { id: req_top, sortKey: -5 }
    method: GET
    url: https://shop.test/top
  - name: Stream
    meta: { id: ws-req_1, sortKey: 3 }
    url: wss://shop.test
  - name: Rpc
    meta: { id: greq_1, sortKey: 4 }
environments:
  name: Base Environment
  data: { host: https://base.test }
  subEnvironments:
    - name: Prod
      meta: { sortKey: 1 }
      data: { host: https://prod.test }
"#,
    );
    assert_eq!(c.name, "Shop v5");
    assert_eq!(c.format, ImportFormat::Insomnia);
    assert_eq!(names(&c.items), ["Top", "Users/"]);
    let ImportItem::Folder { items, .. } = &c.items[1] else {
        panic!("not a folder");
    };
    assert_eq!(names(items), ["A", "B"]);
    assert_eq!(named(&c.items, "B").request.url, "{{host}}/b");
    let a = named(&c.items, "A");
    assert_eq!(
        a.request.body,
        Body::Text {
            kind: TextKind::Json,
            text: "{\"x\":1}".into()
        }
    );
    assert_eq!(a.pre_request_script, None);
    assert_eq!(a.tests.as_deref(), Some("insomnia.test('ok')"));
    assert_eq!(
        c.environments,
        vec![(
            "Prod".into(),
            vec![("host".into(), "https://prod.test".into())]
        )]
    );
    for w in [
        ImportWarning::FolderAuth,
        ImportWarning::UnsupportedRequest("WebSocket".into()),
        ImportWarning::UnsupportedRequest("gRPC".into()),
        ImportWarning::Scripts,
    ] {
        assert!(c.warnings.contains(&w), "missing {w:?}");
    }
}

#[test]
fn real_v5_export_imports() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/insomnia_v5.yaml");
    let c = read(&path).unwrap();
    assert_eq!(c.format, ImportFormat::Insomnia);
    assert_eq!(c.name, "simple");
    // 1 folder; 5 requests (grpc and websocket skipped); production + staging
    assert_eq!(c.counts(), (1, 5, 2));
}

#[test]
fn group_without_ids_does_not_recurse_forever() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("a.json");
    fs::write(
        &path,
        v4(json!([
            { "_type": "workspace", "name": "W" },
            { "_type": "request_group", "name": "F" }
        ]))
        .to_string(),
    )
    .expect("write");
    let _ = read(&path);
}

#[test]
fn group_reusing_an_ancestor_id_does_not_recurse_forever() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("b.json");
    fs::write(
        &path,
        v4(json!([
            { "_id": "wrk", "_type": "workspace", "name": "W" },
            { "_id": "wrk", "_type": "request_group", "parentId": "wrk", "name": "F" }
        ]))
        .to_string(),
    )
    .expect("write");
    let _ = read(&path);
}
