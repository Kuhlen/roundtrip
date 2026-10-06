//! Open, switch and close tabs; the shown tab's form lives in Slint.

use std::path::PathBuf;

use domain::AppError;
use domain::collection::Protocol;
use domain::http::Request;
use domain::session::Session;
use slint::{ComponentHandle, ModelRc, VecModel};

use super::tab::{FormState, ResponseView, Tab};
use super::tabs_rules::{TabFile, next_active, reorder_target, restore_tabs, untitled_title};
use crate::modules::workspace::workspace_controller::WorkspaceController;
use crate::modules::workspace::workspace_rules::{FlatRow, RowKind};
use crate::ui::{TabItem, WorkspaceState};

impl WorkspaceController {
    /// Focus the tab holding `path`, else read the file into a new tab.
    pub(crate) fn open_tab(&self, path: PathBuf) {
        let open = self
            .tabs
            .borrow()
            .iter()
            .find(|t| t.file.as_ref().is_some_and(|f| f.path == path))
            .map(|t| t.id);
        if let Some(id) = open {
            self.switch_to(id);
            return;
        }
        match self.build_tab(path) {
            Ok(Some(id)) => self.switch_to(id),
            Ok(None) => {}
            // no tab for a broken file
            Err(e) => self.banner(&e),
        }
    }

    /// Read `path` into a new tab without showing it; None when the tree has no such request.
    fn build_tab(&self, path: PathBuf) -> Result<Option<u64>, AppError> {
        let row = self.rows.borrow().iter().find(|r| r.path == path).cloned();
        let Some(FlatRow {
            name,
            kind: RowKind::Request { protocol, .. },
            ..
        }) = row
        else {
            return Ok(None);
        };
        let Some(collection) = self.collection_of(&path) else {
            return Ok(None);
        };
        let saved = self.deps.collections.read_request(&path)?;
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        self.tabs.borrow_mut().push(Tab {
            id,
            file: Some(TabFile { path, collection }),
            title: name,
            protocol,
            saved,
            snapshot: None,
            dirty: false,
            pinned: false,
            response: ResponseView::default(),
        });
        Ok(Some(id))
    }

    /// Reopen session tabs; unreadable files are skipped silently.
    pub(crate) fn restore_tabs(&self, session: &Session) {
        let roots: Vec<PathBuf> = self
            .collections
            .borrow()
            .iter()
            .map(|c| c.path.clone())
            .collect();
        let (files, active) = restore_tabs(&session.tabs, session.active_tab, &roots, |p| {
            self.deps.collections.read_request(p).is_ok()
        });
        let ids: Vec<Option<u64>> = files
            .into_iter()
            .map(|f| self.build_tab(f.path).ok().flatten())
            .collect();
        // a skipped build shifts nothing: ids stay index-aligned
        if let Some(id) = active.and_then(|i| ids.get(i).copied().flatten()) {
            self.switch_to(id);
        } else {
            self.push_tabs();
        }
    }

    pub(crate) fn toggle_pin(&self, index: usize) {
        {
            let mut tabs = self.tabs.borrow_mut();
            if index >= tabs.len() {
                return;
            }
            let mut tab = tabs.remove(index);
            tab.pinned = !tab.pinned;
            let at = tabs.iter().filter(|t| t.pinned).count();
            tabs.insert(at, tab);
        }
        self.push_tabs();
        self.save_session();
    }

    pub(crate) fn move_tab(&self, from: usize, to: usize) {
        {
            let mut tabs = self.tabs.borrow_mut();
            if from >= tabs.len() {
                return;
            }
            let pins: Vec<bool> = tabs.iter().map(|t| t.pinned).collect();
            let to = reorder_target(&pins, from, to);
            if to == from {
                return;
            }
            let tab = tabs.remove(from);
            tabs.insert(to, tab);
        }
        self.push_tabs();
        self.save_session();
    }

    pub(crate) fn close_others(&self, index: usize) {
        let Some(keep) = self.tab_id(index as i32) else {
            return;
        };
        let ids = self.tab_ids(|t| t.id != keep);
        self.close_tabs(ids);
    }

    pub(crate) fn close_all(&self) {
        let ids = self.tab_ids(|_| true);
        self.close_tabs(ids);
    }

    pub(crate) fn tab_ids(&self, keep: impl Fn(&Tab) -> bool) -> Vec<u64> {
        self.tabs
            .borrow()
            .iter()
            .filter(|t| keep(t))
            .map(|t| t.id)
            .collect()
    }

    /// Ctrl+T: empty scratch tab at the end.
    pub(crate) fn new_tab(&self) {
        let at = self.tabs.borrow().len();
        let id = self.insert_untitled(None, Protocol::Http, at);
        self.switch_to(id);
    }

    /// Scratch copy of tab `index`'s form, right after it; dirty since nothing is saved.
    pub(crate) fn duplicate_tab(&self, index: usize) {
        let Some((id, protocol)) = self.tabs.borrow().get(index).map(|t| (t.id, t.protocol)) else {
            return;
        };
        // shown form is the only complete copy: a never-shown tab has no snapshot
        self.switch_to(id);
        let mut form = FormState::capture(&self.ui().global::<WorkspaceState>(), self);
        // the copy has no collection: relative paths would point nowhere
        if let Some(root) = self.tab_root() {
            form.absolutize(&root);
        }
        let pins = self.tabs.borrow().iter().filter(|t| t.pinned).count();
        let copy = self.insert_untitled(Some(form), protocol, (index + 1).max(pins));
        self.switch_to(copy);
    }

    fn insert_untitled(&self, form: Option<FormState>, protocol: Protocol, at: usize) -> u64 {
        let title = {
            let tabs = self.tabs.borrow();
            let taken: Vec<&str> = tabs.iter().map(|t| t.title.as_str()).collect();
            untitled_title(&taken)
        };
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        self.tabs.borrow_mut().insert(
            at,
            Tab {
                id,
                file: None,
                title,
                protocol,
                saved: Request::default(),
                snapshot: form,
                // show_tab's changed() sets it
                dirty: false,
                pinned: false,
                response: ResponseView::default(),
            },
        );
        id
    }

    /// Flush and snapshot the shown tab, then show `id`.
    pub(crate) fn switch_to(&self, id: u64) {
        if self.active.get() == Some(id) {
            return;
        }
        if self.active.get().is_some() {
            self.flush();
            let ui = self.ui();
            let s = ui.global::<WorkspaceState>();
            // panel may hold a reply that landed while shown; tab.response is stale
            let form = FormState::capture(&s, self);
            let response = ResponseView::capture(&s);
            self.with_active_mut(|t| {
                t.snapshot = Some(form);
                t.response = response;
            });
        }
        self.show_tab(id);
    }

    /// Render tab `id` into the form: its snapshot, else its saved request.
    fn show_tab(&self, id: u64) {
        self.active.set(Some(id));
        let Some((snapshot, response)) =
            self.with_active_mut(|t| (t.snapshot.take(), t.response.clone()))
        else {
            return;
        };
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        s.set_has_request(true);
        match snapshot {
            Some(form) => form.apply(&s, self),
            None => self.fill_saved(&s),
        }
        response.show(&s);
        self.show_header();
        self.changed();
        self.refresh_tree();
        self.push_tabs();
        self.save_session();
    }

    pub(crate) fn close_tab(&self, index: usize) {
        if self
            .tabs
            .borrow()
            .get(index)
            .is_some_and(|t| Some(t.id) == self.active.get())
        {
            self.flush();
        }
        self.remove_tab(index);
    }

    /// Close without saving; the next tab is shown when the active one goes.
    pub(super) fn remove_tab(&self, index: usize) {
        let removed = {
            let mut tabs = self.tabs.borrow_mut();
            (index < tabs.len()).then(|| tabs.remove(index).id)
        };
        let Some(id) = removed else { return };
        if self.active.get() == Some(id) {
            self.autosave.stop();
            // nothing to capture: the tab is gone
            self.active.set(None);
            let next = next_active(self.tabs.borrow().len(), index)
                .and_then(|i| self.tabs.borrow().get(i).map(|t| t.id));
            match next {
                Some(next) => self.show_tab(next),
                None => {
                    let ui = self.ui();
                    self.clear_request(&ui.global::<WorkspaceState>());
                    self.refresh_tree();
                }
            }
        }
        self.push_tabs();
        self.save_session();
    }

    /// Remove every file-backed tab `gone` matches; the active one last, so one render.
    pub(crate) fn remove_tabs_where(&self, gone: impl Fn(&TabFile) -> bool) {
        let active = self.active.get();
        let mut ids: Vec<u64> = self
            .tabs
            .borrow()
            .iter()
            .filter(|t| t.file.as_ref().is_some_and(&gone))
            .map(|t| t.id)
            .collect();
        ids.sort_by_key(|id| Some(*id) == active);
        for id in ids {
            let index = self.tabs.borrow().iter().position(|t| t.id == id);
            if let Some(index) = index {
                self.remove_tab(index);
            }
        }
    }

    pub(crate) fn tab_id(&self, index: i32) -> Option<u64> {
        let i = usize::try_from(index).ok()?;
        self.tabs.borrow().get(i).map(|t| t.id)
    }

    // full rebuild per change; tab counts stay small
    pub(crate) fn push_tabs(&self) {
        let active = self.active.get();
        let items: Vec<TabItem> = self
            .tabs
            .borrow()
            .iter()
            .map(|t| TabItem {
                title: t.title.as_str().into(),
                dirty: t.dirty,
                pinned: t.pinned,
                active: Some(t.id) == active,
                untitled: t.file.is_none(),
            })
            .collect();
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        s.set_active_tab(self.active_index().map_or(-1, |i| i as i32));
        s.set_tabs(ModelRc::new(VecModel::from(items)));
    }

    /// File-backed tabs only: scratch tabs are not restored.
    pub(crate) fn save_session(&self) {
        let active = self.active.get();
        let (tabs, active_tab) = {
            let tabs = self.tabs.borrow();
            let files: Vec<&Tab> = tabs.iter().filter(|t| t.file.is_some()).collect();
            (
                files
                    .iter()
                    .filter_map(|t| t.file.as_ref().map(|f| f.path.clone()))
                    .collect(),
                files.iter().position(|t| Some(t.id) == active),
            )
        };
        self.deps.session.save(&Session {
            collections: self
                .collections
                .borrow()
                .iter()
                .map(|c| c.path.clone())
                .collect(),
            tabs,
            active_tab,
            environment: self.active_environment(),
        });
    }
}
