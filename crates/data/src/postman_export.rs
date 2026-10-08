//! ApiArk collection folder → Postman v2.1 file. Reads what is on disk, never the open tabs.

use std::fs;
use std::path::Path;

use domain::auth::{ApiKeyPlace, Auth};
use domain::collection::{CollectionStore, Node, Protocol};
use domain::environment::EnvironmentStore;
use domain::http::{self, Body, FormField, Request, TextKind};
use domain::import::{ExportReport, ExportWarning};
use domain::{AppError, curl};
use serde_json::{Map, Value, json};
use serde_yaml::Mapping;

use crate::collection_dir::{CollectionDir, request_from_yaml, write_text_atomic};
use crate::environment_dir::EnvironmentDir;
use crate::storage;

const SCHEMA: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

#[derive(Default)]
struct Tally {
    requests: usize,
    protocol: usize,
    unreadable: usize,
    body: usize,
    auth: usize,
}

pub fn write(root: &Path, dest: &Path) -> Result<ExportReport, AppError> {
    let collection = CollectionDir.load(root)?;
    let mut t = Tally::default();
    let mut doc = Map::new();
    doc.insert(
        "info".into(),
        json!({ "name": collection.name, "schema": SCHEMA }),
    );
    doc.insert(
        "item".into(),
        Value::Array(nodes(root, &collection.children, &mut t)),
    );
    if let Some(a) = &collection.auth {
        match auth(a) {
            Some(v) => {
                doc.insert("auth".into(), v);
            }
            None => t.auth += 1,
        }
    }
    let text = serde_json::to_string_pretty(&Value::Object(doc))
        .map_err(|e| AppError::Storage(e.to_string()))?;
    write_text_atomic(dest, &text)?;
    // a broken environment file must not fail the export; it only changes the count
    let envs = EnvironmentDir.list(root).map_or(0, |e| e.len());
    let mut warnings = Vec::new();
    let mut add = |n: usize, w: fn(usize) -> ExportWarning| {
        if n > 0 {
            warnings.push(w(n));
        }
    };
    add(t.protocol, ExportWarning::SkippedProtocol);
    add(t.unreadable, ExportWarning::Unreadable);
    add(t.body, ExportWarning::UnsupportedBody);
    add(t.auth, ExportWarning::UnsupportedAuth);
    add(envs, ExportWarning::EnvironmentsNotExported);
    Ok(ExportReport {
        requests: t.requests,
        warnings,
    })
}

fn nodes(root: &Path, list: &[Node], t: &mut Tally) -> Vec<Value> {
    list.iter()
        .filter_map(|n| match n {
            Node::Folder { name, children, .. } => {
                Some(json!({ "name": name, "item": nodes(root, children, t) }))
            }
            Node::Request {
                name,
                protocol,
                path,
                ..
            } => {
                if !protocol.is_sendable() {
                    t.protocol += 1;
                    return None;
                }
                match item(root, name, *protocol, path, t) {
                    Ok(v) => {
                        t.requests += 1;
                        Some(v)
                    }
                    Err(_) => {
                        t.unreadable += 1;
                        None
                    }
                }
            }
        })
        .collect()
}

fn item(
    root: &Path,
    name: &str,
    protocol: Protocol,
    file: &Path,
    t: &mut Tally,
) -> Result<Value, AppError> {
    let text = fs::read_to_string(file).map_err(|e| storage(file, e))?;
    // description and scripts are not part of Request
    let doc: Mapping = serde_yaml::from_str(&text).map_err(|e| AppError::Storage(e.to_string()))?;
    let mut request = request_from_yaml(&text)?;
    http::resolve_files(&mut request, root);
    let mut r = Map::new();
    r.insert("method".into(), request.method.as_str().into());
    r.insert(
        "header".into(),
        Value::Array(
            request
                .headers
                .iter()
                .filter(|h| h.is_active())
                .map(|h| json!({ "key": h.key, "value": h.value }))
                .collect(),
        ),
    );
    r.insert("url".into(), url(&request));
    if let Some(b) = body(&request.body, protocol, t) {
        r.insert("body".into(), b);
    }
    if let Some(a) = &request.auth {
        match auth(a) {
            Some(v) => {
                r.insert("auth".into(), v);
            }
            None => t.auth += 1,
        }
    }
    if let Some(d) = yaml_text(&doc, "description") {
        r.insert("description".into(), d.into());
    }
    let events: Vec<Value> = [("prerequest", "preRequestScript"), ("test", "tests")]
        .into_iter()
        .filter_map(|(listen, key)| {
            yaml_text(&doc, key).map(|code| {
                json!({
                    "listen": listen,
                    "script": { "type": "text/javascript", "exec": code.lines().collect::<Vec<_>>() }
                })
            })
        })
        .collect();
    let mut item = Map::new();
    item.insert("name".into(), name.into());
    item.insert("request".into(), Value::Object(r));
    if !events.is_empty() {
        item.insert("event".into(), Value::Array(events));
    }
    Ok(Value::Object(item))
}

fn yaml_text<'a>(doc: &'a Mapping, key: &str) -> Option<&'a str> {
    doc.get(key)
        .and_then(serde_yaml::Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

/// `raw` carries the query too: Postman shows `raw`, our import reads `query`.
fn url(r: &Request) -> Value {
    let active: Vec<_> = r.params.iter().filter(|p| p.is_active()).collect();
    if active.is_empty() {
        return json!({ "raw": r.url });
    }
    let sep = if r.url.contains('?') { '&' } else { '?' };
    let pairs: Vec<String> = active
        .iter()
        .map(|p| format!("{}={}", p.key, p.value))
        .collect();
    let own = curl::split_query(&r.url).1;
    let query: Vec<Value> = own
        .iter()
        .chain(active.iter().copied())
        .map(|p| json!({ "key": p.key, "value": p.value }))
        .collect();
    json!({
        "raw": format!("{}{sep}{}", r.url, pairs.join("&")),
        "query": query
    })
}

fn body(body: &Body, protocol: Protocol, t: &mut Tally) -> Option<Value> {
    Some(match body {
        Body::None => return None,
        Body::Text { text, .. } if protocol == Protocol::Graphql => {
            match crate::graphql::parse(text) {
                Some(g) => json!({
                    "mode": "graphql",
                    "graphql": { "query": g.query, "variables": g.variables }
                }),
                None => raw(text, "json"),
            }
        }
        Body::Text { kind, text } => raw(
            text,
            match kind {
                TextKind::Json => "json",
                TextKind::Xml => "xml",
                TextKind::Raw => "text",
            },
        ),
        Body::Urlencoded(rows) => json!({
            "mode": "urlencoded",
            "urlencoded": rows
                .iter()
                .filter(|r| r.is_active())
                .map(|r| json!({ "key": r.key, "value": r.value }))
                .collect::<Vec<_>>()
        }),
        Body::FormData(fields) => json!({
            "mode": "formdata",
            "formdata": fields
                .iter()
                .filter(|f| !f.key.is_empty())
                .map(form_field)
                .collect::<Vec<_>>()
        }),
        Body::Binary(path) => json!({ "mode": "file", "file": { "src": path } }),
        Body::Unsupported(_) => {
            t.body += 1;
            return None;
        }
    })
}

fn raw(text: &str, language: &str) -> Value {
    json!({ "mode": "raw", "raw": text, "options": { "raw": { "language": language } } })
}

fn form_field(f: &FormField) -> Value {
    let mut v = if f.is_file {
        json!({ "key": f.key, "type": "file", "src": f.value })
    } else {
        json!({ "key": f.key, "value": f.value, "type": "text" })
    };
    if !f.enabled {
        v["disabled"] = true.into();
    }
    v
}

fn auth(a: &Auth) -> Option<Value> {
    let fields = |pairs: &[(&str, &str)]| {
        pairs
            .iter()
            .map(|(k, v)| json!({ "key": k, "value": v, "type": "string" }))
            .collect::<Vec<_>>()
    };
    Some(match a {
        Auth::Bearer { token } => {
            json!({ "type": "bearer", "bearer": fields(&[("token", token.as_str())]) })
        }
        Auth::Basic { username, password } => json!({
            "type": "basic",
            "basic": fields(&[("username", username.as_str()), ("password", password.as_str())])
        }),
        Auth::ApiKey { key, value, place } => json!({
            "type": "apikey",
            "apikey": fields(&[
                ("key", key.as_str()),
                ("value", value.as_str()),
                ("in", match place {
                    ApiKeyPlace::Header => "header",
                    ApiKeyPlace::Query => "query",
                }),
            ])
        }),
        Auth::Unsupported(_) => return None,
    })
}
