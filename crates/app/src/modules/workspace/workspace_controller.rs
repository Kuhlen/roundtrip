//! Workspace page: open a collection, edit one request, send it, save it.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use domain::AppError;
use domain::collection::{Collection, CollectionStore, Protocol};
use domain::environment::{Environment, EnvironmentStore};
use domain::http::{BodyKind, HttpSender, Method, Request};
use domain::session::SessionStore;
use slint::{CloseRequestResponse, ComponentHandle, ModelRc, SharedString, VecModel};

use super::send_flow::reset_response;
use super::workspace_rules::{self, FlatRow};
use crate::ui::{AppWindow, AuthFields, DialogKind, KvRow, WorkspaceState};

/// Concrete types are wired in di.rs.
pub struct Deps {
    pub collections: Rc<dyn CollectionStore>,
    pub environments: Rc<dyn EnvironmentStore>,
    pub sender: Arc<dyn HttpSender>,
    pub session: Rc<dyn SessionStore>,
    /// `$uuid` & co, tried after environment variables
    pub dynamic_var: fn(&str) -> Option<String>,
    pub pretty_json: fn(&str) -> Option<String>,
}

pub(super) enum PendingAction {
    Select(PathBuf),
    OpenCollection,
    CloseWindow,
    Delete(PathBuf),
}

#[derive(Clone)]
pub(super) struct Active {
    pub(super) path: PathBuf,
    pub(super) name: String,
    pub(super) protocol: Protocol,
    pub(super) saved: Request,
}

pub struct WorkspaceController {
    pub(super) deps: Deps,
    pub(super) ui: slint::Weak<AppWindow>,
    pub(super) collection: RefCell<Option<Collection>>,
    pub(super) collapsed: RefCell<HashSet<PathBuf>>,
    pub(super) rows: RefCell<Vec<FlatRow>>,
    pub(super) envs: RefCell<Vec<Environment>>,
    // cached since the last environment change or Send
    pub(super) vars: RefCell<HashMap<String, String>>,
    pub(super) active: RefCell<Option<Active>>,
    pub(super) pending: RefCell<Option<PendingAction>>,
    // row in rename mode
    pub(super) editing: RefCell<Option<PathBuf>>,
    pub(super) params: Rc<VecModel<KvRow>>,
    pub(super) headers: Rc<VecModel<KvRow>>,
}

impl WorkspaceController {
    pub fn new(deps: Deps, ui: &AppWindow) -> Rc<Self> {
        let this = Rc::new(Self {
            deps,
            ui: ui.as_weak(),
            collection: RefCell::default(),
            collapsed: RefCell::default(),
            rows: RefCell::default(),
            envs: RefCell::default(),
            vars: RefCell::default(),
            active: RefCell::default(),
            pending: RefCell::default(),
            editing: RefCell::default(),
            params: Rc::new(VecModel::default()),
            headers: Rc::new(VecModel::default()),
        });
        // globals outlive pages: reset everything Rust owns
        let s = ui.global::<WorkspaceState>();
        s.set_methods(strings(Method::ALL.iter().map(|m| m.as_str())));
        s.set_body_kinds(strings(BodyKind::EDITABLE.iter().map(BodyKind::as_str)));
        s.set_params(ModelRc::from(this.params.clone()));
        s.set_headers(ModelRc::from(this.headers.clone()));
        s.set_has_collection(false);
        s.set_settings_open(false);
        s.set_settings_auth(AuthFields::default());
        s.set_collection_name("".into());
        s.set_collection_path("".into());
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
        s.set_dialog_kind(DialogKind::Unsaved);
        s.set_confirm_folder(false);
        s.set_active_row(-1);
        reset_response(&s);
        this.clear_request(&s);
        this.wire(&s);
        let weak = Rc::downgrade(&this);
        ui.window().on_close_requested(move || {
            weak.upgrade()
                .map_or(CloseRequestResponse::HideWindow, |c| c.close_requested())
        });
        this
    }

    fn wire(self: &Rc<Self>, s: &WorkspaceState) {
        let weak = Rc::downgrade(self);
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
        s.on_save(on(|c| {
            c.save();
        }));
        s.on_dismiss_banner(on(Self::dismiss_banner));
        s.on_confirm_save(on(Self::confirm_save));
        s.on_confirm_discard(on(Self::confirm_discard));
        s.on_confirm_cancel(on(Self::confirm_cancel));
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
        s.on_new_request(on_index(|c, i| c.create(i, false)));
        s.on_new_folder(on_index(|c, i| c.create(i, true)));
        s.on_rename_start(on_index(Self::rename_start));
        s.on_delete_row(on_index(Self::ask_delete));
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

    pub(super) fn root(&self) -> Option<PathBuf> {
        self.collection.borrow().as_ref().map(|c| c.path.clone())
    }

    /// Startup: reopen the last collection + environment; silent when gone.
    pub fn restore(&self) {
        let session = self.deps.session.load();
        let Some(dir) = session.last_collection else {
            return;
        };
        if let Ok(collection) = self.deps.collections.load(&dir) {
            self.show_collection(collection, session.last_environment.as_deref());
        }
    }

    pub fn open_collection(&self, dir: &Path) {
        match self.deps.collections.load(dir) {
            Ok(collection) => self.show_collection(collection, None),
            // previous collection stays open
            Err(e) => self.banner(&e),
        }
    }

    pub fn close_requested(&self) -> CloseRequestResponse {
        if self.is_dirty() {
            self.ask(PendingAction::CloseWindow);
            CloseRequestResponse::KeepWindowShown
        } else {
            CloseRequestResponse::HideWindow
        }
    }

    pub(super) fn ask(&self, action: PendingAction) {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        let Some(name) = self.active.borrow().as_ref().map(|a| a.name.clone()) else {
            return;
        };
        s.set_confirm_name(name.into());
        s.set_confirm_file(s.get_request_file());
        s.set_dialog_kind(DialogKind::Unsaved);
        s.set_confirm_open(true);
        *self.pending.borrow_mut() = Some(action);
    }

    fn close_dialog(&self) -> Option<PendingAction> {
        self.ui().global::<WorkspaceState>().set_confirm_open(false);
        self.pending.borrow_mut().take()
    }

    /// A failed save cancels the pending action.
    fn confirm_save(&self) {
        let action = self.close_dialog();
        if self.save()
            && let Some(action) = action
        {
            self.run(action);
        }
    }

    fn confirm_discard(&self) {
        let action = self.close_dialog();
        let saved = self.active.borrow().clone();
        if let Some(saved) = saved {
            self.fill(saved);
        }
        if let Some(action) = action {
            self.run(action);
        }
    }

    fn confirm_delete(&self) {
        if let Some(action) = self.close_dialog() {
            self.run(action);
        }
    }

    fn confirm_cancel(&self) {
        self.close_dialog();
    }

    fn run(&self, action: PendingAction) {
        match action {
            PendingAction::Select(path) => self.load_request(&path),
            PendingAction::OpenCollection => self.pick_collection(),
            PendingAction::CloseWindow => {
                let _ = self.ui().hide();
            }
            PendingAction::Delete(path) => self.delete(&path),
        }
    }

    pub(super) fn banner(&self, e: &AppError) {
        let (title, hint) = workspace_rules::error_text(e, self.root().as_deref());
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
