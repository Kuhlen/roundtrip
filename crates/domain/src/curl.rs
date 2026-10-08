//! cURL command to Request. Pure text work: no file is read.

use crate::AppError;
use crate::auth::{ApiKeyPlace, Auth};
use crate::http::{Body, FormField, KeyValue, Method, Request, TextKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurlImport {
    pub request: Request,
    /// flags we do not model, for the banner
    pub skipped: Vec<String>,
}

fn import(msg: impl Into<String>) -> AppError {
    AppError::Import(msg.into())
}

fn takes_value(flag: &str) -> bool {
    matches!(
        flag,
        "-X" | "--request"
            | "-H"
            | "--header"
            | "-d"
            | "--data"
            | "--data-raw"
            | "--data-binary"
            | "--data-ascii"
            | "--data-urlencode"
            | "-F"
            | "--form"
            | "--form-string"
            | "-u"
            | "--user"
            | "-b"
            | "--cookie"
            | "-A"
            | "--user-agent"
            | "-e"
            | "--referer"
            | "--url"
    )
}

/// No value; either modelled (-G, -I) or irrelevant to the request.
fn is_switch(flag: &str) -> bool {
    matches!(
        flag,
        "-G" | "--get"
            | "-I"
            | "--head"
            | "-k"
            | "--insecure"
            | "-L"
            | "--location"
            | "--compressed"
            | "-s"
            | "--silent"
            | "-S"
            | "--show-error"
            | "-v"
            | "--verbose"
            | "-i"
            | "--include"
    )
}

enum Data {
    /// sent as given
    Raw(String),
    /// --data-urlencode: value part gets encoded
    Encode(String),
    File(String),
}

#[derive(Default)]
struct Parsed {
    method: Option<Method>,
    url: Option<String>,
    headers: Vec<KeyValue>,
    data: Vec<Data>,
    form: Vec<FormField>,
    user: Option<String>,
    get: bool,
    head: bool,
    skipped: Vec<String>,
}

pub fn parse(command: &str) -> Result<CurlImport, AppError> {
    let tokens = tokenize(command)?;
    let mut args = tokens.into_iter().peekable();
    if args.next().as_deref() != Some("curl") {
        return Err(import("not a curl command"));
    }
    let mut p = Parsed::default();
    while let Some(arg) = args.next() {
        if !arg.starts_with('-') || arg == "-" {
            if p.url.is_none() {
                p.url = Some(arg);
            } else {
                p.skipped.push(arg);
            }
            continue;
        }
        for (flag, inline) in expand(&arg) {
            if is_switch(&flag) {
                match flag.as_str() {
                    "-G" | "--get" => p.get = true,
                    "-I" | "--head" => p.head = true,
                    _ => {}
                }
                continue;
            }
            if !takes_value(&flag) {
                // unknown: eat its value unless the next token looks like a flag or the URL
                if inline.is_none()
                    && args
                        .peek()
                        .is_some_and(|n| !n.starts_with('-') && !n.contains("://"))
                {
                    args.next();
                }
                p.skipped.push(flag);
                continue;
            }
            let value = match inline {
                Some(v) => v,
                None => args
                    .next()
                    .ok_or_else(|| import(format!("{flag} needs a value")))?,
            };
            apply(&mut p, &flag, value)?;
        }
    }
    build(p)
}

/// `-sSL` gives each switch; `-XPOST` / `-H'X: 1'` give flag + attached value.
fn expand(arg: &str) -> Vec<(String, Option<String>)> {
    if arg.starts_with("--") || arg.len() <= 2 {
        return vec![(arg.to_owned(), None)];
    }
    let mut out = Vec::new();
    for (i, c) in arg[1..].char_indices() {
        let flag = format!("-{c}");
        if takes_value(&flag) {
            let rest = &arg[1 + i + c.len_utf8()..];
            out.push((flag, (!rest.is_empty()).then(|| rest.to_owned())));
            break;
        }
        out.push((flag, None));
    }
    out
}

fn apply(p: &mut Parsed, flag: &str, value: String) -> Result<(), AppError> {
    match flag {
        "-X" | "--request" => {
            let upper = value.to_uppercase();
            p.method = Some(
                Method::parse(&upper)
                    .ok_or_else(|| import(format!("unsupported method {value}")))?,
            );
        }
        "-H" | "--header" => match value.split_once(':') {
            Some((k, v)) => p.headers.push(KeyValue::new(k.trim(), v.trim())),
            None => p.skipped.push(format!("-H {value}")),
        },
        "-d" | "--data" | "--data-binary" | "--data-ascii" => match value.strip_prefix('@') {
            Some(path) => p.data.push(Data::File(path.to_owned())),
            None => p.data.push(Data::Raw(value)),
        },
        "--data-raw" => p.data.push(Data::Raw(value)),
        "--data-urlencode" => p.data.push(Data::Encode(value)),
        "-F" | "--form" => {
            let (key, v) = value.split_once('=').unwrap_or((value.as_str(), ""));
            let field = match v.strip_prefix('@') {
                // `;type=...` / `;filename=...` describe the part, not the path
                Some(path) => FormField::file(key, path.split(';').next().unwrap_or_default()),
                None => FormField::text(key, v),
            };
            p.form.push(field);
        }
        "--form-string" => {
            let (key, v) = value.split_once('=').unwrap_or((value.as_str(), ""));
            p.form.push(FormField::text(key, v));
        }
        "-u" | "--user" => p.user = Some(value),
        // no `=`: a cookie jar file, nothing to send
        "-b" | "--cookie" if value.contains('=') => p.headers.push(KeyValue::new("Cookie", value)),
        "-b" | "--cookie" => p.skipped.push(format!("-b {value}")),
        "-A" | "--user-agent" => p.headers.push(KeyValue::new("User-Agent", value)),
        "-e" | "--referer" => p.headers.push(KeyValue::new("Referer", value)),
        "--url" => p.url = Some(value),
        _ => p.skipped.push(flag.to_owned()),
    }
    Ok(())
}

/// URL without its query string, and that query as decoded rows.
pub fn split_query(url: &str) -> (String, Vec<KeyValue>) {
    match url.split_once('?') {
        Some((base, query)) => (base.to_owned(), pairs(query)),
        None => (url.to_owned(), Vec::new()),
    }
}

fn build(p: Parsed) -> Result<CurlImport, AppError> {
    let url = p.url.ok_or_else(|| import("no URL in the curl command"))?;
    let (base, mut params) = split_query(&url);
    if !p.form.is_empty() && !p.data.is_empty() {
        return Err(import("-F cannot be mixed with -d"));
    }
    let file = match p.data.as_slice() {
        [Data::File(path)] => Some(path.clone()),
        data if data.iter().any(|d| matches!(d, Data::File(_))) => {
            return Err(import("@file cannot be mixed with other data"));
        }
        _ => None,
    };
    let joined = p
        .data
        .iter()
        .map(|d| match d {
            Data::Raw(s) => s.clone(),
            Data::Encode(s) => encode_data(s),
            Data::File(_) => String::new(),
        })
        .collect::<Vec<_>>()
        .join("&");
    let has_body = !p.data.is_empty() || !p.form.is_empty();
    let method = p.method.unwrap_or(if p.head {
        Method::Head
    } else if p.get || !has_body {
        Method::Get
    } else {
        Method::Post
    });
    let content_type = p
        .headers
        .iter()
        .find(|h| h.key.eq_ignore_ascii_case("content-type"))
        .map(|h| h.value.to_lowercase());
    let body = if !p.form.is_empty() {
        Body::FormData(p.form)
    } else if let Some(path) = file {
        Body::Binary(path)
    } else if p.data.is_empty() {
        Body::None
    } else if p.get {
        params.extend(pairs(&joined));
        Body::None
    } else {
        text_body(content_type.as_deref(), joined)
    };
    let auth = p.user.map(|u| {
        let (username, password) = u.split_once(':').unwrap_or((u.as_str(), ""));
        Auth::Basic {
            username: username.to_owned(),
            password: password.to_owned(),
        }
    });
    Ok(CurlImport {
        request: Request {
            method,
            url: base,
            params,
            headers: p.headers,
            body,
            auth,
        },
        skipped: p.skipped,
    })
}

fn text_body(content_type: Option<&str>, data: String) -> Body {
    let text = |kind| Body::Text {
        kind,
        text: data.clone(),
    };
    match content_type {
        Some(ct) if ct.contains("json") => text(TextKind::Json),
        Some(ct) if ct.contains("xml") => text(TextKind::Xml),
        Some(ct) if ct.contains("x-www-form-urlencoded") => Body::Urlencoded(pairs(&data)),
        Some(_) => text(TextKind::Raw),
        // pastes often drop the header; JSON parsed as a form is useless
        None if data.trim_start().starts_with(['{', '[']) => text(TextKind::Json),
        None if data.contains('=') => Body::Urlencoded(pairs(&data)),
        None => text(TextKind::Raw),
    }
}

/// `k=v` becomes `k=` + encoded v, as curl's --data-urlencode does.
fn encode_data(s: &str) -> String {
    match s.split_once('=') {
        Some((k, v)) => format!("{k}={}", encode(v)),
        None => encode(s),
    }
}

/// Percent-encode everything but RFC 3986 unreserved characters.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `+` to space, `%XX` to byte; broken escapes stay literal.
fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            // is_char_boundary: a multi-byte char right after `%` must not be sliced
            b'%' if i + 2 < bytes.len() && s.is_char_boundary(i + 3) => {
                match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn pairs(s: &str) -> Vec<KeyValue> {
    s.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            KeyValue::new(decode(k), decode(v))
        })
        .collect()
}

/// Decode one `$'...'` escape after the backslash; malformed ones stay literal.
fn ansi_escape(e: char, chars: &mut std::iter::Peekable<std::str::Chars>, out: &mut String) {
    let simple = match e {
        'n' => Some('\n'),
        't' => Some('\t'),
        'r' => Some('\r'),
        'a' => Some('\u{7}'),
        'b' => Some('\u{8}'),
        'e' | 'E' => Some('\u{1b}'),
        'f' => Some('\u{c}'),
        'v' => Some('\u{b}'),
        '\\' | '\'' | '"' | '?' => Some(e),
        _ => None,
    };
    if let Some(c) = simple {
        out.push(c);
        return;
    }
    let (radix, max) = match e {
        'x' => (16, 2),
        'u' => (16, 4),
        'U' => (16, 8),
        '0'..='7' => (8, 3),
        _ => {
            out.push('\\');
            out.push(e);
            return;
        }
    };
    let mut digits = String::new();
    if e.is_digit(8) {
        digits.push(e);
    }
    while digits.len() < max && chars.peek().is_some_and(|c| c.is_digit(radix)) {
        digits.extend(chars.next());
    }
    match u32::from_str_radix(&digits, radix)
        .ok()
        .and_then(char::from_u32)
    {
        Some(c) if !digits.is_empty() => out.push(c),
        _ => {
            out.push('\\');
            out.push(e);
            out.push_str(&digits);
        }
    }
}

/// POSIX shell words: '...', "...", $'...', backslash escapes; a bare `\` before whitespace is a line continuation.
fn tokenize(s: &str) -> Result<Vec<String>, AppError> {
    let unterminated = || import("unterminated quote");
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_token = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_token = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => cur.push(c),
                        None => return Err(unterminated()),
                    }
                }
            }
            '$' if chars.peek() == Some(&'\'') => {
                chars.next();
                in_token = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some('\\') => match chars.next() {
                            Some(e) => ansi_escape(e, &mut chars, &mut cur),
                            None => return Err(unterminated()),
                        },
                        Some(c) => cur.push(c),
                        None => return Err(unterminated()),
                    }
                }
            }
            '"' => {
                in_token = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(c @ ('"' | '\\' | '$' | '`')) => cur.push(c),
                            Some('\n') => {}
                            Some(c) => {
                                cur.push('\\');
                                cur.push(c);
                            }
                            None => return Err(unterminated()),
                        },
                        Some(c) => cur.push(c),
                        None => return Err(unterminated()),
                    }
                }
            }
            '\\' => match chars.next() {
                // Slint pastes newlines as spaces, so `\` + space is a continuation too
                Some(c) if c.is_whitespace() && !in_token => {}
                Some('\n' | '\r') => {}
                Some(c) => {
                    in_token = true;
                    cur.push(c);
                }
                None => {}
            },
            c if c.is_whitespace() => {
                if in_token {
                    out.push(std::mem::take(&mut cur));
                    in_token = false;
                }
            }
            c => {
                in_token = true;
                cur.push(c);
            }
        }
    }
    if in_token {
        out.push(cur);
    }
    Ok(out)
}

/// Request as a runnable command. Expects interpolated text, effective auth and absolute paths.
pub fn to_command(request: &Request) -> Result<String, AppError> {
    if request.url.trim().is_empty() {
        return Err(import("the URL is empty"));
    }
    let mut query: Vec<String> = request
        .params
        .iter()
        .filter(|p| p.is_active())
        .map(|p| format!("{}={}", encode(&p.key), encode(&p.value)))
        .collect();
    let mut headers: Vec<(String, String)> = request
        .headers
        .iter()
        .filter(|h| h.is_active())
        .map(|h| (h.key.clone(), h.value.clone()))
        .collect();
    let mut user = None;
    match &request.auth {
        None => {}
        Some(Auth::Bearer { token }) => {
            headers.push(("Authorization".into(), format!("Bearer {token}")));
        }
        Some(Auth::Basic { username, password }) => user = Some(format!("{username}:{password}")),
        Some(Auth::ApiKey {
            key,
            value,
            place: ApiKeyPlace::Header,
        }) => headers.push((key.clone(), value.clone())),
        Some(Auth::ApiKey {
            key,
            value,
            place: ApiKeyPlace::Query,
        }) => query.push(format!("{}={}", encode(key), encode(value))),
        Some(Auth::Unsupported(kind)) => {
            return Err(import(format!("{kind} auth has no curl form")));
        }
    }
    if let Body::Text { kind, .. } = &request.body
        && !headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
    {
        // send sets it too; the copy must send the same
        headers.push(("Content-Type".into(), kind.content_type().into()));
    }
    let mut url = request.url.clone();
    if !query.is_empty() {
        url.push(if url.contains('?') { '&' } else { '?' });
        url.push_str(&query.join("&"));
    }
    let mut first = String::from("curl");
    match request.method {
        Method::Get if matches!(request.body, Body::None) => {}
        // curl turns a body into POST unless told otherwise
        Method::Get => first.push_str(" -X GET"),
        // -X HEAD would wait for a body that never comes
        Method::Head => first.push_str(" -I"),
        m => first.push_str(&format!(" -X {}", m.as_str())),
    }
    first.push(' ');
    first.push_str(&quote(&url));
    let mut parts = vec![first];
    for (k, v) in &headers {
        parts.push(format!("-H {}", quote(&format!("{k}: {v}"))));
    }
    if let Some(u) = user {
        parts.push(format!("-u {}", quote(&u)));
    }
    match &request.body {
        Body::None => {}
        Body::Text { text, .. } => parts.push(format!("--data-raw {}", quote(text))),
        Body::Urlencoded(rows) => {
            for r in rows.iter().filter(|r| r.is_active()) {
                parts.push(format!(
                    "--data-urlencode {}",
                    quote(&format!("{}={}", r.key, r.value))
                ));
            }
        }
        Body::FormData(fields) => {
            for f in fields.iter().filter(|f| f.is_active()) {
                parts.push(if f.is_file {
                    format!("-F {}", quote(&format!("{}=@{}", f.key, f.value)))
                } else {
                    // a value starting with @ or < is not a file here
                    format!("--form-string {}", quote(&format!("{}={}", f.key, f.value)))
                });
            }
        }
        Body::Binary(path) => parts.push(format!("--data-binary {}", quote(&format!("@{path}")))),
        Body::Unsupported(kind) => {
            return Err(import(format!("{kind} body has no curl form")));
        }
    }
    Ok(parts.join(" \\\n  "))
}

/// POSIX single quotes; `'` becomes `'\''`.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}
