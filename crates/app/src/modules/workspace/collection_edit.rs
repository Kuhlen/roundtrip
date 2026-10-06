//! Edits to the collection itself: settings, create, rename, delete.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use domain::AppError;
use slint::ComponentHandle;

use super::auth_fields::{auth_fields, auth_from_fields};
use super::workspace_controller::{PendingAction, WorkspaceController};
use super::workspace_rules::RowKind;
use crate::ui::{DialogKind, WorkspaceState};

impl WorkspaceController {
    pub(super) fn open_settings(&self) {
        let auth = self
            .collection
            .borrow()
            .as_ref()
            .and_then(|c| c.auth.clone());
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        s.set_settings_auth(auth_fields(auth.as_ref()));
        s.set_settings_open(true);
    }

    pub(super) fn save_settings(&self) {
        let Some(root) = self.root() else { return };
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        let current = self
            .collection
            .borrow()
            .as_ref()
            .and_then(|c| c.auth.clone());
        let auth = auth_from_fields(&s.get_settings_auth(), current.as_ref());
        match self
            .deps
            .collections
            .save_collection_auth(&root, auth.as_ref())
        {
            Ok(()) => {
                if let Some(c) = self.collection.borrow_mut().as_mut() {
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

    pub(super) fn close_settings(&self) {
        self.ui()
            .global::<WorkspaceState>()
            .set_settings_open(false);
    }

    /// Folder row → inside it; request row → its folder; -1 → collection root.
    fn target_dir(&self, index: i32) -> Option<PathBuf> {
        if index < 0 {
            return self.root();
        }
        let row = self.row(index)?;
        match row.kind {
            RowKind::Folder { .. } => Some(row.path),
            RowKind::Request { .. } => row.path.parent().map(Path::to_path_buf),
        }
    }

    pub(super) fn create(&self, index: i32, folder: bool) {
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
                    self.select_request(path);
                }
            }
            Err(e) => self.banner(&e),
        }
    }

    pub(super) fn rename_start(&self, index: i32) {
        if let Some(row) = self.row(index) {
            *self.editing.borrow_mut() = Some(row.path);
            self.refresh_tree();
        }
    }

    pub(super) fn rename_cancel(&self) {
        if self.editing.borrow_mut().take().is_some() {
            self.refresh_tree();
        }
    }

    pub(super) fn rename_commit(&self, text: &str) {
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

    /// Active request and collapsed folders follow a renamed path.
    fn rebase(&self, old: &Path, new: &Path, name: &str) {
        let moved = |p: &Path| {
            p.strip_prefix(old).ok().map(|rest| {
                // join("") would add a trailing slash
                if rest.as_os_str().is_empty() {
                    new.to_path_buf()
                } else {
                    new.join(rest)
                }
            })
        };
        let collapsed: HashSet<PathBuf> = self
            .collapsed
            .borrow()
            .iter()
            .map(|p| moved(p).unwrap_or_else(|| p.clone()))
            .collect();
        *self.collapsed.borrow_mut() = collapsed;
        let header = match self.active.borrow_mut().as_mut() {
            Some(a) => match moved(&a.path) {
                Some(path) => {
                    if a.path == old {
                        a.name = name.to_owned();
                    }
                    a.path = path;
                    true
                }
                None => false,
            },
            None => false,
        };
        if header {
            self.show_header();
        }
    }

    pub(super) fn ask_delete(&self, index: i32) {
        let Some(row) = self.row(index) else { return };
        let root = self.root().unwrap_or_default();
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

    /// No unsaved prompt for the active request: its file is gone.
    pub(super) fn delete(&self, path: &Path) {
        match self.deps.collections.delete(path) {
            Ok(()) => {
                let gone = self
                    .active
                    .borrow()
                    .as_ref()
                    .is_some_and(|a| a.path.starts_with(path));
                if gone {
                    let ui = self.ui();
                    self.clear_request(&ui.global::<WorkspaceState>());
                }
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
