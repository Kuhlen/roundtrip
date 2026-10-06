//! ApiArk on-disk format: `.apiark/apiark.yaml`, one YAML per request, `_folder.yaml` order.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use domain::AppError;
use domain::collection::{Collection, CollectionStore, Node, Protocol};
use domain::http::{BodyKind, KeyValue, Method, Request};
use serde::Deserialize;
use serde_yaml::{Mapping, Value};

use crate::{scalar, storage};

pub struct CollectionDir;

#[derive(Deserialize)]
struct CollectionConfig {
    name: String,
}

#[derive(Deserialize, Default)]
struct FolderConfig {
    #[serde(default)]
    order: Vec<String>,
}

#[derive(Deserialize)]
struct BodyFile {
    #[serde(rename = "type")]
    kind: String,
    // Value: unsupported kinds may hold lists
    #[serde(default)]
    content: Value,
}

#[derive(Deserialize)]
struct RequestMeta {
    name: String,
    method: String,
    protocol: Option<String>,
    body: Option<BodyFile>,
}

#[derive(Deserialize)]
struct RequestFile {
    method: String,
    url: String,
    // Mapping keeps file order (upstream HashMap shuffles); Option: `headers:` may be null
    #[serde(default)]
    headers: Option<Mapping>,
    #[serde(default)]
    params: Option<Mapping>,
    body: Option<BodyFile>,
}

impl CollectionStore for CollectionDir {
    fn load(&self, dir: &Path) -> Result<Collection, AppError> {
        let config_path = dir.join(".apiark").join("apiark.yaml");
        if !config_path.exists() {
            return Err(AppError::NotACollection(dir.display().to_string()));
        }
        let config: CollectionConfig = parse_yaml(&config_path)?;
        Ok(Collection {
            name: config.name,
            path: dir.to_path_buf(),
            children: scan(dir)?,
        })
    }

    fn read_request(&self, file: &Path) -> Result<Request, AppError> {
        let raw: RequestFile =
            serde_yaml::from_str(&read_checked(file)?).map_err(|e| invalid(file, e))?;
        let (body_kind, body) = match raw.body {
            Some(b) => match BodyKind::parse(&b.kind) {
                // never shown, never written back
                kind @ BodyKind::Unsupported(_) => (kind, String::new()),
                kind => (kind, scalar(&b.content)),
            },
            None => (BodyKind::None, String::new()),
        };
        Ok(Request {
            method: parse_method(&raw.method, file)?,
            url: raw.url,
            params: pairs(raw.params),
            headers: pairs(raw.headers),
            body_kind,
            body,
        })
    }

    fn save_request(&self, file: &Path, request: &Request) -> Result<(), AppError> {
        let mut doc: Mapping =
            serde_yaml::from_str(&read_checked(file)?).map_err(|e| invalid(file, e))?;
        doc.insert("method".into(), request.method.as_str().into());
        doc.insert("url".into(), request.url.as_str().into());
        set_pairs(&mut doc, "params", &request.params);
        set_pairs(&mut doc, "headers", &request.headers);
        match &request.body_kind {
            BodyKind::Unsupported(_) => {}
            BodyKind::None => {
                doc.shift_remove("body");
            }
            kind => {
                let mut body = Mapping::new();
                body.insert("type".into(), kind.as_str().into());
                body.insert("content".into(), request.body.as_str().into());
                doc.insert("body".into(), Value::Mapping(body));
            }
        }
        let yaml = serde_yaml::to_string(&doc).map_err(|e| invalid(file, e))?;
        let tmp = tmp_path(file);
        fs::write(&tmp, yaml).map_err(|e| storage(&tmp, e))?;
        fs::rename(&tmp, file).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            storage(file, e)
        })
    }
}

/// Folders first, then requests; each by `_folder.yaml` order, then name.
fn scan(dir: &Path) -> Result<Vec<Node>, AppError> {
    let order: HashMap<String, usize> = parse_yaml::<FolderConfig>(&dir.join("_folder.yaml"))
        .unwrap_or_default()
        .order
        .into_iter()
        .enumerate()
        .map(|(i, name)| (name, i))
        .collect();
    let rank = |key: &str| order.get(key).copied().unwrap_or(usize::MAX);

    let mut folders = Vec::new();
    let mut requests = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| storage(dir, e))? {
        let path = entry.map_err(|e| storage(dir, e))?.path();
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };
        if file_name.starts_with('.') || file_name == "_folder.yaml" {
            continue;
        }
        if path.is_dir() {
            let children = scan(&path)?;
            folders.push((
                rank(&file_name),
                Node::Folder {
                    name: file_name,
                    path,
                    children,
                },
            ));
        } else if file_name.ends_with(".yaml") || file_name.ends_with(".yml") {
            // upstream logs and skips broken files
            let Ok(meta) = parse_yaml::<RequestMeta>(&path) else {
                continue;
            };
            let Some(method) = Method::parse(&meta.method) else {
                continue;
            };
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_owned();
            let protocol = detect_protocol(&meta, method);
            requests.push((
                rank(&stem),
                Node::Request {
                    name: meta.name,
                    method,
                    protocol,
                    path,
                },
            ));
        }
    }
    let by_rank = |a: &(usize, Node), b: &(usize, Node)| -> Ordering {
        a.0.cmp(&b.0).then_with(|| a.1.name().cmp(b.1.name()))
    };
    folders.sort_by(by_rank);
    requests.sort_by(by_rank);
    Ok(folders
        .into_iter()
        .chain(requests)
        .map(|(_, n)| n)
        .collect())
}

/// Explicit `protocol`, else legacy GraphQL guess: POST + JSON body with `query`.
fn detect_protocol(meta: &RequestMeta, method: Method) -> Protocol {
    if let Some(p) = meta.protocol.as_deref().filter(|p| *p != "http") {
        return Protocol::parse(p);
    }
    let is_graphql = method == Method::Post
        && meta.body.as_ref().is_some_and(|b| {
            b.kind == "json"
                && b.content
                    .as_str()
                    .and_then(|c| serde_json::from_str::<serde_json::Value>(c).ok())
                    .is_some_and(|v| v.get("query").is_some())
        });
    if is_graphql {
        Protocol::Graphql
    } else {
        Protocol::Http
    }
}

fn pairs(map: Option<Mapping>) -> Vec<KeyValue> {
    map.unwrap_or_default()
        .iter()
        .map(|(k, v)| KeyValue::new(scalar(k), scalar(v)))
        .collect()
}

/// ApiArk maps have no `enabled` flag: disabled and blank-key rows are dropped.
fn set_pairs(doc: &mut Mapping, key: &str, rows: &[KeyValue]) {
    let map: Mapping = rows
        .iter()
        .filter(|r| r.is_active())
        .map(|r| (Value::from(r.key.as_str()), Value::from(r.value.as_str())))
        .collect();
    if map.is_empty() {
        // shift_remove keeps the order of the other keys; remove() swaps
        doc.shift_remove(key);
    } else {
        doc.insert(key.into(), Value::Mapping(map));
    }
}

fn tmp_path(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(".tmp");
    name.into()
}

fn parse_method(s: &str, file: &Path) -> Result<Method, AppError> {
    Method::parse(s)
        .ok_or_else(|| AppError::Storage(format!("{}: unknown method {s}", file.display())))
}

fn parse_yaml<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, AppError> {
    let text = fs::read_to_string(path).map_err(|e| storage(path, e))?;
    serde_yaml::from_str(&text).map_err(|e| invalid(path, e))
}

/// parity: upstream has_merge_conflicts
fn read_checked(file: &Path) -> Result<String, AppError> {
    let text = fs::read_to_string(file).map_err(|e| storage(file, e))?;
    if text.contains("<<<<<<<") && text.contains(">>>>>>>") {
        return Err(AppError::MergeConflict(file.display().to_string()));
    }
    Ok(text)
}

fn invalid(path: &Path, e: serde_yaml::Error) -> AppError {
    AppError::Storage(format!("invalid YAML {}: {e}", path.display()))
}
