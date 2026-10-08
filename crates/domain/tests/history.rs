use domain::auth::{ApiKeyPlace, Auth};
use domain::history::{REDACTED, redact};
use domain::http::{Body, KeyValue, Request, TextKind};

fn with(headers: Vec<KeyValue>, auth: Option<Auth>) -> Request {
    Request {
        headers,
        auth,
        body: Body::Text {
            kind: TextKind::Json,
            text: r#"{"password":"p"}"#.into(),
        },
        ..Request::default()
    }
}

#[test]
fn secret_headers_are_masked_by_name() {
    let r = redact(&with(
        vec![
            KeyValue::new("Authorization", "Bearer abc"),
            KeyValue::new("X-Api-Key", "k1"),
            KeyValue::new("Cookie", "sid=1"),
            KeyValue::new("X-Refresh-Token", "t"),
            KeyValue::new("Proxy-Authorization", "Basic x"),
            KeyValue::new("Accept", "application/json"),
        ],
        None,
    ));
    let values: Vec<&str> = r.headers.iter().map(|h| h.value.as_str()).collect();
    assert_eq!(
        values,
        [
            REDACTED,
            REDACTED,
            REDACTED,
            REDACTED,
            REDACTED,
            "application/json"
        ]
    );
}

#[test]
fn api_key_spellings_are_masked() {
    let r = redact(&with(
        vec![
            KeyValue::new("X-ApiKey", "k"),
            KeyValue::new("api_key", "k"),
            KeyValue::new("X-Auth-Token", "k"),
            KeyValue::new("Accept", "application/json"),
        ],
        None,
    ));
    let values: Vec<&str> = r.headers.iter().map(|h| h.value.as_str()).collect();
    assert_eq!(values, [REDACTED, REDACTED, REDACTED, "application/json"]);
}

#[test]
fn auth_secrets_are_masked() {
    let bearer = redact(&with(
        vec![],
        Some(Auth::Bearer {
            token: "abc".into(),
        }),
    ));
    assert_eq!(
        bearer.auth,
        Some(Auth::Bearer {
            token: REDACTED.into()
        })
    );
    let basic = redact(&with(
        vec![],
        Some(Auth::Basic {
            username: "ayu".into(),
            password: "pw".into(),
        }),
    ));
    assert_eq!(
        basic.auth,
        Some(Auth::Basic {
            username: "ayu".into(),
            password: REDACTED.into()
        })
    );
    let key = redact(&with(
        vec![],
        Some(Auth::ApiKey {
            key: "X-Key".into(),
            value: "v".into(),
            place: ApiKeyPlace::Query,
        }),
    ));
    assert_eq!(
        key.auth,
        Some(Auth::ApiKey {
            key: "X-Key".into(),
            value: REDACTED.into(),
            place: ApiKeyPlace::Query
        })
    );
}

#[test]
fn variables_and_empty_values_are_kept() {
    let r = redact(&with(
        vec![
            KeyValue::new("Authorization", "Bearer {{token}}"),
            KeyValue::new("X-Api-Key", ""),
        ],
        Some(Auth::Bearer {
            token: "{{token}}".into(),
        }),
    ));
    assert_eq!(r.headers[0].value, "Bearer {{token}}");
    assert_eq!(r.headers[1].value, "");
    assert_eq!(
        r.auth,
        Some(Auth::Bearer {
            token: "{{token}}".into()
        })
    );
}

#[test]
fn body_is_left_alone() {
    let original = with(vec![], None);
    assert_eq!(redact(&original).body, original.body);
}
