//! Workspace page: open collections, edit one request, send it, save it.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::{Arc, mpsc};

use domain::AppError;
use domain::collection::{Collection, CollectionStore};
use domain::environment::{Environment, EnvironmentStore};
use domain::graphql::GraphqlBody;
use domain::history::HistoryStore;
use domain::http::{Body, HttpSender, Method};
use domain::session::SessionStore;
use slint::{CloseRequestResponse, ComponentHandle, ModelRc, SharedString, VecModel};

use super::send_flow::Delivery;
use super::tabs::tab::Tab;
use super::workspace_rules::{self, FlatRow};
use crate::ui::{AppWindow, AuthFields, DialogKind, KvRow, WorkspaceState};

/// Concrete types are wired in di.rs.
pub struct Deps {
    pub collections: Rc<dyn CollectionStore>,
    pub environments: Rc<dyn EnvironmentStore>,
    pub sender: Arc<dyn HttpSender>,
    pub session: Rc<dyn SessionStore>,
    /// Err: history.db could not be opened; the panel shows why, Send still works
    pub history: Result<Arc<dyn HistoryStore>, AppError>,
    /// `$uuid` & co, tried after environment variables
    pub dynamic_var: fn(&str) -> Option<String>,
    pub pretty_json: fn(&str) -> Option<String>,
    /// sync file dialog, starting in the collection folder
    pub pick_file: fn(&Path) -> Option<PathBuf>,
    pub graphql_parse: fn(&str) -> Option<GraphqlBody>,
    pub graphql_json: fn(&GraphqlBody) -> Result<String, AppError>,
}

pub(crate) enum PendingAction {
    /// tab ids still to close, in order
    CloseTabs(Vec<u64>),
    /// tab ids still to check; the window hides once all pass
    CloseWindow(Vec<u64>),
    Delete(PathBuf),
}

pub struct WorkspaceController {
    pub(super) deps: Deps,
    pub(super) ui: slint::Weak<AppWindow>,
    /// open order; the first one owns the environments
    pub(super) collections: RefCell<Vec<Collection>>,
    // collection whose settings dialog is open
    pub(super) settings_target: RefCell<Option<PathBuf>>,
    pub(super) collapsed: RefCell<HashSet<PathBuf>>,
    pub(super) rows: RefCell<Vec<FlatRow>>,
    pub(super) envs: RefCell<Vec<Environment>>,
    // cached since the last environment change or Send
    pub(super) vars: RefCell<HashMap<String, String>>,
    pub(crate) tabs: RefCell<Vec<Tab>>,
    /// id of the shown tab
    pub(crate) active: Cell<Option<u64>>,
    pub(crate) next_id: Cell<u64>,
    /// last send number; a result carrying an older one is stale
    pub(super) send_seq: Cell<u64>,
    /// ids of the shown history rows, by row index
    pub(super) history_ids: RefCell<Vec<i64>>,
    pub(super) me: Weak<WorkspaceController>,
    pub(super) autosave: slint::Timer,
    pub(super) pending: RefCell<Option<PendingAction>>,
    // row in rename mode
    pub(super) editing: RefCell<Option<PathBuf>>,
    pub(super) params: Rc<VecModel<KvRow>>,
    pub(super) headers: Rc<VecModel<KvRow>>,
    pub(super) form_rows: Rc<VecModel<KvRow>>,
    /// send results from worker threads, drained on response-ready
    pub(super) responses: (mpsc::Sender<Delivery>, RefCell<mpsc::Receiver<Delivery>>),
}

impl WorkspaceController {
    pub fn new(deps: Deps, ui: &AppWindow) -> Rc<Self> {
        let (tx, rx) = mpsc::channel();
        let this = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            autosave: slint::Timer::default(),
            deps,
            ui: ui.as_weak(),
            collections: RefCell::default(),
            settings_target: RefCell::default(),
            collapsed: RefCell::default(),
            rows: RefCell::default(),
            envs: RefCell::default(),
            vars: RefCell::default(),
            tabs: RefCell::default(),
            active: Cell::default(),
            next_id: Cell::default(),
            send_seq: Cell::default(),
            history_ids: RefCell::default(),
            pending: RefCell::default(),
            editing: RefCell::default(),
            params: Rc::new(VecModel::default()),
            headers: Rc::new(VecModel::default()),
            form_rows: Rc::new(VecModel::default()),
            responses: (tx, RefCell::new(rx)),
        });
        // globals outlive pages: reset everything Rust owns
        let s = ui.global::<WorkspaceState>();
        s.set_methods(strings(Method::ALL.iter().map(|m| m.as_str())));
        s.set_body_kinds(strings(Body::KINDS.iter().copied()));
        s.set_params(ModelRc::from(this.params.clone()));
        s.set_headers(ModelRc::from(this.headers.clone()));
        s.set_form_rows(ModelRc::from(this.form_rows.clone()));
        s.set_has_collection(false);
        s.set_settings_open(false);
        s.set_settings_auth(AuthFields::default());
        s.set_environments(strings(["No environment"]));
        s.set_environment_index(0);
        s.set_tree(ModelRc::default());
        s.set_banner_title("".into());
        s.set_banner_hint("".into());
        s.set_pretty(true);
        s.set_editor_tab(0);
        s.set_response_tab(0);
        s.set_confirm_name("".into());
        s.set_confirm_file("".into());
        s.set_dialog_kind(DialogKind::Untitled);
        s.set_confirm_folder(false);
        s.set_active_row(-1);
        s.set_show_history(false);
        s.set_history_query("".into());
        s.set_save_as_open(false);
        s.set_save_as_name("".into());
        s.set_save_as_collections(ModelRc::default());
        s.set_save_as_collection_index(0);
        s.set_save_as_folders(ModelRc::default());
        s.set_save_as_folder_index(0);
        s.set_save_as_error("".into());
        this.clear_request(&s);
        this.push_tabs();
        this.wire(&s);
        this.load_history();
        let weak = this.me.clone();
        ui.window().on_close_requested(move || {
            weak.upgrade()
                .map_or(CloseRequestResponse::HideWindow, |c| c.close_requested())
        });
        this
    }

    fn wire(&self, s: &WorkspaceState) {
        let weak = self.me.clone();
        let on = |f: fn(&Self)| {
            let weak = weak.clone();
            move || {
                if let Some(c) = weak.upgrade() {
                    f(&c);
                }
            }
        };
        s.on_open_collection(on(Self::ask_open_collection));
        s.on_environment_selected(on(Self::environment_selected));
        s.on_changed(on(Self::changed));
        s.on_send(on(Self::send));
        s.on_history_refresh(on(Self::load_history));
        s.on_history_clear(on(Self::ask_clear_history));
        s.on_confirm_clear_history(on(Self::confirm_clear_history));
        s.on_cancel(on(Self::cancel));
        s.on_response_ready(on(Self::deliver_responses));
        s.on_save(on(|c| {
            c.save();
        }));
        s.on_dismiss_banner(on(Self::dismiss_banner));
        s.on_confirm_cancel(on(Self::confirm_cancel));
        s.on_confirm_save_as(on(Self::confirm_save_as));
        s.on_confirm_discard(on(Self::confirm_discard));
        s.on_save_as_confirm(on(Self::save_as_confirm));
        s.on_save_as_cancel(on(Self::save_as_cancel));
        s.on_save_as_collection_selected(on(Self::save_as_collection_selected));
        s.on_tab_new(on(Self::new_tab));
        s.on_open_settings(on(Self::open_settings));
        s.on_save_settings(on(Self::save_settings));
        s.on_close_settings(on(Self::close_settings));
        let on_index = |f: fn(&Self, i32)| {
            let weak = weak.clone();
            move |i: i32| {
                if let Some(c) = weak.upgrade() {
                    f(&c, i);
                }
            }
        };
        s.on_row_clicked(on_index(Self::row_clicked));
        s.on_history_clicked(on_index(Self::open_history));
        s.on_tab_clicked(on_index(|c, i| {
            if let Some(id) = c.tab_id(i) {
                c.switch_to(id);
            }
        }));
        s.on_tab_close(on_index(|c, i| {
            if let Some(id) = c.tab_id(i) {
                c.close_tabs(vec![id]);
            }
        }));
        s.on_tab_duplicate(on_index(|c, i| {
            if let Ok(i) = usize::try_from(i) {
                c.duplicate_tab(i);
            }
        }));
        s.on_tab_pin(on_index(|c, i| {
            if let Ok(i) = usize::try_from(i) {
                c.toggle_pin(i);
            }
        }));
        s.on_tab_close_others(on_index(|c, i| {
            if let Ok(i) = usize::try_from(i) {
                c.close_others(i);
            }
        }));
        s.on_tab_close_all(on(Self::close_all));
        let weak_moved = weak.clone();
        s.on_tab_moved(move |from, to| {
            if let (Some(c), Ok(from), Ok(to)) = (
                weak_moved.upgrade(),
                usize::try_from(from),
                usize::try_from(to),
            ) {
                c.move_tab(from, to);
            }
        });
        s.on_collection_settings(on_index(Self::collection_settings));
        s.on_close_collection(on_index(|c, i| {
            if let Some(root) = c.collection_row(i) {
                c.close_collection(&root);
            }
        }));
        s.on_new_request(on_index(|c, i| c.create(i, false)));
        s.on_new_folder(on_index(|c, i| c.create(i, true)));
        s.on_rename_start(on_index(Self::rename_start));
        s.on_delete_row(on_index(Self::ask_delete));
        s.on_pick_file(on_index(Self::pick_file));
        s.on_rename_cancel(on(Self::rename_cancel));
        s.on_confirm_delete(on(Self::confirm_delete));
        s.on_rename_commit({
            let weak = weak.clone();
            move |text| {
                if let Some(c) = weak.upgrade() {
                    c.rename_commit(&text);
                }
            }
        });
        s.on_kv_edited({
            let weak = weak.clone();
            move |table, index| {
                if let Some(c) = weak.upgrade() {
                    c.kv_edited(table, index);
                }
            }
        });
        s.on_kv_removed({
            let weak = weak.clone();
            move |table, index| {
                if let Some(c) = weak.upgrade() {
                    c.kv_removed(table, index);
                }
            }
        });
    }

    pub(super) fn ui(&self) -> AppWindow {
        self.ui.upgrade().expect("window outlives controller")
    }

    /// Environments always come from the first open collection, as in ApiArk.
    pub(crate) fn env_root(&self) -> Option<PathBuf> {
        self.collections.borrow().first().map(|c| c.path.clone())
    }

    /// Deepest open root containing `path`.
    pub(crate) fn collection_of(&self, path: &Path) -> Option<PathBuf> {
        self.collections
            .borrow()
            .iter()
            .map(|c| &c.path)
            .filter(|r| path.starts_with(r))
            .max_by_key(|r| r.components().count())
            .cloned()
    }

    pub(crate) fn active_index(&self) -> Option<usize> {
        let id = self.active.get()?;
        self.tabs.borrow().iter().position(|t| t.id == id)
    }

    pub(crate) fn with_active<R>(&self, f: impl FnOnce(&Tab) -> R) -> Option<R> {
        let i = self.active_index()?;
        self.tabs.borrow().get(i).map(f)
    }

    pub(crate) fn with_active_mut<R>(&self, f: impl FnOnce(&mut Tab) -> R) -> Option<R> {
        let i = self.active_index()?;
        self.tabs.borrow_mut().get_mut(i).map(f)
    }

    /// Collection of the active tab's file.
    pub(crate) fn tab_root(&self) -> Option<PathBuf> {
        self.with_active(|t| t.file.as_ref().map(|f| f.collection.clone()))
            .flatten()
    }

    /// Startup: reopen the last collections + environment; silent for gone ones.
    pub fn restore(&self) {
        let session = self.deps.session.load();
        for dir in &session.collections {
            if let Ok(collection) = self.deps.collections.load(dir) {
                self.show_collection(collection, session.environment.as_deref());
            }
        }
        self.restore_tabs(&session);
    }

    pub fn open_collection(&self, dir: &Path) {
        match self.deps.collections.load(dir) {
            Ok(collection) => self.show_collection(collection, None),
            // open collections stay as they are
            Err(e) => self.banner(&e),
        }
    }

    /// Quit: every tab saved or answered first; a dialog or failed save keeps the window.
    pub fn close_requested(&self) -> CloseRequestResponse {
        // an open dialog owns `pending`: a close sequence would overwrite it
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        if s.get_confirm_open() || s.get_save_as_open() || s.get_settings_open() {
            return CloseRequestResponse::KeepWindowShown;
        }
        let ids = self.tabs.borrow().iter().map(|t| t.id).collect();
        if self.advance(PendingAction::CloseWindow(ids)) {
            CloseRequestResponse::HideWindow
        } else {
            CloseRequestResponse::KeepWindowShown
        }
    }

    pub(super) fn close_dialog(&self) -> Option<PendingAction> {
        self.ui().global::<WorkspaceState>().set_confirm_open(false);
        self.pending.borrow_mut().take()
    }

    fn confirm_delete(&self) {
        if let Some(PendingAction::Delete(path)) = self.close_dialog() {
            self.delete(&path);
        }
    }

    /// Also stops a close sequence: the remaining tabs stay open.
    fn confirm_cancel(&self) {
        self.close_dialog();
    }

    pub(super) fn banner(&self, e: &AppError) {
        let root = match e {
            AppError::MergeConflict(file) => self.collection_of(Path::new(file)),
            _ => None,
        };
        let (title, hint) = workspace_rules::error_text(e, root.as_deref());
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        s.set_banner_title(title.into());
        s.set_banner_hint(hint.into());
    }

    pub(super) fn dismiss_banner(&self) {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        s.set_banner_title("".into());
        s.set_banner_hint("".into());
    }
}

pub(super) fn strings<'a>(items: impl IntoIterator<Item = &'a str>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        items
            .into_iter()
            .map(SharedString::from)
            .collect::<Vec<_>>(),
    ))
}
