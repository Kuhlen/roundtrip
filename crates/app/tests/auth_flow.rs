mod support;

use app::ui::AuthFields;
use domain::AppError;
use domain::auth::{ApiKeyPlace, Auth};
use domain::http::Request;
use support::{ROOT, open_with_env, p, row_of, set_collection_auth, setup, state};

fn bearer(token: &str) -> AuthFields {
    AuthFields {
        kind: 1,
        token: token.into(),
        ..AuthFields::default()
    }
}

#[test]
fn own_auth_edit_marks_dirty_and_saves() {
    let (f, ui, c) = setup();
    open_with_env(&ui, &c);
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    assert_eq!(s.get_request_auth().kind, 0);
    s.set_request_auth(bearer("abc"));
    s.invoke_changed();
    assert!(s.get_dirty());
    s.invoke_save();
    let saved = f.collections.saved.borrow().last().cloned().unwrap();
    assert_eq!(saved.0, p("users/list-users.yaml"));
    assert_eq!(
        saved.1.auth,
        Some(Auth::Bearer {
            token: "abc".into()
        })
    );
    assert!(!s.get_dirty());
}

#[test]
fn inherit_shows_the_collection_auth() {
    let (f, ui, c) = setup();
    set_collection_auth(
        &f,
        Some(Auth::Bearer {
            token: "{{token}}".into(),
        }),
    );
    open_with_env(&ui, &c);
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "Health check"));
    assert_eq!(s.get_inherited_auth(), "Uses the collection auth: Bearer.");
    assert!(s.get_can_send());
}

#[test]
fn unsupported_collection_auth_blocks_inheriting_requests() {
    let (f, ui, c) = setup();
    set_collection_auth(&f, Some(Auth::Unsupported("oauth2".into())));
    open_with_env(&ui, &c);
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "Health check"));
    assert!(!s.get_can_send());
    assert_eq!(
        s.get_inherited_auth(),
        "Uses the collection auth: oauth2, not supported yet."
    );
    s.set_request_auth(bearer("own"));
    s.invoke_changed();
    assert!(s.get_can_send(), "own auth overrides the collection's");
}

#[test]
fn own_unsupported_auth_blocks_send_and_survives_save() {
    let (f, ui, c) = setup();
    f.collections.files.borrow_mut().insert(
        p("health.yaml"),
        Ok(Request {
            url: "http://h".into(),
            auth: Some(Auth::Unsupported("digest".into())),
            ..Request::default()
        }),
    );
    open_with_env(&ui, &c);
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "Health check"));
    assert_eq!(s.get_request_auth().unsupported, "digest");
    assert!(!s.get_can_send());
    s.set_url("http://h2".into());
    s.invoke_changed();
    s.invoke_save();
    let saved = f.collections.saved.borrow().last().cloned().unwrap();
    assert_eq!(saved.1.auth, Some(Auth::Unsupported("digest".into())));
}

#[test]
fn settings_show_and_save_the_collection_auth() {
    let (f, ui, c) = setup();
    set_collection_auth(
        &f,
        Some(Auth::ApiKey {
            key: "k".into(),
            value: "v".into(),
            place: ApiKeyPlace::Query,
        }),
    );
    open_with_env(&ui, &c);
    let s = state(&ui);
    s.invoke_collection_settings(row_of(&ui, "httpbin-demo"));
    assert!(s.get_settings_open());
    let shown = s.get_settings_auth();
    assert_eq!((shown.kind, shown.place), (3, 1));
    assert_eq!(shown.key, "k");

    s.set_settings_auth(bearer("t"));
    s.invoke_save_settings();
    assert!(!s.get_settings_open());
    assert_eq!(
        f.collections.auth_saves.borrow().last().cloned(),
        Some((ROOT.into(), Some(Auth::Bearer { token: "t".into() })))
    );
    s.invoke_row_clicked(row_of(&ui, "Health check"));
    assert_eq!(s.get_inherited_auth(), "Uses the collection auth: Bearer.");
}

#[test]
fn failed_settings_save_keeps_the_dialog_open() {
    let (f, ui, c) = setup();
    open_with_env(&ui, &c);
    *f.collections.edit_error.borrow_mut() = Some(AppError::Storage("disk full".into()));
    let s = state(&ui);
    // Auth tab link: the active request's collection
    s.invoke_row_clicked(row_of(&ui, "Health check"));
    s.invoke_open_settings();
    s.set_settings_auth(bearer("t"));
    s.invoke_save_settings();
    assert!(s.get_settings_open());
    assert_eq!(s.get_banner_title(), "Could not read or write a file");
    s.invoke_close_settings();
    assert!(!s.get_settings_open());
}
