use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use app::modules::workspace::workspace_rules::{
    RowKind, error_text, flatten, format_size, interpolate_request, resolved_url, row_label,
};
use domain::AppError;
use domain::collection::{Node, Protocol};
use domain::http::{BodyKind, KeyValue, Method, Request};

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
        body_kind: BodyKind::Json,
        body: r#"{"id":"{{id}}"}"#.into(),
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
            body_kind: BodyKind::Json,
            body: r#"{"id":"7"}"#.into(),
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
