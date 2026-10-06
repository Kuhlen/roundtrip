//! Own binary: the event-loop testing backend can be set once per process.
mod support;

use std::time::{Duration, Instant};

use slint::{Timer, TimerMode};
use support::{build, fakes, open_with_env, row_of, state};

#[test]
fn edit_is_saved_about_one_second_later() {
    i_slint_backend_testing::init_integration_test_with_system_time();
    let f = fakes();
    let (ui, c) = build(&f);
    open_with_env(&ui, &c);
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    s.set_url("https://example.com/edited".into());
    s.invoke_changed();

    let start = Instant::now();
    let collections = f.collections.clone();
    let poll = Timer::default();
    poll.start(TimerMode::Repeated, Duration::from_millis(10), move || {
        if !collections.saved.borrow().is_empty() {
            slint::quit_event_loop().expect("quit");
        }
    });
    let cap = Timer::default();
    cap.start(TimerMode::SingleShot, Duration::from_secs(3), || {
        slint::quit_event_loop().expect("quit")
    });
    slint::run_event_loop().expect("event loop");

    let saved = f.collections.saved.borrow();
    assert_eq!(saved.len(), 1, "auto-save wrote the file");
    assert_eq!(saved[0].1.url, "https://example.com/edited");
    assert!(start.elapsed() >= Duration::from_millis(900));
}
