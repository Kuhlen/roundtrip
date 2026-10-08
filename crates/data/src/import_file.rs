//! Import file → ImportedCollection: size check, JSON or YAML, format from the parsed shape.

use std::fs;
use std::path::Path;

use domain::AppError;
use domain::import::ImportedCollection;
use serde_json::Value;

use crate::storage;

const MAX_BYTES: u64 = 50 * 1024 * 1024;

pub fn read(path: &Path) -> Result<ImportedCollection, AppError> {
    let size = fs::metadata(path).map_err(|e| storage(path, e))?.len();
    if size > MAX_BYTES {
        return Err(AppError::Import(format!(
            "{} is larger than 50 MB",
            path.display()
        )));
    }
    let content = fs::read_to_string(path).map_err(|e| storage(path, e))?;
    // Windows editors prepend a BOM; serde_json rejects it
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let root: Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(_) => serde_yaml::from_str(content)
            .map_err(|e| AppError::Import(format!("neither JSON nor YAML: {e}")))?,
    };
    from_value(&root)
}

fn from_value(root: &Value) -> Result<ImportedCollection, AppError> {
    let kind = root.get("type").and_then(Value::as_str).unwrap_or_default();
    if root.get("info").is_some() && root.get("item").is_some_and(Value::is_array) {
        return crate::postman::from_value(root);
    }
    if let Some(v) = root.get("openapi") {
        // YAML `openapi: 3.0` is a number
        let version = v.as_str().map_or_else(|| v.to_string(), str::to_owned);
        if version.starts_with("3.") {
            return Ok(crate::openapi::from_value(root, version));
        }
        return Err(AppError::Import(format!(
            "OpenAPI {version} isn't supported; only 3.x"
        )));
    }
    if root.get("swagger").is_some() {
        return Err(AppError::Import(
            "Swagger 2.0 isn't supported; convert it to OpenAPI 3".into(),
        ));
    }
    if root.get("_type").and_then(Value::as_str) == Some("export")
        && root.get("resources").is_some_and(Value::is_array)
    {
        return crate::insomnia::from_v4(root);
    }
    if kind.starts_with("collection.insomnia.rest/5.") {
        return Ok(crate::insomnia::from_v5(root));
    }
    if kind.starts_with("spec.insomnia.rest/5.") {
        return Err(AppError::Import(
            "Insomnia design documents aren't supported; export the spec as OpenAPI".into(),
        ));
    }
    Err(AppError::Import(
        "Unknown format. Supported: Postman v2.0/v2.1, OpenAPI 3.x, Insomnia v4/v5".into(),
    ))
}
