use std::fs::File;
use std::path::Path;
use std::time::{Duration, Instant};

use domain::AppError;
use domain::auth::{ApiKeyPlace, Auth};
use domain::http::{Body, HttpSender, KeyValue, Request, Response};
use reqwest::Url;
use reqwest::blocking::multipart::Form;
use reqwest::blocking::{Client, RequestBuilder};

const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BODY: u64 = 10 * 1024 * 1024;
const DISPLAY_LIMIT: usize = 1024 * 1024;

// one shared client, no per-request proxy/TLS/cookie jar; upstream builds one per request
pub struct ReqwestSender {
    client: Client,
}

impl ReqwestSender {
    pub fn new() -> Result<Self, AppError> {
        let client = Client::builder()
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .map_err(|e| AppError::Request(e.to_string()))?;
        Ok(Self { client })
    }
}

impl HttpSender for ReqwestSender {
    fn send(&self, request: &Request) -> Result<Response, AppError> {
        // A request that differs from the file is never sent
        if let Body::Unsupported(kind) = &request.body {
            return Err(AppError::Request(format!("{kind} body is not supported")));
        }
        if let Some(Auth::Unsupported(kind)) = &request.auth {
            return Err(AppError::Request(format!("{kind} auth is not supported")));
        }
        let mut params = request.params.clone();
        if let Some(Auth::ApiKey {
            key,
            value,
            place: ApiKeyPlace::Query,
        }) = &request.auth
            && !key.is_empty()
        {
            params.push(KeyValue::new(key, value));
        }
        let url = build_url(&request.url, &params)?;
        let method = reqwest::Method::from_bytes(request.method.as_str().as_bytes())
            .map_err(|e| AppError::Request(e.to_string()))?;
        let mut builder = self.client.request(method, url);
        let multipart = matches!(request.body, Body::FormData(_));
        for h in request.headers.iter().filter(|h| h.is_active()) {
            // multipart needs its own boundary in Content-Type
            if multipart && h.key.eq_ignore_ascii_case("content-type") {
                continue;
            }
            builder = builder.header(&h.key, &h.value);
        }
        // upstream appends a second Authorization; a header the user wrote wins
        let user_set = |name: &str| {
            request
                .headers
                .iter()
                .any(|h| h.is_active() && h.key.eq_ignore_ascii_case(name))
        };
        match &request.auth {
            Some(Auth::Bearer { token }) if !user_set("authorization") => {
                builder = builder.bearer_auth(token);
            }
            Some(Auth::Basic { username, password }) if !user_set("authorization") => {
                builder = builder.basic_auth(username, Some(password));
            }
            Some(Auth::ApiKey {
                key,
                value,
                place: ApiKeyPlace::Header,
            }) if !key.is_empty() && !user_set(key) => {
                builder = builder.header(key, value);
            }
            _ => {}
        }
        builder = apply_body(builder, &request.body, user_set("content-type"))?;

        let start = Instant::now();
        let response = builder.send().map_err(classify)?;
        let status = response.status();
        let headers = response
            .headers()
            .iter()
            .map(|(k, v)| KeyValue::new(k.as_str(), v.to_str().unwrap_or("<binary>")))
            .collect();
        if response.content_length().is_some_and(|len| len > MAX_BODY) {
            return Err(AppError::ResponseTooLarge(MAX_BODY));
        }
        let bytes = response.bytes().map_err(classify)?;
        let elapsed = start.elapsed();
        if bytes.len() as u64 > MAX_BODY {
            return Err(AppError::ResponseTooLarge(MAX_BODY));
        }
        let (body, truncated) = display_body(&bytes);
        Ok(Response {
            status: status.as_u16(),
            status_text: status.canonical_reason().unwrap_or("Unknown").to_owned(),
            headers,
            body,
            elapsed,
            size_bytes: bytes.len() as u64,
            truncated,
        })
    }
}

fn build_url(base: &str, params: &[KeyValue]) -> Result<Url, AppError> {
    let mut url = Url::parse(base).map_err(|e| AppError::InvalidUrl(format!("{e}: {base}")))?;
    let active: Vec<_> = params.iter().filter(|p| p.is_active()).collect();
    if !active.is_empty() {
        let mut query = url.query_pairs_mut();
        for p in active {
            query.append_pair(&p.key, &p.value);
        }
    }
    Ok(url)
}

fn apply_body(
    builder: RequestBuilder,
    body: &Body,
    user_type: bool,
) -> Result<RequestBuilder, AppError> {
    // upstream always appends, duplicating a user-set Content-Type
    let typed = |b: RequestBuilder, t: &str| {
        if user_type {
            b
        } else {
            b.header("Content-Type", t)
        }
    };
    Ok(match body {
        Body::None | Body::Unsupported(_) => builder,
        Body::Text { kind, text } => typed(builder, kind.content_type()).body(text.clone()),
        // .form() would overwrite a user-set Content-Type
        Body::Urlencoded(rows) => {
            let encoded = form_urlencoded::Serializer::new(String::new())
                .extend_pairs(
                    rows.iter()
                        .filter(|r| r.is_active())
                        .map(|r| (&r.key, &r.value)),
                )
                .finish();
            typed(builder, "application/x-www-form-urlencoded").body(encoded)
        }
        Body::FormData(fields) => {
            let mut form = Form::new();
            for f in fields.iter().filter(|f| f.is_active()) {
                form = if f.is_file {
                    form.file(f.key.clone(), file_path(&f.value)?)
                        .map_err(|_| AppError::File(f.value.clone()))?
                } else {
                    form.text(f.key.clone(), f.value.clone())
                };
            }
            builder.multipart(form)
        }
        Body::Binary(path) => {
            let file = File::open(file_path(path)?).map_err(|_| AppError::File(path.clone()))?;
            typed(builder, "application/octet-stream").body(file)
        }
    })
}

/// Empty, missing or a folder: reqwest would fail mid-send with a vague error.
fn file_path(path: &str) -> Result<&Path, AppError> {
    let p = Path::new(path);
    if p.is_file() {
        Ok(p)
    } else {
        Err(AppError::File(path.to_owned()))
    }
}

/// Cut at a char boundary; upstream cut mid-char and fell back to "<binary>".
fn display_body(bytes: &[u8]) -> (String, bool) {
    let truncated = bytes.len() > DISPLAY_LIMIT;
    let shown = &bytes[..bytes.len().min(DISPLAY_LIMIT)];
    match std::str::from_utf8(shown) {
        Ok(s) => (s.to_owned(), truncated),
        Err(e) if truncated && e.error_len().is_none() => (
            String::from_utf8_lossy(&shown[..e.valid_up_to()]).into_owned(),
            true,
        ),
        Err(_) => (format!("<binary data: {} bytes>", bytes.len()), truncated),
    }
}

/// None: not JSON.
pub fn pretty_json(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    serde_json::to_string_pretty(&value).ok()
}

/// Parity: apiark http/error_classifier.rs
fn classify(err: reqwest::Error) -> AppError {
    let msg = format!("{err:#}");
    if err.is_timeout() {
        return AppError::Timeout(TIMEOUT.as_millis() as u64);
    }
    if err.is_connect() {
        let dns = [
            "dns error",
            "Name or service not known",
            "getaddrinfo",
            "failed to lookup",
        ];
        return if dns.iter().any(|d| msg.contains(d)) {
            AppError::DnsFailure(msg)
        } else {
            AppError::ConnectionRefused(msg)
        };
    }
    if err.is_builder() {
        return AppError::InvalidUrl(msg);
    }
    let lower = msg.to_lowercase();
    if ["tls", "ssl", "certificate"]
        .iter()
        .any(|t| lower.contains(t))
    {
        return AppError::Tls(msg);
    }
    AppError::Request(msg)
}
