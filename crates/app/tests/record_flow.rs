//! Own binary: the event-loop testing backend can be set once per process.
mod support;

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use app::ui::{SendStatus, WorkspaceState};
use domain::AppError;
use domain::auth::Auth;
use domain::collection::Protocol;
use domain::history::REDACTED;
use domain::http::{Body, KeyValue, Request};
use slint::ComponentHandle;
use support::{
    build, fakes, open_with_env, row_of, run_until, set_collection_auth, set_health, state,
};

#[test]
fn completed_sends_are_recorded_cancelled_ones_are_not() {
    i_slint_backend_testing::init_integration_test_with_system_time();
    let f = fakes();
    set_health(
        &f,
        Request {
            url: "{{baseUrl}}/anything".into(),
            headers: vec![
                KeyValue::new("Authorization", "Bearer abc"),
                KeyValue::new("X-Trace", "1"),
            ],
            body: Body::Binary("files/a.bin".into()),
            ..Request::default()
        },
        Protocol::Http,
    );
    set_collection_auth(
        &f,
        Some(Auth::Basic {
            username: "ayu".into(),
            password: "pw".into(),
        }),
    );
    let (ui, c) = build(&f);
    open_with_env(&ui, &c);
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "Health check"));

    s.invoke_send();
    let w = ui.clone_strong();
    run_until(move || w.global::<WorkspaceState>().get_send_status() != SendStatus::Sending);
    {
        let entries = f.history.entries.lock().unwrap();
        assert_eq!(entries.len(), 1);
        let (entry, request) = &entries[0];
        assert_eq!(entry.url, "https://httpbin.org/anything");
        assert_eq!(entry.status, Some(200));
        assert_eq!(entry.name.as_deref(), Some("Health check"));
        assert_eq!(entry.collection, Some(PathBuf::from("/c")));
        assert_eq!(request.url, "{{baseUrl}}/anything", "variables kept");
        assert_eq!(request.headers[0].value, REDACTED);
        assert_eq!(request.headers[1].value, "1");
        assert_eq!(request.body, Body::Binary("/c/files/a.bin".into()));
        assert_eq!(
            request.auth,
            Some(Auth::Basic {
                username: "ayu".into(),
                password: REDACTED.into()
            }),
            "inherited auth stored in place of Inherit"
        );
    }

    *f.sender.reply.lock().unwrap() = Err(AppError::ConnectionRefused("refused".into()));
    s.invoke_send();
    let w = ui.clone_strong();
    run_until(move || w.global::<WorkspaceState>().get_send_status() != SendStatus::Sending);
    {
        let entries = f.history.entries.lock().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].0.status, None);
    }

    f.sender.hold.store(true, Ordering::SeqCst);
    s.invoke_send();
    s.invoke_cancel();
    let before = s.get_history();
    f.sender.hold.store(false, Ordering::SeqCst);
    // dropped result still reloads the list: a swapped model means it arrived
    let w = ui.clone_strong();
    run_until(move || w.global::<WorkspaceState>().get_history() != before);
    assert_eq!(s.get_send_status(), SendStatus::Cancelled);
    assert_eq!(
        f.history.entries.lock().unwrap().len(),
        2,
        "cancelled send not recorded"
    );
    assert_eq!(
        slint::Model::row_count(&s.get_history()),
        2,
        "list reloads after each delivered result"
    );
}
