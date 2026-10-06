//! ApiArk on-disk format: `.apiark/apiark.yaml`, one YAML per request, `_folder.yaml` order.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use domain::AppError;
use domain::auth::{ApiKeyPlace, Auth};
use domain::collection::{Collection, CollectionStore, Node, Protocol};
use domain::http::{Body, KeyValue, Method, Request};
use serde::Deserialize;
use serde_yaml::{Mapping, Value};

use crate::body_file;
use crate::{scalar, storage};

pub struct CollectionDir;

#[derive(Deserialize)]
struct CollectionConfig {
    name: String,
    #[serde(default)]
    defaults: Option<Value>,
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
    #[serde(default)]
    auth: Option<Value>,
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
            auth: parse_auth(config.defaults.as_ref().and_then(|d| d.get("auth"))),
        })
    }

    fn read_request(&self, file: &Path) -> Result<Request, AppError> {
        let raw: RequestFile =
            serde_yaml::from_str(&read_checked(file)?).map_err(|e| invalid(file, e))?;
        Ok(Request {
            method: parse_method(&raw.method, file)?,
            url: raw.url,
            params: pairs(raw.params),
            headers: pairs(raw.headers),
            body: raw
                .body
                .map_or(Body::None, |b| body_file::parse(&b.kind, &b.content)),
            auth: parse_auth(raw.auth.as_ref()),
        })
    }

    fn save_request(&self, file: &Path, request: &Request) -> Result<(), AppError> {
        let mut doc = read_doc(file)?;
        doc.insert("method".into(), request.method.as_str().into());
        doc.insert("url".into(), request.url.as_str().into());
        set_pairs(&mut doc, "params", &request.params);
        set_pairs(&mut doc, "headers", &request.headers);
        match &request.body {
            Body::Unsupported(_) => {}
            Body::None => {
                doc.shift_remove("body");
            }
            body => {
                let mut map = Mapping::new();
                map.insert("type".into(), body.as_str().into());
                map.insert(
                    "content".into(),
                    body_file::content(body).unwrap_or_default().into(),
                );
                doc.insert("body".into(), Value::Mapping(map));
            }
        }
        set_auth(&mut doc, request.auth.as_ref());
        write_atomic(file, &doc)
    }

    fn save_collection_auth(&self, root: &Path, auth: Option<&Auth>) -> Result<(), AppError> {
        if matches!(auth, Some(Auth::Unsupported(_))) {
            return Ok(());
        }
        let file = root.join(".apiark").join("apiark.yaml");
        let mut doc = read_doc(&file)?;
        if !doc.contains_key("defaults") {
            if auth.is_none() {
                return Ok(());
            }
            doc.insert("defaults".into(), Value::Mapping(Mapping::new()));
        }
        let Some(Value::Mapping(defaults)) = doc.get_mut("defaults") else {
            return Err(AppError::Storage(format!(
                "{}: defaults is not a mapping",
                file.display()
            )));
        };
        set_auth(defaults, auth);
        write_atomic(&file, &doc)
    }

    fn create_request(&self, dir: &Path, name: &str) -> Result<PathBuf, AppError> {
        let file = dir.join(format!("{}.yaml", request_stem(name)?));
        let mut doc = Mapping::new();
        doc.insert("name".into(), name.trim().into());
        doc.insert("method".into(), "GET".into());
        doc.insert("url".into(), "".into());
        let yaml = serde_yaml::to_string(&doc).map_err(|e| invalid(&file, e))?;
        // create_new: no check-then-write race
        let mut out = fs::File::create_new(&file).map_err(|e| create_error(&file, e))?;
        if let Err(e) = out.write_all(yaml.as_bytes()) {
            // half-written file would block a retry with AlreadyExists
            let _ = fs::remove_file(&file);
            return Err(storage(&file, e));
        }
        Ok(file)
    }

    fn create_folder(&self, dir: &Path, name: &str) -> Result<PathBuf, AppError> {
        let path = dir.join(clean_name(name)?);
        fs::create_dir(&path).map_err(|e| create_error(&path, e))?;
        Ok(path)
    }

    fn rename(&self, path: &Path, new_name: &str) -> Result<PathBuf, AppError> {
        let dir = path
            .parent()
            .ok_or_else(|| AppError::InvalidName(new_name.to_owned()))?;
        let is_dir = path.is_dir();
        let (target, old_key, new_key) = if is_dir {
            let clean = clean_name(new_name)?;
            (dir.join(&clean), file_name(path), clean)
        } else {
            let stem = request_stem(new_name)?;
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("yaml");
            let old = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_owned();
            (dir.join(format!("{stem}.{ext}")), old, stem)
        };
        // read everything fallible before the first mutation
        let mut request_doc = if is_dir { None } else { Some(read_doc(path)?) };
        let mut order = None;
        if target != path {
            if target.exists() {
                return Err(AppError::AlreadyExists(file_name(&target)));
            }
            order = prepare_order(dir, &old_key, &new_key)?;
            fs::rename(path, &target).map_err(|e| storage(path, e))?;
        }
        if let Some(doc) = request_doc.as_mut() {
            doc.insert("name".into(), new_name.trim().into());
            write_atomic(&target, doc)?;
        }
        if let Some((file, doc)) = order {
            write_atomic(&file, &doc)?;
        }
        Ok(target)
    }

    fn delete(&self, path: &Path) -> Result<(), AppError> {
        if path.is_dir() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        }
        .map_err(|e| storage(path, e))
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

/// `none` / missing → None; unknown or broken → Unsupported, so save never rewrites it.
fn parse_auth(v: Option<&Value>) -> Option<Auth> {
    let v = v.filter(|v| !v.is_null())?;
    let field = |k: &str| v.get(k).map(scalar).unwrap_or_default();
    let kind = v.get("type").and_then(Value::as_str).unwrap_or("unknown");
    Some(match kind {
        "none" => return None,
        "bearer" => Auth::Bearer {
            token: field("token"),
        },
        "basic" => Auth::Basic {
            username: field("username"),
            password: field("password"),
        },
        "api-key" => Auth::ApiKey {
            key: field("key"),
            value: field("value"),
            place: if field("addTo") == "query" {
                ApiKeyPlace::Query
            } else {
                ApiKeyPlace::Header
            },
        },
        other => Auth::Unsupported(other.to_owned()),
    })
}

/// None removes the key; Unsupported leaves it as the file has it.
fn set_auth(doc: &mut Mapping, auth: Option<&Auth>) {
    let pairs: Vec<(&str, &str)> = match auth {
        None => {
            doc.shift_remove("auth");
            return;
        }
        Some(Auth::Unsupported(_)) => return,
        Some(Auth::Bearer { token }) => vec![("type", "bearer"), ("token", token.as_str())],
        Some(Auth::Basic { username, password }) => vec![
            ("type", "basic"),
            ("username", username.as_str()),
            ("password", password.as_str()),
        ],
        Some(Auth::ApiKey { key, value, place }) => vec![
            ("type", "api-key"),
            ("key", key.as_str()),
            ("value", value.as_str()),
            (
                "addTo",
                match place {
                    ApiKeyPlace::Header => "header",
                    ApiKeyPlace::Query => "query",
                },
            ),
        ],
    };
    let map: Mapping = pairs
        .into_iter()
        .map(|(k, v)| (Value::from(k), Value::from(v)))
        .collect();
    doc.insert("auth".into(), Value::Mapping(map));
}

fn read_doc(file: &Path) -> Result<Mapping, AppError> {
    serde_yaml::from_str(&read_checked(file)?).map_err(|e| invalid(file, e))
}

/// `.tmp` + rename: a crash never leaves half a file.
fn write_atomic(file: &Path, doc: &Mapping) -> Result<(), AppError> {
    let yaml = serde_yaml::to_string(doc).map_err(|e| invalid(file, e))?;
    let tmp = tmp_path(file);
    fs::write(&tmp, yaml).map_err(|e| storage(&tmp, e))?;
    fs::rename(&tmp, file).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        storage(file, e)
    })
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

/// Upstream rename rule; empty or leading dot → InvalidName (the tree scan hides dot names).
fn clean_name(name: &str) -> Result<String, AppError> {
    let replaced: String = name
        .chars()
        .map(|c| {
            if c.is_control() || r#"<>:"/\|?*"#.contains(c) {
                '-'
            } else {
                c
            }
        })
        .collect();
    let clean = replaced.trim();
    if clean.is_empty() || clean.starts_with('.') {
        return Err(AppError::InvalidName(name.to_owned()));
    }
    Ok(clean.to_owned())
}

/// Upstream new-request rule: lowercase, whitespace runs → '-'.
fn request_stem(name: &str) -> Result<String, AppError> {
    let stem = clean_name(name)?
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-");
    // `_folder.yaml` is the folder config, never a request
    if stem == "_folder" {
        return Err(AppError::InvalidName(name.to_owned()));
    }
    Ok(stem)
}

/// Parent `_folder.yaml` with the entry renamed; None when absent or unchanged.
fn prepare_order(dir: &Path, old: &str, new: &str) -> Result<Option<(PathBuf, Mapping)>, AppError> {
    let file = dir.join("_folder.yaml");
    if !file.exists() {
        return Ok(None);
    }
    let mut doc = read_doc(&file)?;
    let Some(Value::Sequence(order)) = doc.get_mut("order") else {
        return Ok(None);
    };
    let mut hit = false;
    for item in order.iter_mut().filter(|i| i.as_str() == Some(old)) {
        *item = new.into();
        hit = true;
    }
    Ok(hit.then_some((file, doc)))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn create_error(path: &Path, e: std::io::Error) -> AppError {
    if e.kind() == ErrorKind::AlreadyExists {
        AppError::AlreadyExists(file_name(path))
    } else {
        storage(path, e)
    }
}
