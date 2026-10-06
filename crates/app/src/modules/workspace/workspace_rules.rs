//! Pure decisions for the workspace page; user-facing text lives here, not in domain.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use domain::AppError;
use domain::auth::{ApiKeyPlace, Auth};
use domain::collection::{Node, Protocol};
use domain::http::{KeyValue, Method, Request};
use domain::interpolation::interpolate;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKind {
    Folder { expanded: bool },
    Request { method: Method, protocol: Protocol },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatRow {
    pub depth: usize,
    pub name: String,
    pub path: PathBuf,
    pub kind: RowKind,
}

/// Tree → visible rows; children of collapsed folders are skipped.
pub fn flatten(nodes: &[Node], collapsed: &HashSet<PathBuf>) -> Vec<FlatRow> {
    let mut rows = Vec::new();
    walk(nodes, 0, collapsed, &mut rows);
    rows
}

fn walk(nodes: &[Node], depth: usize, collapsed: &HashSet<PathBuf>, rows: &mut Vec<FlatRow>) {
    for node in nodes {
        match node {
            Node::Folder {
                name,
                path,
                children,
            } => {
                let expanded = !collapsed.contains(path);
                rows.push(FlatRow {
                    depth,
                    name: name.clone(),
                    path: path.clone(),
                    kind: RowKind::Folder { expanded },
                });
                if expanded {
                    walk(children, depth + 1, collapsed, rows);
                }
            }
            Node::Request {
                name,
                method,
                protocol,
                path,
            } => rows.push(FlatRow {
                depth,
                name: name.clone(),
                path: path.clone(),
                kind: RowKind::Request {
                    method: *method,
                    protocol: *protocol,
                },
            }),
        }
    }
}

pub fn row_label(kind: &RowKind) -> &'static str {
    match kind {
        RowKind::Folder { .. } => "",
        RowKind::Request { method, protocol } => match protocol {
            Protocol::Http => method.as_str(),
            Protocol::Graphql => "GQL",
            Protocol::WebSocket => "WS",
            Protocol::Sse => "SSE",
            Protocol::Grpc => "gRPC",
        },
    }
}

pub fn protocol_name(p: Protocol) -> &'static str {
    match p {
        Protocol::Http => "HTTP",
        Protocol::Graphql => "GraphQL",
        Protocol::WebSocket => "WebSocket",
        Protocol::Sse => "SSE",
        Protocol::Grpc => "gRPC",
    }
}

// display only, not percent-encoded like the real request; encode if users misread it
pub fn resolved_url(request: &Request, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut url = interpolate(&request.url, &lookup);
    let mut query: Vec<String> = request
        .params
        .iter()
        .filter(|p| p.is_active())
        .map(|p| {
            format!(
                "{}={}",
                interpolate(&p.key, &lookup),
                interpolate(&p.value, &lookup)
            )
        })
        .collect();
    if let Some(Auth::ApiKey {
        key,
        value,
        place: ApiKeyPlace::Query,
    }) = &request.auth
        && !key.is_empty()
    {
        query.push(format!(
            "{}={}",
            interpolate(key, &lookup),
            interpolate(value, &lookup)
        ));
    }
    if !query.is_empty() {
        url.push(if url.contains('?') { '&' } else { '?' });
        url.push_str(&query.join("&"));
    }
    url
}

pub fn auth_label(auth: &Auth) -> String {
    match auth {
        Auth::Bearer { .. } => "Bearer".into(),
        Auth::Basic { .. } => "Basic".into(),
        Auth::ApiKey { .. } => "API key".into(),
        Auth::Unsupported(kind) => kind.clone(),
    }
}

/// Note under "Inherit" in the Auth tab.
pub fn inherit_note(collection: Option<&Auth>) -> String {
    match collection {
        None => "No auth. The collection has none either.".into(),
        Some(a) if !a.is_sendable() => {
            format!(
                "Uses the collection auth: {}, not supported yet.",
                auth_label(a)
            )
        }
        Some(a) => format!("Uses the collection auth: {}.", auth_label(a)),
    }
}

pub fn interpolate_request(request: &Request, lookup: impl Fn(&str) -> Option<String>) -> Request {
    let rows = |rows: &[KeyValue]| -> Vec<KeyValue> {
        rows.iter()
            .map(|r| KeyValue {
                key: interpolate(&r.key, &lookup),
                value: interpolate(&r.value, &lookup),
                enabled: r.enabled,
            })
            .collect()
    };
    Request {
        method: request.method,
        url: interpolate(&request.url, &lookup),
        params: rows(&request.params),
        headers: rows(&request.headers),
        body_kind: request.body_kind.clone(),
        body: interpolate(&request.body, &lookup),
        auth: request.auth.as_ref().map(|a| match a {
            Auth::Bearer { token } => Auth::Bearer {
                token: interpolate(token, &lookup),
            },
            Auth::Basic { username, password } => Auth::Basic {
                username: interpolate(username, &lookup),
                password: interpolate(password, &lookup),
            },
            Auth::ApiKey { key, value, place } => Auth::ApiKey {
                key: interpolate(key, &lookup),
                value: interpolate(value, &lookup),
                place: *place,
            },
            Auth::Unsupported(kind) => Auth::Unsupported(kind.clone()),
        }),
    }
}

/// (title, hint) for the banner and the failed response; hints from upstream models/error.rs.
pub fn error_text(e: &AppError, root: Option<&Path>) -> (String, String) {
    let (title, hint): (String, &str) = match e {
        AppError::InvalidUrl(_) => (
            "Invalid URL".into(),
            "Check the URL format. It should start with http:// or https://",
        ),
        AppError::DnsFailure(_) => (
            "Could not resolve host".into(),
            "Could not resolve the hostname. Check the URL or your network connection.",
        ),
        AppError::ConnectionRefused(_) => (
            "Connection refused".into(),
            "Is the server running? Check the host and port.",
        ),
        AppError::Timeout(ms) => {
            return (
                "Timed out".into(),
                format!("Request timed out after {ms}ms. The server may be slow or unreachable."),
            );
        }
        // upstream suggests turning off verification; no such switch yet
        AppError::Tls(_) => (
            "TLS handshake failed".into(),
            "SSL/TLS handshake failed. Check the server certificate.",
        ),
        AppError::Request(_) => ("Request failed".into(), ""),
        AppError::ResponseTooLarge(_) => (
            "Response too large".into(),
            "The response exceeds the 10MB display limit.",
        ),
        AppError::NotACollection(_) => (
            "Not an ApiArk collection".into(),
            "Open a folder that contains .apiark/apiark.yaml",
        ),
        AppError::MergeConflict(file) => {
            let file = Path::new(file);
            let shown = root.and_then(|r| file.strip_prefix(r).ok()).unwrap_or(file);
            (
                format!("Merge conflict in {}", shown.display()),
                "Fix the conflict markers in your editor, then open the request again.",
            )
        }
        AppError::AlreadyExists(name) => {
            return (
                format!("{name} already exists"),
                "Pick another name.".into(),
            );
        }
        AppError::InvalidName(name) => {
            return (
                format!("Invalid name \"{name}\""),
                "Use a non-empty name that does not start with a dot and is not _folder.".into(),
            );
        }
        AppError::Storage(msg) => return ("Could not read or write a file".into(), msg.clone()),
    };
    (title, hint.into())
}

pub fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} KB", b as f64 / KB),
        b => format!("{:.1} MB", b as f64 / (KB * KB)),
    }
}
