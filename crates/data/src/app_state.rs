//! Open collections, tabs and environment as JSON in `<config dir>/roundtrip/state.json`.

use std::fs;
use std::path::PathBuf;

use domain::session::{Session, SessionStore};
use serde::{Deserialize, Serialize};

pub struct AppStateFile {
    path: PathBuf,
}

#[derive(Serialize, Deserialize, Default)]
struct StateJson {
    #[serde(default)]
    collections: Vec<PathBuf>,
    #[serde(default)]
    tabs: Vec<PathBuf>,
    #[serde(default)]
    active_tab: Option<usize>,
    #[serde(default)]
    environment: Option<String>,
    // single-collection format: read once, never written
    #[serde(default, skip_serializing)]
    last_collection: Option<PathBuf>,
    #[serde(default, skip_serializing)]
    last_environment: Option<String>,
}

impl AppStateFile {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// XDG on Linux, platform dir on Windows/macOS; temp dir if the platform has none.
    pub fn in_config_dir() -> Self {
        let base = dirs::config_dir().unwrap_or_else(std::env::temp_dir);
        Self::new(base.join("roundtrip").join("state.json"))
    }
}

impl SessionStore for AppStateFile {
    fn load(&self) -> Session {
        let state: StateJson = fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let collections = if state.collections.is_empty() {
            state.last_collection.into_iter().collect()
        } else {
            state.collections
        };
        Session {
            collections,
            tabs: state.tabs,
            active_tab: state.active_tab,
            environment: state.environment.or(state.last_environment),
        }
    }

    // state errors are ignored
    fn save(&self, session: &Session) {
        let state = StateJson {
            collections: session.collections.clone(),
            tabs: session.tabs.clone(),
            active_tab: session.active_tab,
            environment: session.environment.clone(),
            ..StateJson::default()
        };
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(&state) {
            let _ = fs::write(&self.path, json);
        }
    }
}
