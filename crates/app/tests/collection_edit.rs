mod support;

use app::ui::DialogKind;
use domain::AppError;
use slint::{ComponentHandle, Model};
use support::{opened, p, state, tree_names};

#[test]
fn plus_twice_creates_numbered_requests_and_opens_them() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_new_request(-1);
    s.invoke_new_request(-1);
    let names = tree_names(&ui);
    assert!(names.contains(&"New request".to_string()));
    assert!(names.contains(&"New request 2".to_string()));
    assert_eq!(s.get_request_name(), "New request 2");
    assert_eq!(s.get_request_file(), "new-request-2.yaml");
    let row = s.get_tree().row_data(s.get_active_row() as usize).unwrap();
    assert!(row.editing, "new row starts in rename mode");
}

#[test]
fn row_menu_creates_inside_the_folder() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_new_folder(0);
    // the fake appends; the real scan sorts folders first
    let row = s
        .get_tree()
        .iter()
        .find(|r| r.name == "New folder")
        .expect("new folder row");
    assert_eq!((row.depth, row.folder), (1, true));
    assert!(row.editing);
}

#[test]
fn rename_dirty_active_request_keeps_edits_and_saves_to_new_path() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(1);
    s.set_url("edited".into());
    s.invoke_changed();
    s.invoke_rename_start(1);
    assert!(s.get_tree().row_data(1).unwrap().editing);
    s.invoke_rename_commit("All users".into());
    assert!(s.get_dirty());
    assert_eq!(s.get_request_name(), "All users");
    assert_eq!(s.get_request_file(), "users/all-users.yaml");
    assert!(!s.get_tree().iter().any(|r| r.editing));
    s.invoke_save();
    assert_eq!(
        f.collections.saved.borrow().last().unwrap().0,
        p("users/all-users.yaml")
    );
}

#[test]
fn rename_folder_rebases_active_request() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(2);
    s.set_url("edited".into());
    s.invoke_changed();
    s.invoke_rename_start(0);
    s.invoke_rename_commit("people".into());
    assert_eq!(s.get_request_file(), "people/create-user.yaml");
    s.invoke_save();
    assert_eq!(
        f.collections.saved.borrow().last().unwrap().0,
        p("people/create-user.yaml")
    );
}

#[test]
fn rename_collision_shows_banner_and_keeps_the_name() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_rename_start(1);
    s.invoke_rename_commit("Create user".into());
    assert_eq!(s.get_banner_title(), "create-user.yaml already exists");
    assert_eq!(tree_names(&ui)[1], "List users");
    assert!(!s.get_tree().row_data(1).unwrap().editing);
}

#[test]
fn delete_active_dirty_request_clears_form_without_unsaved_dialog() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(1);
    s.set_url("edited".into());
    s.invoke_changed();
    s.invoke_delete_row(1);
    assert!(s.get_confirm_open());
    assert_eq!(s.get_dialog_kind(), DialogKind::Delete);
    assert_eq!(s.get_confirm_name(), "List users");
    assert_eq!(s.get_confirm_file(), "users/list-users.yaml");
    s.invoke_confirm_delete();
    assert!(!s.get_confirm_open());
    assert!(!s.get_has_request());
    assert!(!tree_names(&ui).contains(&"List users".to_string()));
    assert_eq!(
        *f.collections.deleted.borrow(),
        [p("users/list-users.yaml")]
    );
}

#[test]
fn delete_folder_with_active_request_clears_form() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(2);
    s.invoke_delete_row(0);
    assert!(s.get_confirm_folder());
    s.invoke_confirm_delete();
    assert!(!s.get_has_request());
    assert_eq!(
        tree_names(&ui),
        ["Health check", "Echo socket", "Upload avatar"]
    );
}

#[test]
fn cancel_delete_keeps_the_item() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_delete_row(3);
    s.invoke_confirm_cancel();
    assert!(!s.get_confirm_open());
    assert_eq!(tree_names(&ui).len(), 6);
    assert!(f.collections.deleted.borrow().is_empty());
}

#[test]
fn failed_delete_shows_banner() {
    let (f, ui, _c) = opened();
    *f.collections.edit_error.borrow_mut() = Some(AppError::Storage("busy".into()));
    let s = state(&ui);
    s.invoke_delete_row(3);
    s.invoke_confirm_delete();
    assert_eq!(s.get_banner_title(), "Could not read or write a file");
    assert_eq!(tree_names(&ui).len(), 6);
}

#[test]
fn failed_rename_shows_banner_and_ends_editing() {
    let (f, ui, _c) = opened();
    *f.collections.edit_error.borrow_mut() = Some(AppError::Storage("busy".into()));
    let s = state(&ui);
    s.invoke_rename_start(1);
    s.invoke_rename_commit("Other".into());
    assert_eq!(s.get_banner_title(), "Could not read or write a file");
    assert_eq!(tree_names(&ui)[1], "List users");
    assert!(!s.get_tree().iter().any(|r| r.editing));
}

#[test]
fn f2_starts_rename_on_the_active_row() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(3);
    assert_eq!(s.get_active_row(), 3);
    let f2: slint::SharedString = slint::platform::Key::F2.into();
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: f2.clone() });
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyReleased { text: f2 });
    assert!(s.get_tree().row_data(3).unwrap().editing);
}

#[test]
fn clicking_the_renamed_active_row_ends_rename() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(3);
    s.invoke_rename_start(3);
    s.invoke_row_clicked(3);
    assert!(!s.get_tree().iter().any(|r| r.editing));
}
