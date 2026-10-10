//! `.apiark/environments{,.local}/*.y{a,}ml` + `.env` files, as upstream storage/environment.rs.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use domain::AppError;
use domain::environment::{self, Environment, EnvironmentStore, Scope, Variable};
use serde::Deserialize;
use serde_yaml::{Mapping, Value};

use crate::collection_dir::{
    create_error, read_doc, request_stem, write_atomic, write_text_atomic,
};
use crate::{scalar, storage};

pub struct EnvironmentDir;

#[derive(Deserialize)]
struct EnvironmentFile {
    name: String,
    // Mapping, not HashMap<String, String>: `port: 3000` must load (upstream rejects it)
    #[serde(default)]
    variables: Option<Mapping>,
    #[serde(default)]
    secrets: Vec<String>,
}

impl EnvironmentStore for EnvironmentDir {
    fn list(&self, collection: &Path) -> Result<Vec<Environment>, AppError> {
        let secrets = parse_dotenv(&secrets_file(collection));
        let mut envs = Vec::new();
        for scope in [Scope::Shared, Scope::Personal] {
            load_dir(&scope_dir(collection, scope), scope, &secrets, &mut envs)?;
        }
        envs.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(envs)
    }

    fn resolve(&self, collection: &Path, name: &str) -> Result<HashMap<String, String>, AppError> {
        let envs = self.list(collection)?;
        let env = envs
            .iter()
            .find(|e| e.name == name)
            .ok_or_else(|| AppError::Storage(format!("environment {name} not found")))?;
        Ok(environment::resolve(
            parse_dotenv(&collection.join(".env")),
            env,
        ))
    }

    fn save(
        &self,
        collection: &Path,
        old: Option<(&str, Scope)>,
        env: &Environment,
    ) -> Result<(), AppError> {
        let others: Vec<Environment> = self
            .list(collection)?
            .into_iter()
            .filter(|e| old != Some((e.name.as_str(), e.scope)))
            .collect();
        let env = environment::validate(env.clone(), &others)?;
        // one .env line per secret: these would corrupt the shared file
        if let Some(v) = env.variables.iter().filter(|v| v.secret).find(|v| {
            v.key.contains('=') || v.key.starts_with('#') || v.value.contains(['\n', '\r'])
        }) {
            return Err(AppError::Storage(format!(
                "secret {} can't be stored in .apiark/.env",
                v.key
            )));
        }
        let found = match old {
            Some((name, scope)) => find_file(collection, name, scope)?,
            None => None,
        };
        let (old_path, mut doc) = found.map_or((None, Mapping::new()), |(p, d)| (Some(p), d));
        let was_secret = secret_keys(&doc);
        fill(&mut doc, &env);
        prepare_dir(collection, env.scope)?;
        let same_file = old == Some((env.name.as_str(), env.scope));
        match old_path {
            Some(path) if same_file => write_atomic(&path, &doc)?,
            old_path => {
                // stem only when a file is created: imported names may be no valid stem
                let new_path = scope_dir(collection, env.scope)
                    .join(format!("{}.yaml", request_stem(&env.name)?));
                match old_path {
                    Some(path) if path == new_path => write_atomic(&path, &doc)?,
                    old_path => {
                        create_file(&new_path, &doc, &env.name)?;
                        // new file first: a failure here leaves a duplicate, never a loss
                        if let Some(path) = old_path {
                            fs::remove_file(&path).map_err(|e| storage(&path, e))?;
                        }
                    }
                }
            }
        }
        let wanted: Vec<&Variable> = env.variables.iter().filter(|v| v.secret).collect();
        sync_dotenv(collection, &wanted, &was_secret, &others)
    }

    fn delete(&self, collection: &Path, name: &str, scope: Scope) -> Result<(), AppError> {
        let (path, doc) = find_file(collection, name, scope)?
            .ok_or_else(|| AppError::Storage(format!("environment {name} not found")))?;
        let others: Vec<Environment> = self
            .list(collection)?
            .into_iter()
            .filter(|e| !(e.name == name && e.scope == scope))
            .collect();
        fs::remove_file(&path).map_err(|e| storage(&path, e))?;
        sync_dotenv(collection, &[], &secret_keys(&doc), &others)
    }
}

fn scope_dir(collection: &Path, scope: Scope) -> PathBuf {
    let dir = match scope {
        Scope::Shared => "environments",
        Scope::Personal => "environments.local",
    };
    collection.join(".apiark").join(dir)
}

fn secrets_file(collection: &Path) -> PathBuf {
    collection.join(".apiark").join(".env")
}

/// File whose `name` matches, with its whole document (unknown keys kept on save).
fn find_file(
    collection: &Path,
    name: &str,
    scope: Scope,
) -> Result<Option<(PathBuf, Mapping)>, AppError> {
    let dir = scope_dir(collection, scope);
    if !dir.exists() {
        return Ok(None);
    }
    for entry in fs::read_dir(&dir).map_err(|e| storage(&dir, e))? {
        let path = entry.map_err(|e| storage(&dir, e))?.path();
        if !path.extension().is_some_and(|e| e == "yaml" || e == "yml") {
            continue;
        }
        let doc = read_doc(&path)?;
        if doc.get("name").map(scalar).as_deref() == Some(name) {
            return Ok(Some((path, doc)));
        }
    }
    Ok(None)
}

fn secret_keys(doc: &Mapping) -> Vec<String> {
    doc.get("secrets")
        .and_then(Value::as_sequence)
        .map(|s| s.iter().map(scalar).collect())
        .unwrap_or_default()
}

/// Only the keys Roundtrip owns; everything else in the file stays.
fn fill(doc: &mut Mapping, env: &Environment) {
    doc.insert("name".into(), env.name.as_str().into());
    let plain: Mapping = env
        .variables
        .iter()
        .filter(|v| !v.secret)
        .map(|v| (Value::from(v.key.as_str()), Value::from(v.value.as_str())))
        .collect();
    doc.insert("variables".into(), Value::Mapping(plain));
    let secrets: Vec<Value> = env
        .variables
        .iter()
        .filter(|v| v.secret)
        .map(|v| Value::from(v.key.as_str()))
        .collect();
    if secrets.is_empty() {
        doc.remove("secrets");
    } else {
        doc.insert("secrets".into(), Value::Sequence(secrets));
    }
}

/// Personal folder gets upstream's `.gitignore` so it never gets committed.
fn prepare_dir(collection: &Path, scope: Scope) -> Result<(), AppError> {
    let dir = scope_dir(collection, scope);
    fs::create_dir_all(&dir).map_err(|e| storage(&dir, e))?;
    let ignore = dir.join(".gitignore");
    if scope == Scope::Personal && !ignore.exists() {
        write_text_atomic(&ignore, "*\n!.gitignore\n")?;
    }
    Ok(())
}

// create_new: no check-then-write race, never overwrites
fn create_file(path: &Path, doc: &Mapping, name: &str) -> Result<(), AppError> {
    let yaml = serde_yaml::to_string(doc)
        .map_err(|e| AppError::Storage(format!("{}: {e}", path.display())))?;
    let mut out = fs::File::create_new(path).map_err(|e| match create_error(path, e) {
        AppError::AlreadyExists(_) => AppError::AlreadyExists(name.to_owned()),
        other => other,
    })?;
    if let Err(e) = out.write_all(yaml.as_bytes()) {
        // half-written file would block a retry with AlreadyExists
        let _ = fs::remove_file(path);
        return Err(storage(path, e));
    }
    Ok(())
}

/// `.apiark/.env` is shared by every environment: a key's line goes only when no other
/// environment still lists it as secret.
fn sync_dotenv(
    collection: &Path,
    wanted: &[&Variable],
    was_secret: &[String],
    others: &[Environment],
) -> Result<(), AppError> {
    let still_used: HashSet<&str> = others
        .iter()
        .flat_map(|e| &e.variables)
        .filter(|v| v.secret)
        .map(|v| v.key.as_str())
        .collect();
    let file = secrets_file(collection);
    let text = match fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(storage(&file, e)),
    };
    // keep the file's own line endings
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut seen = HashSet::new();
    let mut out = String::new();
    for line in text.lines() {
        if let Some(key) = dotenv_key(line) {
            if let Some(v) = wanted.iter().find(|v| v.key == key) {
                // later duplicates of a wanted key are dropped
                if seen.insert(key) {
                    out.push_str(&format!("{key}={}{nl}", quote(&v.value)));
                }
                continue;
            }
            if was_secret.iter().any(|k| k == key) && !still_used.contains(key) {
                continue;
            }
        }
        out.push_str(line);
        out.push_str(nl);
    }
    for v in wanted.iter().filter(|v| !seen.contains(v.key.as_str())) {
        out.push_str(&format!("{}={}{nl}", v.key, quote(&v.value)));
    }
    if out.trim_end() == text.trim_end() {
        return Ok(());
    }
    ensure_ignored(collection)?;
    write_text_atomic(&file, &out)
}

/// A secret file must never be committed.
fn ensure_ignored(collection: &Path) -> Result<(), AppError> {
    let file = collection.join(".apiark").join(".gitignore");
    let text = match fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(storage(&file, e)),
    };
    if text.lines().any(|l| l.trim() == ".env") {
        return Ok(());
    }
    let sep = if text.is_empty() || text.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    write_text_atomic(&file, &format!("{text}{sep}.env\n"))
}

fn dotenv_key(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    line.split_once('=').map(|(k, _)| k.trim())
}

// parse_dotenv trims and strips one outer quote pair, so wrapping always reads back
fn quote(value: &str) -> String {
    if value != value.trim() || value.starts_with(['"', '\'']) {
        format!("\"{value}\"")
    } else {
        value.to_owned()
    }
}

fn load_dir(
    dir: &Path,
    scope: Scope,
    secrets: &HashMap<String, String>,
    envs: &mut Vec<Environment>,
) -> Result<(), AppError> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir).map_err(|e| storage(dir, e))? {
        let path = entry.map_err(|e| storage(dir, e))?.path();
        if !path.extension().is_some_and(|e| e == "yaml" || e == "yml") {
            continue;
        }
        let text = fs::read_to_string(&path).map_err(|e| storage(&path, e))?;
        let file: EnvironmentFile = serde_yaml::from_str(&text).map_err(|e| {
            AppError::Storage(format!("invalid environment YAML {}: {e}", path.display()))
        })?;
        envs.push(Environment {
            name: file.name,
            scope,
            variables: variables(&file.variables.unwrap_or_default(), &file.secrets, secrets),
        });
    }
    Ok(())
}

/// Secret value from `.apiark/.env`, else the YAML one (upstream merge);
/// a listed secret without a YAML entry is a row too.
fn variables(
    yaml: &Mapping,
    listed: &[String],
    secrets: &HashMap<String, String>,
) -> Vec<Variable> {
    let mut out: Vec<Variable> = yaml
        .iter()
        .map(|(k, v)| {
            let key = scalar(k);
            let secret = listed.contains(&key);
            let value = secrets
                .get(&key)
                .filter(|_| secret)
                .cloned()
                .unwrap_or_else(|| scalar(v));
            Variable { key, value, secret }
        })
        .collect();
    for key in listed {
        if !out.iter().any(|v| &v.key == key) {
            out.push(Variable {
                key: key.clone(),
                value: secrets.get(key).cloned().unwrap_or_default(),
                secret: true,
            });
        }
    }
    out
}

/// Missing file = no variables (upstream).
fn parse_dotenv(path: &Path) -> HashMap<String, String> {
    let Ok(text) = fs::read_to_string(path) else {
        return HashMap::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.trim().to_owned(), unquote(v.trim()).to_owned()))
        .collect()
}

// len >= 2: upstream panics on a lone quote
fn unquote(v: &str) -> &str {
    for q in ['"', '\''] {
        if v.len() >= 2 && v.starts_with(q) && v.ends_with(q) {
            return &v[1..v.len() - 1];
        }
    }
    v
}
