use std::fs;
use std::path::{Path, PathBuf};

use data::collection_dir::CollectionDir;
use data::import_file::read;
use data::postman_export::write;
use domain::AppError;
use domain::auth::{ApiKeyPlace, Auth};
use domain::collection::{CollectionStore, Protocol};
use domain::graphql::GraphqlBody;
use domain::http::{Body, FormField, KeyValue, Method, Request, TextKind};
use domain::import::{
    ExportReport, ExportWarning, ImportItem, ImportedCollection, ImportedRequest,
};
use serde_json::Value;

fn item(name: &str, request: Request) -> ImportItem {
    ImportItem::Request(ImportedRequest {
        name: name.into(),
        request,
        ..ImportedRequest::default()
    })
}

fn graphql_text() -> String {
    data::graphql::to_json(&GraphqlBody {
        query: "{ me { id } }".into(),
        variables: "{\"a\":1}".into(),
        operation_name: String::new(),
    })
    .expect("graphql json")
}

/// On disk: folder Users (Create, Upload), then Search (GraphQL), Blob, Form, Msgpack,
/// OAuth, Socket and Broken; one environment.
fn shop(parent: &Path) -> PathBuf {
    let create = Request {
        method: Method::Post,
        url: "https://shop.test/users".into(),
        params: vec![KeyValue::new("page", "2")],
        headers: vec![KeyValue::new("X-Trace", "1")],
        body: Body::Text {
            kind: TextKind::Json,
            text: "{\"name\":\"ann\"}".into(),
        },
        auth: Some(Auth::Basic {
            username: "u".into(),
            password: "p".into(),
        }),
    };
    let upload = Request {
        method: Method::Post,
        url: "https://shop.test/upload".into(),
        body: Body::FormData(vec![
            FormField::text("note", "hi"),
            FormField {
                enabled: false,
                ..FormField::text("off", "x")
            },
            FormField::file("pic", "avatar.png"),
        ]),
        ..Request::default()
    };
    let data = ImportedCollection {
        name: "Shop".into(),
        auth: Some(Auth::Bearer {
            token: "{{token}}".into(),
        }),
        items: vec![
            ImportItem::Folder {
                name: "Users".into(),
                items: vec![
                    ImportItem::Request(ImportedRequest {
                        name: "Create".into(),
                        request: create,
                        description: Some("Makes a user".into()),
                        pre_request_script: Some("a()\nb()".into()),
                        tests: Some("t()".into()),
                        ..ImportedRequest::default()
                    }),
                    item("Upload", upload),
                ],
            },
            ImportItem::Request(ImportedRequest {
                name: "Search".into(),
                protocol: Protocol::Graphql,
                request: Request {
                    method: Method::Post,
                    url: "https://shop.test/graphql".into(),
                    body: Body::Text {
                        kind: TextKind::Json,
                        text: graphql_text(),
                    },
                    ..Request::default()
                },
                ..ImportedRequest::default()
            }),
            item(
                "Blob",
                Request {
                    method: Method::Put,
                    url: "https://shop.test/blob".into(),
                    body: Body::Binary("data/blob.bin".into()),
                    ..Request::default()
                },
            ),
            item(
                "Form",
                Request {
                    method: Method::Post,
                    url: "https://shop.test/form".into(),
                    body: Body::Urlencoded(vec![KeyValue::new("u", "ann")]),
                    auth: Some(Auth::ApiKey {
                        key: "k".into(),
                        value: "v".into(),
                        place: ApiKeyPlace::Query,
                    }),
                    ..Request::default()
                },
            ),
        ],
        environments: vec![("Dev".into(), vec![("token".into(), "t".into())])],
        ..ImportedCollection::default()
    };
    let root = CollectionDir
        .create_collection(parent, &data)
        .expect("collection");
    // shapes the writer cannot produce
    fs::write(root.join("msgpack.yaml"), "name: Msgpack\nmethod: POST\nurl: https://shop.test/m\nbody:\n  type: msgpack\n  content: x\n").expect("write");
    fs::write(
        root.join("oauth.yaml"),
        "name: OAuth\nmethod: GET\nurl: https://shop.test/o\nauth:\n  type: oauth2\n",
    )
    .expect("write");
    fs::write(
        root.join("socket.yaml"),
        "name: Socket\nprotocol: websocket\nmethod: GET\nurl: wss://shop.test\n",
    )
    .expect("write");
    fs::write(root.join("broken.yaml"), "name: Broken\nmethod: GET\n").expect("write");
    root
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

#[test]
fn export_round_trips_through_the_postman_import() {
    let dir = tempfile::tempdir().unwrap();
    let root = shop(dir.path());
    let dest = dir.path().join("shop.postman_collection.json");
    let report = write(&root, &dest).unwrap();
    assert_eq!(
        report,
        ExportReport {
            requests: 7,
            warnings: vec![
                ExportWarning::SkippedProtocol(1),
                ExportWarning::Unreadable(1),
                ExportWarning::UnsupportedBody(1),
                ExportWarning::UnsupportedAuth(1),
                ExportWarning::EnvironmentsNotExported(1),
            ],
        }
    );
    let back = read(&dest).unwrap();
    assert_eq!(back.name, "Shop");
    assert_eq!(
        back.auth,
        Some(Auth::Bearer {
            token: "{{token}}".into()
        })
    );
    let ImportItem::Folder { name, items } = &back.items[0] else {
        panic!("folder first");
    };
    assert_eq!(name, "Users");
    let create = match &items[0] {
        ImportItem::Request(r) => r,
        other => panic!("not a request: {other:?}"),
    };
    assert_eq!(create.name, "Create");
    assert_eq!(create.request.params, vec![KeyValue::new("page", "2")]);
    assert_eq!(create.request.headers, vec![KeyValue::new("X-Trace", "1")]);
    assert_eq!(
        create.request.body,
        Body::Text {
            kind: TextKind::Json,
            text: "{\"name\":\"ann\"}".into()
        }
    );
    assert_eq!(
        create.request.auth,
        Some(Auth::Basic {
            username: "u".into(),
            password: "p".into()
        })
    );
    assert_eq!(create.description.as_deref(), Some("Makes a user"));
    assert_eq!(create.pre_request_script.as_deref(), Some("a()\nb()"));
    assert_eq!(create.tests.as_deref(), Some("t()"));

    let all = requests(&back.items);
    let by = |n: &str| {
        *all.iter()
            .find(|r| r.name == n)
            .unwrap_or_else(|| panic!("no {n}"))
    };
    let pic = root.join("avatar.png").display().to_string();
    assert_eq!(
        by("Upload").request.body,
        Body::FormData(vec![
            FormField::text("note", "hi"),
            FormField {
                enabled: false,
                ..FormField::text("off", "x")
            },
            FormField::file("pic", pic),
        ])
    );
    let search = by("Search");
    assert_eq!(search.protocol, Protocol::Graphql);
    assert_eq!(
        search.request.body,
        Body::Text {
            kind: TextKind::Json,
            text: graphql_text()
        }
    );
    assert_eq!(
        by("Blob").request.body,
        Body::Binary(root.join("data/blob.bin").display().to_string())
    );
    let form = &by("Form").request;
    assert_eq!(form.body, Body::Urlencoded(vec![KeyValue::new("u", "ann")]));
    assert_eq!(
        form.auth,
        Some(Auth::ApiKey {
            key: "k".into(),
            value: "v".into(),
            place: ApiKeyPlace::Query
        })
    );
    assert_eq!(by("Msgpack").request.body, Body::None);
    assert_eq!(by("OAuth").request.auth, None);
    assert!(all.iter().all(|r| r.name != "Socket" && r.name != "Broken"));
}

#[test]
fn file_is_postman_v2_1_with_query_in_raw() {
    let dir = tempfile::tempdir().unwrap();
    let root = shop(dir.path());
    let dest = dir.path().join("out.json");
    write(&root, &dest).unwrap();
    let doc: Value = serde_json::from_str(&fs::read_to_string(&dest).unwrap()).unwrap();
    assert_eq!(
        doc["info"]["schema"],
        "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
    );
    let url = &doc["item"][0]["item"][0]["request"]["url"];
    assert_eq!(url["raw"], "https://shop.test/users?page=2");
    assert_eq!(url["query"][0]["key"], "page");
}

#[test]
fn export_replaces_an_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = shop(dir.path());
    let dest = dir.path().join("out.json");
    fs::write(&dest, "x".repeat(1_000_000)).unwrap();
    write(&root, &dest).unwrap();
    let text = fs::read_to_string(&dest).unwrap();
    assert!(
        serde_json::from_str::<Value>(&text).is_ok(),
        "valid JSON, no leftover bytes"
    );
    assert!(!dir.path().join("out.json.tmp").exists());
}

#[test]
fn unwritable_destination_leaves_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = shop(dir.path());
    let dest = dir.path().join("missing").join("out.json");
    assert!(matches!(write(&root, &dest), Err(AppError::Storage(_))));
    assert!(!dest.exists());
    assert!(!dir.path().join("missing").exists());
}

#[test]
fn not_a_collection_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("out.json");
    assert!(matches!(
        write(dir.path(), &dest),
        Err(AppError::NotACollection(_))
    ));
    assert!(!dest.exists());
}

#[test]
fn url_with_own_query_keeps_it_in_query_pairs() {
    let dir = tempfile::tempdir().unwrap();
    let data = ImportedCollection {
        name: "Q".into(),
        items: vec![item(
            "Find",
            Request {
                url: "https://x.test/a?v=1".into(),
                params: vec![KeyValue::new("page", "2")],
                ..Request::default()
            },
        )],
        ..ImportedCollection::default()
    };
    let root = CollectionDir.create_collection(dir.path(), &data).unwrap();
    let dest = dir.path().join("out.json");
    write(&root, &dest).unwrap();
    let doc: Value = serde_json::from_str(&fs::read_to_string(&dest).unwrap()).unwrap();
    let url = &doc["item"][0]["request"]["url"];
    assert_eq!(url["raw"], "https://x.test/a?v=1&page=2");
    assert_eq!(url["query"][0]["key"], "v");
    assert_eq!(url["query"][1]["key"], "page");
    let back = read(&dest).unwrap();
    let ImportItem::Request(r) = &back.items[0] else {
        panic!("request expected");
    };
    assert_eq!(r.request.params.len(), 2);
}
