use std::time::{Duration, Instant};

use domain::AppError;
use domain::http::{BodyKind, HttpSender, KeyValue, Request, Response};
use reqwest::Url;
use reqwest::blocking::Client;

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
        if let BodyKind::Unsupported(kind) = &request.body_kind {
            return Err(AppError::Request(format!("{kind} body is not supported")));
        }
        let url = build_url(&request.url, &request.params)?;
        let method = reqwest::Method::from_bytes(request.method.as_str().as_bytes())
            .map_err(|e| AppError::Request(e.to_string()))?;
        let mut builder = self.client.request(method, url);
        for h in request.headers.iter().filter(|h| h.is_active()) {
            builder = builder.header(&h.key, &h.value);
        }
        if let Some(content_type) = request.body_kind.content_type() {
            // upstream always appends, duplicating a user-set Content-Type
            let user_set = request
                .headers
                .iter()
                .any(|h| h.is_active() && h.key.eq_ignore_ascii_case("content-type"));
            if !user_set {
                builder = builder.header("Content-Type", content_type);
            }
            builder = builder.body(request.body.clone());
        }

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
