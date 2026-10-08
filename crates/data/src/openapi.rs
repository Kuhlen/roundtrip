//! OpenAPI 3.0 / 3.1 → ImportedCollection. URLs use `{{baseUrl}}`; each server is an environment.

use std::collections::HashSet;

use domain::auth::{ApiKeyPlace, Auth};
use domain::http::{Body, FormField, KeyValue, Method, Request, TextKind};
use domain::import::{
    ImportFormat, ImportItem, ImportWarning, ImportedCollection, ImportedRequest,
};
use serde_json::Value;

use crate::json_text as text;

type Warnings = Vec<ImportWarning>;

/// Path item keys that are operations; `parameters`, `summary`, `servers` are not.
const OPERATIONS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

pub(crate) fn from_value(root: &Value, version: String) -> ImportedCollection {
    let mut w = Warnings::new();
    let global = root.get("security").and_then(|s| security(root, s, &mut w));
    // declared tags first, in their order; HashMap order would change per import
    let mut folders: Vec<(String, Vec<ImportItem>)> = root
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|t| (text(t, "name"), Vec::new()))
        .collect();
    let mut loose = Vec::new();
    let paths = root.get("paths").and_then(Value::as_object);
    for (path, item) in paths.into_iter().flatten() {
        let Some(item) = resolve(root, item, &mut w) else {
            continue;
        };
        for (key, op) in item.as_object().into_iter().flatten() {
            let key = key.to_lowercase();
            if !OPERATIONS.contains(&key.as_str()) {
                continue;
            }
            let upper = key.to_uppercase();
            let Some(method) = Method::parse(&upper) else {
                w.push(ImportWarning::UnknownMethod(upper));
                continue;
            };
            let req = ImportItem::Request(request(
                root,
                path,
                method,
                (item, op),
                global.as_ref(),
                &mut w,
            ));
            let tag = op
                .get("tags")
                .and_then(Value::as_array)
                .and_then(|t| t.first())
                .and_then(Value::as_str);
            match tag {
                Some(tag) => match folders.iter_mut().find(|(name, _)| name == tag) {
                    Some((_, list)) => list.push(req),
                    None => folders.push((tag.to_owned(), vec![req])),
                },
                None => loose.push(req),
            }
        }
    }
    let items = folders
        .into_iter()
        .filter(|(_, list)| !list.is_empty())
        .map(|(name, items)| ImportItem::Folder { name, items })
        .chain(loose)
        .collect();
    let title = text(root.get("info").unwrap_or(&Value::Null), "title");
    ImportedCollection {
        name: if title.trim().is_empty() {
            "Imported API".into()
        } else {
            title.trim().to_owned()
        },
        format: ImportFormat::OpenApi(version),
        auth: global,
        items,
        environments: environments(root),
        warnings: w,
    }
}

fn request(
    root: &Value,
    path: &str,
    method: Method,
    (item, op): (&Value, &Value),
    global: Option<&Auth>,
    w: &mut Warnings,
) -> ImportedRequest {
    let name = [text(op, "summary"), text(op, "operationId")]
        .into_iter()
        .find(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("{} {path}", method.as_str()));
    if path.contains('{') {
        w.push(ImportWarning::PathVariables);
    }
    let (mut params, mut headers) = (Vec::new(), Vec::new());
    for p in parameters(root, item, op, w) {
        let row = KeyValue::new(text(p, "name"), example(p));
        match text(p, "in").as_str() {
            "query" => params.push(row),
            "header" => headers.push(row),
            "cookie" => w.push(ImportWarning::CookieParams),
            _ => {}
        }
    }
    let (body, media) = body(root, op, w);
    if let (Body::Text { kind, .. }, Some(media)) = (&body, media)
        && media != kind.content_type()
    {
        headers.push(KeyValue::new("Content-Type", media));
    }
    let auth = match op.get("security") {
        Some(Value::Array(list)) if list.is_empty() => {
            if global.is_some() {
                w.push(ImportWarning::NoAuthInherits);
            }
            None
        }
        Some(list) => security(root, list, w).filter(|a| Some(a) != global),
        None => None,
    };
    let description = text(op, "description");
    ImportedRequest {
        name,
        request: Request {
            method,
            url: format!("{{{{baseUrl}}}}{path}"),
            params,
            headers,
            body,
            auth,
        },
        description: (!description.trim().is_empty()).then_some(description),
        ..ImportedRequest::default()
    }
}

/// Path-level then operation-level; the operation wins on the same name + location.
fn parameters<'a>(
    root: &'a Value,
    item: &'a Value,
    op: &'a Value,
    w: &mut Warnings,
) -> Vec<&'a Value> {
    let mut out: Vec<&Value> = Vec::new();
    let lists = [item, op]
        .into_iter()
        .filter_map(|v| v.get("parameters").and_then(Value::as_array));
    for p in lists.flatten() {
        let Some(p) = resolve(root, p, w) else {
            continue;
        };
        out.retain(|q| (text(q, "name"), text(q, "in")) != (text(p, "name"), text(p, "in")));
        out.push(p);
    }
    out
}

/// parity: `example`, then `schema.default`
fn example(p: &Value) -> String {
    p.get("example")
        .or_else(|| p.pointer("/schema/default"))
        .map(scalar)
        .unwrap_or_default()
}

/// Form property: its own `example` or `default`.
fn prop_value(p: &Value) -> String {
    p.get("example")
        .or_else(|| p.get("default"))
        .map(scalar)
        .unwrap_or_default()
}

/// Upstream reads strings only, so `limit: 10` came out empty; numbers and bools keep their text.
fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// First media type Roundtrip can edit: JSON, XML, urlencoded, multipart, binary, text.
/// The media type comes back for Text bodies so a non-default one becomes a header.
fn body(root: &Value, op: &Value, w: &mut Warnings) -> (Body, Option<String>) {
    let content = op
        .get("requestBody")
        .and_then(|b| resolve(root, b, w))
        .and_then(|b| b.get("content"))
        .and_then(Value::as_object);
    let Some(content) = content else {
        return (Body::None, None);
    };
    let find = |want: fn(&str) -> bool| {
        content
            .iter()
            .map(|(k, m)| (essence(k), m))
            .find(|(k, _)| want(k))
    };
    if let Some((media, m)) = find(|t| t == "application/json" || t.ends_with("+json")) {
        let text = match media_example(root, m, w) {
            Some(e) => as_text(&e),
            None => m
                .get("schema")
                .map(|s| sample(root, s, 0, &mut 2000, w))
                .and_then(|v| serde_json::to_string_pretty(&v).ok())
                .unwrap_or_default(),
        };
        return (
            Body::Text {
                kind: TextKind::Json,
                text,
            },
            Some(media),
        );
    }
    if let Some((media, m)) = find(|t| t == "application/xml" || t == "text/xml") {
        let text = media_example(root, m, w)
            .map(|e| as_text(&e))
            .unwrap_or_default();
        return (
            Body::Text {
                kind: TextKind::Xml,
                text,
            },
            Some(media),
        );
    }
    if let Some((_, m)) = find(|t| t == "application/x-www-form-urlencoded") {
        let rows = properties(root, m, w)
            .into_iter()
            .map(|(k, p)| KeyValue::new(k, prop_value(p)))
            .collect();
        return (Body::Urlencoded(rows), None);
    }
    if let Some((_, m)) = find(|t| t == "multipart/form-data") {
        let fields = properties(root, m, w)
            .into_iter()
            .map(|(k, p)| {
                if text(p, "format") == "binary" || p.get("contentMediaType").is_some() {
                    FormField::file(k, "")
                } else {
                    FormField::text(k, prop_value(p))
                }
            })
            .collect();
        return (Body::FormData(fields), None);
    }
    if find(|t| t == "application/octet-stream").is_some() {
        return (Body::Binary(String::new()), None);
    }
    if let Some((media, m)) = find(|t| t.starts_with("text/")) {
        let text = media_example(root, m, w)
            .map(|e| as_text(&e))
            .unwrap_or_default();
        return (
            Body::Text {
                kind: TextKind::Raw,
                text,
            },
            Some(media),
        );
    }
    (Body::None, None)
}

/// `application/json; charset=utf-8` → `application/json`
fn essence(media: &str) -> String {
    media
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_lowercase()
}

/// `example`, else the first of `examples` (each may be a `$ref`).
fn media_example(root: &Value, m: &Value, w: &mut Warnings) -> Option<Value> {
    if let Some(e) = m.get("example") {
        return Some(e.clone());
    }
    let first = m.get("examples")?.as_object()?.values().next()?;
    resolve(root, first, w)?.get("value").cloned()
}

/// Strings as written (an XML or JSON text example), the rest as pretty JSON.
fn as_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

fn properties<'a>(root: &'a Value, m: &'a Value, w: &mut Warnings) -> Vec<(&'a str, &'a Value)> {
    let props = m
        .get("schema")
        .and_then(|s| resolve(root, s, w))
        .and_then(|s| s.get("properties"))
        .and_then(Value::as_object);
    props
        .into_iter()
        .flatten()
        .filter_map(|(k, p)| Some((k.as_str(), resolve(root, p, w)?)))
        .collect()
}

/// parity: upstream example generator. Depth 5 (a `$ref` costs one) also ends cycles.
fn sample(
    root: &Value,
    schema: &Value,
    depth: usize,
    budget: &mut usize,
    w: &mut Warnings,
) -> Value {
    // depth caps height only; budget bounds width. Charged first: capped leaves dominate
    if *budget == 0 {
        return Value::Null;
    }
    *budget -= 1;
    if depth > 5 {
        return Value::Null;
    }
    if schema.get("$ref").is_some() {
        return match resolve(root, schema, w) {
            Some(target) => sample(root, target, depth + 1, budget, w),
            None => Value::Null,
        };
    }
    if let Some(v) = schema.get("example").or_else(|| schema.get("default")) {
        return v.clone();
    }
    // 3.1 allows `type: [string, "null"]`
    let ty = match schema.get("type") {
        Some(Value::String(t)) => t.as_str(),
        Some(Value::Array(types)) => types
            .iter()
            .filter_map(Value::as_str)
            .find(|t| *t != "null")
            .unwrap_or_default(),
        _ => "",
    };
    match ty {
        "object" => Value::Object(
            schema
                .get("properties")
                .and_then(Value::as_object)
                .map(|props| {
                    props
                        .iter()
                        .map(|(k, p)| (k.clone(), sample(root, p, depth + 1, budget, w)))
                        .collect()
                })
                .unwrap_or_default(),
        ),
        "array" => Value::Array(
            schema
                .get("items")
                .map(|i| vec![sample(root, i, depth + 1, budget, w)])
                .unwrap_or_default(),
        ),
        "string" => Value::from(
            match schema
                .get("format")
                .and_then(Value::as_str)
                .unwrap_or_default()
            {
                "email" => "user@example.com",
                "date-time" => "2024-01-01T00:00:00Z",
                "date" => "2024-01-01",
                "uri" | "url" => "https://example.com",
                "uuid" => "550e8400-e29b-41d4-a716-446655440000",
                _ => "string",
            },
        ),
        "integer" => Value::from(0),
        "number" => Value::from(0.0),
        "boolean" => Value::from(false),
        _ => Value::Null,
    }
}

/// Follows local `$ref`s. External → None + warning; broken or looping → None.
fn resolve<'a>(root: &'a Value, mut v: &'a Value, w: &mut Warnings) -> Option<&'a Value> {
    let mut seen = HashSet::new();
    while let Some(r) = v.get("$ref").and_then(Value::as_str) {
        let Some(pointer) = r.strip_prefix('#') else {
            w.push(ImportWarning::ExternalRef);
            return None;
        };
        if !seen.insert(pointer) {
            return None;
        }
        v = root.pointer(pointer)?;
    }
    Some(v)
}

/// First alternative of a `security` list, with placeholders the user fills in an environment.
fn security(root: &Value, list: &Value, w: &mut Warnings) -> Option<Auth> {
    let (name, _) = list.as_array()?.first()?.as_object()?.iter().next()?;
    let scheme = root.pointer("/components/securitySchemes")?.get(name)?;
    let scheme = resolve(root, scheme, w)?;
    let var = |name: &str| format!("{{{{{name}}}}}");
    let kind = text(scheme, "type");
    let http = text(scheme, "scheme").to_lowercase();
    let place = text(scheme, "in");
    match (kind.as_str(), http.as_str(), place.as_str()) {
        ("http", "bearer", _) => Some(Auth::Bearer {
            token: var("token"),
        }),
        ("http", "basic", _) => Some(Auth::Basic {
            username: var("username"),
            password: var("password"),
        }),
        ("apiKey", _, "header" | "query") => Some(Auth::ApiKey {
            key: text(scheme, "name"),
            value: var("apiKey"),
            place: if place == "query" {
                ApiKeyPlace::Query
            } else {
                ApiKeyPlace::Header
            },
        }),
        ("apiKey", _, _) => {
            w.push(ImportWarning::UnsupportedAuth("cookie apiKey".into()));
            None
        }
        ("http", other, _) => {
            w.push(ImportWarning::UnsupportedAuth(format!("http {other}")));
            None
        }
        (other, _, _) => {
            w.push(ImportWarning::UnsupportedAuth(other.to_owned()));
            None
        }
    }
}

/// One per server: `baseUrl` with `{var}` filled from its default; name from description, else URL.
fn environments(root: &Value) -> Vec<(String, Vec<(String, String)>)> {
    let mut used = HashSet::new();
    let servers = root.get("servers").and_then(Value::as_array);
    servers
        .into_iter()
        .flatten()
        .map(|s| {
            let mut url = text(s, "url");
            for (k, v) in s
                .get("variables")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
            {
                url = url.replace(&format!("{{{k}}}"), &text(v, "default"));
            }
            // paths start with '/'
            let url = url.trim_end_matches('/').to_owned();
            let label = text(s, "description");
            let base = if label.trim().is_empty() {
                url.clone()
            } else {
                label.trim().to_owned()
            };
            let mut name = base.clone();
            let mut n = 2;
            while !used.insert(name.clone()) {
                name = format!("{base} {n}");
                n += 1;
            }
            (name, vec![("baseUrl".to_owned(), url)])
        })
        .collect()
}
