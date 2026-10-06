//! ApiArk built-ins: `{{$uuid}}`, `{{$timestamp}}`, ...

use chrono::Utc;

pub fn resolve(name: &str) -> Option<String> {
    let value = match name {
        "$uuid" => uuid::Uuid::new_v4().to_string(),
        "$timestamp" => Utc::now().timestamp().to_string(),
        "$timestampMs" => Utc::now().timestamp_millis().to_string(),
        "$isoTimestamp" => Utc::now().to_rfc3339(),
        "$randomInt" => rand::random_range(0..=1000u32).to_string(),
        "$randomFloat" => format!("{:.6}", rand::random::<f64>()),
        "$randomString" => random_chars(16, b"0123456789abcdefghijklmnopqrstuvwxyz"),
        "$randomEmail" => format!(
            "{}@example.com",
            random_chars(8, b"abcdefghijklmnopqrstuvwxyz")
        ),
        _ => return None,
    };
    Some(value)
}

fn random_chars(len: usize, alphabet: &[u8]) -> String {
    (0..len)
        .map(|_| alphabet[rand::random_range(0..alphabet.len())] as char)
        .collect()
}
