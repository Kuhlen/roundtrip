//! Collection read by an importer, before it is written as an ApiArk folder. Format-neutral.

use crate::auth::Auth;
use crate::collection::Protocol;
use crate::http::Request;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportedCollection {
    pub name: String,
    /// becomes `defaults.auth`
    pub auth: Option<Auth>,
    pub items: Vec<ImportItem>,
    /// (name, variables), file order
    pub environments: Vec<(String, Vec<(String, String)>)>,
    /// one per occurrence; the app groups them
    pub warnings: Vec<ImportWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// tree is built once, boxing would change the importer API
#[allow(clippy::large_enum_variant)]
pub enum ImportItem {
    Folder {
        name: String,
        items: Vec<ImportItem>,
    },
    Request(ImportedRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportedRequest {
    pub name: String,
    pub request: Request,
    pub protocol: Protocol,
    pub description: Option<String>,
    /// kept in the file, never run
    pub pre_request_script: Option<String>,
    pub tests: Option<String>,
}

/// What an import could not carry over as-is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportWarning {
    /// auth type name; the request falls back to inherit
    UnsupportedAuth(String),
    FolderAuth,
    /// "no auth" under a collection auth: Send will use the collection's
    NoAuthInherits,
    PathVariables,
    Scripts,
    /// root or folder scripts; nowhere to keep them
    CollectionScriptsDropped,
    /// file format has no off switch for headers, params, variables
    DisabledDropped,
    /// method name; the request is skipped
    UnknownMethod(String),
    /// body mode name; sent without body
    UnsupportedBody(String),
}

impl ImportedCollection {
    /// (folders, requests, environments)
    pub fn counts(&self) -> (usize, usize, usize) {
        fn walk(items: &[ImportItem], folders: &mut usize, requests: &mut usize) {
            for item in items {
                match item {
                    ImportItem::Folder { items, .. } => {
                        *folders += 1;
                        walk(items, folders, requests);
                    }
                    ImportItem::Request(_) => *requests += 1,
                }
            }
        }
        let (mut folders, mut requests) = (0, 0);
        walk(&self.items, &mut folders, &mut requests);
        (folders, requests, self.environments.len())
    }
}
