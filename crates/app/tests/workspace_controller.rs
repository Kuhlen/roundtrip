mod support;

use std::path::{Path, PathBuf};

use app::ui::{KvRow, KvTable, SendStatus};
use domain::AppError;
use domain::http::{BodyKind, KeyValue};
use domain::session::Session;
use slint::{CloseRequestResponse, Model};
use support::{ROOT, build, fakes, kv, opened, p, setup, state, strings, tree_labels, tree_names};

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
    assert_eq!(s.get_collection_name(), "httpbin-demo");
    assert_eq!(
        tree_names(&ui),
        [
            "users",
            "List users",
            "Create user",
            "Health check",
            "Echo socket",
            "Upload avatar"
        ]
    );
    assert_eq!(tree_labels(&ui), ["", "GET", "POST", "GET", "WS", "POST"]);
    assert_eq!(
        strings(s.get_environments()),
        ["No environment", "dev", "mine (personal)"]
    );
    assert_eq!(
        f.session.saves.borrow().last().cloned(),
        Some(Session {
            last_collection: Some(PathBuf::from(ROOT)),
            last_environment: Some("dev".into())
        })
    );
}

#[test]
fn not_a_collection_shows_banner_and_keeps_previous() {
    let (_f, ui, c) = opened();
    c.open_collection(Path::new("/elsewhere"));
    let s = state(&ui);
    assert_eq!(s.get_banner_title(), "Not an ApiArk collection");
    assert_eq!(s.get_collection_name(), "httpbin-demo");
    s.invoke_dismiss_banner();
    assert_eq!(s.get_banner_title(), "");
}

#[test]
fn folder_click_toggles_children() {
    let (_f, ui, _c) = opened();
    state(&ui).invoke_row_clicked(0);
    assert_eq!(
        tree_names(&ui),
        ["users", "Health check", "Echo socket", "Upload avatar"]
    );
    assert!(!state(&ui).get_tree().row_data(0).unwrap().expanded);
    state(&ui).invoke_row_clicked(0);
    assert_eq!(tree_names(&ui).len(), 6);
}

#[test]
fn select_fills_form_and_resolved_url() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(1);
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
    assert!(s.get_tree().row_data(1).unwrap().active);
    assert!(s.get_can_send());
    assert!(!s.get_dirty());
}

#[test]
fn edit_marks_dirty_and_undo_clears_it() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(1);
    s.set_url("{{baseUrl}}/anything".into());
    s.invoke_changed();
    assert!(s.get_dirty());
    assert!(s.get_tree().row_data(1).unwrap().dirty);
    assert_eq!(s.get_resolved_url(), "https://httpbin.org/anything?page=2");
    s.set_url("{{baseUrl}}/get".into());
    s.invoke_changed();
    assert!(!s.get_dirty());
    assert!(!s.get_tree().row_data(1).unwrap().dirty);
}

#[test]
fn switch_while_dirty_asks_and_cancel_changes_nothing() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(1);
    s.set_url("edited".into());
    s.invoke_changed();
    s.invoke_row_clicked(2);
    assert!(s.get_confirm_open());
    assert_eq!(s.get_confirm_name(), "List users");
    assert_eq!(s.get_confirm_file(), "users/list-users.yaml");
    s.invoke_confirm_cancel();
    assert!(!s.get_confirm_open());
    assert_eq!(s.get_request_name(), "List users");
    assert_eq!(s.get_url(), "edited");
    assert!(s.get_dirty());
    assert!(f.collections.saved.borrow().is_empty());
}

#[test]
fn discard_switches_without_saving() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(1);
    s.set_url("edited".into());
    s.invoke_changed();
    s.invoke_row_clicked(2);
    s.invoke_confirm_discard();
    assert_eq!(s.get_request_name(), "Create user");
    assert_eq!(s.get_url(), "{{baseUrl}}/post");
    assert!(!s.get_dirty());
    assert!(f.collections.saved.borrow().is_empty());
}

#[test]
fn confirm_save_saves_then_switches() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(1);
    s.set_url("edited".into());
    s.invoke_changed();
    s.invoke_row_clicked(2);
    s.invoke_confirm_save();
    let saved = f.collections.saved.borrow();
    assert_eq!(saved[0].0, p("users/list-users.yaml"));
    assert_eq!(saved[0].1.url, "edited");
    assert_eq!(s.get_request_name(), "Create user");
}

#[test]
fn save_writes_the_form_and_clears_dirty() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(1);
    s.get_headers().set_row_data(
        0,
        KvRow {
            enabled: true,
            key: "X-Debug".into(),
            value: "1".into(),
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
fn failed_save_stays_dirty_shows_banner_and_cancels_switch() {
    let (f, ui, _c) = opened();
    *f.collections.save_error.borrow_mut() = Some(AppError::Storage("disk full".into()));
    let s = state(&ui);
    s.invoke_row_clicked(1);
    s.set_url("edited".into());
    s.invoke_changed();
    s.invoke_row_clicked(2);
    s.invoke_confirm_save();
    assert_eq!(s.get_banner_title(), "Could not read or write a file");
    assert_eq!(s.get_banner_hint(), "disk full");
    assert_eq!(s.get_request_name(), "List users");
    assert!(s.get_dirty());
    assert!(!s.get_confirm_open());
}

#[test]
fn typing_into_placeholder_appends_row_and_remove_drops_it() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(1);
    s.get_params().set_row_data(
        1,
        KvRow {
            enabled: true,
            key: "limit".into(),
            value: "".into(),
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
    s.invoke_row_clicked(1);
    s.get_params().set_row_data(
        0,
        KvRow {
            enabled: false,
            key: "page".into(),
            value: "2".into(),
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
    s.invoke_row_clicked(5);
    assert_eq!(s.get_unsupported_body(), "form-data");
    assert!(!s.get_can_send());
    s.invoke_row_clicked(4);
    assert_eq!(s.get_unsupported_protocol(), "WebSocket");
    assert!(!s.get_can_send());
}

#[test]
fn saving_unsupported_body_keeps_it() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(5);
    s.set_url("{{baseUrl}}/anything".into());
    s.invoke_changed();
    s.invoke_save();
    assert_eq!(
        f.collections.saved.borrow()[0].1.body_kind,
        BodyKind::Unsupported("form-data".into())
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
    s.invoke_row_clicked(1);
    s.invoke_row_clicked(3);
    assert_eq!(s.get_banner_title(), "Merge conflict in health.yaml");
    assert_eq!(s.get_request_name(), "List users");
}

#[test]
fn environment_change_is_saved_and_updates_resolved_url() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(1);
    s.set_environment_index(0);
    s.invoke_environment_selected();
    assert_eq!(s.get_resolved_url(), "{{baseUrl}}/get?page=2");
    assert_eq!(
        f.session.saves.borrow().last().cloned(),
        Some(Session {
            last_collection: Some(PathBuf::from(ROOT)),
            last_environment: None
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
    s.invoke_row_clicked(1);
    assert_eq!(s.get_resolved_url(), "{{baseUrl}}/get?page=2");
    assert!(s.get_can_send());
}

#[test]
fn restore_reopens_last_collection_and_environment() {
    i_slint_backend_testing::init_no_event_loop();
    let f = fakes();
    *f.session.session.borrow_mut() = Session {
        last_collection: Some(PathBuf::from(ROOT)),
        last_environment: Some("mine".into()),
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
        last_collection: Some(PathBuf::from("/gone")),
        last_environment: None,
    };
    let (ui, c) = build(&f);
    c.restore();
    assert!(!state(&ui).get_has_collection());
    assert_eq!(state(&ui).get_banner_title(), "");
}

#[test]
fn close_while_dirty_keeps_window_and_asks() {
    let (_f, ui, c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(1);
    assert!(matches!(
        c.close_requested(),
        CloseRequestResponse::HideWindow
    ));
    s.set_url("edited".into());
    s.invoke_changed();
    assert!(matches!(
        c.close_requested(),
        CloseRequestResponse::KeepWindowShown
    ));
    assert!(s.get_confirm_open());
}

#[test]
fn reopening_mid_send_keeps_send_disabled() {
    let (_f, ui, c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(1);
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
    s.invoke_row_clicked(1);
    s.invoke_save();
    assert!(f.collections.saved.borrow().is_empty());
}

#[test]
fn saved_method_edit_updates_the_sidebar_label() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(2);
    assert_eq!(tree_labels(&ui)[2], "POST");
    s.set_method_index(0);
    s.invoke_changed();
    s.invoke_save();
    assert_eq!(tree_labels(&ui)[2], "GET");
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
fn opening_another_collection_clears_the_old_response() {
    let (_f, ui, c) = opened();
    let s = state(&ui);
    s.set_status_text("200 OK".into());
    s.set_response_raw("old".into());
    c.open_collection(Path::new(ROOT));
    assert_eq!(s.get_status_text(), "");
    assert_eq!(s.get_response_raw(), "");
}
