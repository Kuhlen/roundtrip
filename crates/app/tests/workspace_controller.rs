mod support;

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use app::ui::{KvRow, KvTable, SendStatus};
use domain::AppError;
use domain::http::{Body, KeyValue};
use domain::session::Session;
use slint::{CloseRequestResponse, ComponentHandle, Model};
use support::{
    Fakes, ROOT, build, fakes, kv, opened, p, row_of, setup, state, strings, tree_labels,
    tree_names,
};

#[test]
fn starts_empty() {
    let (_f, ui, _c) = setup();
    let s = state(&ui);
    assert!(!s.get_has_collection());
    assert!(!s.get_has_request());
    assert!(!s.get_can_send());
    assert_eq!(s.get_send_status(), SendStatus::Idle);
    assert_eq!(strings(s.get_environments()), ["No environment"]);
}

#[test]
fn open_shows_tree_and_environments() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    assert!(s.get_has_collection());
    assert_eq!(
        tree_names(&ui),
        [
            "httpbin-demo",
            "users",
            "List users",
            "Create user",
            "Health check",
            "Echo socket",
            "Upload avatar"
        ]
    );
    assert_eq!(
        tree_labels(&ui),
        ["", "", "GET", "POST", "GET", "WS", "POST"]
    );
    assert_eq!(
        strings(s.get_environments()),
        ["No environment", "dev", "mine (personal)"]
    );
    assert_eq!(
        f.session.saves.borrow().last().cloned(),
        Some(Session {
            collections: vec![PathBuf::from(ROOT)],
            environment: Some("dev".into()),
            ..Session::default()
        })
    );
}

#[test]
fn not_a_collection_shows_banner_and_keeps_previous() {
    let (_f, ui, c) = opened();
    c.open_collection(Path::new("/elsewhere"));
    let s = state(&ui);
    assert_eq!(s.get_banner_title(), "Not an ApiArk collection");
    assert_eq!(tree_names(&ui)[0], "httpbin-demo");
    s.invoke_dismiss_banner();
    assert_eq!(s.get_banner_title(), "");
}

#[test]
fn folder_click_toggles_children() {
    let (_f, ui, _c) = opened();
    state(&ui).invoke_row_clicked(row_of(&ui, "users"));
    assert_eq!(
        tree_names(&ui),
        [
            "httpbin-demo",
            "users",
            "Health check",
            "Echo socket",
            "Upload avatar"
        ]
    );
    assert!(!state(&ui).get_tree().row_data(1).unwrap().expanded);
    state(&ui).invoke_row_clicked(row_of(&ui, "users"));
    assert_eq!(tree_names(&ui).len(), 7);
}

#[test]
fn select_fills_form_and_resolved_url() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    assert!(s.get_has_request());
    assert_eq!(s.get_request_name(), "List users");
    assert_eq!(s.get_crumb(), "users /");
    assert_eq!(s.get_request_file(), "users/list-users.yaml");
    assert_eq!(s.get_url(), "{{baseUrl}}/get");
    assert_eq!(
        kv(&s.get_params()),
        [
            ("page".to_string(), "2".to_string(), true),
            (String::new(), String::new(), true)
        ]
    );
    assert_eq!(s.get_param_count(), 1);
    assert_eq!(s.get_resolved_url(), "https://httpbin.org/get?page=2");
    assert!(s.get_tree().row_data(2).unwrap().active);
    assert!(s.get_can_send());
    assert!(!s.get_dirty());
}

#[test]
fn edit_marks_dirty_and_undo_clears_it() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    s.set_url("{{baseUrl}}/anything".into());
    s.invoke_changed();
    assert!(s.get_dirty());
    assert!(s.get_tree().row_data(2).unwrap().dirty);
    assert_eq!(s.get_resolved_url(), "https://httpbin.org/anything?page=2");
    s.set_url("{{baseUrl}}/get".into());
    s.invoke_changed();
    assert!(!s.get_dirty());
    assert!(!s.get_tree().row_data(2).unwrap().dirty);
}

#[test]
fn save_writes_the_form_and_clears_dirty() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    s.get_headers().set_row_data(
        0,
        KvRow {
            enabled: true,
            key: "X-Debug".into(),
            value: "1".into(),
            file: false,
        },
    );
    s.invoke_kv_edited(KvTable::Headers, 0);
    assert_eq!(s.get_headers().row_count(), 2, "placeholder appended");
    assert_eq!(s.get_header_count(), 1);
    assert!(s.get_dirty());
    s.invoke_save();
    assert_eq!(
        f.collections.saved.borrow()[0].1.headers,
        vec![KeyValue::new("X-Debug", "1")]
    );
    assert!(!s.get_dirty());
}

#[test]
fn failed_save_on_switch_shows_banner_and_still_opens_the_new_request() {
    let (f, ui, _c) = opened();
    *f.collections.save_error.borrow_mut() = Some(AppError::Storage("disk full".into()));
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    s.set_url("edited".into());
    s.invoke_changed();
    s.invoke_row_clicked(row_of(&ui, "Create user"));
    assert_eq!(s.get_banner_title(), "Could not read or write a file");
    assert_eq!(s.get_banner_hint(), "disk full");
    assert_eq!(s.get_request_name(), "Create user");
    assert!(f.collections.saved.borrow().is_empty());
    assert!(!s.get_confirm_open());
}

#[test]
fn typing_into_placeholder_appends_row_and_remove_drops_it() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    s.get_params().set_row_data(
        1,
        KvRow {
            enabled: true,
            key: "limit".into(),
            value: "".into(),
            file: false,
        },
    );
    s.invoke_kv_edited(KvTable::Params, 1);
    assert_eq!(s.get_params().row_count(), 3);
    assert_eq!(s.get_param_count(), 2);
    s.invoke_kv_removed(KvTable::Params, 0);
    let keys: Vec<String> = kv(&s.get_params()).into_iter().map(|r| r.0).collect();
    assert_eq!(keys, ["limit", ""]);
    s.invoke_kv_removed(KvTable::Params, 1);
    assert_eq!(
        s.get_params().row_count(),
        2,
        "placeholder cannot be removed"
    );
}

#[test]
fn disabled_param_is_not_counted_or_resolved() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    s.get_params().set_row_data(
        0,
        KvRow {
            enabled: false,
            key: "page".into(),
            value: "2".into(),
            file: false,
        },
    );
    s.invoke_kv_edited(KvTable::Params, 0);
    assert_eq!(s.get_param_count(), 0);
    assert_eq!(s.get_resolved_url(), "https://httpbin.org/get");
    assert!(s.get_dirty());
}

#[test]
fn unsupported_body_and_protocol_disable_send() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "Upload avatar"));
    assert_eq!(s.get_unsupported_body(), "form-data");
    assert!(!s.get_can_send());
    s.invoke_row_clicked(row_of(&ui, "Echo socket"));
    assert_eq!(s.get_unsupported_protocol(), "WebSocket");
    assert!(!s.get_can_send());
}

#[test]
fn saving_unsupported_body_keeps_it() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "Upload avatar"));
    s.set_url("{{baseUrl}}/anything".into());
    s.invoke_changed();
    s.invoke_save();
    assert_eq!(
        f.collections.saved.borrow()[0].1.body,
        Body::Unsupported("form-data".into())
    );
}

#[test]
fn unreadable_request_shows_banner_and_keeps_form() {
    let (f, ui, _c) = opened();
    let conflict = AppError::MergeConflict(p("health.yaml").display().to_string());
    f.collections
        .files
        .borrow_mut()
        .insert(p("health.yaml"), Err(conflict));
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    s.invoke_row_clicked(row_of(&ui, "Health check"));
    assert_eq!(s.get_banner_title(), "Merge conflict in health.yaml");
    assert_eq!(s.get_request_name(), "List users");
}

#[test]
fn environment_change_is_saved_and_updates_resolved_url() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    s.set_environment_index(0);
    s.invoke_environment_selected();
    assert_eq!(s.get_resolved_url(), "{{baseUrl}}/get?page=2");
    assert_eq!(
        f.session.saves.borrow().last().cloned(),
        Some(Session {
            collections: vec![PathBuf::from(ROOT)],
            environment: None,
            tabs: vec![p("users/list-users.yaml")],
            active_tab: Some(0),
        })
    );
}

#[test]
fn broken_environment_shows_banner_and_still_sends() {
    let (f, ui, c) = setup();
    *f.environments.list_error.borrow_mut() =
        Some(AppError::Storage("invalid environment YAML".into()));
    c.open_collection(Path::new(ROOT));
    let s = state(&ui);
    assert_eq!(s.get_banner_title(), "Could not read or write a file");
    assert_eq!(strings(s.get_environments()), ["No environment"]);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    assert_eq!(s.get_resolved_url(), "{{baseUrl}}/get?page=2");
    assert!(s.get_can_send());
}

#[test]
fn restore_reopens_last_collection_and_environment() {
    i_slint_backend_testing::init_no_event_loop();
    let f = fakes();
    *f.session.session.borrow_mut() = Session {
        collections: vec![PathBuf::from(ROOT)],
        environment: Some("mine".into()),
        ..Session::default()
    };
    let (ui, c) = build(&f);
    c.restore();
    assert!(state(&ui).get_has_collection());
    assert_eq!(state(&ui).get_environment_index(), 2);
}

#[test]
fn restore_with_missing_collection_starts_empty_without_banner() {
    i_slint_backend_testing::init_no_event_loop();
    let f = fakes();
    *f.session.session.borrow_mut() = Session {
        collections: vec![PathBuf::from("/gone")],
        environment: None,
        ..Session::default()
    };
    let (ui, c) = build(&f);
    c.restore();
    assert!(!state(&ui).get_has_collection());
    assert_eq!(state(&ui).get_banner_title(), "");
}

#[test]
fn close_while_dirty_saves_and_hides() {
    let (f, ui, c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    s.set_url("edited".into());
    s.invoke_changed();
    assert!(matches!(
        c.close_requested(),
        CloseRequestResponse::HideWindow
    ));
    assert_eq!(f.collections.saved.borrow()[0].1.url, "edited");
    assert!(!s.get_confirm_open());
}

#[test]
fn reopening_mid_send_keeps_send_disabled() {
    let (_f, ui, c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    s.invoke_send();
    assert_eq!(s.get_send_status(), SendStatus::Sending);
    c.open_collection(Path::new(ROOT));
    assert_eq!(s.get_send_status(), SendStatus::Sending);
    assert!(!s.get_can_send());
}

#[test]
fn save_without_edits_does_not_touch_the_file() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    s.invoke_save();
    assert!(f.collections.saved.borrow().is_empty());
}

#[test]
fn saved_method_edit_updates_the_sidebar_label() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "Create user"));
    assert_eq!(tree_labels(&ui)[3], "POST");
    s.set_method_index(0);
    s.invoke_changed();
    s.invoke_save();
    assert_eq!(tree_labels(&ui)[3], "GET");
}

#[test]
fn opening_another_collection_clears_the_old_banner() {
    let (_f, ui, c) = opened();
    c.open_collection(Path::new("/elsewhere"));
    assert_ne!(state(&ui).get_banner_title(), "");
    c.open_collection(Path::new(ROOT));
    assert_eq!(state(&ui).get_banner_title(), "");
}

#[test]
fn opening_another_collection_keeps_the_open_request_and_response() {
    let (_f, ui, c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    s.set_status_text("200 OK".into());
    s.set_response_raw("old".into());
    c.open_collection(Path::new(support::ORDERS));
    assert_eq!(s.get_request_name(), "List users");
    assert_eq!(s.get_status_text(), "200 OK");
    assert_eq!(s.get_response_raw(), "old");
}

#[test]
fn escape_cancels_the_confirm_dialog() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_delete_row(row_of(&ui, "List users"));
    assert!(s.get_confirm_open());
    let escape: slint::SharedString = slint::platform::Key::Escape.into();
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed {
            text: escape.clone(),
        });
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyReleased { text: escape });
    assert!(!s.get_confirm_open());
    assert!(f.collections.deleted.borrow().is_empty());
}

#[test]
fn cancel_shows_cancelled_and_reenables_send() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    f.sender.hold.store(true, Ordering::SeqCst);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    s.invoke_send();
    assert_eq!(s.get_send_status(), SendStatus::Sending);
    s.invoke_cancel();
    assert_eq!(s.get_send_status(), SendStatus::Cancelled);
    assert!(s.get_can_send());
    let flags = wait_for_flags(&f, 1);
    assert!(flags[0].is_cancelled());
    f.sender.hold.store(false, Ordering::SeqCst);
}

#[test]
fn escape_cancels_after_a_mouse_click_on_send() {
    use slint::platform::{Key, PointerEventButton, WindowEvent};
    let (f, ui, _c) = opened();
    ui.window().set_size(slint::LogicalSize::new(1180.0, 700.0));
    let s = state(&ui);
    f.sender.hold.store(true, Ordering::SeqCst);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    // Send sits at the right end of the url row, 14 px padding
    let position = slint::LogicalPosition::new(1180.0 - 14.0 - 20.0, 90.0);
    let button = PointerEventButton::Left;
    ui.window()
        .dispatch_event(WindowEvent::PointerPressed { position, button });
    ui.window()
        .dispatch_event(WindowEvent::PointerReleased { position, button });
    assert_eq!(s.get_send_status(), SendStatus::Sending, "click hit Send");
    let escape: slint::SharedString = Key::Escape.into();
    ui.window().dispatch_event(WindowEvent::KeyPressed {
        text: escape.clone(),
    });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text: escape });
    assert_eq!(
        s.get_send_status(),
        SendStatus::Cancelled,
        "focus survived the click"
    );
    wait_for_flags(&f, 1);
    f.sender.hold.store(false, Ordering::SeqCst);
}

#[test]
fn closing_a_sending_tab_cancels_it() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    f.sender.hold.store(true, Ordering::SeqCst);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    s.invoke_send();
    s.invoke_tab_close(0);
    let flags = wait_for_flags(&f, 1);
    assert!(flags[0].is_cancelled());
    f.sender.hold.store(false, Ordering::SeqCst);
}

#[test]
fn cancel_without_a_send_does_nothing() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    s.invoke_cancel();
    assert_eq!(s.get_send_status(), SendStatus::Idle);
}

/// Worker threads start asynchronously; wait until `n` sends reached the fake.
fn wait_for_flags(f: &Fakes, n: usize) -> Vec<domain::http::CancelFlag> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let flags = f.sender.flags.lock().expect("flags").clone();
        if flags.len() >= n || Instant::now() > deadline {
            return flags;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
