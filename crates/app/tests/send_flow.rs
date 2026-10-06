//! Own binary: the event-loop testing backend can be set once per process.
mod support;

use std::time::Duration;

use app::ui::{AppWindow, SendStatus, StatusClass};
use domain::AppError;
use domain::auth::Auth;
use domain::http::KeyValue;
use slint::{ComponentHandle, Timer, TimerMode};
use support::{build, fakes, open_with_env, set_collection_auth, state};

/// Run the event loop until the worker's result lands (5 s cap).
fn settle(ui: &AppWindow) {
    let weak = ui.as_weak();
    let poll = Timer::default();
    poll.start(TimerMode::Repeated, Duration::from_millis(5), move || {
        if weak
            .upgrade()
            .is_some_and(|ui| state(&ui).get_send_status() != SendStatus::Sending)
        {
            slint::quit_event_loop().expect("quit");
        }
    });
    let cap = Timer::default();
    cap.start(TimerMode::SingleShot, Duration::from_secs(5), || {
        slint::quit_event_loop().expect("quit")
    });
    slint::run_event_loop().expect("event loop");
}

#[test]
fn send_shows_sending_then_done_then_failed() {
    i_slint_backend_testing::init_integration_test_with_system_time();
    let f = fakes();
    let (ui, c) = build(&f);
    set_collection_auth(
        &f,
        Some(Auth::Bearer {
            token: "{{token}}".into(),
        }),
    );
    open_with_env(&ui, &c);
    let s = state(&ui);
    s.invoke_row_clicked(1);

    s.invoke_send();
    assert_eq!(s.get_send_status(), SendStatus::Sending);
    assert!(!s.get_can_send());
    settle(&ui);
    assert_eq!(s.get_send_status(), SendStatus::Done);
    assert_eq!(s.get_status_text(), "200 OK");
    assert_eq!(s.get_status_class(), StatusClass::Ok);
    assert_eq!(s.get_elapsed(), "84 ms");
    assert_eq!(s.get_size(), "21 B");
    assert_eq!(
        s.get_response_pretty(),
        "{\n  \"args\": {\n    \"page\": \"2\"\n  }\n}"
    );
    {
        let sent = f.sender.requests.lock().unwrap();
        assert_eq!(sent[0].url, "https://httpbin.org/get");
        assert_eq!(sent[0].params, vec![KeyValue::new("page", "2")]);
        assert_eq!(
            sent[0].auth,
            Some(Auth::Bearer {
                token: "secret".into()
            }),
            "collection auth inherited and interpolated"
        );
    }
    assert_eq!(
        s.get_url(),
        "{{baseUrl}}/get",
        "form keeps the placeholders"
    );

    *f.sender.reply.lock().unwrap() = Err(AppError::ConnectionRefused("tcp connect error".into()));
    s.invoke_send();
    settle(&ui);
    assert_eq!(s.get_send_status(), SendStatus::Failed);
    assert_eq!(s.get_fail_title(), "Connection refused");
    assert_eq!(
        s.get_fail_hint(),
        "Is the server running? Check the host and port."
    );
    assert_eq!(s.get_fail_detail(), "connection refused: tcp connect error");
    assert!(s.get_can_send());
}
