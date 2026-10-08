//! Collections, environments and tree: what is open and which request is active.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use domain::collection::{Collection, Node};
use domain::environment::Scope;
use domain::http::Method;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::modules::workspace::workspace_controller::{
    PendingAction, WorkspaceController, strings,
};
use crate::modules::workspace::workspace_rules::{self, FlatRow, RowKind};
use crate::ui::{TreeRow, WorkspaceState};

impl WorkspaceController {
    /// Adds `collection`, or refreshes it in place when already open.
    pub(crate) fn show_collection(&self, collection: Collection, environment: Option<&str>) {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        // stale banner from the previous open; load_environments may set a new one
        self.dismiss_banner();
        s.set_has_collection(true);
        s.set_settings_open(false);
        self.collapsed.borrow_mut().remove(&collection.path);
        let first = {
            let mut open = self.collections.borrow_mut();
            match open.iter().position(|c| c.path == collection.path) {
                Some(i) => {
                    open[i] = collection;
                    i == 0
                }
                None => {
                    open.push(collection);
                    open.len() == 1
                }
            }
        };
        *self.editing.borrow_mut() = None;
        self.refresh_tree();
        if first {
            let name = environment
                .map(str::to_owned)
                .or_else(|| self.active_environment());
            self.load_environments(name.as_deref());
            self.changed();
        }
        self.save_session();
    }

    /// Root of the collection row at `index`.
    pub(crate) fn collection_row(&self, index: i32) -> Option<PathBuf> {
        self.row(index)
            .filter(|r| matches!(r.kind, RowKind::Collection { .. }))
            .map(|r| r.path)
    }

    /// Stays open when one of its tabs will not save.
    pub fn close_collection(&self, root: &Path) {
        let ids = self.tab_ids(|t| t.file.as_ref().is_some_and(|f| f.collection == root));
        if !self.advance(PendingAction::CloseTabs(ids)) {
            return;
        }
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        let was_first = self.env_root().as_deref() == Some(root);
        let name = self.active_environment();
        self.collections.borrow_mut().retain(|c| c.path != root);
        self.collapsed.borrow_mut().retain(|p| !p.starts_with(root));
        *self.editing.borrow_mut() = None;
        if self.settings_target.borrow().as_deref() == Some(root) {
            s.set_settings_open(false);
        }
        if self.collections.borrow().is_empty() {
            s.set_has_collection(false);
            s.set_environments(strings(["No environment"]));
            s.set_environment_index(0);
            self.envs.borrow_mut().clear();
            self.refresh_vars();
        } else if was_first {
            self.load_environments(name.as_deref());
        }
        self.refresh_tree();
        if was_first {
            // other collections' requests resolve with the new variables
            self.changed();
        }
        self.save_session();
    }

    pub(crate) fn ask_open_collection(&self) {
        self.flush();
        self.pick_collection();
    }

    pub(crate) fn pick_collection(&self) {
        if let Some(dir) = (self.deps.pick_dir)() {
            self.open_collection(&dir);
        }
    }

    pub(crate) fn load_environments(&self, preferred: Option<&str>) {
        let Some(root) = self.env_root() else { return };
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

    pub(crate) fn active_environment(&self) -> Option<String> {
        let index = self.ui().global::<WorkspaceState>().get_environment_index();
        let i = usize::try_from(index).ok()?.checked_sub(1)?;
        self.envs.borrow().get(i).map(|e| e.name.clone())
    }

    pub(crate) fn environment_selected(&self) {
        self.refresh_vars();
        self.save_session();
        self.changed();
    }

    pub(crate) fn refresh_vars(&self) {
        let vars = match (self.env_root(), self.active_environment()) {
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

    pub(crate) fn lookup(&self, name: &str) -> Option<String> {
        self.vars
            .borrow()
            .get(name)
            .cloned()
            .or_else(|| (self.deps.dynamic_var)(name))
    }

    pub(crate) fn refresh_tree(&self) {
        let rows = workspace_rules::flatten_collections(
            &self.collections.borrow(),
            &self.collapsed.borrow(),
        );
        let active = self.active_path();
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
                    collection: matches!(r.kind, RowKind::Collection { .. }),
                    expanded: matches!(
                        r.kind,
                        RowKind::Folder { expanded: true } | RowKind::Collection { expanded: true }
                    ),
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
    // ponytail: re-scans every open collection; reload only the edited one if big trees lag
    pub(crate) fn reload_collection(&self) {
        let roots: Vec<PathBuf> = self
            .collections
            .borrow()
            .iter()
            .map(|c| c.path.clone())
            .collect();
        for root in roots {
            match self.deps.collections.load(&root) {
                Ok(fresh) => {
                    if let Some(c) = self
                        .collections
                        .borrow_mut()
                        .iter_mut()
                        .find(|c| c.path == root)
                    {
                        *c = fresh;
                    }
                }
                Err(e) => self.banner(&e),
            }
        }
        self.refresh_tree();
    }

    /// File of the active tab; None for none or scratch.
    pub(crate) fn active_path(&self) -> Option<PathBuf> {
        self.with_active(|t| t.file.as_ref().map(|f| f.path.clone()))
            .flatten()
    }

    pub(crate) fn row(&self, index: i32) -> Option<FlatRow> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.rows.borrow().get(i).cloned())
    }

    pub(crate) fn row_clicked(&self, index: i32) {
        // a click may not rebuild the tree; clear the rename row now
        self.rename_cancel();
        let Some(row) = self.row(index) else { return };
        match row.kind {
            RowKind::Collection { .. } | RowKind::Folder { .. } => {
                {
                    let mut collapsed = self.collapsed.borrow_mut();
                    if !collapsed.remove(&row.path) {
                        collapsed.insert(row.path);
                    }
                }
                self.refresh_tree();
            }
            RowKind::Request { .. } => self.open_tab(row.path),
        }
    }
}

// sidebar label reads the method from the tree, not the saved file
pub(crate) fn set_method(nodes: &mut [Node], file: &Path, new: Method) {
    for n in nodes {
        match n {
            Node::Request { method, path, .. } if path == file => *method = new,
            Node::Folder { children, .. } => set_method(children, file, new),
            _ => {}
        }
    }
}
