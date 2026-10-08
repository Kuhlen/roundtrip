//! Insomnia v4 (`resources[]`, JSON or YAML) and v5 (YAML tree) exports → ImportedCollection.

use domain::auth::{ApiKeyPlace, Auth};
use domain::collection::Protocol;
use domain::http::{Body, FormField, KeyValue, Method, Request, TextKind};
use domain::import::{
    ImportFormat, ImportItem, ImportWarning, ImportedCollection, ImportedRequest,
};
use domain::{AppError, curl};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

use crate::json_text as text;

type Warnings = Vec<ImportWarning>;
type Vars = Vec<(String, String)>;

pub(crate) fn from_v4(root: &Value) -> Result<ImportedCollection, AppError> {
    let mut w = Warnings::new();
    let resources: Vec<&Value> = root
        .get("resources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect();
    let workspaces = of_kind(&resources, "workspace");
    let Some(ws) = workspaces.first() else {
        return Err(AppError::Import(
            "Insomnia export without a workspace".into(),
        ));
    };
    // upstream silently took the last one
    if workspaces.len() > 1 {
        w.push(ImportWarning::OtherWorkspaces(workspaces.len() - 1));
    }
    let ws_id = text(ws, "_id");
    let mut by_parent: HashMap<String, Vec<&Value>> = HashMap::new();
    for r in &resources {
        by_parent.entry(text(r, "parentId")).or_default().push(r);
    }
    let mut visited = HashSet::from([ws_id.clone()]);
    let items = v4_items(&by_parent, &ws_id, &mut visited, &mut w);
    let envs_under = |parent: &str| {
        sorted(
            of_kind(&resources, "environment")
                .into_iter()
                .filter(|e| text(e, "parentId") == parent)
                .collect(),
        )
    };
    let base = envs_under(&ws_id).into_iter().next();
    let subs = base
        .map(|b| {
            envs_under(&text(b, "_id"))
                .into_iter()
                .map(|s| (text(s, "name"), s.get("data").unwrap_or(&Value::Null)))
                .collect()
        })
        .unwrap_or_default();
    let environments = environments(base.and_then(|b| b.get("data")), subs);
    Ok(finish(text(ws, "name"), items, environments, w))
}

pub(crate) fn from_v5(root: &Value) -> ImportedCollection {
    let mut w = Warnings::new();
    let list = root
        .get("collection")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let items = v5_items(list, &mut w);
    let envs = root.get("environments");
    let subs = sorted(
        envs.and_then(|e| e.get("subEnvironments"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .collect(),
    )
    .into_iter()
    .map(|s| (text(s, "name"), s.get("data").unwrap_or(&Value::Null)))
    .collect();
    let environments = environments(envs.and_then(|e| e.get("data")), subs);
    finish(text(root, "name"), items, environments, w)
}

fn v5_items(list: &[Value], w: &mut Warnings) -> Vec<ImportItem> {
    sorted(list.iter().collect())
        .into_iter()
        .filter_map(|r| {
            if let Some(children) = r.get("children").and_then(Value::as_array) {
                let items = v5_items(children, w);
                return Some(folder(r, items, w));
            }
            let id = r
                .pointer("/meta/id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if id.starts_with("greq_") {
                skipped("gRPC", w)
            } else if id.starts_with("ws-req_") {
                skipped("WebSocket", w)
            } else {
                request(r, w).map(ImportItem::Request)
            }
        })
        .collect()
}

fn of_kind<'a>(resources: &[&'a Value], kind: &str) -> Vec<&'a Value> {
    resources
        .iter()
        .copied()
        .filter(|r| text(r, "_type") == kind)
        .collect()
}

/// `visited` stops cycles: a group with no id or a repeated id would recurse forever.
fn v4_items(
    by_parent: &HashMap<String, Vec<&Value>>,
    parent: &str,
    visited: &mut HashSet<String>,
    w: &mut Warnings,
) -> Vec<ImportItem> {
    let children = sorted(by_parent.get(parent).cloned().unwrap_or_default());
    children
        .into_iter()
        .filter_map(|r| match text(r, "_type").as_str() {
            "request_group" => {
                let id = text(r, "_id");
                if id.is_empty() || !visited.insert(id.clone()) {
                    return None;
                }
                let items = v4_items(by_parent, &id, visited, w);
                Some(folder(r, items, w))
            }
            "request" => request(r, w).map(ImportItem::Request),
            "grpc_request" => skipped("gRPC", w),
            "websocket_request" => skipped("WebSocket", w),
            _ => None,
        })
        .collect()
}

/// Insomnia's display order; the sort is stable, so equal keys keep file order.
fn sorted(mut list: Vec<&Value>) -> Vec<&Value> {
    let key = |r: &Value| {
        r.get("metaSortKey")
            .or_else(|| r.pointer("/meta/sortKey"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
    };
    list.sort_by(|a, b| key(a).total_cmp(&key(b)));
    list
}

fn skipped(kind: &str, w: &mut Warnings) -> Option<ImportItem> {
    w.push(ImportWarning::UnsupportedRequest(kind.into()));
    None
}

/// Folder auth and variables have no home in the ApiArk layout.
fn folder(r: &Value, items: Vec<ImportItem>, w: &mut Warnings) -> ImportItem {
    let auth = r
        .get("authentication")
        .map(|a| text(a, "type"))
        .unwrap_or_default();
    if !auth.is_empty() && auth != "none" {
        w.push(ImportWarning::FolderAuth);
    }
    if r.get("environment")
        .and_then(Value::as_object)
        .is_some_and(|e| !e.is_empty())
    {
        w.push(ImportWarning::FolderVariables);
    }
    ImportItem::Folder {
        name: text(r, "name"),
        items,
    }
}

fn request(r: &Value, w: &mut Warnings) -> Option<ImportedRequest> {
    let method_name = match text(r, "method").to_uppercase() {
        m if m.is_empty() => "GET".to_owned(),
        m => m,
    };
    let Some(method) = Method::parse(&method_name) else {
        w.push(ImportWarning::UnknownMethod(method_name));
        return None;
    };
    let (url, mut params) = curl::split_query(&text(r, "url"));
    params.extend(rows(r.get("parameters"), w));
    let mut headers = rows(r.get("headers"), w);
    let (body, protocol) = body(r.get("body"), w);
    let auth = match auth(r.get("authentication"), w) {
        AuthOut::Auth(a) => a,
        AuthOut::Header(h) => {
            headers.push(h);
            None
        }
    };
    // v4 top-level keys, v5 under `scripts`
    let script = |v4: &str, v5: &str| {
        Some(text(r, v4))
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                r.pointer(v5)
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .map(str::to_owned)
            })
    };
    let pre_request_script = script("preRequestScript", "/scripts/preRequest");
    let tests = script("afterResponseScript", "/scripts/afterResponse");
    if pre_request_script.is_some() || tests.is_some() {
        w.push(ImportWarning::Scripts);
    }
    let description = text(r, "description");
    Some(ImportedRequest {
        name: text(r, "name"),
        request: Request {
            method,
            url,
            params,
            headers,
            body,
            auth,
        },
        protocol,
        description: (!description.trim().is_empty()).then_some(description),
        pre_request_script,
        tests,
    })
}

/// `[{name, value, disabled}]`; the ApiArk file has no off switch, so disabled rows go.
fn rows(v: Option<&Value>, w: &mut Warnings) -> Vec<KeyValue> {
    let mut dropped = false;
    let out = v
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|x| {
            let off = x.get("disabled").and_then(Value::as_bool) == Some(true);
            dropped |= off;
            !off
        })
        .map(|x| KeyValue::new(text(x, "name"), text(x, "value")))
        .collect();
    if dropped {
        w.push(ImportWarning::DisabledDropped);
    }
    out
}

fn body(v: Option<&Value>, w: &mut Warnings) -> (Body, Protocol) {
    let Some(b) = v.filter(|b| b.is_object()) else {
        return (Body::None, Protocol::Http);
    };
    let mime = text(b, "mimeType").to_lowercase();
    let raw = text(b, "text");
    if mime == "application/graphql" {
        let json = crate::graphql::parse(&raw).and_then(|g| crate::graphql::to_json(&g).ok());
        return match json {
            Some(text) => (
                Body::Text {
                    kind: TextKind::Json,
                    text,
                },
                Protocol::Graphql,
            ),
            None => {
                w.push(ImportWarning::UnsupportedBody("graphql".into()));
                (Body::None, Protocol::Http)
            }
        };
    }
    let file = text(b, "fileName");
    let body = if mime.contains("urlencoded") {
        Body::Urlencoded(rows(b.get("params"), w))
    } else if mime.starts_with("multipart/") {
        Body::FormData(
            b.get("params")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(form_field)
                .collect(),
        )
    } else if !file.is_empty() {
        Body::Binary(file)
    } else if raw.is_empty() {
        Body::None
    } else {
        let kind = if mime.contains("json") {
            TextKind::Json
        } else if mime.contains("xml") {
            TextKind::Xml
        } else {
            TextKind::Raw
        };
        Body::Text { kind, text: raw }
    };
    (body, Protocol::Http)
}

fn form_field(f: &Value) -> FormField {
    let field = if text(f, "type") == "file" {
        FormField::file(text(f, "name"), text(f, "fileName"))
    } else {
        FormField::text(text(f, "name"), text(f, "value"))
    };
    FormField {
        enabled: f.get("disabled").and_then(Value::as_bool) != Some(true),
        ..field
    }
}

enum AuthOut {
    Auth(Option<Auth>),
    /// bearer with a custom prefix: no Auth variant holds it
    Header(KeyValue),
}

fn auth(v: Option<&Value>, w: &mut Warnings) -> AuthOut {
    let Some(a) = v.filter(|a| a.is_object()) else {
        return AuthOut::Auth(None);
    };
    if a.get("disabled").and_then(Value::as_bool) == Some(true) {
        return AuthOut::Auth(None);
    }
    let auth = match text(a, "type").as_str() {
        "" | "none" => return AuthOut::Auth(None),
        "bearer" => {
            let (prefix, token) = (text(a, "prefix"), text(a, "token"));
            if !(prefix.is_empty() || prefix.eq_ignore_ascii_case("bearer")) {
                return AuthOut::Header(KeyValue::new(
                    "Authorization",
                    format!("{prefix} {token}"),
                ));
            }
            Auth::Bearer { token }
        }
        "basic" => Auth::Basic {
            username: text(a, "username"),
            password: text(a, "password"),
        },
        "apikey" => Auth::ApiKey {
            key: text(a, "key"),
            value: text(a, "value"),
            place: if text(a, "addTo") == "queryParams" {
                ApiKeyPlace::Query
            } else {
                ApiKeyPlace::Header
            },
        },
        other => {
            w.push(ImportWarning::UnsupportedAuth(other.to_owned()));
            return AuthOut::Auth(None);
        }
    };
    AuthOut::Auth(Some(auth))
}

/// One environment per sub, base merged under it; no subs → the base alone as "Base".
fn environments(base: Option<&Value>, subs: Vec<(String, &Value)>) -> Vec<(String, Vars)> {
    let mut base_vars = Vars::new();
    if let Some(b) = base {
        flatten("", b, &mut base_vars);
    }
    if subs.is_empty() {
        return if base_vars.is_empty() {
            vec![]
        } else {
            vec![("Base".into(), base_vars)]
        };
    }
    subs.into_iter()
        .map(|(name, data)| {
            let mut vars = base_vars.clone();
            let mut own = Vars::new();
            flatten("", data, &mut own);
            for (k, v) in own {
                match vars.iter_mut().find(|(b, _)| *b == k) {
                    Some(slot) => slot.1 = v,
                    None => vars.push((k, v)),
                }
            }
            (name, vars)
        })
        .collect()
}

/// `{api: {host}}` → `api.host`, which `{{api.host}}` already interpolates.
fn flatten(prefix: &str, v: &Value, out: &mut Vars) {
    if let Value::Object(map) = v {
        for (k, x) in map {
            let key = if prefix.is_empty() {
                k.clone()
            } else {
                format!("{prefix}.{k}")
            };
            flatten(&key, x, out);
        }
        return;
    }
    if prefix.is_empty() {
        return;
    }
    let value = match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    out.push((prefix.to_owned(), value));
}

fn finish(
    name: String,
    items: Vec<ImportItem>,
    environments: Vec<(String, Vars)>,
    warnings: Warnings,
) -> ImportedCollection {
    let mut c = ImportedCollection {
        name: if name.trim().is_empty() {
            "Imported collection".into()
        } else {
            name
        },
        format: ImportFormat::Insomnia,
        auth: None,
        items,
        environments,
        warnings,
    };
    let mut tags = false;
    each_string(&mut c, &mut |s| {
        tags |= s.contains("{%");
        *s = plain_vars(s);
    });
    if tags {
        c.warnings.push(ImportWarning::TemplateTags);
    }
    c
}

/// Every string a template can sit in.
fn each_string(c: &mut ImportedCollection, f: &mut impl FnMut(&mut String)) {
    fn items(list: &mut [ImportItem], f: &mut impl FnMut(&mut String)) {
        for item in list {
            match item {
                ImportItem::Folder {
                    items: children, ..
                } => items(children, f),
                ImportItem::Request(r) => fix_request(&mut r.request, f),
            }
        }
    }
    fn fix_request(r: &mut Request, f: &mut impl FnMut(&mut String)) {
        f(&mut r.url);
        for kv in r.params.iter_mut().chain(r.headers.iter_mut()) {
            f(&mut kv.key);
            f(&mut kv.value);
        }
        match &mut r.body {
            Body::Text { text, .. } | Body::Binary(text) => f(text),
            Body::Urlencoded(rows) => rows.iter_mut().for_each(|kv| f(&mut kv.value)),
            Body::FormData(fields) => fields.iter_mut().for_each(|x| f(&mut x.value)),
            Body::None | Body::Unsupported(_) => {}
        }
        match &mut r.auth {
            Some(Auth::Bearer { token }) => f(token),
            Some(Auth::Basic { username, password }) => {
                f(username);
                f(password);
            }
            Some(Auth::ApiKey { key, value, .. }) => {
                f(key);
                f(value);
            }
            Some(Auth::Unsupported(_)) | None => {}
        }
    }
    items(&mut c.items, f);
    for (_, vars) in &mut c.environments {
        vars.iter_mut().for_each(|(_, v)| f(v));
    }
}

/// `{{ _.a.b }}` → `{{a.b}}`; other `{{…}}` stay as written.
fn plain_vars(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        let inner = &rest[i + 2..];
        let var = inner
            .find("}}")
            .and_then(|end| Some((end, inner[..end].trim().strip_prefix("_.")?)));
        match var {
            Some((end, name)) => {
                out.push_str("{{");
                out.push_str(name);
                out.push_str("}}");
                rest = &inner[end + 2..];
            }
            None => {
                out.push_str("{{");
                rest = inner;
            }
        }
    }
    out.push_str(rest);
    out
}
