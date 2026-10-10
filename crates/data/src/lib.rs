//! Roundtrip infra: ApiArk collection folders, environments, reqwest sender, history, app state.

use std::path::Path;

use domain::AppError;
use serde_yaml::Value;

pub mod app_state;
mod body_file;
pub mod collection_dir;
mod collection_import;
mod dotenv;
pub mod dynamic_vars;
pub mod environment_dir;
pub mod graphql;
pub mod history_db;
pub mod http;
pub mod import_file;
mod insomnia;
mod openapi;
mod postman;
pub mod postman_export;

/// YAML scalar as text: numbers/bools keep their spelling, null is empty.
pub(crate) fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => serde_yaml::to_string(other)
            .unwrap_or_default()
            .trim_end()
            .to_owned(),
    }
}

pub(crate) fn storage(path: &Path, e: std::io::Error) -> AppError {
    AppError::Storage(format!("{}: {e}", path.display()))
}

/// JSON string field; numbers and bools as written, missing or null = "".
pub(crate) fn json_text(v: &serde_json::Value, key: &str) -> String {
    match v.get(key) {
        Some(serde_json::Value::String(s)) => s.clone(),
        None | Some(serde_json::Value::Null) => String::new(),
        Some(other) => other.to_string(),
    }
}
