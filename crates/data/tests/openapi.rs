use std::fs;

use data::import_file::read;
use domain::auth::{ApiKeyPlace, Auth};
use domain::http::{Body, FormField, KeyValue, Method, TextKind};
use domain::import::{
    ImportFormat, ImportItem, ImportWarning, ImportedCollection, ImportedRequest,
};
use serde_json::{Value, json};

fn import(yaml: &str) -> ImportedCollection {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("spec.yaml");
    fs::write(&path, yaml).expect("write");
    read(&path).expect("import")
}

fn requests(items: &[ImportItem]) -> Vec<&ImportedRequest> {
    items
        .iter()
        .flat_map(|i| match i {
            ImportItem::Request(r) => vec![r],
            ImportItem::Folder { items, .. } => requests(items),
        })
        .collect()
}

fn named<'a>(c: &'a ImportedCollection, name: &str) -> &'a ImportedRequest {
    requests(&c.items)
        .into_iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no request {name:?}"))
}

fn json_body(r: &ImportedRequest) -> Value {
    match &r.request.body {
        Body::Text {
            kind: TextKind::Json,
            text,
        } => serde_json::from_str(text).expect("json"),
        other => panic!("not a JSON body: {other:?}"),
    }
}

#[test]
fn yaml_spec_with_status_code_keys_imports() {
    let c = import(
        r#"
openapi: 3.1.0
info: { title: Petstore }
paths:
  /pets:
    get:
      summary: List pets
      responses:
        200:
          description: ok
"#,
    );
    assert_eq!(c.name, "Petstore");
    assert_eq!(c.format, ImportFormat::OpenApi("3.1.0".into()));
    assert_eq!(named(&c, "List pets").request.url, "{{baseUrl}}/pets");
}

#[test]
fn json_spec_and_numeric_version_import() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("spec.json");
    fs::write(
        &path,
        r#"{"openapi":"3.0.3","info":{"title":"J"},"paths":{}}"#,
    )
    .unwrap();
    assert_eq!(
        read(&path).unwrap().format,
        ImportFormat::OpenApi("3.0.3".into())
    );
    let c = import("openapi: 3.0\ninfo: {}\npaths: {}\n");
    assert_eq!(c.format, ImportFormat::OpenApi("3.0".into()));
    assert_eq!(c.name, "Imported API");
}

#[test]
fn servers_become_environments() {
    let c = import(
        r#"
openapi: 3.0.0
info: { title: S }
servers:
  - url: https://{region}.api.test/v1/
    description: Production
    variables:
      region: { default: eu }
  - url: https://staging.api.test
  - url: https://other.test
    description: Production
paths:
  /users:
    get: { operationId: listUsers }
"#,
    );
    assert_eq!(
        c.environments,
        vec![
            (
                "Production".into(),
                vec![("baseUrl".into(), "https://eu.api.test/v1".into())]
            ),
            (
                "https://staging.api.test".into(),
                vec![("baseUrl".into(), "https://staging.api.test".into())]
            ),
            (
                "Production 2".into(),
                vec![("baseUrl".into(), "https://other.test".into())]
            ),
        ]
    );
    assert_eq!(named(&c, "listUsers").request.url, "{{baseUrl}}/users");
    assert!(
        import("openapi: 3.0.0\ninfo: {}\npaths: {}\n")
            .environments
            .is_empty()
    );
}

#[test]
fn names_fall_back_and_path_params_stay() {
    let c = import(
        r#"
openapi: 3.0.0
info: { title: N }
paths:
  /users/{id}:
    get: { summary: Get user, operationId: getUser, description: One user }
    delete: { operationId: deleteUser }
    put: {}
    trace: {}
"#,
    );
    let get = named(&c, "Get user");
    assert_eq!(get.request.url, "{{baseUrl}}/users/{id}");
    assert_eq!(get.description.as_deref(), Some("One user"));
    assert_eq!(named(&c, "deleteUser").request.method, Method::Delete);
    assert_eq!(named(&c, "PUT /users/{id}").request.method, Method::Put);
    assert!(c.warnings.contains(&ImportWarning::PathVariables));
    assert!(
        c.warnings
            .contains(&ImportWarning::UnknownMethod("TRACE".into()))
    );
    assert_eq!(c.counts(), (0, 3, 0));
}

#[test]
fn folders_follow_tag_order() {
    let c = import(
        r#"
openapi: 3.0.0
info: { title: T }
tags: [{ name: pets }, { name: empty }, { name: users }]
paths:
  /health:
    get: { operationId: health }
  /users:
    get: { operationId: listUsers, tags: [users] }
  /pets:
    get: { operationId: listPets, tags: [pets, users] }
  /orders:
    get: { operationId: listOrders, tags: [orders] }
"#,
    );
    let names: Vec<&str> = c
        .items
        .iter()
        .map(|i| match i {
            ImportItem::Folder { name, .. } => name.as_str(),
            ImportItem::Request(r) => r.name.as_str(),
        })
        .collect();
    assert_eq!(names, ["pets", "users", "orders", "health"]);
}

#[test]
fn params_merge_and_take_examples() {
    let c = import(
        r#"
openapi: 3.0.0
info: { title: P }
components:
  parameters:
    Trace: { name: X-Trace, in: header, schema: { default: abc } }
paths:
  /items:
    parameters:
      - { name: limit, in: query, example: 5 }
      - { name: X-Tenant, in: header, example: acme }
    get:
      operationId: list
      parameters:
        - { name: limit, in: query, example: 10 }
        - { name: sort, in: query, schema: { type: string } }
        - { name: session, in: cookie }
        - $ref: '#/components/parameters/Trace'
"#,
    );
    let r = &named(&c, "list").request;
    assert_eq!(
        r.params,
        vec![KeyValue::new("limit", "10"), KeyValue::new("sort", "")]
    );
    assert_eq!(
        r.headers,
        vec![
            KeyValue::new("X-Tenant", "acme"),
            KeyValue::new("X-Trace", "abc")
        ]
    );
    assert!(c.warnings.contains(&ImportWarning::CookieParams));
}

#[test]
fn bodies_by_media_type() {
    let c = import(
        r#"
openapi: 3.0.0
info: { title: B }
paths:
  /json:
    post:
      operationId: json
      requestBody:
        content:
          application/json:
            examples:
              first: { value: { name: Rex } }
  /vnd:
    post:
      operationId: vnd
      requestBody:
        content:
          application/vnd.api+json; charset=utf-8:
            example: { data: 1 }
  /xml:
    post:
      operationId: xml
      requestBody:
        content:
          text/xml:
            example: "<a/>"
  /form:
    post:
      operationId: form
      requestBody:
        content:
          application/x-www-form-urlencoded:
            schema:
              properties:
                user: { type: string, example: ann }
                age: { type: integer, default: 3 }
  /upload:
    post:
      operationId: upload
      requestBody:
        content:
          multipart/form-data:
            schema:
              properties:
                note: { type: string }
                file: { type: string, format: binary }
  /bin:
    put:
      operationId: bin
      requestBody:
        content:
          application/octet-stream: {}
  /text:
    post:
      operationId: text
      requestBody:
        content:
          text/plain:
            example: hello
"#,
    );
    assert_eq!(json_body(named(&c, "json")), json!({ "name": "Rex" }));
    let vnd = named(&c, "vnd");
    assert_eq!(json_body(vnd), json!({ "data": 1 }));
    assert_eq!(
        vnd.request.headers,
        vec![KeyValue::new("Content-Type", "application/vnd.api+json")]
    );
    let xml = &named(&c, "xml").request;
    assert_eq!(
        xml.body,
        Body::Text {
            kind: TextKind::Xml,
            text: "<a/>".into()
        }
    );
    assert_eq!(xml.headers, vec![KeyValue::new("Content-Type", "text/xml")]);
    assert_eq!(
        named(&c, "form").request.body,
        Body::Urlencoded(vec![
            KeyValue::new("user", "ann"),
            KeyValue::new("age", "3")
        ])
    );
    assert_eq!(
        named(&c, "upload").request.body,
        Body::FormData(vec![
            FormField::text("note", ""),
            FormField::file("file", "")
        ])
    );
    assert_eq!(named(&c, "bin").request.body, Body::Binary(String::new()));
    let text = &named(&c, "text").request;
    assert_eq!(
        text.body,
        Body::Text {
            kind: TextKind::Raw,
            text: "hello".into()
        }
    );
    assert!(text.headers.is_empty(), "text/plain is the Raw default");
}

#[test]
fn schema_sample_matches_upstream() {
    let c = import(
        r#"
openapi: 3.0.0
info: { title: S }
components:
  schemas:
    Tag: { type: object, properties: { label: { type: string } } }
paths:
  /pets:
    post:
      operationId: create
      requestBody:
        content:
          application/json:
            schema:
              type: object
              properties:
                id: { type: integer }
                price: { type: number }
                ok: { type: boolean }
                email: { type: string, format: email }
                at: { type: string, format: date-time }
                day: { type: string, format: date }
                site: { type: string, format: uri }
                key: { type: string, format: uuid }
                nick: { type: [string, "null"] }
                kind: { type: string, default: dog }
                tags: { type: array, items: { $ref: '#/components/schemas/Tag' } }
"#,
    );
    assert_eq!(
        json_body(named(&c, "create")),
        json!({
            "id": 0, "price": 0.0, "ok": false,
            "email": "user@example.com", "at": "2024-01-01T00:00:00Z", "day": "2024-01-01",
            "site": "https://example.com", "key": "550e8400-e29b-41d4-a716-446655440000",
            "nick": "string", "kind": "dog",
            "tags": [{ "label": "string" }]
        })
    );
}

#[test]
fn recursive_schema_stops() {
    let c = import(
        r#"
openapi: 3.0.0
info: { title: R }
components:
  schemas:
    Node:
      type: object
      properties:
        children: { type: array, items: { $ref: '#/components/schemas/Node' } }
paths:
  /tree:
    post:
      operationId: tree
      requestBody:
        content:
          application/json:
            schema: { $ref: '#/components/schemas/Node' }
"#,
    );
    let body = json_body(named(&c, "tree"));
    assert!(body["children"][0]["children"].is_array());
}

#[test]
fn external_refs_warn() {
    let c = import(
        r#"
openapi: 3.0.0
info: { title: E }
paths:
  /x:
    post:
      operationId: x
      requestBody: { $ref: 'common.yaml#/components/requestBodies/X' }
"#,
    );
    assert_eq!(named(&c, "x").request.body, Body::None);
    assert!(c.warnings.contains(&ImportWarning::ExternalRef));
}

#[test]
fn security_maps_to_auth_placeholders() {
    let c = import(
        r#"
openapi: 3.0.0
info: { title: A }
security: [{ bearer: [] }]
components:
  securitySchemes:
    bearer: { type: http, scheme: bearer }
    basic: { type: http, scheme: basic }
    key: { type: apiKey, in: query, name: api_key }
    oauth: { type: oauth2, flows: {} }
paths:
  /a:
    get: { operationId: inherits }
  /b:
    get: { operationId: basic, security: [{ basic: [] }] }
  /c:
    get: { operationId: key, security: [{ key: [] }, { basic: [] }] }
  /d:
    get: { operationId: open, security: [] }
  /e:
    get: { operationId: oauth, security: [{ oauth: [read] }] }
  /f:
    get: { operationId: same, security: [{ bearer: [] }] }
"#,
    );
    assert_eq!(
        c.auth,
        Some(Auth::Bearer {
            token: "{{token}}".into()
        })
    );
    assert_eq!(named(&c, "inherits").request.auth, None);
    assert_eq!(named(&c, "same").request.auth, None);
    assert_eq!(
        named(&c, "basic").request.auth,
        Some(Auth::Basic {
            username: "{{username}}".into(),
            password: "{{password}}".into()
        })
    );
    assert_eq!(
        named(&c, "key").request.auth,
        Some(Auth::ApiKey {
            key: "api_key".into(),
            value: "{{apiKey}}".into(),
            place: ApiKeyPlace::Query
        })
    );
    assert_eq!(named(&c, "open").request.auth, None);
    assert!(c.warnings.contains(&ImportWarning::NoAuthInherits));
    assert!(
        c.warnings
            .contains(&ImportWarning::UnsupportedAuth("oauth2".into()))
    );
}

#[test]
fn wide_self_referencing_schema_finishes_fast() {
    let props: String = (0..300)
        .map(|i| format!("        p{i}: {{ $ref: '#/components/schemas/A' }}\n"))
        .collect();
    let c = import(&format!(
        "openapi: 3.0.0\ninfo: {{ title: S }}\npaths:\n  /a:\n    post:\n      operationId: go\n      requestBody:\n        content:\n          application/json:\n            schema: {{ $ref: '#/components/schemas/A' }}\ncomponents:\n  schemas:\n    A:\n      type: object\n      properties:\n{props}"
    ));
    assert!(json_body(named(&c, "go")).is_object());
}
