//! composition root

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use data::app_state::AppStateFile;
use data::collection_dir::CollectionDir;
use data::environment_dir::EnvironmentDir;
use data::http::ReqwestSender;
use domain::AppError;

use crate::modules::workspace::workspace_controller::{Deps, WorkspaceController};
use crate::ui::AppWindow;

pub fn build(ui: &AppWindow) -> Result<Rc<WorkspaceController>, AppError> {
    let deps = Deps {
        collections: Rc::new(CollectionDir),
        environments: Rc::new(EnvironmentDir),
        sender: Arc::new(ReqwestSender::new()?),
        session: Rc::new(AppStateFile::in_config_dir()),
        dynamic_var: data::dynamic_vars::resolve,
        pretty_json: data::http::pretty_json,
        pick_file,
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
