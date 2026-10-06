//! Auth as ApiArk stores it: request `auth:` and collection `defaults.auth`.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ApiKeyPlace {
    #[default]
    Header,
    Query,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Auth {
    Bearer {
        token: String,
    },
    Basic {
        username: String,
        password: String,
    },
    ApiKey {
        key: String,
        value: String,
        place: ApiKeyPlace,
    },
    /// oauth2, digest, aws-v4, ...: never sent, never rewritten
    Unsupported(String),
}

impl Auth {
    pub fn is_sendable(&self) -> bool {
        !matches!(self, Auth::Unsupported(_))
    }
}

/// Parity: upstream commands/http.rs. Folders are not walked.
pub fn effective<'a>(request: Option<&'a Auth>, collection: Option<&'a Auth>) -> Option<&'a Auth> {
    request.or(collection)
}
