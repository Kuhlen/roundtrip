use std::collections::HashMap;
use std::path::Path;

use crate::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// `.apiark/environments/`, committed
    Shared,
    /// `.apiark/environments.local/`, gitignored
    Personal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    pub name: String,
    pub scope: Scope,
    pub variables: HashMap<String, String>,
    /// keys taken from `.apiark/.env`
    pub secrets: Vec<String>,
}

/// Priority: root `.env` < environment `variables` < `.apiark/.env` (listed `secrets` only).
pub fn resolve(
    root_dotenv: HashMap<String, String>,
    env: &Environment,
    secrets_dotenv: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut vars = root_dotenv;
    vars.extend(env.variables.clone());
    for key in &env.secrets {
        if let Some(value) = secrets_dotenv.get(key) {
            vars.insert(key.clone(), value.clone());
        }
    }
    vars
}

pub trait EnvironmentStore {
    /// Shared + personal, sorted by name.
    fn list(&self, collection: &Path) -> Result<Vec<Environment>, AppError>;
    /// Re-reads every file: call once per Send.
    fn resolve(&self, collection: &Path, name: &str) -> Result<HashMap<String, String>, AppError>;
}
