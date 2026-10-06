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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum BodyKind {
    #[default]
    None,
    Json,
    Xml,
    Raw,
    /// form-data, urlencoded, binary, graphql, ...: never sent, never rewritten
    Unsupported(String),
}

impl BodyKind {
    /// body-select order in the editor
    pub const EDITABLE: [BodyKind; 4] =
        [BodyKind::None, BodyKind::Json, BodyKind::Xml, BodyKind::Raw];

    pub fn parse(s: &str) -> BodyKind {
        match s {
            "none" => BodyKind::None,
            "json" => BodyKind::Json,
            "xml" => BodyKind::Xml,
            "raw" => BodyKind::Raw,
            other => BodyKind::Unsupported(other.to_owned()),
        }
    }

    /// ApiArk `body.type` spelling
    pub fn as_str(&self) -> &str {
        match self {
            BodyKind::None => "none",
            BodyKind::Json => "json",
            BodyKind::Xml => "xml",
            BodyKind::Raw => "raw",
            BodyKind::Unsupported(kind) => kind,
        }
    }

    pub fn content_type(&self) -> Option<&'static str> {
        match self {
            BodyKind::None | BodyKind::Unsupported(_) => None,
            BodyKind::Json => Some("application/json"),
            BodyKind::Xml => Some("application/xml"),
            BodyKind::Raw => Some("text/plain"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub params: Vec<KeyValue>,
    pub headers: Vec<KeyValue>,
    pub body_kind: BodyKind,
    pub body: String,
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

pub trait HttpSender: Send + Sync {
    /// Blocking: call from a worker thread.
    fn send(&self, request: &Request) -> Result<Response, AppError>;
}
