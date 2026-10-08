use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use app::modules::workspace::workspace_rules::{
    RowKind, error_text, export_lines, flatten, flatten_collections, folder_choices, format_size,
    import_counts, import_title, import_warnings, inherit_note, interpolate_request, is_curl_paste,
    resolve_files, resolved_url, row_label, unsupported_body_note,
};
use domain::AppError;
use domain::auth::{ApiKeyPlace, Auth};
use domain::collection::{Collection, Node, Protocol};
use domain::http::{Body, FormField, KeyValue, Method, Request, TextKind};
use domain::import::{ExportReport, ExportWarning, ImportFormat, ImportWarning};

fn request(name: &str, path: &str) -> Node {
    Node::Request {
        name: name.into(),
        method: Method::Get,
        protocol: Protocol::Http,
        path: path.into(),
    }
}

fn tree() -> Vec<Node> {
    vec![
        Node::Folder {
            name: "a".into(),
            path: "/c/a".into(),
            children: vec![
                request("R1", "/c/a/r1.yaml"),
                Node::Folder {
                    name: "b".into(),
                    path: "/c/a/b".into(),
                    children: vec![request("R2", "/c/a/b/r2.yaml")],
                },
            ],
        },
        request("Top", "/c/top.yaml"),
    ]
}

fn shape(collapsed: &[&str]) -> Vec<(String, usize)> {
    let collapsed: HashSet<PathBuf> = collapsed.iter().map(PathBuf::from).collect();
    flatten(&tree(), &collapsed)
        .into_iter()
        .map(|r| (r.name, r.depth))
        .collect()
}

fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |name| map.get(name).cloned()
}

#[test]
fn flatten_walks_depth_first() {
    let expected = [("a", 0), ("R1", 1), ("b", 1), ("R2", 2), ("Top", 0)];
    assert_eq!(shape(&[]), expected.map(|(n, d)| (n.to_string(), d)));
}

#[test]
fn collapsed_folder_hides_descendants() {
    assert_eq!(shape(&["/c/a/b"]).len(), 4);
    assert_eq!(
        shape(&["/c/a"]),
        [("a".to_string(), 0), ("Top".to_string(), 0)]
    );
    let rows = flatten(&tree(), &HashSet::from([PathBuf::from("/c/a")]));
    assert_eq!(rows[0].kind, RowKind::Folder { expanded: false });
}

#[test]
fn flatten_collections_puts_children_under_each_root() {
    let other = Collection {
        name: "d".into(),
        path: "/d".into(),
        auth: None,
        children: vec![request("D1", "/d/d1.yaml")],
    };
    let c = Collection {
        name: "c".into(),
        path: "/c".into(),
        auth: None,
        children: tree(),
    };
    let rows = flatten_collections(&[c, other], &HashSet::from([PathBuf::from("/d")]));
    let shape: Vec<(&str, usize)> = rows.iter().map(|r| (r.name.as_str(), r.depth)).collect();
    assert_eq!(
        shape,
        [
            ("c", 0),
            ("a", 1),
            ("R1", 2),
            ("b", 2),
            ("R2", 3),
            ("Top", 1),
            ("d", 0)
        ]
    );
    assert_eq!(rows[0].kind, RowKind::Collection { expanded: true });
    assert_eq!(rows[6].kind, RowKind::Collection { expanded: false });
    assert_eq!(rows[6].path, PathBuf::from("/d"));
}

#[test]
fn folder_choices_list_every_folder_from_the_root() {
    let c = Collection {
        name: "c".into(),
        path: "/c".into(),
        auth: None,
        children: tree(),
    };
    let choices: Vec<(String, PathBuf)> = folder_choices(&c);
    assert_eq!(
        choices,
        [
            ("/".to_string(), PathBuf::from("/c")),
            ("/ a".to_string(), PathBuf::from("/c/a")),
            ("/ a / b".to_string(), PathBuf::from("/c/a/b")),
        ]
    );
}

#[test]
fn row_label_prefers_protocol_over_method() {
    let label = |method, protocol| row_label(&RowKind::Request { method, protocol });
    assert_eq!(label(Method::Delete, Protocol::Http), "DELETE");
    assert_eq!(label(Method::Get, Protocol::WebSocket), "WS");
    assert_eq!(label(Method::Post, Protocol::Graphql), "GQL");
    assert_eq!(label(Method::Get, Protocol::Grpc), "gRPC");
    assert_eq!(row_label(&RowKind::Folder { expanded: true }), "");
}

#[test]
fn resolved_url_interpolates_and_appends_active_params() {
    let mut req = Request {
        url: "{{baseUrl}}/get".into(),
        ..Request::default()
    };
    req.params = vec![
        KeyValue::new("page", "2"),
        KeyValue::new("limit", "{{pageSize}}"),
        KeyValue {
            key: "debug".into(),
            value: "true".into(),
            enabled: false,
        },
    ];
    let lookup = vars(&[("baseUrl", "https://httpbin.org"), ("pageSize", "20")]);
    assert_eq!(
        resolved_url(&req, &lookup),
        "https://httpbin.org/get?page=2&limit=20"
    );
    req.url = "{{baseUrl}}/get?a=1".into();
    assert_eq!(
        resolved_url(&req, &lookup),
        "https://httpbin.org/get?a=1&page=2&limit=20"
    );
    let bare = Request {
        url: "{{missing}}/x".into(),
        ..Request::default()
    };
    assert_eq!(resolved_url(&bare, &lookup), "{{missing}}/x");
}

#[test]
fn interpolate_request_fills_every_text_field() {
    let req = Request {
        method: Method::Post,
        url: "{{host}}/u".into(),
        params: vec![KeyValue::new("{{k}}", "{{v}}")],
        headers: vec![KeyValue {
            key: "Authorization".into(),
            value: "Bearer {{token}}".into(),
            enabled: false,
        }],
        body: Body::Text {
            kind: TextKind::Json,
            text: r#"{"id":"{{id}}"}"#.into(),
        },
        auth: None,
    };
    let out = interpolate_request(
        &req,
        vars(&[
            ("host", "http://h"),
            ("k", "a"),
            ("v", "b"),
            ("token", "t"),
            ("id", "7"),
        ]),
    );
    assert_eq!(
        out,
        Request {
            method: Method::Post,
            url: "http://h/u".into(),
            params: vec![KeyValue::new("a", "b")],
            headers: vec![KeyValue {
                key: "Authorization".into(),
                value: "Bearer t".into(),
                enabled: false
            }],
            body: Body::Text {
                kind: TextKind::Json,
                text: r#"{"id":"7"}"#.into(),
            },
            auth: None,
        }
    );
}

#[test]
fn merge_conflict_title_is_relative_to_the_collection() {
    let e = AppError::MergeConflict("/c/users/create-user.yaml".into());
    assert_eq!(
        error_text(&e, Some(Path::new("/c"))).0,
        "Merge conflict in users/create-user.yaml"
    );
    assert_eq!(
        error_text(&e, None).0,
        "Merge conflict in /c/users/create-user.yaml"
    );
}

#[test]
fn hints_follow_upstream() {
    assert_eq!(
        error_text(&AppError::ConnectionRefused("x".into()), None).1,
        "Is the server running? Check the host and port."
    );
    assert!(
        error_text(&AppError::Timeout(30000), None)
            .1
            .contains("30000ms")
    );
}

#[test]
fn sizes_are_human_readable() {
    assert_eq!(format_size(412), "412 B");
    assert_eq!(format_size(1536), "1.5 KB");
    assert_eq!(format_size(2 * 1024 * 1024), "2.0 MB");
}

#[test]
fn interpolate_request_fills_auth_fields() {
    let vars = HashMap::from([("t".to_string(), "secret".to_string())]);
    let lookup = |n: &str| vars.get(n).cloned();
    let with = |auth| Request {
        auth: Some(auth),
        ..Request::default()
    };
    let out = interpolate_request(
        &with(Auth::Bearer {
            token: "{{t}}".into(),
        }),
        lookup,
    );
    assert_eq!(
        out.auth,
        Some(Auth::Bearer {
            token: "secret".into()
        })
    );
    let out = interpolate_request(
        &with(Auth::ApiKey {
            key: "X-{{t}}".into(),
            value: "{{t}}".into(),
            place: ApiKeyPlace::Query,
        }),
        lookup,
    );
    assert_eq!(
        out.auth,
        Some(Auth::ApiKey {
            key: "X-secret".into(),
            value: "secret".into(),
            place: ApiKeyPlace::Query
        })
    );
}

#[test]
fn name_errors_have_text() {
    let (title, _) = error_text(&AppError::AlreadyExists("ping.yaml".into()), None);
    assert_eq!(title, "ping.yaml already exists");
    let (title, _) = error_text(&AppError::InvalidName(".env".into()), None);
    assert_eq!(title, "Invalid name \".env\"");
}

#[test]
fn resolved_url_shows_a_query_api_key() {
    let req = Request {
        url: "http://h/p".into(),
        params: vec![KeyValue::new("a", "1")],
        auth: Some(Auth::ApiKey {
            key: "k".into(),
            value: "v".into(),
            place: ApiKeyPlace::Query,
        }),
        ..Request::default()
    };
    assert_eq!(resolved_url(&req, |_| None), "http://h/p?a=1&k=v");
}

#[test]
fn resolve_files_joins_relative_paths_only() {
    let mut req = Request {
        body: Body::FormData(vec![
            FormField::file("a", "img/a.png"),
            FormField::file("b", "/abs/b.png"),
            FormField::file("c", ""),
            FormField::text("t", "img/t.png"),
        ]),
        ..Request::default()
    };
    resolve_files(&mut req, Path::new("/c"));
    assert_eq!(
        req.body,
        Body::FormData(vec![
            FormField::file("a", Path::new("/c").join("img/a.png").display().to_string()),
            FormField::file("b", "/abs/b.png"),
            FormField::file("c", ""),
            FormField::text("t", "img/t.png"),
        ])
    );
    let mut bin = Request {
        body: Body::Binary("x.bin".into()),
        ..Request::default()
    };
    resolve_files(&mut bin, Path::new("/c"));
    assert_eq!(
        bin.body,
        Body::Binary(Path::new("/c").join("x.bin").display().to_string())
    );
}

#[test]
fn unsupported_note_tells_unreadable_from_unknown() {
    assert_eq!(
        unsupported_body_note("form-data"),
        "form-data body: Roundtrip can't read this content. Saving leaves it as it is in the file."
    );
    assert_eq!(
        unsupported_body_note("graphql"),
        "graphql body: not supported yet. Saving leaves it as it is in the file."
    );
}

#[test]
fn file_errors_name_the_fix() {
    assert_eq!(
        error_text(&AppError::File(String::new()), None),
        (
            "No file chosen".to_string(),
            "Pick a file in the Body tab.".to_string()
        )
    );
    assert_eq!(
        error_text(&AppError::File("/x.png".into()), None).0,
        "Cannot read file"
    );
}

#[test]
fn inherit_note_names_the_collection_auth() {
    assert_eq!(
        inherit_note(None),
        "No auth. The collection has none either."
    );
    assert_eq!(
        inherit_note(Some(&Auth::Bearer { token: "t".into() })),
        "Uses the collection auth: Bearer."
    );
    assert_eq!(
        inherit_note(Some(&Auth::Unsupported("oauth2".into()))),
        "Uses the collection auth: oauth2, not supported yet."
    );
}

#[test]
fn interpolate_request_fills_form_fields_and_paths() {
    let lookup = vars(&[("k", "a"), ("v", "b"), ("dir", "/d")]);
    let out = |body| {
        interpolate_request(
            &Request {
                body,
                ..Request::default()
            },
            &lookup,
        )
        .body
    };
    assert_eq!(
        out(Body::Urlencoded(vec![KeyValue::new("{{k}}", "{{v}}")])),
        Body::Urlencoded(vec![KeyValue::new("a", "b")])
    );
    assert_eq!(
        out(Body::FormData(vec![
            FormField::text("{{k}}", "{{v}}"),
            FormField::file("f", "{{dir}}/a.png"),
        ])),
        Body::FormData(vec![
            FormField::text("a", "b"),
            FormField::file("f", "/d/a.png"),
        ])
    );
    assert_eq!(
        out(Body::Binary("{{dir}}/x.bin".into())),
        Body::Binary("/d/x.bin".into())
    );
}

#[test]
fn curl_paste_is_told_apart_from_typing() {
    // pasted into an empty field
    assert!(is_curl_paste("", "curl https://x.test"));
    // pasted over a selected URL, longer or shorter
    assert!(is_curl_paste(
        "https://a.very.long.test/path/that/is/long",
        "curl https://x.test"
    ));
    assert!(is_curl_paste(
        "https://a.test",
        "curl -H 'A: 1' https://x.test"
    ));
    // typing letter by letter
    assert!(!is_curl_paste("curl", "curl "));
    assert!(!is_curl_paste("curl ", "curl h"));
    assert!(!is_curl_paste("curl https://x.tes", "curl https://x.test"));
    // not curl at all
    assert!(!is_curl_paste("", "https://curl.test"));
    assert!(!is_curl_paste("", "curling"));
}

#[test]
fn import_counts_read_naturally() {
    assert_eq!(
        import_counts((1, 2, 1)),
        "1 folder · 2 requests · 1 environment"
    );
    assert_eq!(import_counts((0, 1, 0)), "1 request");
    assert_eq!(
        import_counts((3, 0, 2)),
        "3 folders · 0 requests · 2 environments"
    );
}

#[test]
fn import_warnings_are_grouped_with_counts() {
    let lines = import_warnings(&[
        ImportWarning::UnsupportedAuth("oauth2".into()),
        ImportWarning::Scripts,
        ImportWarning::UnsupportedAuth("oauth2".into()),
        ImportWarning::UnsupportedAuth("digest".into()),
        ImportWarning::CollectionScriptsDropped,
    ]);
    assert_eq!(
        lines,
        vec![
            "oauth2 auth is not supported; those requests inherit the collection auth, if any (2×)",
            "Scripts are kept in the files but not run",
            "digest auth is not supported; those requests inherit the collection auth, if any",
            "Collection and folder scripts were left out",
        ]
    );
}

#[test]
fn import_title_names_the_format() {
    assert_eq!(
        import_title(&ImportFormat::Postman),
        "Import Postman collection"
    );
    assert_eq!(
        import_title(&ImportFormat::OpenApi("3.1.0".into())),
        "Import OpenAPI 3.1.0 spec"
    );
    assert_eq!(
        import_title(&ImportFormat::Insomnia),
        "Import Insomnia collection"
    );
}

#[test]
fn new_import_warnings_have_text() {
    let lines = import_warnings(&[
        ImportWarning::UnsupportedRequest("gRPC".into()),
        ImportWarning::TemplateTags,
        ImportWarning::ExternalRef,
        ImportWarning::CookieParams,
        ImportWarning::FolderVariables,
        ImportWarning::OtherWorkspaces(2),
        ImportWarning::PathVariables,
    ]);
    assert_eq!(
        lines,
        vec![
            "gRPC requests were skipped: not supported",
            "Template tags ({% … %}) stay as text; replace them by hand",
            "References to other files were left empty",
            "Cookie parameters were left out",
            "Folder environments were left out",
            "Only the first workspace was imported; 2 more were skipped",
            "Path variables (:id, {id}) stay in the URL; fill them in by hand",
        ]
    );
}

#[test]
fn export_lines_count_and_explain() {
    let report = ExportReport {
        requests: 1,
        warnings: vec![
            ExportWarning::SkippedProtocol(2),
            ExportWarning::Unreadable(1),
            ExportWarning::UnsupportedBody(1),
            ExportWarning::UnsupportedAuth(3),
            ExportWarning::EnvironmentsNotExported(1),
        ],
    };
    assert_eq!(
        export_lines(&report, 0),
        (
            "Exported 1 request".to_owned(),
            "2 WebSocket, SSE or gRPC requests skipped. \
             1 unreadable request file skipped. \
             Body type not supported, left out (1). \
             Auth type not supported, left out (3). \
             1 environment not exported; Postman keeps environments in separate files"
                .to_owned()
        )
    );
    let (title, hint) = export_lines(
        &ExportReport {
            requests: 3,
            warnings: vec![],
        },
        2,
    );
    assert_eq!(title, "Exported 3 requests");
    assert_eq!(hint, "2 unsaved requests exported in their saved state");
    let (_, hint) = export_lines(&ExportReport::default(), 1);
    assert_eq!(hint, "1 unsaved request exported in its saved state");
}
