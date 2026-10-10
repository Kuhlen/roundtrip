mod support;

use app::ui::{DialogKind, KvRow};
use domain::AppError;
use domain::environment::Scope;
use slint::{CloseRequestResponse, Model};
use support::{opened, row_of, state};

#[test]
fn renaming_the_active_environment_keeps_it_selected() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_open_environments();
    assert!(s.get_envs_open());
    assert_eq!(s.get_env_name(), "dev");
    assert!(!s.get_env_dirty());
    s.set_env_name("staging".into());
    s.invoke_env_changed();
    assert!(s.get_env_dirty());
    s.invoke_env_save();
    let (old, env) = f.environments.saves.borrow()[0].clone();
    assert_eq!(old, Some(("dev".to_string(), Scope::Shared)));
    assert_eq!(env.name, "staging");
    let picked = s
        .get_environments()
        .row_data(s.get_environment_index() as usize)
        .unwrap();
    assert_eq!(picked, "staging");
    let session = f.session.saves.borrow().last().cloned().unwrap();
    assert_eq!(session.environment.as_deref(), Some("staging"));
    assert!(!s.get_env_dirty());
}

#[test]
fn saving_the_active_environment_updates_the_resolved_url() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    s.invoke_open_environments();
    let rows = s.get_env_rows();
    let mut row = rows.row_data(0).unwrap();
    assert_eq!(row.key, "baseUrl");
    row.value = "https://example.test".into();
    rows.set_row_data(0, row);
    s.invoke_env_edited(0);
    s.invoke_env_save();
    assert_eq!(s.get_resolved_url(), "https://example.test/get?page=2");
}

#[test]
fn switching_while_dirty_asks_and_discard_saves_nothing() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_open_environments();
    s.set_env_name("changed".into());
    s.invoke_env_changed();
    s.invoke_env_selected(1);
    assert!(s.get_confirm_open());
    assert_eq!(s.get_dialog_kind(), DialogKind::EnvironmentUnsaved);
    s.invoke_confirm_env_discard();
    assert!(!s.get_confirm_open());
    assert!(f.environments.saves.borrow().is_empty());
    assert_eq!(s.get_env_name(), "mine");
    assert_eq!(s.get_env_scope_index(), 1);
    assert!(!s.get_env_dirty());
}

#[test]
fn prompt_save_then_switches_to_the_clicked_environment() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_open_environments();
    // "zz" sorts after "mine": the list order changes on save
    s.set_env_name("zz".into());
    s.invoke_env_changed();
    s.invoke_env_selected(1);
    s.invoke_confirm_env_save();
    assert_eq!(s.get_env_name(), "mine");
    let names: Vec<String> = s
        .get_env_list()
        .iter()
        .map(|r| r.name.to_string())
        .collect();
    assert_eq!(names, ["mine", "zz"]);
}

#[test]
fn deleting_the_active_environment_selects_no_environment() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_open_environments();
    s.invoke_env_delete();
    assert_eq!(s.get_dialog_kind(), DialogKind::EnvironmentDelete);
    s.invoke_confirm_env_delete();
    assert_eq!(
        *f.environments.deletes.borrow(),
        [("dev".to_string(), Scope::Shared)]
    );
    assert_eq!(s.get_environment_index(), 0);
    assert_eq!(s.get_env_name(), "mine");
}

#[test]
fn new_then_cancel_writes_nothing() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_open_environments();
    s.invoke_env_new();
    assert_eq!(s.get_env_name(), "New environment");
    assert!(s.get_env_dirty());
    assert_eq!(s.get_env_list().row_count(), 3);
    assert!(s.get_env_list().row_data(2).unwrap().draft);
    s.invoke_env_cancel();
    assert_eq!(s.get_env_list().row_count(), 2);
    assert!(f.environments.saves.borrow().is_empty());
}

#[test]
fn duplicate_copies_rows_under_a_new_name() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_open_environments();
    s.invoke_env_duplicate();
    assert_eq!(s.get_env_name(), "dev copy");
    s.invoke_env_save();
    let (old, env) = f.environments.saves.borrow()[0].clone();
    assert_eq!(old, None);
    assert_eq!(env.variables.len(), 2);
    assert!(env.variables[1].secret);
}

#[test]
fn duplicate_key_stays_in_the_dialog() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_open_environments();
    let rows = s.get_env_rows();
    let last = rows.row_count() - 1;
    rows.set_row_data(
        last,
        KvRow {
            enabled: true,
            key: "baseUrl".into(),
            value: "x".into(),
            file: false,
            secret: false,
        },
    );
    s.invoke_env_edited(last as i32);
    s.invoke_env_save();
    assert_eq!(s.get_env_error(), "Variable \"baseUrl\" appears twice");
    assert!(s.get_env_dirty());
    assert!(f.environments.saves.borrow().is_empty());
}

#[test]
fn store_error_stays_in_the_dialog() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    *f.environments.save_error.borrow_mut() = Some(AppError::AlreadyExists("x".into()));
    s.invoke_open_environments();
    s.set_env_name("x".into());
    s.invoke_env_changed();
    s.invoke_env_save();
    assert_eq!(
        s.get_env_error(),
        "An environment named \"x\" already exists"
    );
    assert!(s.get_envs_open());
    assert!(s.get_env_dirty());
}

#[test]
fn window_close_with_unsaved_edits_asks_and_keeps_the_window() {
    let (_f, ui, c) = opened();
    let s = state(&ui);
    s.invoke_open_environments();
    s.set_env_name("changed".into());
    s.invoke_env_changed();
    assert!(matches!(
        c.close_requested(),
        CloseRequestResponse::KeepWindowShown
    ));
    assert!(s.get_confirm_open());
    assert_eq!(s.get_dialog_kind(), DialogKind::EnvironmentUnsaved);
    assert!(s.get_envs_open());
}

#[test]
fn cancel_on_a_draft_returns_to_the_previous_environment() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_open_environments();
    s.invoke_env_selected(1);
    s.invoke_env_new();
    s.invoke_env_cancel();
    assert_eq!(s.get_env_name(), "mine");
    assert_eq!(s.get_env_index(), 1);
}

#[test]
fn same_name_in_both_scopes_are_both_selectable() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    f.environments
        .envs
        .borrow_mut()
        .get_mut(std::path::Path::new(support::ROOT))
        .unwrap()
        .push(domain::environment::Environment {
            name: "dev".into(),
            scope: Scope::Personal,
            variables: vec![],
        });
    s.invoke_open_environments();
    let at = s
        .get_env_list()
        .iter()
        .position(|r| r.personal && r.name == "dev")
        .unwrap() as i32;
    s.invoke_env_selected(at);
    assert_eq!(s.get_env_index(), at);
    assert_eq!(s.get_env_scope_index(), 1);
}
