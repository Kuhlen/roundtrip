use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::{self, JoinHandle};

use data::http::{ReqwestSender, pretty_json};
use domain::AppError;
use domain::auth::{ApiKeyPlace, Auth};
use domain::http::{Body, FormField, HttpSender, KeyValue, Method, Request, TextKind};

/// One-shot local server: answers `response`, returns the raw request bytes.
fn serve_raw(response: Vec<u8>) -> (String, JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = stream.read(&mut chunk).expect("read");
            buf.extend_from_slice(&chunk[..n]);
            if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..end]).to_lowercase();
                let done = if head.contains("transfer-encoding: chunked") {
                    buf.ends_with(b"0\r\n\r\n")
                } else {
                    let len = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .map_or(0, |v| v.trim().parse::<usize>().expect("length"));
                    buf.len() >= end + 4 + len
                };
                if done {
                    break;
                }
            }
            if n == 0 {
                break;
            }
        }
        stream.write_all(&response).expect("write");
        buf
    });
    (base, handle)
}

/// Same, request lowercased as text.
fn serve(response: Vec<u8>) -> (String, JoinHandle<String>) {
    let (base, raw) = serve_raw(response);
    let handle =
        thread::spawn(move || String::from_utf8_lossy(&raw.join().expect("server")).to_lowercase());
    (base, handle)
}

fn post(base: String, body: Body) -> Request {
    Request {
        method: Method::Post,
        url: base,
        body,
        ..Request::default()
    }
}

fn reply(body: &[u8], extra_headers: &str) -> Vec<u8> {
    let mut r = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n",
        body.len()
    )
    .into_bytes();
    r.extend_from_slice(body);
    r
}

fn get(url: String) -> Request {
    Request {
        method: Method::Get,
        url,
        ..Request::default()
    }
}

#[test]
fn status_headers_and_body() {
    let (base, server) = serve(reply(b"hello", "X-Test: 1\r\n"));
    let resp = ReqwestSender::new()
        .unwrap()
        .send(&get(format!("{base}/path")))
        .unwrap();
    server.join().unwrap();
    assert_eq!((resp.status, resp.status_text.as_str()), (200, "OK"));
    assert_eq!(resp.body, "hello");
    assert_eq!(resp.size_bytes, 5);
    assert!(!resp.truncated);
    assert!(
        resp.headers
            .iter()
            .any(|h| h.key == "x-test" && h.value == "1")
    );
}

#[test]
fn active_params_are_appended_to_the_url() {
    let (base, server) = serve(reply(b"", ""));
    let mut req = get(format!("{base}/search?q=a"));
    req.params = vec![
        KeyValue::new("page", "2"),
        KeyValue {
            key: "debug".into(),
            value: "true".into(),
            enabled: false,
        },
        KeyValue::new("", "blank key"),
    ];
    ReqwestSender::new().unwrap().send(&req).unwrap();
    assert!(
        server
            .join()
            .unwrap()
            .starts_with("get /search?q=a&page=2 http/1.1")
    );
}

#[test]
fn content_type_is_not_duplicated() {
    let (base, server) = serve(reply(b"", ""));
    let mut req = get(base);
    req.method = Method::Post;
    req.headers = vec![KeyValue::new("Content-Type", "application/vnd.api+json")];
    req.body = Body::Text {
        kind: TextKind::Json,
        text: "{}".into(),
    };
    ReqwestSender::new().unwrap().send(&req).unwrap();
    let raw = server.join().unwrap();
    assert_eq!(raw.matches("content-type:").count(), 1);
    assert!(raw.contains("content-type: application/vnd.api+json"));
}

#[test]
fn body_kind_sets_content_type_when_user_did_not() {
    let (base, server) = serve(reply(b"", ""));
    let mut req = get(base);
    req.method = Method::Post;
    req.body = Body::Text {
        kind: TextKind::Json,
        text: "{}".into(),
    };
    ReqwestSender::new().unwrap().send(&req).unwrap();
    assert!(
        server
            .join()
            .unwrap()
            .contains("content-type: application/json")
    );
}

#[test]
fn closed_port_is_connection_refused() {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let result = ReqwestSender::new()
        .unwrap()
        .send(&get(format!("http://127.0.0.1:{port}/")));
    assert!(
        matches!(result, Err(AppError::ConnectionRefused(_))),
        "{result:?}"
    );
}

#[test]
fn url_without_scheme_is_invalid_url() {
    let result = ReqwestSender::new()
        .unwrap()
        .send(&get("httpbin.org/get".into()));
    assert!(matches!(result, Err(AppError::InvalidUrl(_))), "{result:?}");
}

#[test]
fn unsupported_body_is_never_sent() {
    let mut req = get("http://127.0.0.1:9/".into());
    req.body = Body::Unsupported("form-data".into());
    assert!(matches!(
        ReqwestSender::new().unwrap().send(&req),
        Err(AppError::Request(_))
    ));
}

#[test]
fn large_body_is_cut_on_a_char_boundary() {
    // 3-byte chars: the 1 MiB cut lands mid-char
    let body = "€".repeat(1024 * 1024 / 3 + 10);
    let (base, server) = serve(reply(body.as_bytes(), ""));
    let resp = ReqwestSender::new().unwrap().send(&get(base)).unwrap();
    server.join().unwrap();
    assert!(resp.truncated);
    assert_eq!(resp.size_bytes, body.len() as u64);
    assert_eq!(resp.body.len(), 1024 * 1024 - 1);
    assert!(resp.body.chars().all(|c| c == '€'));
}

#[test]
fn binary_body_is_described() {
    let (base, server) = serve(reply(&[0xff, 0xfe, 0x00, 0x89], ""));
    let resp = ReqwestSender::new().unwrap().send(&get(base)).unwrap();
    server.join().unwrap();
    assert_eq!(resp.body, "<binary data: 4 bytes>");
}

#[test]
fn pretty_json_keeps_key_order() {
    assert_eq!(
        pretty_json(r#"{"b":1,"a":[1]}"#).unwrap(),
        "{\n  \"b\": 1,\n  \"a\": [\n    1\n  ]\n}"
    );
    assert_eq!(pretty_json("<xml/>"), None);
    assert_eq!(pretty_json(""), None);
}

fn with_auth(url: String, auth: Auth) -> Request {
    Request {
        auth: Some(auth),
        ..get(url)
    }
}

#[test]
fn bearer_sets_authorization() {
    let (base, server) = serve(reply(b"", ""));
    ReqwestSender::new()
        .unwrap()
        .send(&with_auth(
            base,
            Auth::Bearer {
                token: "abc".into(),
            },
        ))
        .unwrap();
    assert!(
        server
            .join()
            .unwrap()
            .contains("authorization: bearer abc\r\n")
    );
}

#[test]
fn basic_is_base64_of_user_and_password() {
    let (base, server) = serve(reply(b"", ""));
    let auth = Auth::Basic {
        username: "user".into(),
        password: "pass".into(),
    };
    ReqwestSender::new()
        .unwrap()
        .send(&with_auth(base, auth))
        .unwrap();
    // base64("user:pass") = dXNlcjpwYXNz, lowercased by serve()
    assert!(
        server
            .join()
            .unwrap()
            .contains("authorization: basic dxnlcjpwyxnz\r\n")
    );
}

#[test]
fn api_key_goes_to_header_or_query() {
    let (base, server) = serve(reply(b"", ""));
    let auth = Auth::ApiKey {
        key: "X-Key".into(),
        value: "k1".into(),
        place: ApiKeyPlace::Header,
    };
    ReqwestSender::new()
        .unwrap()
        .send(&with_auth(base, auth))
        .unwrap();
    assert!(server.join().unwrap().contains("x-key: k1\r\n"));

    let (base, server) = serve(reply(b"", ""));
    let mut req = with_auth(
        format!("{base}/s"),
        Auth::ApiKey {
            key: "api_key".into(),
            value: "k2".into(),
            place: ApiKeyPlace::Query,
        },
    );
    req.params = vec![KeyValue::new("q", "1")];
    ReqwestSender::new().unwrap().send(&req).unwrap();
    assert!(server.join().unwrap().starts_with("get /s?q=1&api_key=k2 "));
}

#[test]
fn user_authorization_header_wins() {
    let (base, server) = serve(reply(b"", ""));
    let mut req = with_auth(
        base,
        Auth::Bearer {
            token: "abc".into(),
        },
    );
    req.headers = vec![KeyValue::new("Authorization", "Custom x")];
    ReqwestSender::new().unwrap().send(&req).unwrap();
    let raw = server.join().unwrap();
    assert_eq!(raw.matches("authorization:").count(), 1);
    assert!(raw.contains("authorization: custom x\r\n"));
}

#[test]
fn api_key_with_empty_name_is_skipped() {
    let (base, server) = serve(reply(b"", ""));
    let auth = Auth::ApiKey {
        key: "".into(),
        value: "v".into(),
        place: ApiKeyPlace::Header,
    };
    let resp = ReqwestSender::new()
        .unwrap()
        .send(&with_auth(base, auth))
        .unwrap();
    server.join().unwrap();
    assert_eq!(resp.status, 200);
}

#[test]
fn unsupported_auth_is_never_sent() {
    let req = with_auth(
        "http://127.0.0.1:9".into(),
        Auth::Unsupported("oauth2".into()),
    );
    assert!(matches!(
        ReqwestSender::new().unwrap().send(&req),
        Err(AppError::Request(m)) if m == "oauth2 auth is not supported"
    ));
}

#[test]
fn urlencoded_body_is_encoded() {
    let (base, server) = serve(reply(b"", ""));
    let req = post(
        base,
        Body::Urlencoded(vec![
            KeyValue::new("q", "a b&c"),
            KeyValue {
                key: "off".into(),
                value: "1".into(),
                enabled: false,
            },
        ]),
    );
    ReqwestSender::new().unwrap().send(&req).unwrap();
    let raw = server.join().unwrap();
    assert!(raw.contains("content-type: application/x-www-form-urlencoded"));
    assert!(raw.ends_with("\r\n\r\nq=a+b%26c"));
}

#[test]
fn urlencoded_keeps_a_user_content_type() {
    let (base, server) = serve(reply(b"", ""));
    let mut req = post(base, Body::Urlencoded(vec![KeyValue::new("a", "1")]));
    req.headers = vec![KeyValue::new("Content-Type", "text/plain")];
    ReqwestSender::new().unwrap().send(&req).unwrap();
    let raw = server.join().unwrap();
    assert_eq!(raw.matches("content-type:").count(), 1);
    assert!(raw.contains("content-type: text/plain"));
}

#[test]
fn form_data_sends_text_and_file_parts() {
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("a.png");
    std::fs::write(&png, b"PNGDATA").unwrap();
    let (base, server) = serve(reply(b"", ""));
    let mut req = post(
        base,
        Body::FormData(vec![
            FormField::text("name", "Ana"),
            FormField::file("avatar", png.to_str().unwrap()),
        ]),
    );
    req.headers = vec![KeyValue::new("Content-Type", "text/plain")];
    ReqwestSender::new().unwrap().send(&req).unwrap();
    let raw = server.join().unwrap();
    assert_eq!(
        raw.matches("content-type: multipart/form-data; boundary=")
            .count(),
        1
    );
    assert!(!raw.contains("content-type: text/plain"));
    assert!(raw.contains("content-disposition: form-data; name=\"name\"\r\n\r\nana"));
    assert!(raw.contains(
        "content-disposition: form-data; name=\"avatar\"; filename=\"a.png\"\r\ncontent-type: image/png\r\n\r\npngdata"
    ));
}

#[test]
fn binary_sends_file_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("b.bin");
    std::fs::write(&bin, [0u8, 0xff, 0x10, b'A']).unwrap();
    let (base, server) = serve_raw(reply(b"", ""));
    let req = post(base, Body::Binary(bin.display().to_string()));
    ReqwestSender::new().unwrap().send(&req).unwrap();
    let raw = server.join().unwrap();
    assert!(raw.ends_with(&[b'\r', b'\n', b'\r', b'\n', 0, 0xff, 0x10, b'A']));
    assert!(
        String::from_utf8_lossy(&raw)
            .to_lowercase()
            .contains("content-type: application/octet-stream")
    );
}

#[test]
fn bad_file_paths_are_file_errors() {
    let dir = tempfile::tempdir().unwrap();
    let paths = [
        String::new(),
        dir.path().display().to_string(),
        dir.path().join("nope.bin").display().to_string(),
    ];
    for path in paths {
        let binary = post("http://127.0.0.1:9/".into(), Body::Binary(path.clone()));
        assert_eq!(
            ReqwestSender::new().unwrap().send(&binary).unwrap_err(),
            AppError::File(path.clone())
        );
        let form = post(
            "http://127.0.0.1:9/".into(),
            Body::FormData(vec![FormField::file("f", path.clone())]),
        );
        assert_eq!(
            ReqwestSender::new().unwrap().send(&form).unwrap_err(),
            AppError::File(path)
        );
    }
}
