mod support;

use app::ui::{SegmentKind, VarMode};
use domain::environment::Scope;
use slint::Model;
use std::path::{Path, PathBuf};
use support::{ROOT, opened, row_of, setup, state};

#[test]
fn opening_a_request_highlights_its_variables() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    let segs: Vec<_> = s.get_url_segments().iter().collect();
    assert_eq!(segs.len(), 2);
    assert_eq!(segs[0].text, "{{baseUrl}}");
    assert_eq!(segs[0].kind, SegmentKind::Resolved);
    assert_eq!(segs[0].tip, "baseUrl = https://httpbin.org");
    assert_eq!(segs[1].kind, SegmentKind::Text);
}

fn with_url(ui: &app::ui::AppWindow, url: &str) {
    let s = state(ui);
    s.invoke_row_clicked(row_of(ui, "List users"));
    s.set_url(url.into());
    s.invoke_changed();
}

#[test]
fn clicking_a_resolved_variable_opens_it_for_editing() {
    let (_f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_var_clicked("baseUrl".into());
    assert!(s.get_var_open());
    assert_eq!(s.get_var_mode(), VarMode::Edit);
    assert_eq!(s.get_var_name(), "baseUrl");
    assert_eq!(s.get_var_value(), "https://httpbin.org");
    assert_eq!(s.get_var_env(), "dev");
    assert_eq!(s.get_var_error(), "");
}

#[test]
fn saving_updates_the_environment_and_the_resolved_url() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    s.invoke_var_clicked("baseUrl".into());
    s.set_var_value("https://new.test".into());
    s.invoke_var_save();
    assert!(!s.get_var_open());
    let (old, env) = f.environments.saves.borrow()[0].clone();
    assert_eq!(old, Some(("dev".to_string(), Scope::Shared)));
    let base = env.variables.iter().find(|v| v.key == "baseUrl").unwrap();
    assert_eq!(base.value, "https://new.test");
    // the other variables ride along unchanged
    assert!(env.variables.iter().any(|v| v.key == "token" && v.secret));
    assert_eq!(s.get_resolved_url(), "https://new.test/get?page=2");
}

#[test]
fn unresolved_variable_is_appended_as_plain() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    with_url(&ui, "{{baseUrl}}/users/{{id}}");
    let kinds: Vec<_> = s.get_url_segments().iter().map(|g| g.kind).collect();
    assert_eq!(kinds.last(), Some(&SegmentKind::Unresolved));
    s.invoke_var_clicked("id".into());
    assert_eq!(s.get_var_mode(), VarMode::Edit);
    assert_eq!(s.get_var_value(), "");
    s.set_var_value("7".into());
    s.invoke_var_save();
    let (_, env) = f.environments.saves.borrow()[0].clone();
    let last = env.variables.last().unwrap();
    assert_eq!(
        (last.key.as_str(), last.value.as_str(), last.secret),
        ("id", "7", false)
    );
    assert_eq!(s.get_resolved_url(), "https://httpbin.org/users/7?page=2");
}

#[test]
fn secret_variable_is_masked_and_stays_secret() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    with_url(&ui, "{{baseUrl}}/{{token}}");
    let tip = s.get_url_segments().iter().last().unwrap().tip;
    assert_eq!(tip, "token = ••••");
    s.invoke_var_clicked("token".into());
    assert_eq!(s.get_var_mode(), VarMode::Secret);
    s.set_var_value("rotated".into());
    s.invoke_var_save();
    let (_, env) = f.environments.saves.borrow()[0].clone();
    let token = env.variables.iter().find(|v| v.key == "token").unwrap();
    assert_eq!((token.value.as_str(), token.secret), ("rotated", true));
}

#[test]
fn root_dotenv_only_variable_saves_an_override_into_the_environment() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    with_url(&ui, "{{rootOnly}}");
    assert_eq!(
        s.get_url_segments().row_data(0).unwrap().kind,
        SegmentKind::Resolved
    );
    s.invoke_var_clicked("rootOnly".into());
    assert_eq!(s.get_var_value(), "from-dotenv");
    s.set_var_value("override".into());
    s.invoke_var_save();
    let (_, env) = f.environments.saves.borrow()[0].clone();
    assert!(
        env.variables
            .iter()
            .any(|v| v.key == "rootOnly" && v.value == "override" && !v.secret)
    );
    assert!(f.environments.dotenv_saves.borrow().is_empty());
}

#[test]
fn dynamic_variable_is_read_only() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    with_url(&ui, "{{baseUrl}}/{{$uuid}}");
    assert_eq!(
        s.get_url_segments().iter().last().unwrap().kind,
        SegmentKind::Dynamic
    );
    s.invoke_var_clicked("$uuid".into());
    assert_eq!(s.get_var_mode(), VarMode::Dynamic);
    s.invoke_var_save();
    assert!(f.environments.saves.borrow().is_empty());
}

#[test]
fn without_an_environment_nothing_is_written() {
    let (f, ui, c) = setup();
    c.open_collection(Path::new(ROOT));
    let s = state(&ui);
    s.invoke_var_clicked("baseUrl".into());
    assert_eq!(s.get_var_mode(), VarMode::NoEnvironment);
    s.invoke_var_save();
    assert!(f.environments.saves.borrow().is_empty());
}

#[test]
fn without_a_collection_the_popover_says_so() {
    let (_f, ui, _c) = setup();
    let s = state(&ui);
    s.invoke_var_clicked("baseUrl".into());
    assert_eq!(s.get_var_mode(), VarMode::NoCollection);
}

#[test]
fn missing_environment_keeps_the_popover_open_with_an_error() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_var_clicked("baseUrl".into());
    // deleted on disk after loading
    f.environments
        .envs
        .borrow_mut()
        .get_mut(&PathBuf::from(ROOT))
        .unwrap()
        .retain(|e| e.name != "dev");
    s.set_var_value("typed".into());
    s.invoke_var_save();
    assert!(s.get_var_open());
    assert_ne!(s.get_var_error(), "");
    assert_eq!(s.get_var_value(), "typed");
    assert!(f.environments.saves.borrow().is_empty());
}

#[test]
fn close_discards() {
    let (f, ui, _c) = opened();
    let s = state(&ui);
    s.invoke_var_clicked("baseUrl".into());
    s.set_var_value("typed".into());
    s.invoke_var_close();
    assert!(!s.get_var_open());
    assert!(f.environments.saves.borrow().is_empty());
}
