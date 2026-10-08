mod support;

use std::path::PathBuf;

use domain::AppError;
use domain::import::{ExportReport, ExportWarning};
use support::{EXPORT_CALLS, EXPORTED, ROOT, SAVE_AS, SAVE_OFFERED, opened, row_of, state};

fn answers(save: Option<&str>, result: Result<ExportReport, AppError>) {
    SAVE_AS.with(|p| *p.borrow_mut() = save.map(PathBuf::from));
    EXPORTED.with(|e| *e.borrow_mut() = result);
    EXPORT_CALLS.with(|c| c.borrow_mut().clear());
}

fn calls() -> Vec<(PathBuf, PathBuf)> {
    EXPORT_CALLS.with(|c| c.borrow().clone())
}

#[test]
fn exports_the_collection_and_reports() {
    let (_f, ui, _c) = opened();
    answers(
        Some("/out/demo.json"),
        Ok(ExportReport {
            requests: 4,
            warnings: vec![ExportWarning::EnvironmentsNotExported(2)],
        }),
    );
    let s = state(&ui);
    s.invoke_export_collection(row_of(&ui, "httpbin-demo"));
    assert_eq!(
        SAVE_OFFERED.with(|o| o.borrow().clone()),
        "httpbin-demo.postman_collection.json"
    );
    assert_eq!(
        calls(),
        vec![(PathBuf::from(ROOT), PathBuf::from("/out/demo.json"))]
    );
    assert_eq!(s.get_banner_title(), "Exported 4 requests");
    assert_eq!(
        s.get_banner_hint(),
        "2 environments not exported; Postman keeps environments in separate files"
    );
}

#[test]
fn unsaved_edits_are_counted_not_saved() {
    let (f, ui, _c) = opened();
    answers(
        Some("/out/demo.json"),
        Ok(ExportReport {
            requests: 4,
            warnings: vec![],
        }),
    );
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));
    s.set_url("{{baseUrl}}/edited".into());
    s.invoke_changed();
    s.invoke_export_collection(row_of(&ui, "httpbin-demo"));
    assert_eq!(
        s.get_banner_hint(),
        "1 unsaved request exported in its saved state"
    );
    assert!(
        f.collections.saved.borrow().is_empty(),
        "export never saves tabs"
    );
    assert!(s.get_dirty());
}

#[test]
fn cancelled_save_dialog_does_nothing() {
    let (_f, ui, _c) = opened();
    answers(None, Ok(ExportReport::default()));
    let s = state(&ui);
    s.invoke_export_collection(row_of(&ui, "httpbin-demo"));
    assert!(calls().is_empty());
    assert_eq!(s.get_banner_title(), "");
}

#[test]
fn failed_export_shows_the_error() {
    let (_f, ui, _c) = opened();
    answers(
        Some("/out/demo.json"),
        Err(AppError::Storage("disk full".into())),
    );
    let s = state(&ui);
    s.invoke_export_collection(row_of(&ui, "httpbin-demo"));
    assert_eq!(s.get_banner_title(), "Could not read or write a file");
    assert_eq!(s.get_banner_hint(), "disk full");
}

#[test]
fn rows_that_are_not_collections_are_ignored() {
    let (_f, ui, _c) = opened();
    answers(Some("/out/demo.json"), Ok(ExportReport::default()));
    state(&ui).invoke_export_collection(row_of(&ui, "users"));
    assert!(calls().is_empty());
}
