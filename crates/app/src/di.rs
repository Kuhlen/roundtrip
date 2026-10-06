//! composition root

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
    };
    let controller = WorkspaceController::new(deps, ui);
    controller.restore();
    Ok(controller)
}
