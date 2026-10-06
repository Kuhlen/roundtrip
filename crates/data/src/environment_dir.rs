//! `.apiark/environments{,.local}/*.y{a,}ml` + `.env` files, as upstream storage/environment.rs.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use domain::AppError;
use domain::environment::{self, Environment, EnvironmentStore, Scope};
use serde::Deserialize;
use serde_yaml::Mapping;

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
        let apiark = collection.join(".apiark");
        let mut envs = Vec::new();
        load_dir(&apiark.join("environments"), Scope::Shared, &mut envs)?;
        load_dir(
            &apiark.join("environments.local"),
            Scope::Personal,
            &mut envs,
        )?;
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
            &parse_dotenv(&collection.join(".apiark").join(".env")),
        ))
    }
}

fn load_dir(dir: &Path, scope: Scope, envs: &mut Vec<Environment>) -> Result<(), AppError> {
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
            variables: file
                .variables
                .unwrap_or_default()
                .iter()
                .map(|(k, v)| (scalar(k), scalar(v)))
                .collect(),
            secrets: file.secrets,
        });
    }
    Ok(())
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
