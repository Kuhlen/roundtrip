mod support;

use std::path::PathBuf;

use domain::AppError;
use domain::http::Request;
use domain::import::{
    ImportFormat, ImportItem, ImportWarning, ImportedCollection, ImportedRequest,
};
use support::{IMPORTED, PICKED_DIR, PICKED_JSON, setup, state, strings, tree_names};

fn shop() -> ImportedCollection {
    let req = |name: &str| {
        ImportItem::Request(ImportedRequest {
            name: name.into(),
            request: Request::default(),
            ..ImportedRequest::default()
        })
    };
    ImportedCollection {
        name: "Shop API".into(),
        items: vec![ImportItem::Folder {
            name: "Users".into(),
            items: vec![req("List"), req("Create")],
        }],
        environments: vec![("Collection Variables".into(), vec![])],
        warnings: vec![
            ImportWarning::Scripts,
            ImportWarning::UnsupportedAuth("oauth2".into()),
            ImportWarning::UnsupportedAuth("oauth2".into()),
        ],
        ..ImportedCollection::default()
    }
}

fn picks(json: Option<&str>, dir: Option<&str>, data: Result<ImportedCollection, AppError>) {
    PICKED_JSON.with(|p| *p.borrow_mut() = json.map(PathBuf::from));
    PICKED_DIR.with(|p| *p.borrow_mut() = dir.map(PathBuf::from));
    IMPORTED.with(|i| *i.borrow_mut() = data);
}

#[test]
fn preview_shows_counts_and_warnings() {
    let (_f, ui, _c) = setup();
    picks(Some("/in/shop.json"), None, Ok(shop()));
    let s = state(&ui);
    s.invoke_import_collection();
    assert!(s.get_import_open());
    assert_eq!(s.get_import_title(), "Import Postman collection");
    assert_eq!(s.get_import_name(), "Shop API");
    assert_eq!(
        s.get_import_counts(),
        "1 folder · 2 requests · 1 environment"
    );
    assert_eq!(strings(s.get_import_warnings()).len(), 2);
    assert_eq!(s.get_import_error(), "");
}

#[test]
fn confirm_writes_under_the_picked_folder_and_opens_it() {
    let (f, ui, _c) = setup();
    picks(Some("/in/shop.json"), Some("/work"), Ok(shop()));
    let s = state(&ui);
    s.invoke_import_collection();
    s.set_import_name("Shop".into());
    s.invoke_import_confirm();
    let created = f.collections.created.borrow();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].0, PathBuf::from("/work/Shop"));
    assert_eq!(created[0].1.name, "Shop");
    assert!(!s.get_import_open());
    assert!(tree_names(&ui).contains(&"Shop".to_owned()));
}

#[test]
fn existing_folder_keeps_the_dialog_open() {
    let (f, ui, _c) = setup();
    picks(Some("/in/shop.json"), Some("/work"), Ok(shop()));
    *f.collections.edit_error.borrow_mut() = Some(AppError::AlreadyExists("Shop API".into()));
    let s = state(&ui);
    s.invoke_import_collection();
    s.invoke_import_confirm();
    assert!(s.get_import_open());
    assert_eq!(
        s.get_import_error(),
        "Shop API already exists. Pick another name."
    );
    // a second try with a new name still has the data
    *f.collections.edit_error.borrow_mut() = None;
    s.set_import_name("Shop 2".into());
    s.invoke_import_confirm();
    assert_eq!(
        f.collections.created.borrow()[0].0,
        PathBuf::from("/work/Shop 2")
    );
}

#[test]
fn cancelled_folder_picker_keeps_the_dialog() {
    let (f, ui, _c) = setup();
    picks(Some("/in/shop.json"), None, Ok(shop()));
    let s = state(&ui);
    s.invoke_import_collection();
    s.invoke_import_confirm();
    assert!(s.get_import_open());
    assert!(f.collections.created.borrow().is_empty());
}

#[test]
fn cancel_writes_nothing() {
    let (f, ui, _c) = setup();
    picks(Some("/in/shop.json"), Some("/work"), Ok(shop()));
    let s = state(&ui);
    s.invoke_import_collection();
    s.invoke_import_cancel();
    assert!(!s.get_import_open());
    s.invoke_import_confirm();
    assert!(f.collections.created.borrow().is_empty());
}

#[test]
fn no_file_or_bad_file_opens_no_dialog() {
    let (_f, ui, _c) = setup();
    let s = state(&ui);
    picks(None, None, Ok(shop()));
    s.invoke_import_collection();
    assert!(!s.get_import_open());
    picks(
        Some("/in/x.json"),
        None,
        Err(AppError::Import("not a Postman collection".into())),
    );
    s.invoke_import_collection();
    assert!(!s.get_import_open());
    assert_eq!(s.get_banner_title(), "Import failed");
    assert_eq!(s.get_banner_hint(), "not a Postman collection");
}

#[test]
fn title_names_an_openapi_spec() {
    let (_f, ui, _c) = setup();
    let spec = ImportedCollection {
        format: ImportFormat::OpenApi("3.1.0".into()),
        ..shop()
    };
    picks(Some("/in/petstore.yaml"), None, Ok(spec));
    let s = state(&ui);
    s.invoke_import_collection();
    assert_eq!(s.get_import_title(), "Import OpenAPI 3.1.0 spec");
}
