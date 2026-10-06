//! `body.content` ⇄ Body. Form fields live in `content`, as ApiArk's importers write them.

use domain::http::{Body, FormField, KeyValue, TextKind};
use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use crate::scalar;

#[derive(Serialize, Deserialize)]
// extra keys would be lost on save: such content stays Unsupported
#[serde(deny_unknown_fields)]
struct FieldFile {
    key: String,
    #[serde(default)]
    value: String,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    enabled: bool,
    // Postman imports carry no type; only "file" matters
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
}

fn yes() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

/// Content we can't read exactly is Unsupported, so save never rewrites it.
pub(crate) fn parse(kind: &str, content: &Value) -> Body {
    let text = scalar(content);
    let plain = matches!(content, Value::String(_) | Value::Null);
    let text_body = |kind| Body::Text {
        kind,
        text: text.clone(),
    };
    match kind {
        "none" => Body::None,
        "json" => text_body(TextKind::Json),
        "xml" => text_body(TextKind::Xml),
        "raw" => text_body(TextKind::Raw),
        // encoded content never has a space or line break; that is a Bruno `key: value` block
        "urlencoded" if plain && !text.trim().contains(['\n', ' ']) => Body::Urlencoded(
            form_urlencoded::parse(text.trim().as_bytes())
                .map(|(k, v)| KeyValue::new(k, v))
                .collect(),
        ),
        "form-data" if plain && text.trim().is_empty() => Body::FormData(vec![]),
        "form-data" if plain => match serde_json::from_str::<Vec<FieldFile>>(&text) {
            Ok(fields) => Body::FormData(
                fields
                    .into_iter()
                    .map(|f| FormField {
                        is_file: f.kind.as_deref() == Some("file"),
                        key: f.key,
                        value: f.value,
                        enabled: f.enabled,
                    })
                    .collect(),
            ),
            Err(_) => Body::Unsupported(kind.to_owned()),
        },
        "binary" if plain => Body::Binary(text),
        other => Body::Unsupported(other.to_owned()),
    }
}

/// None: `body` stays as it is (Unsupported) or goes away (None).
pub(crate) fn content(body: &Body) -> Option<String> {
    match body {
        Body::None | Body::Unsupported(_) => None,
        Body::Text { text, .. } => Some(text.clone()),
        Body::Binary(path) => Some(path.clone()),
        // ApiArk content maps have no enabled flag
        Body::Urlencoded(rows) => Some(
            form_urlencoded::Serializer::new(String::new())
                .extend_pairs(
                    rows.iter()
                        .filter(|r| r.is_active())
                        .map(|r| (&r.key, &r.value)),
                )
                .finish()
                // {{var}} stays readable in the file
                .replace("%7B", "{")
                .replace("%7D", "}"),
        ),
        Body::FormData(fields) => {
            let rows: Vec<FieldFile> = fields
                .iter()
                .map(|f| FieldFile {
                    key: f.key.clone(),
                    value: f.value.clone(),
                    enabled: f.enabled,
                    kind: f.is_file.then(|| "file".to_owned()),
                })
                .collect();
            serde_json::to_string_pretty(&rows).ok()
        }
    }
}
