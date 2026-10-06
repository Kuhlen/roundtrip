use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Session {
    /// open order; the first one provides the environments
    pub collections: Vec<PathBuf>,
    /// file-backed tabs in tab-bar order; scratch tabs are never stored
    pub tabs: Vec<PathBuf>,
    pub active_tab: Option<usize>,
    pub environment: Option<String>,
}

/// Best effort: a missing or broken store is an empty session.
pub trait SessionStore {
    fn load(&self) -> Session;
    fn save(&self, session: &Session);
}
