//! Save as: write the active scratch tab into a collection and make it file-backed.

use std::path::PathBuf;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::tabs_rules::TabFile;
use crate::modules::workspace::workspace_controller::{WorkspaceController, strings};
use crate::modules::workspace::workspace_rules::{error_text, folder_choices};
use crate::ui::WorkspaceState;

impl WorkspaceController {
    pub(crate) fn open_save_as(&self) {
        let Some(title) = self.with_active(|t| t.title.clone()) else {
            return;
        };
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        let names: Vec<SharedString> = self
            .collections
            .borrow()
            .iter()
            .map(|c| c.name.as_str().into())
            .collect();
        s.set_save_as_collections(ModelRc::new(VecModel::from(names)));
        s.set_save_as_collection_index(0);
        let name = if title.starts_with("Untitled") {
            ""
        } else {
            title.as_str()
        };
        s.set_save_as_name(name.into());
        s.set_save_as_error("".into());
        self.save_as_collection_selected();
        s.set_save_as_open(true);
    }

    /// (collection root, folder choices) of the selected collection.
    fn save_as_choices(&self, s: &WorkspaceState) -> Option<(PathBuf, Vec<(String, PathBuf)>)> {
        let i = usize::try_from(s.get_save_as_collection_index()).ok()?;
        self.collections
            .borrow()
            .get(i)
            .map(|c| (c.path.clone(), folder_choices(c)))
    }

    pub(crate) fn save_as_collection_selected(&self) {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        let (_, choices) = self.save_as_choices(&s).unwrap_or_default();
        s.set_save_as_folders(strings(choices.iter().map(|(label, _)| label.as_str())));
        s.set_save_as_folder_index(0);
    }

    pub(crate) fn save_as_confirm(&self) {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        let Some((root, choices)) = self.save_as_choices(&s) else {
            return;
        };
        let Some(dir) = usize::try_from(s.get_save_as_folder_index())
            .ok()
            .and_then(|i| choices.into_iter().nth(i))
            .map(|(_, dir)| dir)
        else {
            return;
        };
        let Some(form) = self.with_active(|t| self.form(&s, &t.saved)) else {
            return;
        };
        let name = s.get_save_as_name();
        let path = match self.deps.collections.create_request(&dir, &name) {
            Ok(path) => path,
            Err(e) => {
                s.set_save_as_error(error_text(&e, None).0.into());
                return;
            }
        };
        s.set_save_as_open(false);
        if let Err(e) = self.deps.collections.save_request(&path, &form) {
            // no empty file left behind; the tab keeps its edits
            let _ = self.deps.collections.delete(&path);
            *self.pending.borrow_mut() = None;
            self.reload_collection();
            self.banner(&e);
            return;
        }
        self.with_active_mut(|t| {
            t.file = Some(TabFile {
                path,
                collection: root,
            });
            t.title = name.trim().to_owned();
            t.saved = form;
        });
        self.reload_collection();
        self.show_header();
        self.changed();
        self.push_tabs();
        self.save_session();
        self.resume_pending();
    }

    /// Also stops a close sequence waiting on this tab.
    pub(crate) fn save_as_cancel(&self) {
        self.ui().global::<WorkspaceState>().set_save_as_open(false);
        *self.pending.borrow_mut() = None;
    }
}
