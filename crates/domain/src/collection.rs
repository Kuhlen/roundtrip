use std::path::{Path, PathBuf};

use crate::AppError;
use crate::auth::Auth;
use crate::http::{Method, Request};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Protocol {
    #[default]
    Http,
    Graphql,
    WebSocket,
    Sse,
    Grpc,
}

impl Protocol {
    pub fn parse(s: &str) -> Protocol {
        match s {
            "graphql" => Protocol::Graphql,
            "websocket" => Protocol::WebSocket,
            "sse" => Protocol::Sse,
            "grpc" => Protocol::Grpc,
            _ => Protocol::Http,
        }
    }

    /// GraphQL files are plain POSTs with a JSON body
    pub fn is_sendable(self) -> bool {
        matches!(self, Protocol::Http | Protocol::Graphql)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Folder {
        name: String,
        path: PathBuf,
        children: Vec<Node>,
    },
    Request {
        name: String,
        method: Method,
        protocol: Protocol,
        path: PathBuf,
    },
}

impl Node {
    pub fn name(&self) -> &str {
        match self {
            Node::Folder { name, .. } | Node::Request { name, .. } => name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collection {
    pub name: String,
    pub path: PathBuf,
    pub children: Vec<Node>,
    /// `defaults.auth` in `.apiark/apiark.yaml`
    pub auth: Option<Auth>,
}

pub trait CollectionStore {
    fn load(&self, dir: &Path) -> Result<Collection, AppError>;
    fn read_request(&self, file: &Path) -> Result<Request, AppError>;
    /// Patches only method/url/params/headers/body; every other key stays.
    fn save_request(&self, file: &Path, request: &Request) -> Result<(), AppError>;
    /// Sets or removes only `defaults.auth`; Unsupported is left as the file has it.
    fn save_collection_auth(&self, root: &Path, auth: Option<&Auth>) -> Result<(), AppError>;
    /// `<slug>.yaml` with name/method/url; AlreadyExists on collision.
    fn create_request(&self, dir: &Path, name: &str) -> Result<PathBuf, AppError>;
    fn create_folder(&self, dir: &Path, name: &str) -> Result<PathBuf, AppError>;
    /// Renames on disk and in the parent `_folder.yaml` order; requests also get `name:`.
    fn rename(&self, path: &Path, new_name: &str) -> Result<PathBuf, AppError>;
    /// Permanent: no trash.
    fn delete(&self, path: &Path) -> Result<(), AppError>;
}
