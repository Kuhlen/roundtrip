//! composition root

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use data::app_state::AppStateFile;
use data::collection_dir::CollectionDir;
use data::environment_dir::EnvironmentDir;
use data::history_db::HistoryDb;
use data::http::ReqwestSender;
use domain::AppError;
use domain::history::HistoryStore;

use crate::modules::workspace::workspace_controller::{Deps, WorkspaceController};
use crate::ui::AppWindow;

pub fn build(ui: &AppWindow) -> Result<Rc<WorkspaceController>, AppError> {
    let deps = Deps {
        collections: Rc::new(CollectionDir),
        environments: Rc::new(EnvironmentDir),
        sender: Arc::new(ReqwestSender::new()?),
        session: Rc::new(AppStateFile::in_config_dir()),
        history: HistoryDb::in_config_dir().map(|h| Arc::new(h) as Arc<dyn HistoryStore>),
        dynamic_var: data::dynamic_vars::resolve,
        pretty_json: data::http::pretty_json,
        pick_file,
        read_import: data::import_file::read,
        pick_import,
        pick_dir,
        export_postman: data::postman_export::write,
        pick_save,
        graphql_parse: data::graphql::parse,
        graphql_json: data::graphql::to_json,
    };
    let controller = WorkspaceController::new(deps, ui);
    controller.restore();
    Ok(controller)
}

fn pick_file(start: &Path) -> Option<PathBuf> {
    // sync dialog on the UI thread, like Open collection
    rfd::FileDialog::new()
        .set_title("Choose file")
        .set_directory(start)
        .pick_file()
}

fn pick_import() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Import collection")
        .add_filter("Collection or API spec", &["json", "yaml", "yml"])
        .pick_file()
}

fn pick_dir() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Choose folder")
        .pick_folder()
}

fn pick_save(name: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Export as Postman")
        .add_filter("Postman collection", &["json"])
        .set_file_name(name)
        .save_file()
}
