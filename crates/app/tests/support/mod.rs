//! Fake stores + a fixture collection; build with setup()/opened()
#![allow(dead_code)] // each test binary uses a subset

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use app::modules::workspace::workspace_controller::{Deps, WorkspaceController};
use app::ui::{AppWindow, KvRow, WorkspaceState};
use domain::AppError;
use domain::collection::{Collection, CollectionStore, Node, Protocol};
use domain::environment::{Environment, EnvironmentStore, Scope};
use domain::http::{BodyKind, HttpSender, KeyValue, Method, Request, Response};
use domain::session::{Session, SessionStore};
use slint::{ComponentHandle, Model, ModelRc, SharedString};

pub const ROOT: &str = "/c";

pub fn p(rel: &str) -> PathBuf {
    Path::new(ROOT).join(rel)
}

/// rows: 0 users, 1 List users, 2 Create user, 3 Health check, 4 Echo socket, 5 Upload avatar
pub fn tree() -> Collection {
    let req = |name: &str, method, protocol, rel: &str| Node::Request {
        name: name.into(),
        method,
        protocol,
        path: p(rel),
    };
    Collection {
        name: "httpbin-demo".into(),
        path: PathBuf::from(ROOT),
        children: vec![
            Node::Folder {
                name: "users".into(),
                path: p("users"),
                children: vec![
                    req(
                        "List users",
                        Method::Get,
                        Protocol::Http,
                        "users/list-users.yaml",
                    ),
                    req(
                        "Create user",
                        Method::Post,
                        Protocol::Http,
                        "users/create-user.yaml",
                    ),
                ],
            },
            req("Health check", Method::Get, Protocol::Http, "health.yaml"),
            req("Echo socket", Method::Get, Protocol::WebSocket, "echo.yaml"),
            req("Upload avatar", Method::Post, Protocol::Http, "upload.yaml"),
        ],
    }
}

#[derive(Default)]
pub struct FakeCollections {
    pub files: RefCell<HashMap<PathBuf, Result<Request, AppError>>>,
    pub saved: RefCell<Vec<(PathBuf, Request)>>,
    pub save_error: RefCell<Option<AppError>>,
}

impl CollectionStore for FakeCollections {
    fn load(&self, dir: &Path) -> Result<Collection, AppError> {
        if dir == Path::new(ROOT) {
            Ok(tree())
        } else {
            Err(AppError::NotACollection(dir.display().to_string()))
        }
    }

    fn read_request(&self, file: &Path) -> Result<Request, AppError> {
        self.files
            .borrow()
            .get(file)
            .cloned()
            .unwrap_or_else(|| Err(AppError::Storage(format!("no fixture {}", file.display()))))
    }

    fn save_request(&self, file: &Path, request: &Request) -> Result<(), AppError> {
        if let Some(e) = self.save_error.borrow().clone() {
            return Err(e);
        }
        self.saved
            .borrow_mut()
            .push((file.to_path_buf(), request.clone()));
        self.files
            .borrow_mut()
            .insert(file.to_path_buf(), Ok(request.clone()));
        Ok(())
    }
}

#[derive(Default)]
pub struct FakeEnvironments {
    pub list_error: RefCell<Option<AppError>>,
}

impl EnvironmentStore for FakeEnvironments {
    fn list(&self, _collection: &Path) -> Result<Vec<Environment>, AppError> {
        if let Some(e) = self.list_error.borrow().clone() {
            return Err(e);
        }
        let env = |name: &str, scope| Environment {
            name: name.into(),
            scope,
            variables: HashMap::new(),
            secrets: vec![],
        };
        Ok(vec![
            env("dev", Scope::Shared),
            env("mine", Scope::Personal),
        ])
    }

    fn resolve(&self, _collection: &Path, name: &str) -> Result<HashMap<String, String>, AppError> {
        Ok(match name {
            "dev" => HashMap::from([("baseUrl".to_string(), "https://httpbin.org".to_string())]),
            _ => HashMap::new(),
        })
    }
}

pub struct FakeSender {
    pub reply: Mutex<Result<Response, AppError>>,
    pub requests: Mutex<Vec<Request>>,
}

impl HttpSender for FakeSender {
    fn send(&self, request: &Request) -> Result<Response, AppError> {
        self.requests
            .lock()
            .expect("requests")
            .push(request.clone());
        self.reply.lock().expect("reply").clone()
    }
}

#[derive(Default)]
pub struct FakeSession {
    pub session: RefCell<Session>,
    pub saves: RefCell<Vec<Session>>,
}

impl SessionStore for FakeSession {
    fn load(&self) -> Session {
        self.session.borrow().clone()
    }

    fn save(&self, session: &Session) {
        self.saves.borrow_mut().push(session.clone());
    }
}

pub struct Fakes {
    pub collections: Rc<FakeCollections>,
    pub environments: Rc<FakeEnvironments>,
    pub sender: Arc<FakeSender>,
    pub session: Rc<FakeSession>,
}

pub fn ok_response() -> Response {
    Response {
        status: 200,
        status_text: "OK".into(),
        headers: vec![KeyValue::new("content-type", "application/json")],
        body: r#"{"args":{"page":"2"}}"#.into(),
        elapsed: Duration::from_millis(84),
        size_bytes: 21,
        truncated: false,
    }
}

pub fn fakes() -> Fakes {
    let files = HashMap::from([
        (
            p("users/list-users.yaml"),
            Ok(Request {
                url: "{{baseUrl}}/get".into(),
                params: vec![KeyValue::new("page", "2")],
                ..Request::default()
            }),
        ),
        (
            p("users/create-user.yaml"),
            Ok(Request {
                method: Method::Post,
                url: "{{baseUrl}}/post".into(),
                body_kind: BodyKind::Json,
                body: r#"{"name":"Ayu"}"#.into(),
                ..Request::default()
            }),
        ),
        (
            p("health.yaml"),
            Ok(Request {
                url: "{{baseUrl}}/status/200".into(),
                ..Request::default()
            }),
        ),
        (
            p("echo.yaml"),
            Ok(Request {
                url: "wss://echo.websocket.org".into(),
                ..Request::default()
            }),
        ),
        (
            p("upload.yaml"),
            Ok(Request {
                method: Method::Post,
                url: "{{baseUrl}}/post".into(),
                body_kind: BodyKind::Unsupported("form-data".into()),
                ..Request::default()
            }),
        ),
    ]);
    Fakes {
        collections: Rc::new(FakeCollections {
            files: RefCell::new(files),
            ..FakeCollections::default()
        }),
        environments: Rc::new(FakeEnvironments::default()),
        sender: Arc::new(FakeSender {
            reply: Mutex::new(Ok(ok_response())),
            requests: Mutex::default(),
        }),
        session: Rc::new(FakeSession::default()),
    }
}

/// Backend must be initialized by the caller.
pub fn build(f: &Fakes) -> (AppWindow, Rc<WorkspaceController>) {
    let ui = AppWindow::new().expect("window");
    let deps = Deps {
        collections: f.collections.clone(),
        environments: f.environments.clone(),
        sender: f.sender.clone(),
        session: f.session.clone(),
        dynamic_var: |_| None,
        pretty_json: data::http::pretty_json,
    };
    let controller = WorkspaceController::new(deps, &ui);
    (ui, controller)
}

pub fn setup() -> (Fakes, AppWindow, Rc<WorkspaceController>) {
    i_slint_backend_testing::init_no_event_loop();
    let f = fakes();
    let (ui, c) = build(&f);
    (f, ui, c)
}

pub fn open_with_env(ui: &AppWindow, c: &WorkspaceController) {
    c.open_collection(Path::new(ROOT));
    state(ui).set_environment_index(1);
    state(ui).invoke_environment_selected();
}

/// collection open, environment "dev" selected, no request yet
pub fn opened() -> (Fakes, AppWindow, Rc<WorkspaceController>) {
    let (f, ui, c) = setup();
    open_with_env(&ui, &c);
    (f, ui, c)
}

pub fn state(ui: &AppWindow) -> WorkspaceState<'_> {
    ui.global::<WorkspaceState>()
}

pub fn tree_names(ui: &AppWindow) -> Vec<String> {
    state(ui)
        .get_tree()
        .iter()
        .map(|r| r.name.to_string())
        .collect()
}

pub fn tree_labels(ui: &AppWindow) -> Vec<String> {
    state(ui)
        .get_tree()
        .iter()
        .map(|r| r.label.to_string())
        .collect()
}

pub fn strings(model: ModelRc<SharedString>) -> Vec<String> {
    model.iter().map(|s| s.to_string()).collect()
}

pub fn kv(model: &ModelRc<KvRow>) -> Vec<(String, String, bool)> {
    model
        .iter()
        .map(|r| (r.key.to_string(), r.value.to_string(), r.enabled))
        .collect()
}
