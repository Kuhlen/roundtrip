use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::AppError;
use crate::auth::Auth;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Method {
    #[default]
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
}

impl Method {
    pub const ALL: [Method; 7] = [
        Method::Get,
        Method::Post,
        Method::Put,
        Method::Patch,
        Method::Delete,
        Method::Head,
        Method::Options,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Patch => "PATCH",
            Method::Delete => "DELETE",
            Method::Head => "HEAD",
            Method::Options => "OPTIONS",
        }
    }

    /// Case-sensitive like ApiArk's serde `UPPERCASE`.
    pub fn parse(s: &str) -> Option<Method> {
        Method::ALL.into_iter().find(|m| m.as_str() == s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyValue {
    pub key: String,
    pub value: String,
    pub enabled: bool,
}

impl KeyValue {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            enabled: true,
        }
    }

    /// ApiArk skips disabled and blank-key rows.
    pub fn is_active(&self) -> bool {
        self.enabled && !self.key.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextKind {
    Json,
    Xml,
    Raw,
}

impl TextKind {
    pub fn content_type(self) -> &'static str {
        match self {
            TextKind::Json => "application/json",
            TextKind::Xml => "application/xml",
            TextKind::Raw => "text/plain",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormField {
    pub key: String,
    /// file path when `is_file`
    pub value: String,
    pub enabled: bool,
    pub is_file: bool,
}

impl FormField {
    pub fn text(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            enabled: true,
            is_file: false,
        }
    }

    pub fn file(key: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            is_file: true,
            ..Self::text(key, path)
        }
    }

    pub fn is_active(&self) -> bool {
        self.enabled && !self.key.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Body {
    #[default]
    None,
    Text {
        kind: TextKind,
        text: String,
    },
    Urlencoded(Vec<KeyValue>),
    FormData(Vec<FormField>),
    /// file path; may hold {{var}}, relative = collection root
    Binary(String),
    /// upstream type or content we can't read: never sent, never rewritten
    Unsupported(String),
}

impl Body {
    /// body-select order in the editor, ApiArk `body.type` spelling
    pub const KINDS: [&'static str; 7] = [
        "none",
        "json",
        "xml",
        "raw",
        "urlencoded",
        "form-data",
        "binary",
    ];

    pub fn as_str(&self) -> &str {
        match self {
            Body::None => "none",
            Body::Text {
                kind: TextKind::Json,
                ..
            } => "json",
            Body::Text {
                kind: TextKind::Xml,
                ..
            } => "xml",
            Body::Text {
                kind: TextKind::Raw,
                ..
            } => "raw",
            Body::Urlencoded(_) => "urlencoded",
            Body::FormData(_) => "form-data",
            Body::Binary(_) => "binary",
            Body::Unsupported(kind) => kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub params: Vec<KeyValue>,
    pub headers: Vec<KeyValue>,
    pub body: Body,
    /// None: inherit the collection's
    pub auth: Option<Auth>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<KeyValue>,
    pub body: String,
    pub elapsed: Duration,
    pub size_bytes: u64,
    /// body cut at the display limit
    pub truncated: bool,
}

/// UI sets it, the worker polls it; never reset, one flag per Send.
#[derive(Debug, Clone, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

pub trait HttpSender: Send + Sync {
    /// Blocking: call from a worker thread. Returns `Cancelled` soon after `cancel` is set.
    fn send(&self, request: &Request, cancel: &CancelFlag) -> Result<Response, AppError>;
}

/// Relative paths point into the collection, so the same file works on every machine.
pub fn resolve_files(request: &mut Request, root: &Path) {
    let resolve = |p: &mut String| {
        // empty stays empty: the sender reports "no file chosen"
        if !p.is_empty() && Path::new(p.as_str()).is_relative() {
            *p = root.join(&*p).display().to_string();
        }
    };
    match &mut request.body {
        Body::Binary(path) => resolve(path),
        Body::FormData(fields) => fields
            .iter_mut()
            .filter(|f| f.is_file)
            .for_each(|f| resolve(&mut f.value)),
        _ => {}
    }
}
