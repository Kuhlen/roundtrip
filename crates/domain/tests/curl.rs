use domain::AppError;
use domain::auth::{ApiKeyPlace, Auth};
use domain::curl::{parse, to_command};
use domain::http::{Body, FormField, KeyValue, Method, Request, TextKind};

#[test]
fn plain_get() {
    let r = parse("curl https://api.test/users").unwrap().request;
    assert_eq!(r.method, Method::Get);
    assert_eq!(r.url, "https://api.test/users");
    assert_eq!(r.body, Body::None);
}

#[test]
fn query_string_becomes_params() {
    let r = parse("curl 'https://api.test/s?q=a%20b&page=2'")
        .unwrap()
        .request;
    assert_eq!(r.url, "https://api.test/s");
    assert_eq!(
        r.params,
        vec![KeyValue::new("q", "a b"), KeyValue::new("page", "2")]
    );
}

#[test]
fn quotes_and_escapes() {
    let r = parse(r#"curl -H "X-A: say \"hi\"" -H 'X-B: it'\''s' https://x.test"#)
        .unwrap()
        .request;
    assert_eq!(
        r.headers,
        vec![
            KeyValue::new("X-A", r#"say "hi""#),
            KeyValue::new("X-B", "it's"),
        ]
    );
}

#[test]
fn continuation_lines_from_a_single_line_field() {
    // Slint turned each newline into a space
    let r = parse("curl \\  -H 'X: 1' \\  https://x.test")
        .unwrap()
        .request;
    assert_eq!(r.url, "https://x.test");
    assert_eq!(r.headers, vec![KeyValue::new("X", "1")]);
}

#[test]
fn continuation_lines_with_real_newlines() {
    let r = parse("curl \\\n  -X PUT \\\n  https://x.test")
        .unwrap()
        .request;
    assert_eq!(r.method, Method::Put);
    assert_eq!(r.url, "https://x.test");
}

#[test]
fn data_means_post_and_joins_with_ampersand() {
    let r = parse("curl -d a=1 -d 'b=two words' https://x.test")
        .unwrap()
        .request;
    assert_eq!(r.method, Method::Post);
    assert_eq!(
        r.body,
        Body::Urlencoded(vec![
            KeyValue::new("a", "1"),
            KeyValue::new("b", "two words")
        ])
    );
}

#[test]
fn explicit_method_wins_over_data() {
    let r = parse("curl -X PATCH -d a=1 https://x.test")
        .unwrap()
        .request;
    assert_eq!(r.method, Method::Patch);
}

#[test]
fn body_kind_from_content_type() {
    let json = parse(r#"curl -H 'Content-Type: application/json' -d '{"a":1}' https://x.test"#)
        .unwrap()
        .request;
    assert_eq!(
        json.body,
        Body::Text {
            kind: TextKind::Json,
            text: r#"{"a":1}"#.into()
        }
    );
    let xml = parse("curl -H 'content-type: text/xml' -d '<a/>' https://x.test")
        .unwrap()
        .request;
    assert!(matches!(
        xml.body,
        Body::Text {
            kind: TextKind::Xml,
            ..
        }
    ));
    let raw = parse("curl -H 'Content-Type: text/plain' -d 'hello' https://x.test")
        .unwrap()
        .request;
    assert!(matches!(
        raw.body,
        Body::Text {
            kind: TextKind::Raw,
            ..
        }
    ));
}

#[test]
fn json_without_content_type_is_json() {
    let r = parse(r#"curl -d '{"a":1}' https://x.test"#)
        .unwrap()
        .request;
    assert!(matches!(
        r.body,
        Body::Text {
            kind: TextKind::Json,
            ..
        }
    ));
}

#[test]
fn data_urlencode_encodes_the_value() {
    let r = parse("curl --data-urlencode 'q=a&b' https://x.test")
        .unwrap()
        .request;
    assert_eq!(r.body, Body::Urlencoded(vec![KeyValue::new("q", "a&b")]));
}

#[test]
fn get_flag_moves_data_to_params() {
    let r = parse("curl -G -d q=1 https://x.test/s").unwrap().request;
    assert_eq!(r.method, Method::Get);
    assert_eq!(r.params, vec![KeyValue::new("q", "1")]);
    assert_eq!(r.body, Body::None);
}

#[test]
fn head_flag() {
    let r = parse("curl -I https://x.test").unwrap().request;
    assert_eq!(r.method, Method::Head);
}

#[test]
fn form_fields_text_and_file() {
    let r = parse("curl -F name=Ayu -F 'avatar=@/tmp/a.png;type=image/png' --form-string 'note=@not a file' https://x.test")
        .unwrap()
        .request;
    assert_eq!(r.method, Method::Post);
    assert_eq!(
        r.body,
        Body::FormData(vec![
            FormField::text("name", "Ayu"),
            FormField::file("avatar", "/tmp/a.png"),
            FormField::text("note", "@not a file"),
        ])
    );
}

#[test]
fn data_file_is_binary_body() {
    let r = parse("curl --data-binary @/tmp/body.bin https://x.test")
        .unwrap()
        .request;
    assert_eq!(r.body, Body::Binary("/tmp/body.bin".into()));
}

#[test]
fn data_file_mixed_with_other_data_is_an_error() {
    let e = parse("curl -d @/tmp/a -d b=1 https://x.test").unwrap_err();
    assert!(matches!(e, AppError::Import(_)), "{e:?}");
}

#[test]
fn user_is_basic_auth() {
    let r = parse("curl -u ayu:s3cret https://x.test").unwrap().request;
    assert_eq!(
        r.auth,
        Some(Auth::Basic {
            username: "ayu".into(),
            password: "s3cret".into()
        })
    );
    let no_pass = parse("curl -u ayu https://x.test").unwrap().request;
    assert_eq!(
        no_pass.auth,
        Some(Auth::Basic {
            username: "ayu".into(),
            password: String::new()
        })
    );
}

#[test]
fn cookie_agent_referer_become_headers() {
    let r = parse("curl -b 'sid=1; theme=dark' -A 'bot/1' -e https://from.test https://x.test")
        .unwrap()
        .request;
    assert_eq!(
        r.headers,
        vec![
            KeyValue::new("Cookie", "sid=1; theme=dark"),
            KeyValue::new("User-Agent", "bot/1"),
            KeyValue::new("Referer", "https://from.test"),
        ]
    );
}

#[test]
fn attached_values_and_short_clusters() {
    let r = parse("curl -sSL -XDELETE -H'X: 1' https://x.test").unwrap();
    assert_eq!(r.request.method, Method::Delete);
    assert_eq!(r.request.headers, vec![KeyValue::new("X", "1")]);
    assert!(r.skipped.is_empty(), "{:?}", r.skipped);
}

#[test]
fn url_flag() {
    let r = parse("curl --url https://x.test/a").unwrap().request;
    assert_eq!(r.url, "https://x.test/a");
}

#[test]
fn unknown_flags_are_skipped_with_their_value() {
    let r = parse("curl --max-time 5 --http2 https://x.test").unwrap();
    assert_eq!(r.request.url, "https://x.test");
    assert_eq!(
        r.skipped,
        vec!["--max-time".to_owned(), "--http2".to_owned()]
    );
}

#[test]
fn chrome_devtools_paste() {
    let text = "curl 'https://api.test/graphql' \\  -H 'accept: */*' \\  -H 'content-type: application/json' \\  --data-raw $'{\"q\":\"it\\'s\"}' \\  --compressed";
    let r = parse(text).unwrap().request;
    assert_eq!(r.method, Method::Post);
    assert_eq!(r.headers.len(), 2);
    assert_eq!(
        r.body,
        Body::Text {
            kind: TextKind::Json,
            text: r#"{"q":"it's"}"#.into()
        }
    );
}

#[test]
fn unterminated_quote_is_an_error() {
    let e = parse("curl 'https://x.test").unwrap_err();
    assert!(matches!(e, AppError::Import(_)), "{e:?}");
}

#[test]
fn no_url_is_an_error() {
    let e = parse("curl -X POST").unwrap_err();
    assert!(matches!(e, AppError::Import(_)), "{e:?}");
}

#[test]
fn unknown_method_is_an_error() {
    let e = parse("curl -X PROPFIND https://x.test").unwrap_err();
    assert!(matches!(e, AppError::Import(_)), "{e:?}");
}

fn post_json() -> Request {
    Request {
        method: Method::Post,
        url: "https://api.test/users".into(),
        params: vec![KeyValue::new("page", "2"), KeyValue::new("q", "a b")],
        headers: vec![
            KeyValue::new("Content-Type", "application/json"),
            KeyValue::new("X-Note", "it's"),
        ],
        body: Body::Text {
            kind: TextKind::Json,
            text: r#"{"name":"O'Neil"}"#.into(),
        },
        auth: Some(Auth::Basic {
            username: "ayu".into(),
            password: "p@ss".into(),
        }),
    }
}

#[test]
fn round_trip_with_quotes() {
    let r = post_json();
    let back = parse(&to_command(&r).unwrap()).unwrap();
    assert_eq!(back.request, r);
    assert!(back.skipped.is_empty());
}

#[test]
fn round_trip_through_a_single_line_field() {
    let r = post_json();
    let one_line = to_command(&r).unwrap().replace('\n', " ");
    assert_eq!(parse(&one_line).unwrap().request, r);
}

#[test]
fn get_has_no_method_flag_and_skips_disabled_rows() {
    let r = Request {
        url: "https://x.test/a".into(),
        params: vec![KeyValue {
            enabled: false,
            ..KeyValue::new("off", "1")
        }],
        ..Request::default()
    };
    assert_eq!(to_command(&r).unwrap(), "curl 'https://x.test/a'");
}

#[test]
fn bearer_and_query_api_key() {
    let bearer = Request {
        url: "https://x.test".into(),
        auth: Some(Auth::Bearer {
            token: "t0k".into(),
        }),
        ..Request::default()
    };
    assert!(
        to_command(&bearer)
            .unwrap()
            .contains("-H 'Authorization: Bearer t0k'")
    );
    let key = Request {
        url: "https://x.test/a?x=1".into(),
        auth: Some(Auth::ApiKey {
            key: "api_key".into(),
            value: "k 1".into(),
            place: ApiKeyPlace::Query,
        }),
        ..Request::default()
    };
    assert!(
        to_command(&key)
            .unwrap()
            .contains("'https://x.test/a?x=1&api_key=k%201'")
    );
}

#[test]
fn text_body_gets_a_content_type_when_missing() {
    let r = Request {
        method: Method::Post,
        url: "https://x.test".into(),
        body: Body::Text {
            kind: TextKind::Xml,
            text: "<a/>".into(),
        },
        ..Request::default()
    };
    let cmd = to_command(&r).unwrap();
    assert!(cmd.contains("-H 'Content-Type: application/xml'"), "{cmd}");
}

#[test]
fn files_become_at_paths() {
    let binary = Request {
        method: Method::Put,
        url: "https://x.test".into(),
        body: Body::Binary("/tmp/a b.bin".into()),
        ..Request::default()
    };
    assert!(
        to_command(&binary)
            .unwrap()
            .contains("--data-binary '@/tmp/a b.bin'")
    );
    let form = Request {
        method: Method::Post,
        url: "https://x.test".into(),
        body: Body::FormData(vec![
            FormField::text("note", "@literal"),
            FormField::file("avatar", "/tmp/a.png"),
        ]),
        ..Request::default()
    };
    let cmd = to_command(&form).unwrap();
    assert!(cmd.contains("--form-string 'note=@literal'"), "{cmd}");
    assert!(cmd.contains("-F 'avatar=@/tmp/a.png'"), "{cmd}");
    assert_eq!(parse(&cmd).unwrap().request, form);
}

#[test]
fn urlencoded_round_trip() {
    let r = Request {
        method: Method::Post,
        url: "https://x.test".into(),
        body: Body::Urlencoded(vec![KeyValue::new("q", "a&b c")]),
        ..Request::default()
    };
    assert_eq!(parse(&to_command(&r).unwrap()).unwrap().request, r);
}

#[test]
fn head_uses_dash_i() {
    let r = Request {
        method: Method::Head,
        url: "https://x.test".into(),
        ..Request::default()
    };
    assert_eq!(to_command(&r).unwrap(), "curl -I 'https://x.test'");
}

#[test]
fn unsupported_body_auth_or_empty_url_fail() {
    let base = Request {
        url: "https://x.test".into(),
        ..Request::default()
    };
    let body = Request {
        body: Body::Unsupported("graphql-file".into()),
        ..base.clone()
    };
    let auth = Request {
        auth: Some(Auth::Unsupported("oauth2".into())),
        ..base.clone()
    };
    let empty = Request::default();
    for r in [body, auth, empty] {
        assert!(matches!(to_command(&r), Err(AppError::Import(_))), "{r:?}");
    }
}

#[test]
fn ansi_c_quotes_decode_unicode_and_hex() {
    let r = parse(
        r#"curl -X POST https://x.test -d $'{"p":"Hunter2!","t":"a\u0009b","h":"\x41\101\e"}'"#,
    )
    .unwrap()
    .request;
    assert_eq!(
        r.body,
        Body::Text {
            kind: TextKind::Json,
            text: "{\"p\":\"Hunter2!\",\"t\":\"a\tb\",\"h\":\"AA\u{1b}\"}".into()
        }
    );
}

#[test]
fn ansi_c_quotes_keep_malformed_escapes() {
    let r = parse(r"curl https://x.test -d $'a\uZZ\q\''")
        .unwrap()
        .request;
    assert_eq!(
        r.body,
        Body::Text {
            kind: TextKind::Raw,
            text: r"a\uZZ\q'".into()
        }
    );
}

#[test]
fn get_with_body_round_trips() {
    let r = Request {
        method: Method::Get,
        url: "https://x.test".into(),
        headers: vec![KeyValue::new("Content-Type", "application/json")],
        body: Body::Text {
            kind: TextKind::Json,
            text: "{\"a\":1}".into(),
        },
        ..Request::default()
    };
    let back = parse(&to_command(&r).unwrap()).unwrap().request;
    assert_eq!(back.method, Method::Get);
    assert_eq!(back.body, r.body);
}
