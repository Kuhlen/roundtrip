//! Pure decisions for the workspace page; user-facing text lives here, not in domain.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use domain::AppError;
use domain::auth::{ApiKeyPlace, Auth};
use domain::collection::{Collection, Node, Protocol};
use domain::http::{Body, FormField, KeyValue, Method, Request};
use domain::import::{ExportReport, ExportWarning, ImportFormat, ImportWarning};
use domain::interpolation::{Segment, interpolate, segments};

pub use domain::http::resolve_files;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKind {
    Collection { expanded: bool },
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

/// Every open collection as a root row with its tree below.
pub fn flatten_collections(
    collections: &[Collection],
    collapsed: &HashSet<PathBuf>,
) -> Vec<FlatRow> {
    let mut rows = Vec::new();
    for c in collections {
        let expanded = !collapsed.contains(&c.path);
        rows.push(FlatRow {
            depth: 0,
            name: c.name.clone(),
            path: c.path.clone(),
            kind: RowKind::Collection { expanded },
        });
        if expanded {
            walk(&c.children, 1, collapsed, &mut rows);
        }
    }
    rows
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
        RowKind::Collection { .. } | RowKind::Folder { .. } => "",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartKind {
    Text,
    Resolved,
    Unresolved,
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlPart {
    pub text: String,
    /// lookup name; "" for text
    pub name: String,
    pub kind: PartKind,
    /// hover text
    pub tip: String,
}

/// URL overlay pieces; empty when nothing to highlight. Same precedence as Send:
/// environment value, then dynamic generator.
pub fn url_segments(
    url: &str,
    lookup: impl Fn(&str) -> Option<String>,
    is_secret: impl Fn(&str) -> bool,
    is_dynamic: impl Fn(&str) -> bool,
) -> Vec<UrlPart> {
    let all = segments(url);
    if !all.iter().any(|s| matches!(s, Segment::Var { .. })) {
        return Vec::new();
    }
    let text = |t: String| UrlPart {
        text: t,
        name: String::new(),
        kind: PartKind::Text,
        tip: String::new(),
    };
    all.into_iter()
        .map(|s| match s {
            Segment::Text(t) => text(t),
            // nothing to save under an empty key
            Segment::Var { name, raw } if name.is_empty() => text(raw),
            Segment::Var { name, raw } => {
                let (kind, tip) = match lookup(&name) {
                    Some(_) if is_secret(&name) => (PartKind::Resolved, format!("{name} = ••••")),
                    Some(value) => (PartKind::Resolved, format!("{name} = {value}")),
                    None if is_dynamic(&name) => {
                        (PartKind::Dynamic, "Generated on each send".to_owned())
                    }
                    None => (PartKind::Unresolved, format!("{name} is not set")),
                };
                UrlPart {
                    text: raw,
                    name,
                    kind,
                    tip,
                }
            }
        })
        .collect()
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

/// Note under "Inherit" for a tab saved in no collection.
pub fn inherit_note_untitled() -> String {
    "No collection: no auth.".into()
}

pub fn interpolate_request(request: &Request, lookup: impl Fn(&str) -> Option<String>) -> Request {
    let text = |t: &str| interpolate(t, &lookup);
    let rows = |rows: &[KeyValue]| -> Vec<KeyValue> {
        rows.iter()
            .map(|r| KeyValue {
                key: text(&r.key),
                value: text(&r.value),
                enabled: r.enabled,
            })
            .collect()
    };
    let body = match &request.body {
        Body::Text { kind, text: t } => Body::Text {
            kind: *kind,
            text: text(t),
        },
        Body::Urlencoded(r) => Body::Urlencoded(rows(r)),
        Body::FormData(fields) => Body::FormData(
            fields
                .iter()
                .map(|f| FormField {
                    key: text(&f.key),
                    value: text(&f.value),
                    ..f.clone()
                })
                .collect(),
        ),
        Body::Binary(path) => Body::Binary(text(path)),
        Body::None | Body::Unsupported(_) => request.body.clone(),
    };
    Request {
        method: request.method,
        url: text(&request.url),
        params: rows(&request.params),
        headers: rows(&request.headers),
        body,
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
        AppError::DuplicateKey(key) => {
            return (
                format!("Duplicate key \"{key}\""),
                "Each variable key can appear once.".into(),
            );
        }
        AppError::File(path) if path.is_empty() => {
            ("No file chosen".into(), "Pick a file in the Body tab.")
        }
        AppError::File(_) => ("Cannot read file".into(), "Check the path in the Body tab."),
        AppError::InvalidJson(_) => (
            "Variables: invalid JSON".into(),
            "Fix Variables before saving.",
        ),
        AppError::Cancelled => ("Request cancelled".into(), ""),
        AppError::History(msg) => return ("History error".into(), msg.clone()),
        AppError::Import(msg) => return ("Import failed".into(), msg.clone()),
        AppError::Storage(msg) => return ("Could not read or write a file".into(), msg.clone()),
    };
    (title, hint.into())
}

/// Note shown instead of a body editor.
pub fn unsupported_body_note(kind: &str) -> String {
    if Body::KINDS.contains(&kind) {
        format!(
            "{kind} body: Roundtrip can't read this content. Saving leaves it as it is in the file."
        )
    } else {
        format!("{kind} body: not supported yet. Saving leaves it as it is in the file.")
    }
}

pub fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} KB", b as f64 / KB),
        b => format!("{:.1} MB", b as f64 / (KB * KB)),
    }
}

/// Save as targets: the root, then every folder depth-first, labelled "/ a / b".
pub fn folder_choices(collection: &Collection) -> Vec<(String, PathBuf)> {
    fn walk(nodes: &[Node], label: &str, out: &mut Vec<(String, PathBuf)>) {
        for n in nodes {
            if let Node::Folder {
                name,
                path,
                children,
            } = n
            {
                let label = if label == "/" {
                    format!("/ {name}")
                } else {
                    format!("{label} / {name}")
                };
                out.push((label.clone(), path.clone()));
                walk(children, &label, out);
            }
        }
    }
    let mut out = vec![("/".to_owned(), collection.path.clone())];
    walk(&collection.children, "/", &mut out);
    out
}

/// A paste, not typing: `curl <something>` that was not curl before, or grew by more than a key.
pub fn is_curl_paste(before: &str, after: &str) -> bool {
    let Some(rest) = after.trim_start().strip_prefix("curl ") else {
        return false;
    };
    !rest.trim().is_empty()
        && (!before.trim_start().starts_with("curl ")
            || after.chars().count() > before.chars().count() + 1)
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// "1 folder · 2 requests · 1 environment"; empty parts left out except requests.
pub fn import_counts((folders, requests, environments): (usize, usize, usize)) -> String {
    let mut parts = Vec::new();
    if folders > 0 {
        parts.push(plural(folders, "folder"));
    }
    parts.push(plural(requests, "request"));
    if environments > 0 {
        parts.push(plural(environments, "environment"));
    }
    parts.join(" · ")
}

/// One line per kind, first-seen order, `(n×)` when repeated.
pub fn import_warnings(warnings: &[ImportWarning]) -> Vec<String> {
    let mut grouped: Vec<(&ImportWarning, usize)> = Vec::new();
    for w in warnings {
        match grouped.iter_mut().find(|(g, _)| *g == w) {
            Some((_, n)) => *n += 1,
            None => grouped.push((w, 1)),
        }
    }
    grouped
        .into_iter()
        .map(|(w, n)| {
            let text = match w {
                ImportWarning::UnsupportedAuth(kind) => {
                    format!(
                        "{kind} auth is not supported; those requests inherit the collection auth, if any"
                    )
                }
                ImportWarning::FolderAuth => {
                    "Folder auth is not supported; requests use the collection auth".into()
                }
                ImportWarning::NoAuthInherits => {
                    "\"No auth\" requests will send the collection auth".into()
                }
                ImportWarning::PathVariables => {
                    "Path variables (:id, {id}) stay in the URL; fill them in by hand".into()
                }
                ImportWarning::Scripts => "Scripts are kept in the files but not run".into(),
                ImportWarning::CollectionScriptsDropped => {
                    "Collection and folder scripts were left out".into()
                }
                ImportWarning::DisabledDropped => {
                    "Disabled headers, params and variables were left out".into()
                }
                ImportWarning::UnknownMethod(m) => {
                    format!("{m} requests were skipped: method not supported")
                }
                ImportWarning::UnsupportedBody(mode) => format!("{mode} bodies were left out"),
                ImportWarning::UnsupportedRequest(kind) => {
                    format!("{kind} requests were skipped: not supported")
                }
                ImportWarning::TemplateTags => {
                    "Template tags ({% … %}) stay as text; replace them by hand".into()
                }
                ImportWarning::ExternalRef => "References to other files were left empty".into(),
                ImportWarning::CookieParams => "Cookie parameters were left out".into(),
                ImportWarning::FolderVariables => "Folder environments were left out".into(),
                ImportWarning::OtherWorkspaces(n) => {
                    format!("Only the first workspace was imported; {n} more were skipped")
                }
            };
            if n > 1 {
                format!("{text} ({n}×)")
            } else {
                text
            }
        })
        .collect()
}

pub fn import_title(format: &ImportFormat) -> String {
    match format {
        ImportFormat::Postman => "Import Postman collection".into(),
        ImportFormat::OpenApi(version) => format!("Import OpenAPI {version} spec"),
        ImportFormat::Insomnia => "Import Insomnia collection".into(),
    }
}

/// Banner (title, hint) after an export; `dirty` = that collection's unsaved tabs.
pub fn export_lines(report: &ExportReport, dirty: usize) -> (String, String) {
    let mut lines: Vec<String> = report
        .warnings
        .iter()
        .map(|w| match *w {
            ExportWarning::SkippedProtocol(n) => {
                format!("{} skipped", plural(n, "WebSocket, SSE or gRPC request"))
            }
            ExportWarning::Unreadable(n) => {
                format!("{} skipped", plural(n, "unreadable request file"))
            }
            ExportWarning::UnsupportedBody(n) => format!("Body type not supported, left out ({n})"),
            ExportWarning::UnsupportedAuth(n) => format!("Auth type not supported, left out ({n})"),
            ExportWarning::EnvironmentsNotExported(n) => format!(
                "{} not exported; Postman keeps environments in separate files",
                plural(n, "environment")
            ),
        })
        .collect();
    match dirty {
        0 => {}
        1 => lines.push("1 unsaved request exported in its saved state".into()),
        n => lines.push(format!(
            "{n} unsaved requests exported in their saved state"
        )),
    }
    (
        format!("Exported {}", plural(report.requests, "request")),
        lines.join(". "),
    )
}
