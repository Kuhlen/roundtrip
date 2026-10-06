use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Session {
    pub last_collection: Option<PathBuf>,
    pub last_environment: Option<String>,
}

/// Best effort: a missing or broken store is an empty session.
pub trait SessionStore {
    fn load(&self) -> Session;
    fn save(&self, session: &Session);
}
