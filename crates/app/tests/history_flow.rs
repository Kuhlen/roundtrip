mod support;

use std::time::{Duration, SystemTime};

use app::ui::{DialogKind, SendStatus, StatusClass};
use domain::AppError;
use domain::history::HistoryEntry;
use domain::http::{Body, KeyValue, Method, Request, TextKind};
use slint::Model;
use support::{build, build_with, fakes, history_entry, open_with_env, row_of, state};

fn with_entries(f: &support::Fakes, items: Vec<(HistoryEntry, Request)>) {
    let mut entries = f.history.entries.lock().expect("history lock");
    for (i, (e, r)) in items.into_iter().enumerate() {
        entries.push((
            HistoryEntry {
                id: i as i64 + 1,
                ..e
            },
            r,
        ));
    }
}

#[test]
fn list_shows_newest_first() {
    i_slint_backend_testing::init_no_event_loop();
    let f = fakes();
    let old = HistoryEntry {
        at: SystemTime::now() - Duration::from_secs(300),
        ..history_entry(Method::Get, "https://a/get", Some(200))
    };
    with_entries(
        &f,
        vec![
            (old, Request::default()),
            (
                history_entry(Method::Post, "https://bad", None),
                Request::default(),
            ),
        ],
    );
    let (ui, _c) = build(&f);
    let rows: Vec<_> = state(&ui).get_history().iter().collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].url, "https://bad");
    assert_eq!(rows[0].status, "—");
    assert!(rows[0].failed);
    assert_eq!(rows[1].method, "GET");
    assert_eq!(rows[1].status, "200");
    assert_eq!(rows[1].status_class, StatusClass::Ok);
    assert!(!rows[1].failed);
    assert_eq!(rows[1].time, "5m ago");
    assert_eq!(state(&ui).get_history_note(), "");
}

#[test]
fn search_filters_and_notes() {
    i_slint_backend_testing::init_no_event_loop();
    let f = fakes();
    let (ui, _c) = build(&f);
    let s = state(&ui);
    assert_eq!(s.get_history_note(), "No history yet");
    with_entries(
        &f,
        vec![
            (
                history_entry(Method::Get, "https://a/users", Some(200)),
                Request::default(),
            ),
            (
                history_entry(Method::Post, "https://a/orders", Some(201)),
                Request::default(),
            ),
        ],
    );
    s.set_history_query("post".into());
    s.invoke_history_refresh();
    assert_eq!(s.get_history().row_count(), 1);
    s.set_history_query("zzz".into());
    s.invoke_history_refresh();
    assert_eq!(s.get_history().row_count(), 0);
    assert_eq!(s.get_history_note(), "No matches");
    s.set_history_query("".into());
    s.invoke_history_refresh();
    assert_eq!(s.get_history().row_count(), 2);
}

#[test]
fn clicking_an_entry_opens_a_clean_untitled_tab() {
    i_slint_backend_testing::init_no_event_loop();
    let f = fakes();
    let request = Request {
        method: Method::Post,
        url: "{{baseUrl}}/post".into(),
        headers: vec![KeyValue::new("X-Trace", "1")],
        body: Body::Text {
            kind: TextKind::Json,
            text: r#"{"a":1}"#.into(),
        },
        ..Request::default()
    };
    with_entries(
        &f,
        vec![(
            history_entry(Method::Post, "https://x/post", Some(200)),
            request,
        )],
    );
    let (ui, _c) = build(&f);
    let s = state(&ui);
    s.invoke_history_clicked(0);
    let tabs: Vec<_> = s.get_tabs().iter().collect();
    assert_eq!(tabs.len(), 1);
    assert!(tabs[0].untitled);
    assert!(!tabs[0].dirty);
    assert_eq!(s.get_url(), "{{baseUrl}}/post");
    assert_eq!(s.get_method_index(), 1);
    assert_eq!(s.get_body(), r#"{"a":1}"#);
    assert!(!s.get_dirty());
}

#[test]
fn clear_all_asks_first() {
    i_slint_backend_testing::init_no_event_loop();
    let f = fakes();
    with_entries(
        &f,
        vec![(
            history_entry(Method::Get, "https://a", Some(200)),
            Request::default(),
        )],
    );
    let (ui, _c) = build(&f);
    let s = state(&ui);
    s.invoke_history_clear();
    assert!(s.get_confirm_open());
    assert_eq!(s.get_dialog_kind(), DialogKind::ClearHistory);
    s.invoke_confirm_cancel();
    assert_eq!(s.get_history().row_count(), 1);
    s.invoke_history_clear();
    s.invoke_confirm_clear_history();
    assert!(!s.get_confirm_open());
    assert_eq!(s.get_history().row_count(), 0);
    assert!(f.history.entries.lock().unwrap().is_empty());
}

#[test]
fn unreadable_entry_shows_a_banner() {
    i_slint_backend_testing::init_no_event_loop();
    let f = fakes();
    with_entries(
        &f,
        vec![(
            history_entry(Method::Get, "https://a", Some(200)),
            Request::default(),
        )],
    );
    let (ui, _c) = build(&f);
    *f.history.fail.lock().unwrap() = Some(AppError::History("disk I/O error".into()));
    state(&ui).invoke_history_clicked(0);
    assert_eq!(state(&ui).get_banner_title(), "History error");
    assert_eq!(state(&ui).get_tabs().row_count(), 0);
}

#[test]
fn history_off_shows_why_and_send_still_works() {
    i_slint_backend_testing::init_no_event_loop();
    let f = fakes();
    let (ui, c) = build_with(&f, Err(AppError::History("disk full".into())));
    let s = state(&ui);
    assert_eq!(s.get_history_note(), "History is unavailable: disk full");
    open_with_env(&ui, &c);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    s.invoke_send();
    assert_eq!(s.get_send_status(), SendStatus::Sending);
}
