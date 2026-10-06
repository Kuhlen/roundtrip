//! Collection, environments and tree: what is open and which request is active.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use domain::collection::{Collection, Node};
use domain::environment::Scope;
use domain::http::Method;
use domain::session::Session;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::send_flow::reset_response;
use super::workspace_controller::{Active, PendingAction, WorkspaceController};
use super::workspace_rules::{self, FlatRow, RowKind};
use crate::ui::{SendStatus, TreeRow, WorkspaceState};

impl WorkspaceController {
    pub(super) fn show_collection(&self, collection: Collection, environment: Option<&str>) {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        // stale banner from the previous open; load_environments may set a new one
        self.dismiss_banner();
        if s.get_send_status() != SendStatus::Sending {
            reset_response(&s);
        }
        s.set_has_collection(true);
        s.set_settings_open(false);
        s.set_collection_name(collection.name.as_str().into());
        s.set_collection_path(collection.path.display().to_string().into());
        *self.collection.borrow_mut() = Some(collection);
        self.collapsed.borrow_mut().clear();
        *self.editing.borrow_mut() = None;
        self.clear_request(&s);
        self.refresh_tree();
        self.load_environments(environment);
        self.save_session();
    }

    pub(super) fn ask_open_collection(&self) {
        if self.is_dirty() {
            self.ask(PendingAction::OpenCollection);
        } else {
            self.pick_collection();
        }
    }

    pub(super) fn pick_collection(&self) {
        // sync dialog on the UI thread
        if let Some(dir) = rfd::FileDialog::new()
            .set_title("Open collection")
            .pick_folder()
        {
            self.open_collection(&dir);
        }
    }

    pub(super) fn load_environments(&self, preferred: Option<&str>) {
        let Some(root) = self.root() else { return };
        let envs = self.deps.environments.list(&root).unwrap_or_else(|e| {
            self.banner(&e);
            Vec::new()
        });
        let names: Vec<SharedString> = std::iter::once("No environment".into())
            .chain(envs.iter().map(|e| match e.scope {
                Scope::Shared => e.name.as_str().into(),
                Scope::Personal => format!("{} (personal)", e.name).into(),
            }))
            .collect();
        let index = preferred
            .and_then(|p| envs.iter().position(|e| e.name == p))
            .map_or(0, |i| i + 1);
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        s.set_environments(ModelRc::new(VecModel::from(names)));
        s.set_environment_index(index as i32);
        *self.envs.borrow_mut() = envs;
        self.refresh_vars();
    }

    fn active_environment(&self) -> Option<String> {
        let index = self.ui().global::<WorkspaceState>().get_environment_index();
        let i = usize::try_from(index).ok()?.checked_sub(1)?;
        self.envs.borrow().get(i).map(|e| e.name.clone())
    }

    pub(super) fn environment_selected(&self) {
        self.refresh_vars();
        self.save_session();
        self.changed();
    }

    pub(super) fn refresh_vars(&self) {
        let vars = match (self.root(), self.active_environment()) {
            (Some(root), Some(name)) => self
                .deps
                .environments
                .resolve(&root, &name)
                .unwrap_or_else(|e| {
                    self.banner(&e);
                    HashMap::new()
                }),
            _ => HashMap::new(),
        };
        *self.vars.borrow_mut() = vars;
    }

    pub(super) fn lookup(&self, name: &str) -> Option<String> {
        self.vars
            .borrow()
            .get(name)
            .cloned()
            .or_else(|| (self.deps.dynamic_var)(name))
    }

    pub(super) fn save_session(&self) {
        self.deps.session.save(&Session {
            last_collection: self.root(),
            last_environment: self.active_environment(),
        });
    }

    pub(super) fn refresh_tree(&self) {
        let rows = self
            .collection
            .borrow()
            .as_ref()
            .map(|c| workspace_rules::flatten(&c.children, &self.collapsed.borrow()))
            .unwrap_or_default();
        let active = self.active.borrow().as_ref().map(|a| a.path.clone());
        let editing = self.editing.borrow().clone();
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        let dirty = s.get_dirty();
        // full rebuild on every toggle/dirty flip; diff rows if big trees stutter
        let tree: Vec<TreeRow> = rows
            .iter()
            .map(|r| {
                let is_active = active.as_deref() == Some(r.path.as_path());
                TreeRow {
                    depth: r.depth as i32,
                    name: r.name.as_str().into(),
                    label: workspace_rules::row_label(&r.kind).into(),
                    folder: matches!(r.kind, RowKind::Folder { .. }),
                    expanded: matches!(r.kind, RowKind::Folder { expanded: true }),
                    active: is_active,
                    dirty: is_active && dirty,
                    editing: editing.as_deref() == Some(r.path.as_path()),
                }
            })
            .collect();
        s.set_tree(ModelRc::new(VecModel::from(tree)));
        s.set_active_row(
            rows.iter()
                .position(|r| Some(r.path.as_path()) == active.as_deref())
                .map_or(-1, |i| i as i32),
        );
        *self.rows.borrow_mut() = rows;
    }

    /// Re-scan after a tree edit; collapsed folders and the active request stay.
    pub(super) fn reload_collection(&self) {
        let Some(root) = self.root() else { return };
        match self.deps.collections.load(&root) {
            Ok(c) => *self.collection.borrow_mut() = Some(c),
            Err(e) => self.banner(&e),
        }
        self.refresh_tree();
    }

    pub(super) fn row(&self, index: i32) -> Option<FlatRow> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.rows.borrow().get(i).cloned())
    }

    pub(super) fn row_clicked(&self, index: i32) {
        // a click may not rebuild the tree; clear the rename row now
        self.rename_cancel();
        let Some(row) = self.row(index) else { return };
        match row.kind {
            RowKind::Folder { .. } => {
                {
                    let mut collapsed = self.collapsed.borrow_mut();
                    if !collapsed.remove(&row.path) {
                        collapsed.insert(row.path);
                    }
                }
                self.refresh_tree();
            }
            RowKind::Request { .. } => self.select_request(row.path),
        }
    }

    pub(super) fn select_request(&self, path: PathBuf) {
        if self
            .active
            .borrow()
            .as_ref()
            .is_some_and(|a| a.path == path)
        {
            return;
        }
        if self.is_dirty() {
            self.ask(PendingAction::Select(path));
        } else {
            self.load_request(&path);
        }
    }

    pub(super) fn load_request(&self, path: &Path) {
        let row = self.rows.borrow().iter().find(|r| r.path == path).cloned();
        let Some(FlatRow {
            name,
            kind: RowKind::Request { protocol, .. },
            ..
        }) = row
        else {
            return;
        };
        match self.deps.collections.read_request(path) {
            Ok(saved) => self.fill(Active {
                path: path.to_path_buf(),
                name,
                protocol,
                saved,
            }),
            // form keeps the previous request
            Err(e) => self.banner(&e),
        }
    }
}

// sidebar label reads the method from the tree, not the saved file
pub(super) fn set_method(nodes: &mut [Node], file: &Path, new: Method) {
    for n in nodes {
        match n {
            Node::Request { method, path, .. } if path == file => *method = new,
            Node::Folder { children, .. } => set_method(children, file, new),
            _ => {}
        }
    }
}
