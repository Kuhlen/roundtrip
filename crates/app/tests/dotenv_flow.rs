mod support;

use app::ui::{DialogKind, KvRow};
use slint::Model;
use support::{opened, row_of, state};

fn set_row(s: &app::ui::WorkspaceState, i: usize, key: &str, value: &str) {
    let rows = s.get_env_rows();
    rows.set_row_data(
        i,
        KvRow {
            enabled: true,
            key: key.into(),
            value: value.into(),
            file: false,
            secret: false,
        },
    );
    s.invoke_env_edited(i as i32);
}

#[test]
fn dotenv_row_shows_the_root_file() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_open_environments();
    s.invoke_env_dotenv_selected();
    assert!(s.get_env_on_dotenv());
    assert_eq!(s.get_env_index(), -1);
    let rows = s.get_env_rows();
    assert_eq!(rows.row_count(), 2);
    assert_eq!(rows.row_data(0).unwrap().key, "rootOnly");
    assert!(!s.get_env_dirty());
}

#[test]
fn saving_dotenv_writes_cleaned_rows() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_open_environments();
    s.invoke_env_dotenv_selected();
    set_row(&s, 0, "rootOnly", "new");
    set_row(&s, 1, " spaced ", "1");
    assert!(s.get_env_dirty());
    s.invoke_env_save();
    let saved = f.environments.dotenv_saves.borrow()[0].clone();
    let got: Vec<_> = saved
        .iter()
        .map(|v| (v.key.as_str(), v.value.as_str()))
        .collect();
    assert_eq!(got, [("rootOnly", "new"), ("spaced", "1")]);
    assert!(!s.get_env_dirty());
    assert!(s.get_env_on_dotenv());
    assert!(f.environments.saves.borrow().is_empty());
}

#[test]
fn saving_dotenv_updates_resolution() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    s.invoke_open_environments();
    s.invoke_env_dotenv_selected();
    set_row(&s, 0, "rootOnly", "fresh");
    s.invoke_env_save();
    s.invoke_env_close();
    s.set_url("{{rootOnly}}".into());
    s.invoke_changed();
    assert_eq!(s.get_resolved_url(), "fresh?page=2");
}

#[test]
fn duplicate_dotenv_key_stays_in_the_dialog() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_open_environments();
    s.invoke_env_dotenv_selected();
    set_row(&s, 1, "rootOnly", "again");
    s.invoke_env_save();
    assert_eq!(s.get_env_error(), "Variable \"rootOnly\" appears twice");
    assert!(f.environments.dotenv_saves.borrow().is_empty());
}

#[test]
fn leaving_dirty_dotenv_asks_about_dotenv() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_open_environments();
    s.invoke_env_dotenv_selected();
    set_row(&s, 0, "rootOnly", "changed");
    s.invoke_env_selected(0);
    assert!(s.get_confirm_open());
    assert_eq!(s.get_dialog_kind(), DialogKind::EnvironmentUnsaved);
    assert_eq!(s.get_confirm_name(), ".env");
}

#[test]
fn delete_does_nothing_on_dotenv() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_open_environments();
    s.invoke_env_dotenv_selected();
    s.invoke_env_delete();
    assert!(!s.get_confirm_open());
    assert!(f.environments.deletes.borrow().is_empty());
}

#[test]
fn cancel_on_a_draft_returns_to_dotenv() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_open_environments();
    s.invoke_env_dotenv_selected();
    s.invoke_env_new();
    assert!(!s.get_env_on_dotenv());
    s.invoke_env_cancel();
    assert!(s.get_env_on_dotenv());
}

#[test]
fn picking_an_environment_leaves_dotenv() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_open_environments();
    s.invoke_env_dotenv_selected();
    s.invoke_env_selected(0);
    assert!(!s.get_env_on_dotenv());
    assert_eq!(s.get_env_name(), "dev");
}
