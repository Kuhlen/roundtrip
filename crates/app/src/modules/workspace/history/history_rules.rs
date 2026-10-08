//! Pure text for history rows.

use std::time::SystemTime;

/// "now", "5m ago", "3h ago", "2d ago"; an entry from the future reads as now.
pub fn relative_time(now: SystemTime, at: SystemTime) -> String {
    let secs = now.duration_since(at).map_or(0, |d| d.as_secs());
    match secs {
        0..60 => "now".into(),
        60..3600 => format!("{}m ago", secs / 60),
        3600..86_400 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}
