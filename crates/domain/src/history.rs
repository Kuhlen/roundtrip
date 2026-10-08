//! Sent requests, newest first. The store lives in `data`.

use std::path::PathBuf;
use std::time::SystemTime;

use crate::AppError;
use crate::auth::Auth;
use crate::http::{Method, Request};

pub const REDACTED: &str = "[REDACTED]";

const SECRET_HEADERS: [&str; 3] = ["authorization", "proxy-authorization", "cookie"];
// "auth" also catches authorization and x-auth-*
const SECRET_PARTS: [&str; 7] = [
    "token", "secret", "password", "api-key", "apikey", "api_key", "auth",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub id: i64,
    pub method: Method,
    /// as sent: variables filled in
    pub url: String,
    /// None: the send ended in an error
    pub status: Option<u16>,
    pub status_text: Option<String>,
    pub time_ms: Option<u64>,
    pub size_bytes: Option<u64>,
    pub at: SystemTime,
    pub collection: Option<PathBuf>,
    /// None: Untitled tab
    pub name: Option<String>,
}

pub trait HistoryStore: Send + Sync {
    /// `entry.id` is ignored; the store assigns one.
    fn record(&self, entry: &HistoryEntry, request: &Request) -> Result<(), AppError>;
    /// Newest first.
    fn recent(&self, limit: usize) -> Result<Vec<HistoryEntry>, AppError>;
    /// Substring of url, method or name, case-insensitive; newest first.
    fn search(&self, query: &str, limit: usize) -> Result<Vec<HistoryEntry>, AppError>;
    fn request(&self, id: i64) -> Result<Request, AppError>;
    fn clear(&self) -> Result<(), AppError>;
}

/// Secrets out before disk. `{{var}}` holds no secret and keeps a reopened request usable.
pub fn redact(request: &Request) -> Request {
    let mask = |v: &str| {
        if v.is_empty() || v.contains("{{") {
            v.to_owned()
        } else {
            REDACTED.to_owned()
        }
    };
    let mut out = request.clone();
    for h in &mut out.headers {
        let name = h.key.to_ascii_lowercase();
        if SECRET_HEADERS.contains(&name.as_str()) || SECRET_PARTS.iter().any(|p| name.contains(p))
        {
            h.value = mask(&h.value);
        }
    }
    out.auth = out.auth.map(|auth| match auth {
        Auth::Bearer { token } => Auth::Bearer {
            token: mask(&token),
        },
        Auth::Basic { username, password } => Auth::Basic {
            username,
            password: mask(&password),
        },
        Auth::ApiKey { key, value, place } => Auth::ApiKey {
            key,
            value: mask(&value),
            place,
        },
        other => other,
    });
    out
}
