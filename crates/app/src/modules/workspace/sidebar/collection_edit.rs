//! Edits to the collection itself: settings, create, rename, delete.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use domain::AppError;
use domain::auth::Auth;
use slint::ComponentHandle;

use crate::modules::workspace::auth_fields::{auth_fields, auth_from_fields};
use crate::modules::workspace::tabs::tabs_rules::rebase_path;
use crate::modules::workspace::workspace_controller::{PendingAction, WorkspaceController};
use crate::modules::workspace::workspace_rules::RowKind;
use crate::ui::{DialogKind, WorkspaceState};

impl WorkspaceController {
    /// Auth tab link: settings of the active request's collection.
    pub(crate) fn open_settings(&self) {
        if let Some(root) = self.tab_root() {
            self.settings_for(root);
        }
    }

    pub(crate) fn collection_settings(&self, index: i32) {
        if let Some(root) = self.collection_row(index) {
            self.settings_for(root);
        }
    }

    fn settings_for(&self, root: PathBuf) {
        let auth = self.auth_of(&root);
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        s.set_settings_auth(auth_fields(auth.as_ref()));
        s.set_settings_open(true);
        *self.settings_target.borrow_mut() = Some(root);
    }

    pub(crate) fn auth_of(&self, root: &Path) -> Option<Auth> {
        self.collections
            .borrow()
            .iter()
            .find(|c| c.path == root)
            .and_then(|c| c.auth.clone())
    }

    pub(crate) fn save_settings(&self) {
        let Some(root) = self.settings_target.borrow().clone() else {
            return;
        };
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        let current = self.auth_of(&root);
        let auth = auth_from_fields(&s.get_settings_auth(), current.as_ref());
        match self
            .deps
            .collections
            .save_collection_auth(&root, auth.as_ref())
        {
            Ok(()) => {
                if let Some(c) = self
                    .collections
                    .borrow_mut()
                    .iter_mut()
                    .find(|c| c.path == root)
                {
                    c.auth = auth;
                }
                s.set_settings_open(false);
                self.dismiss_banner();
                // inherit note + Send enablement
                self.changed();
            }
            // dialog stays open
            Err(e) => self.banner(&e),
        }
    }

    pub(crate) fn close_settings(&self) {
        self.ui()
            .global::<WorkspaceState>()
            .set_settings_open(false);
    }

    /// Collection/folder row → inside it; request row → its folder; -1 → first collection.
    fn target_dir(&self, index: i32) -> Option<PathBuf> {
        if index < 0 {
            return self.env_root();
        }
        let row = self.row(index)?;
        match row.kind {
            RowKind::Collection { .. } | RowKind::Folder { .. } => Some(row.path),
            RowKind::Request { .. } => row.path.parent().map(Path::to_path_buf),
        }
    }

    pub(crate) fn create(&self, index: i32, folder: bool) {
        let Some(dir) = self.target_dir(index) else {
            return;
        };
        let base = if folder { "New folder" } else { "New request" };
        let store = &self.deps.collections;
        // ponytail: linear probe, fine until someone keeps 99 "New request N"
        let created = (1..100)
            .map(|n| {
                if n == 1 {
                    base.to_owned()
                } else {
                    format!("{base} {n}")
                }
            })
            .map(|name| {
                if folder {
                    store.create_folder(&dir, &name)
                } else {
                    store.create_request(&dir, &name)
                }
            })
            .find(|r| !matches!(r, Err(AppError::AlreadyExists(_))))
            .unwrap_or_else(|| Err(AppError::AlreadyExists(base.to_owned())));
        match created {
            Ok(path) => {
                self.collapsed.borrow_mut().remove(&dir);
                *self.editing.borrow_mut() = Some(path.clone());
                self.reload_collection();
                if !folder {
                    self.open_tab(path);
                }
            }
            Err(e) => self.banner(&e),
        }
    }

    pub(crate) fn rename_start(&self, index: i32) {
        if let Some(row) = self.row(index) {
            *self.editing.borrow_mut() = Some(row.path);
            self.refresh_tree();
        }
    }

    pub(crate) fn rename_cancel(&self) {
        if self.editing.borrow_mut().take().is_some() {
            self.refresh_tree();
        }
    }

    pub(crate) fn rename_commit(&self, text: &str) {
        let Some(old) = self.editing.borrow_mut().take() else {
            return;
        };
        let unchanged = self
            .rows
            .borrow()
            .iter()
            .any(|r| r.path == old && r.name == text.trim());
        if unchanged {
            self.refresh_tree();
            return;
        }
        self.flush();
        match self.deps.collections.rename(&old, text) {
            Ok(new) => {
                self.rebase(&old, &new, text.trim());
                self.reload_collection();
            }
            // disk may have changed anyway
            Err(e) => {
                self.banner(&e);
                self.reload_collection();
            }
        }
    }

    /// Open tabs and collapsed folders follow a renamed path.
    fn rebase(&self, old: &Path, new: &Path, name: &str) {
        let collapsed: HashSet<PathBuf> = self
            .collapsed
            .borrow()
            .iter()
            .map(|p| rebase_path(p, old, new).unwrap_or_else(|| p.clone()))
            .collect();
        *self.collapsed.borrow_mut() = collapsed;
        let mut header = false;
        for t in self.tabs.borrow_mut().iter_mut() {
            if let Some(f) = t.file.as_mut()
                && let Some(path) = rebase_path(&f.path, old, new)
            {
                if f.path == old {
                    t.title = name.to_owned();
                }
                f.path = path;
                header |= self.active.get() == Some(t.id);
            }
        }
        if header {
            self.show_header();
        }
        self.push_tabs();
        self.save_session();
    }

    pub(crate) fn ask_delete(&self, index: i32) {
        let Some(row) = self.row(index) else { return };
        let root = self.collection_of(&row.path).unwrap_or_default();
        let rel = row.path.strip_prefix(&root).unwrap_or(&row.path);
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        s.set_dialog_kind(DialogKind::Delete);
        s.set_confirm_name(row.name.as_str().into());
        s.set_confirm_file(rel.display().to_string().into());
        s.set_confirm_folder(matches!(row.kind, RowKind::Folder { .. }));
        s.set_confirm_open(true);
        *self.pending.borrow_mut() = Some(PendingAction::Delete(row.path));
    }

    /// Tabs under `path` close unsaved: their files are gone.
    pub(crate) fn delete(&self, path: &Path) {
        match self.deps.collections.delete(path) {
            Ok(()) => {
                self.remove_tabs_where(|f| f.path.starts_with(path));
                self.reload_collection();
            }
            // partial remove_dir_all still changed disk
            Err(e) => {
                self.banner(&e);
                self.reload_collection();
            }
        }
    }
}
