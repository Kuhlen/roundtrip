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

/// Keys trimmed, blank-key rows dropped; a key may appear once.
pub fn clean_variables(mut vars: Vec<Variable>) -> Result<Vec<Variable>, AppError> {
    for v in &mut vars {
        v.key = v.key.trim().to_owned();
    }
    vars.retain(|v| !v.key.is_empty());
    let mut seen = HashSet::new();
    if let Some(v) = vars.iter().find(|v| !seen.insert(v.key.as_str())) {
        return Err(AppError::DuplicateKey(v.key.clone()));
    }
    Ok(vars)
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
    env.variables = clean_variables(std::mem::take(&mut env.variables))?;
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
    /// Collection root `.env`, file order, never secret; missing file = empty.
    fn root_dotenv(&self, collection: &Path) -> Result<Vec<Variable>, AppError>;
    /// Comments and foreign lines kept; rows not in `vars` removed.
    fn save_root_dotenv(&self, collection: &Path, vars: &[Variable]) -> Result<(), AppError>;
}
