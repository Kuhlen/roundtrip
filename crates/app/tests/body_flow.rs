mod support;

use std::rc::Rc;

use app::modules::workspace::workspace_controller::WorkspaceController;
use app::ui::{AppWindow, KvRow, KvTable};
use domain::collection::Protocol;
use domain::http::{Body, FormField, KeyValue, Method, Request, TextKind};
use slint::Model;
use support::{Fakes, PICKED, open_with_env, p, set_health, setup, state};

fn post(body: Body) -> Request {
    Request {
        method: Method::Post,
        url: "{{baseUrl}}/post".into(),
        body,
        ..Request::default()
    }
}

/// Collection open, Health check (row 3) holds `body` and is selected.
fn opened_with(body: Body, protocol: Protocol) -> (Fakes, AppWindow, Rc<WorkspaceController>) {
    let (f, ui, c) = setup();
    set_health(&f, post(body), protocol);
    open_with_env(&ui, &c);
    state(&ui).invoke_row_clicked(3);
    (f, ui, c)
}

fn rows(ui: &AppWindow) -> Vec<(String, String, bool)> {
    state(ui)
        .get_form_rows()
        .iter()
        .map(|r| (r.key.to_string(), r.value.to_string(), r.file))
        .collect()
}

fn pick(path: Option<std::path::PathBuf>) {
    PICKED.with(|p| *p.borrow_mut() = path);
}

#[test]
fn form_data_fills_rows_and_saves_edits() {
    let (f, ui, _c) = opened_with(
        Body::FormData(vec![
            FormField::text("name", "Ana"),
            FormField::file("avatar", "img/a.png"),
        ]),
        Protocol::Http,
    );
    let s = state(&ui);
    assert_eq!(s.get_body_kind_index(), 5);
    assert_eq!(s.get_body_count(), 2);
    assert_eq!(
        rows(&ui),
        [
            ("name".to_string(), "Ana".to_string(), false),
            ("avatar".to_string(), "img/a.png".to_string(), true),
            (String::new(), String::new(), false),
        ]
    );
    assert!(!s.get_dirty());
    s.get_form_rows().set_row_data(
        0,
        KvRow {
            value: "Ayu".into(),
            ..s.get_form_rows().row_data(0).unwrap()
        },
    );
    s.invoke_kv_edited(KvTable::Form, 0);
    assert!(s.get_dirty());
    s.invoke_save();
    assert_eq!(
        f.collections.saved.borrow()[0].1.body,
        Body::FormData(vec![
            FormField::text("name", "Ayu"),
            FormField::file("avatar", "img/a.png"),
        ])
    );
}

#[test]
fn urlencoded_opens_clean() {
    let (_f, ui, _c) = opened_with(
        Body::Urlencoded(vec![KeyValue::new("a", "1")]),
        Protocol::Http,
    );
    let s = state(&ui);
    assert_eq!(s.get_body_kind_index(), 4);
    assert_eq!(s.get_body_count(), 1);
    assert!(!s.get_dirty());
    assert!(s.get_can_send());
}

#[test]
fn binary_opens_clean() {
    let (_f, ui, _c) = opened_with(Body::Binary("a.bin".into()), Protocol::Http);
    let s = state(&ui);
    assert_eq!(s.get_body_kind_index(), 6);
    assert_eq!(s.get_binary_path(), "a.bin");
    assert!(!s.get_dirty());
}

#[test]
fn switching_kinds_keeps_each_buffer() {
    let (_f, ui, _c) = opened_with(
        Body::Text {
            kind: TextKind::Json,
            text: "{\"a\":1}".into(),
        },
        Protocol::Http,
    );
    let s = state(&ui);
    s.set_body_kind_index(5);
    s.invoke_changed();
    assert!(s.get_dirty());
    s.set_body_kind_index(1);
    s.invoke_changed();
    assert_eq!(s.get_body(), "{\"a\":1}");
    assert!(!s.get_dirty());
}

#[test]
fn unreadable_form_content_shows_the_note() {
    let (_f, ui, _c) = opened_with(Body::Unsupported("form-data".into()), Protocol::Http);
    let s = state(&ui);
    assert_eq!(
        s.get_body_note(),
        "form-data body: Roundtrip can't read this content. Saving leaves it as it is in the file."
    );
    assert!(!s.get_can_send());
}

#[test]
fn pick_inside_collection_stores_relative_path() {
    let (_f, ui, _c) = opened_with(Body::Binary(String::new()), Protocol::Http);
    pick(Some(p("img/a.png")));
    state(&ui).invoke_pick_file(-1);
    assert_eq!(state(&ui).get_binary_path(), "img/a.png");
    assert!(state(&ui).get_dirty());
}

#[test]
fn pick_outside_collection_keeps_absolute_path() {
    let (_f, ui, _c) = opened_with(Body::Binary(String::new()), Protocol::Http);
    pick(Some("/elsewhere/x.png".into()));
    state(&ui).invoke_pick_file(-1);
    assert_eq!(state(&ui).get_binary_path(), "/elsewhere/x.png");
}

#[test]
fn cancelled_pick_changes_nothing() {
    let (_f, ui, _c) = opened_with(Body::Binary("a.bin".into()), Protocol::Http);
    pick(None);
    state(&ui).invoke_pick_file(-1);
    assert_eq!(state(&ui).get_binary_path(), "a.bin");
    assert!(!state(&ui).get_dirty());
}

#[test]
fn pick_into_the_placeholder_adds_a_file_row() {
    let (_f, ui, _c) = opened_with(Body::FormData(vec![]), Protocol::Http);
    pick(Some(p("a.png")));
    state(&ui).invoke_pick_file(0);
    assert_eq!(
        rows(&ui),
        [
            (String::new(), "a.png".to_string(), true),
            (String::new(), String::new(), false),
        ]
    );
}

fn graphql(text: &str) -> Body {
    Body::Text {
        kind: TextKind::Json,
        text: text.into(),
    }
}

#[test]
fn graphql_opens_clean_in_query_tab() {
    let (_f, ui, _c) = opened_with(
        graphql(r#"{"query":"{ me }","variables":{"id":1}}"#),
        Protocol::Graphql,
    );
    let s = state(&ui);
    assert!(s.get_graphql());
    assert_eq!(s.get_gql_query(), "{ me }");
    assert_eq!(s.get_gql_variables(), "{\n  \"id\": 1\n}");
    assert_eq!(s.get_gql_operation(), "");
    assert!(!s.get_dirty());
    assert!(s.get_can_send());
}

#[test]
fn invalid_variables_block_send_and_save() {
    let (f, ui, _c) = opened_with(graphql(r#"{"query":"{ me }"}"#), Protocol::Graphql);
    let s = state(&ui);
    s.set_gql_variables("{\"id\": }".into());
    s.invoke_changed();
    assert!(s.get_body_error().starts_with("Variables: invalid JSON"));
    assert!(!s.get_can_send());
    assert!(s.get_dirty());
    s.invoke_save();
    assert!(f.collections.saved.borrow().is_empty());
    assert_eq!(s.get_banner_title(), "Variables: invalid JSON");
}

#[test]
fn graphql_save_writes_upstream_json() {
    let (f, ui, _c) = opened_with(
        graphql(r#"{"query":"{ me }","variables":{"id":1}}"#),
        Protocol::Graphql,
    );
    let s = state(&ui);
    s.set_gql_operation("Me".into());
    s.invoke_changed();
    s.invoke_save();
    assert_eq!(
        f.collections.saved.borrow()[0].1.body,
        graphql(
            "{\n  \"query\": \"{ me }\",\n  \"variables\": {\n    \"id\": 1\n  },\n  \"operationName\": \"Me\"\n}"
        )
    );
}

#[test]
fn graphql_protocol_without_query_uses_the_body_tab() {
    let (_f, ui, _c) = opened_with(graphql(r#"{"a":1}"#), Protocol::Graphql);
    let s = state(&ui);
    assert!(!s.get_graphql());
    assert_eq!(s.get_body_kind_index(), 1);
}

#[test]
fn graphql_protocol_with_json_query_object_keeps_the_body() {
    let text = r#"{"query":{"match_all":{}},"size":10}"#;
    let (_f, ui, _c) = opened_with(graphql(text), Protocol::Graphql);
    let s = state(&ui);
    assert!(!s.get_graphql());
    assert_eq!(s.get_body_kind_index(), 1);
    assert_eq!(s.get_body(), text);
    assert!(!s.get_dirty());
}
