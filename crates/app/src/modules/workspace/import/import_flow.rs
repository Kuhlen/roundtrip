//! Pick a Postman file, preview it, write it under a picked folder, open it.

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::modules::workspace::workspace_controller::WorkspaceController;
use crate::modules::workspace::workspace_rules::{error_text, import_counts, import_warnings};
use crate::ui::WorkspaceState;

impl WorkspaceController {
    pub(crate) fn import_collection(&self) {
        self.flush();
        let Some(file) = (self.deps.pick_json)() else {
            return;
        };
        let data = match (self.deps.read_postman)(&file) {
            Ok(data) => data,
            Err(e) => {
                self.banner(&e);
                return;
            }
        };
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        let lines: Vec<SharedString> = import_warnings(&data.warnings)
            .into_iter()
            .map(SharedString::from)
            .collect();
        s.set_import_name(data.name.as_str().into());
        s.set_import_counts(import_counts(data.counts()).into());
        s.set_import_warnings(ModelRc::new(VecModel::from(lines)));
        s.set_import_error("".into());
        *self.pending_import.borrow_mut() = Some(data);
        s.set_import_open(true);
    }

    /// Error stays in the dialog: rename and retry without picking the file again.
    pub(crate) fn import_confirm(&self) {
        let Some(mut data) = self.pending_import.borrow_mut().take() else {
            return;
        };
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        let Some(parent) = (self.deps.pick_dir)() else {
            *self.pending_import.borrow_mut() = Some(data);
            return;
        };
        data.name = s.get_import_name().trim().to_owned();
        match self.deps.collections.create_collection(&parent, &data) {
            Ok(root) => {
                self.import_cancel();
                self.open_collection(&root);
            }
            Err(e) => {
                let (title, hint) = error_text(&e, None);
                let text = if hint.is_empty() {
                    title
                } else {
                    format!("{title}. {hint}")
                };
                s.set_import_error(text.into());
                *self.pending_import.borrow_mut() = Some(data);
            }
        }
    }

    pub(crate) fn import_cancel(&self) {
        *self.pending_import.borrow_mut() = None;
        self.ui().global::<WorkspaceState>().set_import_open(false);
    }
}
