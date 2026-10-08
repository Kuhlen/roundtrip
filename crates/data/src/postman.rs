//! Postman collection v2.0 / v2.1 → ImportedCollection.

use domain::AppError;
use domain::auth::{ApiKeyPlace, Auth};
use domain::collection::Protocol;
use domain::graphql::GraphqlBody;
use domain::http::{Body, FormField, KeyValue, Method, Request, TextKind};
use domain::import::{
    ImportFormat, ImportItem, ImportWarning, ImportedCollection, ImportedRequest,
};
use serde_json::Value;

use crate::json_text as text;

type Warnings = Vec<ImportWarning>;

pub(crate) fn from_value(root: &Value) -> Result<ImportedCollection, AppError> {
    let (Some(info), Some(items)) = (root.get("info"), root.get("item").and_then(Value::as_array))
    else {
        return Err(AppError::Import("not a Postman collection".into()));
    };
    let mut w = Warnings::new();
    let auth = root
        .get("auth")
        .filter(|a| !a.is_null())
        .and_then(|a| auth(a, &mut w));
    dropped_scripts(root.get("event"), &mut w);
    let items = convert(items, auth.is_some(), &mut w);
    let vars: Vec<(String, String)> = rows(root.get("variable"), &mut w)
        .into_iter()
        .map(|r| (r.key, r.value))
        .collect();
    let environments = if vars.is_empty() {
        vec![]
    } else {
        vec![("Collection Variables".to_owned(), vars)]
    };
    let name = text(info, "name");
    Ok(ImportedCollection {
        name: if name.trim().is_empty() {
            "Imported collection".into()
        } else {
            name
        },
        format: ImportFormat::Postman,
        auth,
        items,
        environments,
        warnings: w,
    })
}

fn convert(list: &[Value], collection_auth: bool, w: &mut Warnings) -> Vec<ImportItem> {
    list.iter()
        .filter_map(|item| {
            let name = text(item, "name");
            if let Some(children) = item.get("item").and_then(Value::as_array) {
                if item.get("auth").is_some_and(|a| !a.is_null()) {
                    w.push(ImportWarning::FolderAuth);
                }
                dropped_scripts(item.get("event"), w);
                return Some(ImportItem::Folder {
                    name,
                    items: convert(children, collection_auth, w),
                });
            }
            let req = item.get("request")?;
            request(name, item, req, collection_auth, w).map(ImportItem::Request)
        })
        .collect()
}

fn request(
    name: String,
    item: &Value,
    req: &Value,
    collection_auth: bool,
    w: &mut Warnings,
) -> Option<ImportedRequest> {
    // v2.0 allows the request to be just its URL
    if let Some(url) = req.as_str() {
        return Some(ImportedRequest {
            name,
            request: Request {
                url: url.to_owned(),
                ..Request::default()
            },
            ..ImportedRequest::default()
        });
    }
    let method_name = req
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_uppercase();
    let Some(method) = Method::parse(&method_name) else {
        w.push(ImportWarning::UnknownMethod(method_name));
        return None;
    };
    let (url, params) = url(req.get("url"), w);
    let headers = rows(req.get("header"), w);
    let (body, protocol) = body(req.get("body"), w);
    let auth = match req.get("auth").filter(|a| !a.is_null()) {
        Some(a) => {
            if collection_auth && a.get("type").and_then(Value::as_str) == Some("noauth") {
                w.push(ImportWarning::NoAuthInherits);
            }
            auth(a, w)
        }
        None => None,
    };
    let (pre_request_script, tests) = scripts(item.get("event"), w);
    Some(ImportedRequest {
        name,
        request: Request {
            method,
            url,
            params,
            headers,
            body,
            auth,
        },
        protocol,
        description: description(req.get("description")),
        pre_request_script,
        tests,
    })
}

/// (URL without query, params); query split only when Postman lists it.
fn url(v: Option<&Value>, w: &mut Warnings) -> (String, Vec<KeyValue>) {
    let Some(u) = v.filter(|u| u.is_object()) else {
        return (
            v.and_then(Value::as_str).unwrap_or_default().to_owned(),
            vec![],
        );
    };
    let raw = u
        .get("raw")
        .and_then(Value::as_str)
        .map_or_else(|| rebuild(u), str::to_owned);
    if u.get("variable")
        .and_then(Value::as_array)
        .is_some_and(|v| !v.is_empty())
    {
        w.push(ImportWarning::PathVariables);
    }
    match u.get("query").filter(|q| q.is_array()) {
        Some(q) => {
            let base = raw
                .split_once('?')
                .map_or(raw.as_str(), |(b, _)| b)
                .to_owned();
            (base, rows(Some(q), w))
        }
        None => (raw, vec![]),
    }
}

fn rebuild(u: &Value) -> String {
    let join = |key: &str, sep: &str| match u.get(key) {
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(sep),
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    };
    let protocol = u
        .get("protocol")
        .and_then(Value::as_str)
        .map(|p| format!("{p}://"))
        .unwrap_or_default();
    let path = join("path", "/");
    let path = if path.is_empty() {
        path
    } else {
        format!("/{path}")
    };
    format!("{protocol}{}{path}", join("host", "."))
}

/// `[{key, value, disabled}]`; v2.0 headers may be `"K: V"` lines.
fn rows(v: Option<&Value>, w: &mut Warnings) -> Vec<KeyValue> {
    let Some(list) = v.and_then(Value::as_array) else {
        return v
            .and_then(Value::as_str)
            .map(|s| {
                s.lines()
                    .filter_map(|l| l.split_once(':'))
                    .map(|(k, v)| KeyValue::new(k.trim(), v.trim()))
                    .collect()
            })
            .unwrap_or_default();
    };
    let mut dropped = false;
    let out = list
        .iter()
        .filter(|r| {
            let off = r.get("disabled").and_then(Value::as_bool) == Some(true);
            dropped |= off;
            !off
        })
        .map(|r| KeyValue::new(text(r, "key"), text(r, "value")))
        .collect();
    if dropped {
        w.push(ImportWarning::DisabledDropped);
    }
    out
}

fn body(v: Option<&Value>, w: &mut Warnings) -> (Body, Protocol) {
    let Some(b) = v.filter(|b| !b.is_null()) else {
        return (Body::None, Protocol::Http);
    };
    let body = match b.get("mode").and_then(Value::as_str).unwrap_or_default() {
        "raw" => {
            let raw = text(b, "raw");
            let kind = match b.pointer("/options/raw/language").and_then(Value::as_str) {
                Some("json") => TextKind::Json,
                Some("xml") => TextKind::Xml,
                _ => TextKind::Raw,
            };
            if raw.is_empty() {
                Body::None
            } else {
                Body::Text { kind, text: raw }
            }
        }
        "urlencoded" => Body::Urlencoded(rows(b.get("urlencoded"), w)),
        "formdata" => Body::FormData(
            b.get("formdata")
                .and_then(Value::as_array)
                .map(|fields| fields.iter().map(form_field).collect())
                .unwrap_or_default(),
        ),
        "file" => b
            .pointer("/file/src")
            .and_then(Value::as_str)
            .map_or(Body::None, |p| Body::Binary(p.to_owned())),
        "graphql" => {
            let g = b.get("graphql").cloned().unwrap_or_default();
            let gql = GraphqlBody {
                query: text(&g, "query"),
                variables: text(&g, "variables"),
                operation_name: String::new(),
            };
            return match crate::graphql::to_json(&gql) {
                Ok(json) => (
                    Body::Text {
                        kind: TextKind::Json,
                        text: json,
                    },
                    Protocol::Graphql,
                ),
                Err(_) => {
                    w.push(ImportWarning::UnsupportedBody("graphql".into()));
                    (Body::None, Protocol::Http)
                }
            };
        }
        "" => Body::None,
        other => {
            w.push(ImportWarning::UnsupportedBody(other.to_owned()));
            Body::None
        }
    };
    (body, Protocol::Http)
}

fn form_field(f: &Value) -> FormField {
    let key = text(f, "key");
    let field = if f.get("type").and_then(Value::as_str) == Some("file") {
        let src = match f.get("src") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(a)) => a
                .first()
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            _ => String::new(),
        };
        FormField::file(key, src)
    } else {
        FormField::text(key, text(f, "value"))
    };
    FormField {
        enabled: f.get("disabled").and_then(Value::as_bool) != Some(true),
        ..field
    }
}

fn auth(v: &Value, w: &mut Warnings) -> Option<Auth> {
    let kind = v.get("type").and_then(Value::as_str).unwrap_or("noauth");
    // v2.1: [{key, value}]; v2.0: {key: value}
    let field = |name: &str| match v.get(kind) {
        Some(Value::Array(entries)) => entries
            .iter()
            .find(|e| e.get("key").and_then(Value::as_str) == Some(name))
            .map(|e| text(e, "value"))
            .unwrap_or_default(),
        Some(o @ Value::Object(_)) => text(o, name),
        _ => String::new(),
    };
    match kind {
        "noauth" | "inherit" => None,
        "bearer" => Some(Auth::Bearer {
            token: field("token"),
        }),
        "basic" => Some(Auth::Basic {
            username: field("username"),
            password: field("password"),
        }),
        "apikey" => Some(Auth::ApiKey {
            key: field("key"),
            value: field("value"),
            place: if field("in") == "query" {
                ApiKeyPlace::Query
            } else {
                ApiKeyPlace::Header
            },
        }),
        other => {
            w.push(ImportWarning::UnsupportedAuth(other.to_owned()));
            None
        }
    }
}

/// root and folder scripts have no home in the file layout
fn dropped_scripts(v: Option<&Value>, w: &mut Warnings) {
    if scripts(v, &mut Warnings::new()) != (None, None) {
        w.push(ImportWarning::CollectionScriptsDropped);
    }
}

fn scripts(v: Option<&Value>, w: &mut Warnings) -> (Option<String>, Option<String>) {
    let (mut pre, mut tests) = (None, None);
    for event in v.and_then(Value::as_array).into_iter().flatten() {
        let code = match event.pointer("/script/exec") {
            Some(Value::Array(lines)) => lines
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n"),
            Some(Value::String(s)) => s.clone(),
            _ => continue,
        };
        if code.trim().is_empty() {
            continue;
        }
        match event.get("listen").and_then(Value::as_str) {
            Some("prerequest") => pre = Some(code),
            Some("test") => tests = Some(code),
            _ => {}
        }
    }
    if pre.is_some() || tests.is_some() {
        w.push(ImportWarning::Scripts);
    }
    (pre, tests)
}

/// String, or v2.1 `{content}`.
fn description(v: Option<&Value>) -> Option<String> {
    let d = match v? {
        Value::String(s) => s.clone(),
        o => text(o, "content"),
    };
    (!d.trim().is_empty()).then_some(d)
}
