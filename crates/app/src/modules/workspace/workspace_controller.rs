//! Workspace page: open a collection, edit one request, send it, save it.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use domain::AppError;
use domain::collection::{Collection, CollectionStore, Node, Protocol};
use domain::environment::{Environment, EnvironmentStore, Scope};
use domain::http::{BodyKind, HttpSender, KeyValue, Method, Request, Response};
use domain::session::{Session, SessionStore};
use slint::{CloseRequestResponse, ComponentHandle, Model, ModelRc, SharedString, VecModel};

use super::workspace_rules::{self, FlatRow, RowKind};
use crate::ui::{AppWindow, KvRow, KvTable, SendStatus, StatusClass, TreeRow, WorkspaceState};

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

enum PendingAction {
    Select(PathBuf),
    OpenCollection,
    CloseWindow,
}

#[derive(Clone)]
struct Active {
    path: PathBuf,
    name: String,
    protocol: Protocol,
    saved: Request,
}

pub struct WorkspaceController {
    deps: Deps,
    ui: slint::Weak<AppWindow>,
    collection: RefCell<Option<Collection>>,
    collapsed: RefCell<HashSet<PathBuf>>,
    rows: RefCell<Vec<FlatRow>>,
    envs: RefCell<Vec<Environment>>,
    // cached since the last environment change or Send
    vars: RefCell<HashMap<String, String>>,
    active: RefCell<Option<Active>>,
    pending: RefCell<Option<PendingAction>>,
    params: Rc<VecModel<KvRow>>,
    headers: Rc<VecModel<KvRow>>,
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
        s.on_row_clicked({
            let weak = weak.clone();
            move |index| {
                if let Some(c) = weak.upgrade() {
                    c.row_clicked(index);
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

    fn ui(&self) -> AppWindow {
        self.ui.upgrade().expect("window outlives controller")
    }

    fn root(&self) -> Option<PathBuf> {
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

    fn show_collection(&self, collection: Collection, environment: Option<&str>) {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        // stale banner from the previous open; load_environments may set a new one
        self.dismiss_banner();
        if s.get_send_status() != SendStatus::Sending {
            reset_response(&s);
        }
        s.set_has_collection(true);
        s.set_collection_name(collection.name.as_str().into());
        s.set_collection_path(collection.path.display().to_string().into());
        *self.collection.borrow_mut() = Some(collection);
        self.collapsed.borrow_mut().clear();
        self.clear_request(&s);
        self.refresh_tree();
        self.load_environments(environment);
        self.save_session();
    }

    fn clear_request(&self, s: &WorkspaceState) {
        *self.active.borrow_mut() = None;
        *self.pending.borrow_mut() = None;
        s.set_has_request(false);
        s.set_crumb("".into());
        s.set_request_name("".into());
        s.set_request_file("".into());
        s.set_method_index(0);
        s.set_url("".into());
        s.set_resolved_url("".into());
        s.set_body_kind_index(0);
        s.set_body("".into());
        s.set_unsupported_body("".into());
        s.set_unsupported_protocol("".into());
        s.set_param_count(0);
        s.set_header_count(0);
        s.set_dirty(false);
        // in-flight Send keeps Send disabled until its result lands
        if s.get_send_status() != SendStatus::Sending {
            s.set_send_status(SendStatus::Idle);
        }
        s.set_confirm_open(false);
        self.params.set_vec(vec![placeholder()]);
        self.headers.set_vec(vec![placeholder()]);
    }

    fn ask_open_collection(&self) {
        if self.is_dirty() {
            self.ask(PendingAction::OpenCollection);
        } else {
            self.pick_collection();
        }
    }

    fn pick_collection(&self) {
        // sync dialog on the UI thread
        if let Some(dir) = rfd::FileDialog::new()
            .set_title("Open collection")
            .pick_folder()
        {
            self.open_collection(&dir);
        }
    }

    fn load_environments(&self, preferred: Option<&str>) {
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

    fn environment_selected(&self) {
        self.refresh_vars();
        self.save_session();
        self.changed();
    }

    fn refresh_vars(&self) {
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

    fn lookup(&self, name: &str) -> Option<String> {
        self.vars
            .borrow()
            .get(name)
            .cloned()
            .or_else(|| (self.deps.dynamic_var)(name))
    }

    fn save_session(&self) {
        self.deps.session.save(&Session {
            last_collection: self.root(),
            last_environment: self.active_environment(),
        });
    }

    fn refresh_tree(&self) {
        let rows = self
            .collection
            .borrow()
            .as_ref()
            .map(|c| workspace_rules::flatten(&c.children, &self.collapsed.borrow()))
            .unwrap_or_default();
        let active = self.active.borrow().as_ref().map(|a| a.path.clone());
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
                }
            })
            .collect();
        s.set_tree(ModelRc::new(VecModel::from(tree)));
        *self.rows.borrow_mut() = rows;
    }

    fn row_clicked(&self, index: i32) {
        let row = usize::try_from(index)
            .ok()
            .and_then(|i| self.rows.borrow().get(i).cloned());
        let Some(row) = row else { return };
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

    fn select_request(&self, path: PathBuf) {
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

    fn load_request(&self, path: &Path) {
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

    /// Form ← saved request. Last response stays: a late reply never lands on a reset panel.
    fn fill(&self, active: Active) {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        let root = self.root().unwrap_or_default();
        let rel = active.path.strip_prefix(&root).unwrap_or(&active.path);
        let crumb = rel
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| format!("{} /", p.display()));
        let r = &active.saved;
        s.set_has_request(true);
        s.set_request_name(active.name.as_str().into());
        s.set_crumb(crumb.unwrap_or_default().into());
        s.set_request_file(rel.display().to_string().into());
        s.set_method_index(Method::ALL.iter().position(|m| *m == r.method).unwrap_or(0) as i32);
        s.set_url(r.url.as_str().into());
        match &r.body_kind {
            BodyKind::Unsupported(kind) => {
                s.set_unsupported_body(kind.as_str().into());
                s.set_body_kind_index(0);
                s.set_body("".into());
            }
            kind => {
                s.set_unsupported_body("".into());
                s.set_body_kind_index(
                    BodyKind::EDITABLE
                        .iter()
                        .position(|k| k == kind)
                        .unwrap_or(0) as i32,
                );
                s.set_body(r.body.as_str().into());
            }
        }
        let protocol = if active.protocol.is_sendable() {
            ""
        } else {
            workspace_rules::protocol_name(active.protocol)
        };
        s.set_unsupported_protocol(protocol.into());
        self.params.set_vec(with_placeholder(&r.params));
        self.headers.set_vec(with_placeholder(&r.headers));
        *self.active.borrow_mut() = Some(active);
        s.set_dirty(false);
        self.refresh_tree();
        self.changed();
    }

    /// Form → Request. Unsupported bodies come from the file, never from the form.
    fn form(&self, s: &WorkspaceState, a: &Active) -> Request {
        let (body_kind, body) = match &a.saved.body_kind {
            BodyKind::Unsupported(_) => (a.saved.body_kind.clone(), a.saved.body.clone()),
            _ => (
                usize::try_from(s.get_body_kind_index())
                    .ok()
                    .and_then(|i| BodyKind::EDITABLE.get(i))
                    .cloned()
                    .unwrap_or_default(),
                s.get_body().to_string(),
            ),
        };
        Request {
            method: usize::try_from(s.get_method_index())
                .ok()
                .and_then(|i| Method::ALL.get(i))
                .copied()
                .unwrap_or_default(),
            url: s.get_url().to_string(),
            params: kv_rows(&self.params),
            headers: kv_rows(&self.headers),
            body_kind,
            body,
        }
    }

    fn is_dirty(&self) -> bool {
        self.ui().global::<WorkspaceState>().get_dirty()
    }

    /// Every form edit: dirty flag, tab counts, resolved-URL line.
    fn changed(&self) {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        let current = self.active.borrow().as_ref().map(|a| {
            let form = self.form(&s, a);
            let dirty = form != a.saved;
            (form, dirty)
        });
        let Some((form, dirty)) = current else {
            s.set_resolved_url("".into());
            return;
        };
        s.set_param_count(active_count(&form.params));
        s.set_header_count(active_count(&form.headers));
        s.set_resolved_url(workspace_rules::resolved_url(&form, |n| self.lookup(n)).into());
        if s.get_dirty() != dirty {
            s.set_dirty(dirty);
            self.refresh_tree();
        }
    }

    fn kv_model(&self, table: KvTable) -> &VecModel<KvRow> {
        match table {
            KvTable::Params => self.params.as_ref(),
            KvTable::Headers => self.headers.as_ref(),
        }
    }

    /// Typing into the placeholder row turns it into a real row.
    fn kv_edited(&self, table: KvTable, index: i32) {
        let model = self.kv_model(table);
        let last = model.row_count().saturating_sub(1);
        let filled_last = usize::try_from(index).is_ok_and(|i| i == last)
            && model
                .row_data(last)
                .is_some_and(|r| !r.key.is_empty() || !r.value.is_empty());
        if filled_last {
            model.push(placeholder());
        }
        self.changed();
    }

    fn kv_removed(&self, table: KvTable, index: i32) {
        let model = self.kv_model(table);
        if let Ok(i) = usize::try_from(index)
            && i + 1 < model.row_count()
        {
            model.remove(i);
        }
        self.changed();
    }

    fn save(&self) -> bool {
        // rewriting an unchanged file clobbers outside edits and YAML comments
        if !self.is_dirty() {
            return true;
        }
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        let target = self
            .active
            .borrow()
            .as_ref()
            .map(|a| (a.path.clone(), self.form(&s, a)));
        let Some((path, form)) = target else {
            return false;
        };
        match self.deps.collections.save_request(&path, &form) {
            Ok(()) => {
                if let Some(c) = self.collection.borrow_mut().as_mut() {
                    set_method(&mut c.children, &path, form.method);
                }
                if let Some(a) = self.active.borrow_mut().as_mut() {
                    a.saved = form;
                }
                self.changed();
                true
            }
            Err(e) => {
                self.banner(&e);
                false
            }
        }
    }

    fn send(&self) {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        if !s.get_can_send() {
            return;
        }
        // files re-read on each Send
        self.refresh_vars();
        let request =
            self.active.borrow().as_ref().map(|a| {
                workspace_rules::interpolate_request(&self.form(&s, a), |n| self.lookup(n))
            });
        let Some(request) = request else { return };
        s.set_send_status(SendStatus::Sending);
        let sender = self.deps.sender.clone();
        let pretty_json = self.deps.pretty_json;
        let weak = self.ui.clone();
        std::thread::spawn(move || {
            let start = Instant::now();
            let result = sender.send(&request);
            let elapsed = start.elapsed();
            let pretty = result.as_ref().ok().and_then(|r| pretty_json(&r.body));
            // Send is disabled while sending: no newer response can be overwritten
            let _ = weak.upgrade_in_event_loop(move |ui| show_result(&ui, result, elapsed, pretty));
        });
    }

    fn ask(&self, action: PendingAction) {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        let Some(name) = self.active.borrow().as_ref().map(|a| a.name.clone()) else {
            return;
        };
        s.set_confirm_name(name.into());
        s.set_confirm_file(s.get_request_file());
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
        }
    }

    fn banner(&self, e: &AppError) {
        let (title, hint) = workspace_rules::error_text(e, self.root().as_deref());
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        s.set_banner_title(title.into());
        s.set_banner_hint(hint.into());
    }

    fn dismiss_banner(&self) {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        s.set_banner_title("".into());
        s.set_banner_hint("".into());
    }
}

// sidebar label reads the method from the tree, not the saved file
fn set_method(nodes: &mut [Node], file: &Path, new: Method) {
    for n in nodes {
        match n {
            Node::Request { method, path, .. } if path == file => *method = new,
            Node::Folder { children, .. } => set_method(children, file, new),
            _ => {}
        }
    }
}

fn reset_response(s: &WorkspaceState) {
    s.set_send_status(SendStatus::Idle);
    s.set_status_text("".into());
    s.set_status_class(StatusClass::Ok);
    s.set_elapsed("".into());
    s.set_size("".into());
    s.set_truncated(false);
    s.set_response_raw("".into());
    s.set_response_pretty("".into());
    s.set_response_headers(ModelRc::default());
    s.set_fail_title("".into());
    s.set_fail_hint("".into());
    s.set_fail_detail("".into());
}

/// Runs on the UI thread with the worker's result.
fn show_result(
    ui: &AppWindow,
    result: Result<Response, AppError>,
    elapsed: Duration,
    pretty: Option<String>,
) {
    let s = ui.global::<WorkspaceState>();
    match result {
        Ok(r) => {
            s.set_status_text(format!("{} {}", r.status, r.status_text).into());
            s.set_status_class(match r.status {
                500.. => StatusClass::Err,
                300.. => StatusClass::Warn,
                _ => StatusClass::Ok,
            });
            s.set_elapsed(format!("{} ms", r.elapsed.as_millis()).into());
            s.set_size(workspace_rules::format_size(r.size_bytes).into());
            s.set_truncated(r.truncated);
            s.set_response_raw(r.body.into());
            s.set_response_pretty(pretty.unwrap_or_default().into());
            let headers: Vec<KvRow> = r
                .headers
                .into_iter()
                .map(|h| KvRow {
                    enabled: true,
                    key: h.key.into(),
                    value: h.value.into(),
                })
                .collect();
            s.set_response_headers(ModelRc::new(VecModel::from(headers)));
            s.set_send_status(SendStatus::Done);
        }
        Err(e) => {
            let (title, hint) = workspace_rules::error_text(&e, None);
            s.set_elapsed(format!("{} ms", elapsed.as_millis()).into());
            s.set_fail_title(title.into());
            s.set_fail_hint(hint.into());
            s.set_fail_detail(e.to_string().into());
            s.set_send_status(SendStatus::Failed);
        }
    }
}

fn strings<'a>(items: impl IntoIterator<Item = &'a str>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        items
            .into_iter()
            .map(SharedString::from)
            .collect::<Vec<_>>(),
    ))
}

fn placeholder() -> KvRow {
    KvRow {
        enabled: true,
        key: "".into(),
        value: "".into(),
    }
}

fn with_placeholder(rows: &[KeyValue]) -> Vec<KvRow> {
    rows.iter()
        .map(|r| KvRow {
            enabled: r.enabled,
            key: r.key.as_str().into(),
            value: r.value.as_str().into(),
        })
        .chain(std::iter::once(placeholder()))
        .collect()
}

/// Empty rows (the placeholder, cleared rows) are not part of the request.
fn kv_rows(model: &VecModel<KvRow>) -> Vec<KeyValue> {
    model
        .iter()
        .filter(|r| !r.key.is_empty() || !r.value.is_empty())
        .map(|r| KeyValue {
            key: r.key.to_string(),
            value: r.value.to_string(),
            enabled: r.enabled,
        })
        .collect()
}

fn active_count(rows: &[KeyValue]) -> i32 {
    rows.iter().filter(|r| r.is_active()).count() as i32
}
