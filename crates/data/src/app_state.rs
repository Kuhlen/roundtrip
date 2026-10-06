//! Last collection + environment as JSON in `<config dir>/roundtrip/state.json`.

use std::fs;
use std::path::PathBuf;

use domain::session::{Session, SessionStore};
use serde::{Deserialize, Serialize};

pub struct AppStateFile {
    path: PathBuf,
}

#[derive(Serialize, Deserialize, Default)]
struct StateJson {
    last_collection: Option<PathBuf>,
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
        Session {
            last_collection: state.last_collection,
            last_environment: state.last_environment,
        }
    }

    // state errors are ignored
    fn save(&self, session: &Session) {
        let state = StateJson {
            last_collection: session.last_collection.clone(),
            last_environment: session.last_environment.clone(),
        };
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(&state) {
            let _ = fs::write(&self.path, json);
        }
    }
}
