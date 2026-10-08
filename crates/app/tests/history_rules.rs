use std::time::{Duration, SystemTime};

use app::modules::workspace::history::history_rules::relative_time;

#[test]
fn relative_time_steps() {
    let now = SystemTime::now();
    let ago = |s: u64| relative_time(now, now - Duration::from_secs(s));
    assert_eq!(ago(0), "now");
    assert_eq!(ago(59), "now");
    assert_eq!(ago(60), "1m ago");
    assert_eq!(ago(3599), "59m ago");
    assert_eq!(ago(3600), "1h ago");
    assert_eq!(ago(86_399), "23h ago");
    assert_eq!(ago(86_400), "1d ago");
    // clock moved back: entry looks newer than now
    assert_eq!(relative_time(now, now + Duration::from_secs(90)), "now");
}
