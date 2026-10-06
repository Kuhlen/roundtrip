mod support;

use std::path::Path;
use std::time::{Duration, Instant};

use app::modules::workspace::workspace_rules::error_text;
use app::ui::{AppWindow, AuthFields, DialogKind, KvRow, KvTable};
use domain::AppError;
use domain::auth::Auth;
use domain::http::{Body, FormField};
use slint::{CloseRequestResponse, ComponentHandle, Model};
use support::{
    ORDERS, ROOT, kv, open_with_env, opened, p, row_of, set_collection_auth, setup, state, strings,
    tree_names,
};

/// (title, active) per tab
fn tabs(ui: &AppWindow) -> Vec<(String, bool)> {
    state(ui)
        .get_tabs()
        .iter()
        .map(|t| (t.title.to_string(), t.active))
        .collect()
}

fn open(ui: &AppWindow, name: &str) {
    state(ui).invoke_row_clicked(row_of(ui, name));
}

#[test]
fn edit_then_flush_saves_the_file() {
    let (f, ui, c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    s.set_url("{{baseUrl}}/anything".into());
    s.invoke_changed();
    assert!(s.get_dirty(), "dirty until the timer fires");
    assert!(f.collections.saved.borrow().is_empty());
    c.flush();
    assert_eq!(
        f.collections.saved.borrow()[0].1.url,
        "{{baseUrl}}/anything"
    );
    assert!(!s.get_dirty());
}

#[test]
fn flush_skips_a_body_with_invalid_variables() {
    let (f, ui, c) = support::opened_graphql();
    let s = state(&ui);
    s.set_gql_variables("{\"id\": }".into());
    s.invoke_changed();
    c.flush();
    assert!(f.collections.saved.borrow().is_empty());
    assert!(s.get_dirty());
    assert_eq!(s.get_banner_title(), "", "flush stays quiet");
}

#[test]
fn switching_request_saves_the_old_one_without_asking() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    s.set_url("edited".into());
    s.invoke_changed();
    s.invoke_row_clicked(row_of(&ui, "Create user"));
    assert!(!s.get_confirm_open());
    assert_eq!(f.collections.saved.borrow()[0].1.url, "edited");
    assert_eq!(s.get_request_name(), "Create user");
}

#[test]
fn second_collection_adds_a_root_row() {
    let (_f, ui, c) = opened();
    c.open_collection(Path::new(ORDERS));
    let names = tree_names(&ui);
    assert_eq!(names[..2], ["httpbin-demo", "users"]);
    assert_eq!(names[names.len() - 2..], ["orders-api", "Orders"]);
    c.open_collection(Path::new(ROOT));
    assert_eq!(tree_names(&ui), names, "reopening does not duplicate");
}

#[test]
fn environments_come_from_the_first_collection() {
    let (_f, ui, c) = setup();
    c.open_collection(Path::new(ORDERS));
    c.open_collection(Path::new(ROOT));
    let s = state(&ui);
    assert_eq!(
        strings(s.get_environments()),
        ["No environment", "qa", "dev"]
    );
    s.set_environment_index(2);
    s.invoke_environment_selected();
    s.invoke_row_clicked(row_of(&ui, "List users"));
    assert_eq!(s.get_resolved_url(), "https://orders.test/get?page=2");
}

#[test]
fn closing_first_collection_switches_environments() {
    let (_f, ui, c) = opened();
    c.open_collection(Path::new(ORDERS));
    let s = state(&ui);
    s.invoke_close_collection(row_of(&ui, "httpbin-demo"));
    assert_eq!(
        strings(s.get_environments()),
        ["No environment", "qa", "dev"]
    );
    assert_eq!(s.get_environment_index(), 2, "dev stays selected by name");
    assert_eq!(tree_names(&ui), ["orders-api", "Orders"]);
    s.invoke_close_collection(row_of(&ui, "orders-api"));
    assert_eq!(strings(s.get_environments()), ["No environment"]);
    assert!(!s.get_has_collection());
}

#[test]
fn collection_settings_target_the_row_collection() {
    let (f, ui, c) = opened();
    c.open_collection(Path::new(ORDERS));
    let s = state(&ui);
    s.invoke_collection_settings(row_of(&ui, "orders-api"));
    assert!(s.get_settings_open());
    s.set_settings_auth(AuthFields {
        kind: 1,
        token: "t".into(),
        ..AuthFields::default()
    });
    s.invoke_save_settings();
    assert_eq!(
        f.collections.auth_saves.borrow().last().cloned(),
        Some((ORDERS.into(), Some(Auth::Bearer { token: "t".into() })))
    );
}

#[test]
fn inherit_uses_the_request_collection_auth() {
    let (f, ui, c) = setup();
    set_collection_auth(&f, Some(Auth::Bearer { token: "c".into() }));
    open_with_env(&ui, &c);
    c.open_collection(Path::new(ORDERS));
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "Orders"));
    assert_eq!(s.get_request_auth().kind, 0, "inherit");
    assert_eq!(
        s.get_inherited_auth(),
        "No auth. The collection has none either."
    );
    s.invoke_send();
    // worker thread records the request; no event loop here
    let start = Instant::now();
    while f.sender.requests.lock().unwrap().is_empty() {
        assert!(start.elapsed() < Duration::from_secs(5), "send timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(f.sender.requests.lock().unwrap()[0].auth, None);
}

#[test]
fn clicking_a_request_twice_keeps_one_tab() {
    let (_f, ui, _c) = opened();
    open(&ui, "List users");
    open(&ui, "List users");
    assert_eq!(tabs(&ui), [("List users".to_string(), true)]);
}

#[test]
fn each_request_gets_its_own_tab() {
    let (_f, ui, _c) = opened();
    open(&ui, "List users");
    open(&ui, "Create user");
    assert_eq!(
        tabs(&ui),
        [
            ("List users".to_string(), false),
            ("Create user".to_string(), true)
        ]
    );
    assert_eq!(state(&ui).get_request_name(), "Create user");
    assert_eq!(state(&ui).get_active_tab(), 1);
}

#[test]
fn switching_back_restores_the_form() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    open(&ui, "Create user");
    s.set_body_kind_index(5);
    s.invoke_changed();
    s.get_form_rows().set_row_data(
        0,
        KvRow {
            key: "a".into(),
            value: "1".into(),
            ..s.get_form_rows().row_data(0).unwrap()
        },
    );
    s.invoke_kv_edited(KvTable::Form, 0);
    s.set_editor_tab(2);
    open(&ui, "List users");
    assert_eq!(s.get_body_kind_index(), 0);
    s.invoke_tab_clicked(0);
    assert_eq!(s.get_request_name(), "Create user");
    assert_eq!(s.get_body_kind_index(), 5);
    assert_eq!(
        kv(&s.get_form_rows()),
        [
            ("a".to_string(), "1".to_string(), true),
            (String::new(), String::new(), true)
        ]
    );
    assert_eq!(s.get_editor_tab(), 2);
    assert!(s.get_has_request());
    assert!(!s.get_dirty(), "flushed on the way out");
    let saved = f.collections.saved.borrow();
    assert_eq!(saved[0].0, p("users/create-user.yaml"));
    assert_eq!(
        saved[0].1.body,
        Body::FormData(vec![FormField::text("a", "1")])
    );
}

#[test]
fn switching_tab_flushes_pending_autosave() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    open(&ui, "List users");
    open(&ui, "Create user");
    s.invoke_tab_clicked(0);
    s.set_url("edited".into());
    s.invoke_changed();
    s.invoke_tab_clicked(1);
    let saved = f.collections.saved.borrow();
    assert_eq!(saved.len(), 1, "only the left tab is written");
    assert_eq!(saved[0].0, p("users/list-users.yaml"));
    assert_eq!(saved[0].1.url, "edited");
}

#[test]
fn closing_the_active_tab_activates_its_right_neighbour() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    open(&ui, "List users");
    open(&ui, "Create user");
    open(&ui, "Health check");
    s.invoke_tab_clicked(1);
    s.invoke_tab_close(1);
    assert_eq!(
        tabs(&ui),
        [
            ("List users".to_string(), false),
            ("Health check".to_string(), true)
        ]
    );
    assert_eq!(s.get_request_name(), "Health check");
    s.invoke_tab_close(1);
    assert_eq!(tabs(&ui), [("List users".to_string(), true)]);
    s.invoke_tab_close(0);
    assert!(tabs(&ui).is_empty());
    assert!(!s.get_has_request());
    assert_eq!(s.get_request_name(), "");
    assert_eq!(s.get_active_tab(), -1);
}

#[test]
fn response_belongs_to_its_tab() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    open(&ui, "List users");
    s.set_status_text("200 OK".into());
    open(&ui, "Create user");
    assert_eq!(s.get_status_text(), "");
    s.invoke_tab_clicked(0);
    assert_eq!(s.get_status_text(), "200 OK");
}

#[test]
fn rename_folder_moves_every_open_tab() {
    let (f, ui, c) = opened();
    let s = state(&ui);
    open(&ui, "List users");
    s.set_url("edited".into());
    s.invoke_changed();
    open(&ui, "Create user");
    s.invoke_rename_start(row_of(&ui, "users"));
    s.invoke_rename_commit("people".into());
    assert_eq!(s.get_request_file(), "people/create-user.yaml");
    s.invoke_tab_clicked(0);
    assert_eq!(s.get_request_file(), "people/list-users.yaml");
    assert_eq!(s.get_url(), "edited");
    s.set_url("again".into());
    s.invoke_changed();
    c.flush();
    let saved = f.collections.saved.borrow();
    assert_eq!(
        saved.last().map(|(path, r)| (path.clone(), r.url.clone())),
        Some((p("people/list-users.yaml"), "again".to_string()))
    );
}

#[test]
fn deleting_a_folder_closes_its_tabs() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    open(&ui, "List users");
    open(&ui, "Create user");
    open(&ui, "Health check");
    s.invoke_delete_row(row_of(&ui, "users"));
    s.invoke_confirm_delete();
    assert_eq!(tabs(&ui), [("Health check".to_string(), true)]);
}

#[test]
fn closing_a_collection_closes_its_tabs() {
    let (_f, ui, c) = opened();
    c.open_collection(Path::new(ORDERS));
    let s = state(&ui);
    open(&ui, "Orders");
    open(&ui, "List users");
    s.invoke_close_collection(row_of(&ui, "httpbin-demo"));
    assert_eq!(tabs(&ui), [("Orders".to_string(), true)]);
    assert_eq!(s.get_request_name(), "Orders");
}

fn click(ui: &AppWindow, x: f32, y: f32) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let position = slint::LogicalPosition::new(x, y);
    let button = PointerEventButton::Left;
    ui.window()
        .dispatch_event(WindowEvent::PointerPressed { position, button });
    ui.window()
        .dispatch_event(WindowEvent::PointerReleased { position, button });
}

#[test]
fn tab_chip_switches_and_its_x_closes() {
    let (_f, ui, _c) = opened();
    ui.window().set_size(slint::LogicalSize::new(1180.0, 700.0));
    open(&ui, "List users");
    open(&ui, "Create user");
    // sidebar 248 + divider 1; chips 160 wide, × 18 wide inside 6 px right padding
    click(&ui, 300.0, 17.0);
    assert_eq!(state(&ui).get_active_tab(), 0, "chip body switches");
    click(&ui, 249.0 + 160.0 - 6.0 - 9.0, 17.0);
    assert_eq!(tabs(&ui), [("Create user".to_string(), true)], "× closes");
}

fn titles(ui: &AppWindow) -> Vec<String> {
    tabs(ui).into_iter().map(|(t, _)| t).collect()
}

fn pinned(ui: &AppWindow) -> Vec<bool> {
    state(ui).get_tabs().iter().map(|t| t.pinned).collect()
}

fn open_three(ui: &AppWindow) {
    for name in ["List users", "Create user", "Health check"] {
        open(ui, name);
    }
}

#[test]
fn pinning_moves_a_tab_to_the_pinned_group() {
    let (_f, ui, _c) = opened();
    open_three(&ui);
    let s = state(&ui);
    s.invoke_tab_pin(2);
    assert_eq!(titles(&ui), ["Health check", "List users", "Create user"]);
    assert_eq!(pinned(&ui), [true, false, false]);
    s.invoke_tab_pin(0);
    assert_eq!(titles(&ui), ["Health check", "List users", "Create user"]);
    assert_eq!(pinned(&ui), [false, false, false]);
}

#[test]
fn move_tab_respects_pins() {
    let (_f, ui, _c) = opened();
    open_three(&ui);
    let s = state(&ui);
    s.invoke_tab_pin(2);
    s.invoke_tab_moved(2, 0);
    assert_eq!(titles(&ui), ["Health check", "Create user", "List users"]);
    s.invoke_tab_moved(0, 2);
    assert_eq!(titles(&ui), ["Health check", "Create user", "List users"]);
}

#[test]
fn close_others_and_close_all() {
    let (_f, ui, _c) = opened();
    open_three(&ui);
    let s = state(&ui);
    s.invoke_tab_close_others(1);
    assert_eq!(titles(&ui), ["Create user"]);
    s.invoke_tab_close_all();
    assert!(titles(&ui).is_empty());
    assert!(!s.get_has_request());
}

#[test]
fn session_stores_file_tabs_and_restores_them() {
    use domain::session::Session;
    let (f, ui, _c) = opened();
    open_three(&ui);
    state(&ui).invoke_tab_clicked(1);
    let saved = f.session.saves.borrow().last().cloned().expect("saved");
    assert_eq!(
        saved.tabs,
        [
            p("users/list-users.yaml"),
            p("users/create-user.yaml"),
            p("health.yaml")
        ]
    );
    assert_eq!(saved.active_tab, Some(1));

    let g = support::fakes();
    *g.session.session.borrow_mut() = Session {
        collections: vec![std::path::PathBuf::from(ROOT)],
        tabs: vec![
            saved.tabs[0].clone(),
            p("gone.yaml"),
            saved.tabs[1].clone(),
            saved.tabs[2].clone(),
        ],
        active_tab: Some(2),
        ..Session::default()
    };
    let (ui2, c2) = support::build(&g);
    c2.restore();
    assert_eq!(
        tabs(&ui2),
        [
            ("List users".to_string(), false),
            ("Create user".to_string(), true),
            ("Health check".to_string(), false)
        ]
    );
    assert_eq!(state(&ui2).get_request_name(), "Create user");
}

#[test]
fn middle_click_closes_an_unpinned_tab_only() {
    use slint::platform::{PointerEventButton, WindowEvent};
    let (_f, ui, _c) = opened();
    ui.window().set_size(slint::LogicalSize::new(1180.0, 700.0));
    open(&ui, "List users");
    open(&ui, "Create user");
    let middle = |x: f32| {
        let position = slint::LogicalPosition::new(x, 17.0);
        let button = PointerEventButton::Middle;
        ui.window()
            .dispatch_event(WindowEvent::PointerPressed { position, button });
        ui.window()
            .dispatch_event(WindowEvent::PointerReleased { position, button });
    };
    state(&ui).invoke_tab_pin(0);
    middle(300.0);
    assert_eq!(titles(&ui), ["List users", "Create user"], "pinned stays");
    middle(300.0 + 160.0);
    assert_eq!(titles(&ui), ["List users"]);
}

fn drag(ui: &AppWindow, from: f32, to: f32) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let at = |x: f32| slint::LogicalPosition::new(x, 17.0);
    let button = PointerEventButton::Left;
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position: at(from),
        button,
    });
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position: at(to) });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position: at(to),
        button,
    });
}

fn chip_x(i: usize, dx: f32) -> f32 {
    249.0 + 160.0 * i as f32 + dx
}

#[test]
fn dragging_a_chip_reorders_without_switching() {
    let (f, ui, _c) = opened();
    ui.window().set_size(slint::LogicalSize::new(1180.0, 700.0));
    open_three(&ui);
    drag(&ui, chip_x(0, 40.0), chip_x(2, 40.0));
    assert_eq!(titles(&ui), ["Create user", "Health check", "List users"]);
    assert_eq!(state(&ui).get_active_tab(), 1, "Health check stays active");
    assert_eq!(
        f.session.saves.borrow().last().expect("saved").tabs[2],
        p("users/list-users.yaml")
    );
}

#[test]
fn dragging_inside_a_chip_does_not_switch() {
    let (_f, ui, _c) = opened();
    ui.window().set_size(slint::LogicalSize::new(1180.0, 700.0));
    open_three(&ui);
    drag(&ui, chip_x(0, 20.0), chip_x(0, 60.0));
    assert_eq!(titles(&ui), ["List users", "Create user", "Health check"]);
    assert_eq!(state(&ui).get_active_tab(), 2);
}

#[test]
fn a_small_wiggle_is_still_a_click() {
    let (_f, ui, _c) = opened();
    ui.window().set_size(slint::LogicalSize::new(1180.0, 700.0));
    open_three(&ui);
    drag(&ui, chip_x(1, 40.0), chip_x(1, 44.0));
    assert_eq!(titles(&ui), ["List users", "Create user", "Health check"]);
    assert_eq!(state(&ui).get_active_tab(), 1);
}

fn edit_url(ui: &AppWindow, url: &str) {
    state(ui).set_url(url.into());
    state(ui).invoke_changed();
}

/// First request the fake sender got; waits for the worker thread.
fn first_sent(f: &support::Fakes) -> domain::http::Request {
    let start = Instant::now();
    loop {
        if let Some(r) = f.sender.requests.lock().expect("requests").first() {
            return r.clone();
        }
        assert!(start.elapsed() < Duration::from_secs(5), "send timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn untitled(ui: &AppWindow) -> Vec<bool> {
    state(ui).get_tabs().iter().map(|t| t.untitled).collect()
}

#[test]
fn ctrl_t_opens_numbered_untitled_tabs_without_a_collection() {
    let (f, ui, _c) = setup();
    let s = state(&ui);
    s.invoke_tab_new();
    s.invoke_tab_new();
    assert_eq!(
        tabs(&ui),
        [
            ("Untitled".to_string(), false),
            ("Untitled 2".to_string(), true)
        ]
    );
    assert!(s.get_has_request());
    assert_eq!(s.get_request_name(), "Untitled 2");
    assert_eq!(s.get_crumb(), "");
    assert_eq!(s.get_request_file(), "not saved");
    edit_url(&ui, "https://example.com");
    assert!(s.get_can_send());
    s.invoke_send();
    assert_eq!(first_sent(&f).url, "https://example.com");
}

#[test]
fn untitled_inherit_has_no_auth() {
    let (f, ui, c) = setup();
    set_collection_auth(&f, Some(Auth::Bearer { token: "c".into() }));
    open_with_env(&ui, &c);
    let s = state(&ui);
    s.invoke_tab_new();
    assert_eq!(s.get_request_auth().kind, 0, "inherit");
    assert_eq!(s.get_inherited_auth(), "No collection: no auth.");
    edit_url(&ui, "{{baseUrl}}/get");
    s.invoke_send();
    let sent = first_sent(&f);
    assert_eq!(sent.auth, None);
    assert_eq!(
        sent.url, "https://httpbin.org/get",
        "first collection's environment"
    );
}

#[test]
fn untitled_tab_stores_picked_files_absolute() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_tab_new();
    support::PICKED.with(|p| *p.borrow_mut() = Some("/c/avatar.png".into()));
    s.invoke_pick_file(-1);
    assert_eq!(s.get_binary_path(), "/c/avatar.png");
}

#[test]
fn closing_a_dirty_untitled_tab_asks() {
    let (_f, ui, _c) = setup();
    let s = state(&ui);
    s.invoke_tab_new();
    edit_url(&ui, "https://example.com");
    s.invoke_tab_close(0);
    assert!(s.get_confirm_open());
    assert_eq!(s.get_dialog_kind(), DialogKind::Untitled);
    assert_eq!(s.get_confirm_name(), "Untitled");
    s.invoke_confirm_cancel();
    assert!(!s.get_confirm_open());
    assert_eq!(titles(&ui), ["Untitled"]);
    s.invoke_tab_close(0);
    s.invoke_confirm_discard();
    assert!(!s.get_confirm_open());
    assert!(titles(&ui).is_empty());
}

#[test]
fn close_all_stops_on_cancel() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    open(&ui, "List users");
    s.invoke_tab_new();
    edit_url(&ui, "https://example.com");
    open(&ui, "Health check");
    s.invoke_tab_close_all();
    assert!(s.get_confirm_open());
    assert_eq!(s.get_confirm_name(), "Untitled");
    assert_eq!(
        tabs(&ui)[0],
        ("Untitled".to_string(), true),
        "asked tab is shown"
    );
    s.invoke_confirm_cancel();
    assert_eq!(titles(&ui), ["Untitled", "Health check"]);
}

#[test]
fn save_as_creates_the_file_and_turns_the_tab_into_a_file_tab() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_tab_new();
    edit_url(&ui, "https://example.com");
    s.invoke_save();
    assert!(s.get_save_as_open());
    assert_eq!(s.get_save_as_name(), "");
    assert_eq!(strings(s.get_save_as_collections()), ["httpbin-demo"]);
    assert_eq!(strings(s.get_save_as_folders()), ["/", "/ users"]);
    s.set_save_as_name(" Ping ".into());
    s.set_save_as_folder_index(1);
    s.invoke_save_as_confirm();
    assert!(!s.get_save_as_open());
    {
        let saved = f.collections.saved.borrow();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].0, p("users/ping.yaml"));
        assert_eq!(saved[0].1.url, "https://example.com");
    }
    assert_eq!(tabs(&ui), [("Ping".to_string(), true)]);
    assert_eq!(untitled(&ui), [false]);
    assert!(!s.get_dirty());
    assert_eq!(s.get_request_file(), "users/ping.yaml");
    assert!(tree_names(&ui).contains(&"Ping".to_string()));
    assert_eq!(
        f.session.saves.borrow().last().expect("saved").tabs,
        [p("users/ping.yaml")]
    );
}

#[test]
fn save_as_failure_removes_the_new_file() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_tab_new();
    edit_url(&ui, "https://example.com");
    *f.collections.save_error.borrow_mut() = Some(AppError::Storage("disk full".into()));
    s.invoke_save();
    s.set_save_as_name("Ping".into());
    s.invoke_save_as_confirm();
    assert_eq!(*f.collections.deleted.borrow(), [p("ping.yaml")]);
    assert!(!s.get_save_as_open());
    assert_ne!(s.get_banner_title(), "");
    assert_eq!(titles(&ui), ["Untitled"]);
    assert_eq!(untitled(&ui), [true]);
    assert!(s.get_dirty());
}

#[test]
fn save_as_name_error_stays_in_the_dialog() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_tab_new();
    edit_url(&ui, "https://example.com");
    s.invoke_save();
    s.set_save_as_name("List users".into());
    s.set_save_as_folder_index(1);
    s.invoke_save_as_confirm();
    let expected = error_text(&AppError::AlreadyExists("list-users.yaml".into()), None).0;
    assert_eq!(s.get_save_as_error(), expected.as_str());
    assert!(s.get_save_as_open());
    assert!(f.collections.deleted.borrow().is_empty());
    assert!(f.collections.saved.borrow().is_empty());
}

#[test]
fn save_as_without_a_collection_is_disabled() {
    let (f, ui, _c) = setup();
    let s = state(&ui);
    s.invoke_tab_new();
    edit_url(&ui, "https://example.com");
    s.invoke_save();
    assert!(s.get_save_as_open());
    assert_eq!(s.get_save_as_collections().row_count(), 0);
    s.set_save_as_name("Ping".into());
    s.invoke_save_as_confirm();
    assert!(s.get_save_as_open());
    assert!(f.collections.saved.borrow().is_empty());
    assert_eq!(untitled(&ui), [true]);
}

#[test]
fn duplicate_makes_a_dirty_untitled_copy() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    open(&ui, "List users");
    open(&ui, "Health check");
    s.invoke_tab_duplicate(0);
    assert_eq!(
        tabs(&ui),
        [
            ("List users".to_string(), false),
            ("Untitled".to_string(), true),
            ("Health check".to_string(), false)
        ]
    );
    assert_eq!(untitled(&ui), [false, true, false]);
    assert_eq!(s.get_url(), "{{baseUrl}}/get");
    assert_eq!(kv(&s.get_params())[0], ("page".into(), "2".into(), true));
    assert!(s.get_dirty());
    assert!(
        f.collections.saved.borrow().is_empty(),
        "original untouched"
    );
}

#[test]
fn quit_asks_per_untitled_tab_and_cancel_stops() {
    let (_f, ui, c) = setup();
    ui.show().unwrap();
    let s = state(&ui);
    s.invoke_tab_new();
    edit_url(&ui, "a");
    s.invoke_tab_new();
    edit_url(&ui, "b");
    assert!(matches!(
        c.close_requested(),
        CloseRequestResponse::KeepWindowShown
    ));
    assert_eq!(s.get_confirm_name(), "Untitled");
    s.invoke_confirm_discard();
    assert!(s.get_confirm_open());
    assert_eq!(s.get_confirm_name(), "Untitled 2");
    s.invoke_confirm_cancel();
    assert!(!s.get_confirm_open());
    assert!(ui.window().is_visible());
    assert_eq!(titles(&ui), ["Untitled 2"]);
    c.close_requested();
    s.invoke_confirm_discard();
    assert!(!ui.window().is_visible(), "last answer hides the window");
}

#[test]
fn closing_a_file_tab_that_failed_to_save_keeps_it_open() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    open(&ui, "List users");
    open(&ui, "Health check");
    *f.collections.save_error.borrow_mut() = Some(AppError::Storage("disk full".into()));
    edit_url(&ui, "edited");
    s.invoke_tab_close_all();
    assert_eq!(titles(&ui), ["Health check"]);
    assert!(s.get_dirty());
    assert_eq!(s.get_banner_title(), "Could not read or write a file");
}

#[test]
fn closing_a_tab_with_invalid_variables_keeps_it_open() {
    let (_f, ui, _c) = support::opened_graphql();
    let s = state(&ui);
    s.set_gql_variables("{\"id\": }".into());
    s.invoke_changed();
    s.invoke_tab_close(0);
    assert_eq!(titles(&ui), ["Health check"]);
    assert_eq!(s.get_banner_title(), "Variables: invalid JSON");
}

#[test]
fn quit_with_an_unsaved_file_tab_keeps_the_window() {
    let (f, ui, c) = opened();
    ui.show().unwrap();
    let s = state(&ui);
    open(&ui, "List users");
    *f.collections.save_error.borrow_mut() = Some(AppError::Storage("disk full".into()));
    edit_url(&ui, "edited");
    assert!(matches!(
        c.close_requested(),
        CloseRequestResponse::KeepWindowShown
    ));
    assert!(ui.window().is_visible());
    assert!(!s.get_confirm_open());
    assert_eq!(s.get_banner_title(), "Could not read or write a file");
    assert_eq!(titles(&ui), ["List users"]);
}

#[test]
fn quit_is_ignored_while_a_dialog_is_open() {
    let (_f, ui, c) = setup();
    let s = state(&ui);
    s.invoke_tab_new();
    edit_url(&ui, "https://example.com");
    s.invoke_save();
    assert!(s.get_save_as_open());
    assert!(matches!(
        c.close_requested(),
        CloseRequestResponse::KeepWindowShown
    ));
    assert!(s.get_save_as_open());
    assert!(!s.get_confirm_open());
    assert_eq!(titles(&ui), ["Untitled"]);
}

fn abs(rel: &str) -> String {
    Path::new(ROOT).join(rel).display().to_string()
}

#[test]
fn duplicate_makes_binary_path_absolute() {
    let (_f, ui, _c) = support::opened_with_body(Body::Binary("a.bin".into()));
    let s = state(&ui);
    s.invoke_tab_duplicate(0);
    assert_eq!(s.get_binary_path(), abs("a.bin"));
}

#[test]
fn duplicate_makes_form_file_rows_absolute() {
    let (_f, ui, _c) = support::opened_with_body(Body::FormData(vec![
        FormField::text("n", "1"),
        FormField::file("avatar", "img/a.png"),
    ]));
    let s = state(&ui);
    s.invoke_tab_duplicate(0);
    let rows = s.get_form_rows();
    assert_eq!(rows.row_data(0).unwrap().value, "1");
    assert_eq!(rows.row_data(1).unwrap().value, abs("img/a.png"));
}
