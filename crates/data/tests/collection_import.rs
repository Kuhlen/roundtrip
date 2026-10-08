use std::fs;

use data::collection_dir::CollectionDir;
use data::environment_dir::EnvironmentDir;
use domain::AppError;
use domain::auth::Auth;
use domain::collection::{CollectionStore, Node, Protocol};
use domain::environment::EnvironmentStore;
use domain::http::{Body, KeyValue, Method, Request, TextKind};
use domain::import::{ImportItem, ImportedCollection, ImportedRequest};

fn req(name: &str, url: &str) -> ImportItem {
    ImportItem::Request(ImportedRequest {
        name: name.into(),
        request: Request {
            url: url.into(),
            ..Request::default()
        },
        ..ImportedRequest::default()
    })
}

fn shop() -> ImportedCollection {
    ImportedCollection {
        name: "Shop API".into(),
        auth: Some(Auth::Bearer {
            token: "{{token}}".into(),
        }),
        items: vec![
            req("Zeta", "https://shop.test/z"),
            ImportItem::Folder {
                name: "Users".into(),
                items: vec![
                    ImportItem::Request(ImportedRequest {
                        name: "Create user".into(),
                        request: Request {
                            method: Method::Post,
                            url: "{{baseUrl}}/users".into(),
                            headers: vec![KeyValue::new("X-Trace", "1")],
                            body: Body::Text {
                                kind: TextKind::Json,
                                text: r#"{"a":1}"#.into(),
                            },
                            ..Request::default()
                        },
                        pre_request_script: Some("pm.environment.set('a', 1)".into()),
                        tests: Some("pm.test('ok')".into()),
                        description: Some("Makes a user".into()),
                        ..ImportedRequest::default()
                    }),
                    req("Alpha", "https://shop.test/a"),
                ],
            },
        ],
        environments: vec![(
            "Collection Variables".into(),
            vec![("baseUrl".into(), "https://shop.test".into())],
        )],
        warnings: vec![],
    }
}

#[test]
fn written_collection_loads_back_in_order() {
    let parent = tempfile::tempdir().unwrap();
    let root = CollectionDir
        .create_collection(parent.path(), &shop())
        .unwrap();
    assert_eq!(root, parent.path().join("Shop API"));
    let c = CollectionDir.load(&root).unwrap();
    assert_eq!(c.name, "Shop API");
    assert_eq!(
        c.auth,
        Some(Auth::Bearer {
            token: "{{token}}".into()
        })
    );
    // folders first, then requests: the tree's own rule; order inside comes from _folder.yaml
    let names: Vec<&str> = c.children.iter().map(Node::name).collect();
    assert_eq!(names, ["Users", "Zeta"]);
    let Node::Folder { children, .. } = &c.children[0] else {
        panic!("Users is not a folder");
    };
    let inner: Vec<&str> = children.iter().map(Node::name).collect();
    assert_eq!(inner, ["Create user", "Alpha"]);
    let create = CollectionDir
        .read_request(&root.join("Users").join("create-user.yaml"))
        .unwrap();
    assert_eq!(create.method, Method::Post);
    assert_eq!(create.headers, vec![KeyValue::new("X-Trace", "1")]);
    let envs = EnvironmentDir.list(&root).unwrap();
    assert_eq!(envs[0].name, "Collection Variables");
    assert_eq!(envs[0].variables["baseUrl"], "https://shop.test");
    assert_eq!(
        fs::read_to_string(root.join(".apiark/.gitignore")).unwrap(),
        ".env\n"
    );
}

#[test]
fn scripts_survive_a_later_save() {
    let parent = tempfile::tempdir().unwrap();
    let root = CollectionDir
        .create_collection(parent.path(), &shop())
        .unwrap();
    let file = root.join("Users").join("create-user.yaml");
    let mut r = CollectionDir.read_request(&file).unwrap();
    r.url = "{{baseUrl}}/v2/users".into();
    CollectionDir.save_request(&file, &r).unwrap();
    let text = fs::read_to_string(&file).unwrap();
    assert!(text.contains("preRequestScript:"), "{text}");
    assert!(text.contains("tests:"), "{text}");
    assert!(text.contains("description: Makes a user"), "{text}");
}

#[test]
fn graphql_requests_keep_their_protocol() {
    let parent = tempfile::tempdir().unwrap();
    let data = ImportedCollection {
        name: "G".into(),
        items: vec![ImportItem::Request(ImportedRequest {
            name: "Me".into(),
            request: Request {
                method: Method::Post,
                url: "https://g.test".into(),
                body: Body::Text {
                    kind: TextKind::Json,
                    text: r#"{"query":"{ me }"}"#.into(),
                },
                ..Request::default()
            },
            protocol: Protocol::Graphql,
            ..ImportedRequest::default()
        })],
        ..ImportedCollection::default()
    };
    let root = CollectionDir
        .create_collection(parent.path(), &data)
        .unwrap();
    let c = CollectionDir.load(&root).unwrap();
    assert!(
        matches!(
            &c.children[0],
            Node::Request {
                protocol: Protocol::Graphql,
                ..
            }
        ),
        "{:?}",
        c.children
    );
}

#[test]
fn awkward_names_still_import() {
    let parent = tempfile::tempdir().unwrap();
    let data = ImportedCollection {
        name: "Odd".into(),
        items: vec![
            req("Login", "https://o.test/1"),
            req("login", "https://o.test/2"),
            req("", "https://o.test/3"),
            req(".hidden", "https://o.test/4"),
            req("a/b", "https://o.test/5"),
            ImportItem::Folder {
                name: "".into(),
                items: vec![req("x", "https://o.test/6")],
            },
        ],
        ..ImportedCollection::default()
    };
    let root = CollectionDir
        .create_collection(parent.path(), &data)
        .unwrap();
    let c = CollectionDir.load(&root).unwrap();
    fn count(nodes: &[Node]) -> usize {
        nodes
            .iter()
            .map(|n| match n {
                Node::Folder { children, .. } => count(children),
                Node::Request { .. } => 1,
            })
            .sum()
    }
    assert_eq!(count(&c.children), 6);
}

#[test]
fn existing_folder_is_never_touched() {
    let parent = tempfile::tempdir().unwrap();
    let target = parent.path().join("Shop API");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("keep.txt"), "mine").unwrap();
    let e = CollectionDir
        .create_collection(parent.path(), &shop())
        .unwrap_err();
    assert_eq!(e, AppError::AlreadyExists("Shop API".into()));
    assert_eq!(fs::read_to_string(target.join("keep.txt")).unwrap(), "mine");
    assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
}

#[test]
fn failure_midway_leaves_no_folder() {
    let parent = tempfile::tempdir().unwrap();
    let data = ImportedCollection {
        name: "Broken".into(),
        // file name over 255 bytes: the OS refuses it after the folder exists
        items: vec![
            req("ok", "https://b.test"),
            req(&"a".repeat(300), "https://b.test"),
        ],
        ..ImportedCollection::default()
    };
    assert!(
        CollectionDir
            .create_collection(parent.path(), &data)
            .is_err()
    );
    assert!(!parent.path().join("Broken").exists());
}

#[test]
fn counts_walk_folders() {
    assert_eq!(shop().counts(), (1, 3, 1));
}
