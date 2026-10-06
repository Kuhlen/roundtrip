//! AuthForm fields <-> domain Auth.

use domain::auth::{ApiKeyPlace, Auth};

use crate::ui::AuthFields;

/// kind = AuthForm select index: 0 inherit, 1 bearer, 2 basic, 3 api key
pub(super) fn auth_fields(auth: Option<&Auth>) -> AuthFields {
    let mut f = AuthFields::default();
    match auth {
        None => {}
        Some(Auth::Bearer { token }) => {
            f.kind = 1;
            f.token = token.as_str().into();
        }
        Some(Auth::Basic { username, password }) => {
            f.kind = 2;
            f.username = username.as_str().into();
            f.password = password.as_str().into();
        }
        Some(Auth::ApiKey { key, value, place }) => {
            f.kind = 3;
            f.key = key.as_str().into();
            f.value = value.as_str().into();
            f.place = i32::from(*place == ApiKeyPlace::Query);
        }
        Some(Auth::Unsupported(kind)) => f.unsupported = kind.as_str().into(),
    }
    f
}

/// Unsupported auth comes from the file, never from the form.
pub(super) fn auth_from_fields(f: &AuthFields, saved: Option<&Auth>) -> Option<Auth> {
    if !f.unsupported.is_empty() {
        return saved.cloned();
    }
    match f.kind {
        1 => Some(Auth::Bearer {
            token: f.token.to_string(),
        }),
        2 => Some(Auth::Basic {
            username: f.username.to_string(),
            password: f.password.to_string(),
        }),
        3 => Some(Auth::ApiKey {
            key: f.key.to_string(),
            value: f.value.to_string(),
            place: if f.place == 1 {
                ApiKeyPlace::Query
            } else {
                ApiKeyPlace::Header
            },
        }),
        _ => None,
    }
}
