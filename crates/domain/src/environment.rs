use std::collections::{HashMap, HashSet};
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
pub struct Variable {
    pub key: String,
    pub value: String,
    /// value lives in `.apiark/.env`, key in the file's `secrets`
    pub secret: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    pub name: String,
    pub scope: Scope,
    /// file order; secret values already filled in by the store
    pub variables: Vec<Variable>,
}

/// Priority: root `.env` < environment variables (secrets included).
pub fn resolve(root_dotenv: HashMap<String, String>, env: &Environment) -> HashMap<String, String> {
    let mut vars = root_dotenv;
    vars.extend(
        env.variables
            .iter()
            .map(|v| (v.key.clone(), v.value.clone())),
    );
    vars
}

/// Ready to store: name trimmed, blank-key rows dropped.
/// `others` = every stored environment except the edited one.
pub fn validate(mut env: Environment, others: &[Environment]) -> Result<Environment, AppError> {
    env.name = env.name.trim().to_owned();
    if env.name.is_empty() {
        return Err(AppError::InvalidName(String::new()));
    }
    // one name across both scopes: a personal one would shadow a shared one
    if others.iter().any(|o| o.name == env.name) {
        return Err(AppError::AlreadyExists(env.name));
    }
    for v in &mut env.variables {
        v.key = v.key.trim().to_owned();
    }
    env.variables.retain(|v| !v.key.is_empty());
    let dup = {
        let mut seen = HashSet::new();
        env.variables
            .iter()
            .find(|v| !seen.insert(v.key.as_str()))
            .map(|v| v.key.clone())
    };
    if let Some(key) = dup {
        return Err(AppError::DuplicateKey(key));
    }
    Ok(env)
}

pub trait EnvironmentStore {
    /// Shared + personal, sorted by name.
    fn list(&self, collection: &Path) -> Result<Vec<Environment>, AppError>;
    /// Re-reads every file: call once per Send.
    fn resolve(&self, collection: &Path, name: &str) -> Result<HashMap<String, String>, AppError>;
    /// `old` None creates; another name or scope than `old` renames or moves.
    fn save(
        &self,
        collection: &Path,
        old: Option<(&str, Scope)>,
        env: &Environment,
    ) -> Result<(), AppError>;
    fn delete(&self, collection: &Path, name: &str, scope: Scope) -> Result<(), AppError>;
}
