//! Pure text and naming decisions of the environment dialog.

use domain::AppError;
use domain::environment::Environment;

/// `base`, else `base 2`, `base 3`, … as New request numbers.
pub fn unique_name(base: &str, taken: &[&str]) -> String {
    if !taken.contains(&base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|name| !taken.contains(&name.as_str()))
        .unwrap_or_default()
}

/// One line per secret key another environment also keeps secret: `.apiark/.env` holds one value.
pub fn shared_secret_notes(env: &Environment, others: &[Environment]) -> Vec<String> {
    env.variables
        .iter()
        .filter(|v| v.secret && !v.key.is_empty())
        .filter_map(|v| {
            let names: Vec<&str> = others
                .iter()
                .filter(|o| o.variables.iter().any(|w| w.secret && w.key == v.key))
                .map(|o| o.name.as_str())
                .collect();
            (!names.is_empty()).then(|| {
                format!(
                    "\"{}\" is also a secret in {}: they share one value",
                    v.key,
                    names.join(", ")
                )
            })
        })
        .collect()
}

/// Inline error under the variable table.
pub fn error_text(e: &AppError) -> String {
    match e {
        AppError::InvalidName(name) if name.trim().is_empty() => "Name can't be empty".into(),
        AppError::InvalidName(name) => format!("\"{name}\" can't be used as a file name"),
        AppError::DuplicateKey(key) => format!("Variable \"{key}\" appears twice"),
        AppError::AlreadyExists(name) => {
            format!("An environment named \"{name}\" already exists")
        }
        other => other.to_string(),
    }
}
