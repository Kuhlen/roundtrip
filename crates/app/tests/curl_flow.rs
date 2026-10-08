mod support;

use app::ui::WorkspaceState;
use domain::auth::Auth;
use slint::Model;
use support::{kv, opened, row_of, set_collection_auth, setup, state};

fn paste(s: &WorkspaceState, text: &str) {
    s.set_url(text.into());
    s.invoke_url_edited();
}

#[test]
fn paste_fills_the_empty_untitled_tab() {
    let (_f, ui, _c) = setup();
    let s = state(&ui);
    s.invoke_tab_new();
    paste(
        &s,
        r#"curl -X POST 'https://api.test/users?page=2' -H 'Content-Type: application/json' -d '{"a":1}'"#,
    );
    assert_eq!(s.get_tabs().row_count(), 1);
    assert_eq!(s.get_url(), "https://api.test/users");
    assert_eq!(s.get_method_index(), 1, "POST");
    assert_eq!(kv(&s.get_params())[0], ("page".into(), "2".into(), true));
    assert_eq!(s.get_body(), r#"{"a":1}"#);
    assert!(s.get_dirty());
    assert!(s.get_tabs().row_data(0).unwrap().dirty);
    assert_eq!(s.get_banner_title(), "");
}

#[test]
fn paste_into_collection_tab_opens_new_tab() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "Health check"));
    paste(&s, "curl https://pasted.test/x");
    let tabs: Vec<_> = s.get_tabs().iter().collect();
    assert_eq!(tabs.len(), 2);
    assert!(tabs[1].active && tabs[1].untitled && tabs[1].dirty);
    assert_eq!(s.get_url(), "https://pasted.test/x");
    // the request tab never saw the curl text
    assert!(!tabs[0].dirty);
    assert!(f.collections.saved.borrow().is_empty());
    s.invoke_tab_clicked(0);
    assert_eq!(s.get_url(), "{{baseUrl}}/status/200");
}

#[test]
fn typing_curl_by_hand_does_not_parse() {
    let (_f, ui, _c) = setup();
    let s = state(&ui);
    s.invoke_tab_new();
    let text = "curl https://x.test";
    for end in 1..=text.len() {
        paste(&s, &text[..end]);
    }
    assert_eq!(s.get_url(), text);
    assert_eq!(s.get_tabs().row_count(), 1);
    assert_eq!(s.get_banner_title(), "");
}

#[test]
fn unreadable_paste_stays_and_explains() {
    let (_f, ui, _c) = setup();
    let s = state(&ui);
    s.invoke_tab_new();
    paste(&s, "curl 'https://x.test");
    assert_eq!(s.get_url(), "curl 'https://x.test");
    assert_eq!(s.get_tabs().row_count(), 1);
    assert_eq!(s.get_banner_title(), "Couldn't read cURL command");
    assert_eq!(s.get_banner_hint(), "unterminated quote");
}

#[test]
fn skipped_flags_are_named() {
    let (_f, ui, _c) = setup();
    let s = state(&ui);
    s.invoke_tab_new();
    paste(&s, "curl --max-time 5 https://x.test");
    assert_eq!(s.get_url(), "https://x.test");
    assert_eq!(s.get_banner_title(), "Imported from cURL");
    assert_eq!(s.get_banner_hint(), "Skipped: --max-time");
}

#[test]
fn unreadable_paste_leaves_a_file_tab_untouched() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "Health check"));
    paste(&s, "curl 'https://x.test");
    assert_eq!(s.get_url(), "{{baseUrl}}/status/200");
    assert!(!s.get_tabs().row_data(0).unwrap().dirty);
    assert!(f.collections.saved.borrow().is_empty());
    assert_eq!(s.get_banner_title(), "Couldn't read cURL command");
}

#[test]
fn copy_is_interpolated_with_collection_auth() {
    let (f, ui, c) = setup();
    set_collection_auth(
        &f,
        Some(Auth::Bearer {
            token: "{{token}}".into(),
        }),
    );
    support::open_with_env(&ui, &c);
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    let cmd = s.invoke_curl_command().to_string();
    assert_eq!(
        cmd,
        "curl 'https://httpbin.org/get?page=2' \\\n  -H 'Authorization: Bearer secret'"
    );
    assert_eq!(s.get_banner_title(), "");
}

#[test]
fn copy_refuses_what_curl_cannot_say() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "Upload avatar"));
    assert_eq!(s.invoke_curl_command(), "");
    assert_eq!(s.get_banner_title(), "Can't copy as cURL");
    s.invoke_row_clicked(row_of(&ui, "Echo socket"));
    assert_eq!(s.invoke_curl_command(), "");
    assert_eq!(
        s.get_banner_hint(),
        "Only HTTP and GraphQL requests can be copied."
    );
}
