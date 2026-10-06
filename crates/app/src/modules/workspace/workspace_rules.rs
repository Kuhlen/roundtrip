//! Pure decisions for the workspace page; user-facing text lives here, not in domain.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use domain::AppError;
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
    let query: Vec<String> = request
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
    if !query.is_empty() {
        url.push(if url.contains('?') { '&' } else { '?' });
        url.push_str(&query.join("&"));
    }
    url
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
        // upstream suggests turning off verification; slice 1 has no such switch
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
