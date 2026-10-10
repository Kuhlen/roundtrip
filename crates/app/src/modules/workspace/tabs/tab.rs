//! One open request: its file, saved copy, form snapshot and last response.

use std::path::Path;
use std::time::Duration;

use domain::AppError;
use domain::collection::Protocol;
use domain::http::{CancelFlag, Request, Response};
use slint::{Model, ModelRc, SharedString, VecModel};

use super::tabs_rules::TabFile;
use crate::modules::workspace::workspace_controller::WorkspaceController;
use crate::modules::workspace::workspace_rules;
use crate::ui::{AuthFields, KvRow, SendStatus, StatusClass, WorkspaceState};

pub(crate) struct Tab {
    pub(crate) id: u64,
    /// None = scratch tab, not saved yet
    pub(crate) file: Option<TabFile>,
    pub(crate) title: String,
    pub(crate) protocol: Protocol,
    pub(crate) saved: Request,
    /// None while shown: the form lives in Slint
    pub(crate) snapshot: Option<FormState>,
    pub(crate) dirty: bool,
    pub(crate) pinned: bool,
    pub(crate) response: ResponseView,
    /// send number + flag while a request is in flight
    pub(crate) sending: Option<(u64, CancelFlag)>,
}

/// Every form property of WorkspaceState, so a switch back restores the exact form.
#[derive(Clone, Default)]
pub(crate) struct FormState {
    method_index: i32,
    url: SharedString,
    params: Vec<KvRow>,
    headers: Vec<KvRow>,
    body_kind_index: i32,
    body: SharedString,
    form_rows: Vec<KvRow>,
    binary_path: SharedString,
    body_note: SharedString,
    unsupported_body: SharedString,
    unsupported_protocol: SharedString,
    graphql: bool,
    gql_query: SharedString,
    gql_variables: SharedString,
    gql_operation: SharedString,
    request_auth: AuthFields,
    editor_tab: i32,
    response_tab: i32,
}

impl FormState {
    pub(crate) fn capture(s: &WorkspaceState, c: &WorkspaceController) -> Self {
        Self {
            method_index: s.get_method_index(),
            url: s.get_url(),
            params: c.params.iter().collect(),
            headers: c.headers.iter().collect(),
            body_kind_index: s.get_body_kind_index(),
            body: s.get_body(),
            form_rows: c.form_rows.iter().collect(),
            binary_path: s.get_binary_path(),
            body_note: s.get_body_note(),
            unsupported_body: s.get_unsupported_body(),
            unsupported_protocol: s.get_unsupported_protocol(),
            graphql: s.get_graphql(),
            gql_query: s.get_gql_query(),
            gql_variables: s.get_gql_variables(),
            gql_operation: s.get_gql_operation(),
            request_auth: s.get_request_auth(),
            editor_tab: s.get_editor_tab(),
            response_tab: s.get_response_tab(),
        }
    }

    /// Relative file paths become absolute against `root`.
    pub(crate) fn absolutize(&mut self, root: &Path) {
        let abs = |p: &SharedString| -> SharedString {
            if p.is_empty() || Path::new(p.as_str()).is_absolute() {
                p.clone()
            } else {
                root.join(p.as_str()).display().to_string().into()
            }
        };
        self.binary_path = abs(&self.binary_path);
        for row in self.form_rows.iter_mut().filter(|r| r.file) {
            row.value = abs(&row.value);
        }
    }

    pub(crate) fn apply(&self, s: &WorkspaceState, c: &WorkspaceController) {
        s.set_method_index(self.method_index);
        s.set_url(self.url.clone());
        c.params.set_vec(self.params.clone());
        c.headers.set_vec(self.headers.clone());
        s.set_body_kind_index(self.body_kind_index);
        s.set_body(self.body.clone());
        c.form_rows.set_vec(self.form_rows.clone());
        s.set_binary_path(self.binary_path.clone());
        s.set_body_note(self.body_note.clone());
        s.set_unsupported_body(self.unsupported_body.clone());
        s.set_unsupported_protocol(self.unsupported_protocol.clone());
        s.set_graphql(self.graphql);
        s.set_gql_query(self.gql_query.clone());
        s.set_gql_variables(self.gql_variables.clone());
        s.set_gql_operation(self.gql_operation.clone());
        s.set_request_auth(self.request_auth.clone());
        s.set_editor_tab(self.editor_tab);
        s.set_response_tab(self.response_tab);
    }
}

/// What the response panel shows for one tab.
#[derive(Clone, Default)]
pub(crate) struct ResponseView {
    status: SendStatus,
    status_text: SharedString,
    status_class: StatusClass,
    elapsed: SharedString,
    size: SharedString,
    truncated: bool,
    raw: SharedString,
    pretty: SharedString,
    headers: Vec<KvRow>,
    fail_title: SharedString,
    fail_hint: SharedString,
    fail_detail: SharedString,
}

impl ResponseView {
    pub(crate) fn cancelled() -> Self {
        Self {
            status: SendStatus::Cancelled,
            ..Self::default()
        }
    }

    pub(crate) fn sending() -> Self {
        Self {
            status: SendStatus::Sending,
            ..Self::default()
        }
    }

    pub(crate) fn capture(s: &WorkspaceState) -> Self {
        Self {
            status: s.get_send_status(),
            status_text: s.get_status_text(),
            status_class: s.get_status_class(),
            elapsed: s.get_elapsed(),
            size: s.get_size(),
            truncated: s.get_truncated(),
            raw: s.get_response_raw(),
            pretty: s.get_response_pretty(),
            headers: s.get_response_headers().iter().collect(),
            fail_title: s.get_fail_title(),
            fail_hint: s.get_fail_hint(),
            fail_detail: s.get_fail_detail(),
        }
    }

    pub(crate) fn from_result(
        result: Result<Response, AppError>,
        elapsed: Duration,
        pretty: Option<String>,
    ) -> Self {
        match result {
            Ok(r) => Self {
                status: SendStatus::Done,
                status_text: format!("{} {}", r.status, r.status_text).into(),
                status_class: status_class(r.status),
                elapsed: format!("{} ms", r.elapsed.as_millis()).into(),
                size: workspace_rules::format_size(r.size_bytes).into(),
                truncated: r.truncated,
                raw: r.body.into(),
                pretty: pretty.unwrap_or_default().into(),
                headers: r
                    .headers
                    .into_iter()
                    .map(|h| KvRow {
                        enabled: true,
                        key: h.key.into(),
                        value: h.value.into(),
                        file: false,
                        secret: false,
                    })
                    .collect(),
                ..Self::default()
            },
            Err(e) => {
                let (title, hint) = workspace_rules::error_text(&e, None);
                Self {
                    status: SendStatus::Failed,
                    elapsed: format!("{} ms", elapsed.as_millis()).into(),
                    fail_title: title.into(),
                    fail_hint: hint.into(),
                    fail_detail: e.to_string().into(),
                    ..Self::default()
                }
            }
        }
    }

    pub(crate) fn show(&self, s: &WorkspaceState) {
        s.set_send_status(self.status);
        s.set_status_text(self.status_text.clone());
        s.set_status_class(self.status_class);
        s.set_elapsed(self.elapsed.clone());
        s.set_size(self.size.clone());
        s.set_truncated(self.truncated);
        s.set_response_raw(self.raw.clone());
        s.set_response_pretty(self.pretty.clone());
        s.set_response_headers(ModelRc::new(VecModel::from(self.headers.clone())));
        s.set_fail_title(self.fail_title.clone());
        s.set_fail_hint(self.fail_hint.clone());
        s.set_fail_detail(self.fail_detail.clone());
    }
}

/// Same colours in the response panel and the history list.
pub(crate) fn status_class(code: u16) -> StatusClass {
    match code {
        500.. => StatusClass::Err,
        300.. => StatusClass::Warn,
        _ => StatusClass::Ok,
    }
}
