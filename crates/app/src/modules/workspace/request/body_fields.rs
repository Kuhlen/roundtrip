//! Body editor <-> Body. One buffer per kind, so switching kinds loses nothing until Save.

use domain::AppError;
use domain::graphql::GraphqlBody;
use domain::http::{Body, FormField, KeyValue, TextKind};
use slint::{ComponentHandle, Model};

use super::request_form::placeholder;
use crate::modules::workspace::workspace_controller::WorkspaceController;
use crate::modules::workspace::workspace_rules;
use crate::ui::{KvRow, KvTable, WorkspaceState};

impl WorkspaceController {
    /// Saved body -> its buffer; the other buffers start empty.
    pub(crate) fn fill_body(&self, s: &WorkspaceState, body: &Body, gql: Option<&GraphqlBody>) {
        s.set_graphql(gql.is_some());
        let g = gql.cloned().unwrap_or_default();
        s.set_gql_query(g.query.into());
        s.set_gql_variables(g.variables.into());
        s.set_gql_operation(g.operation_name.into());
        s.set_body_error("".into());
        s.set_unsupported_body("".into());
        s.set_body_note("".into());
        s.set_body("".into());
        s.set_binary_path("".into());
        let row = |key: &str, value: &str, enabled, file| KvRow {
            enabled,
            key: key.into(),
            value: value.into(),
            file,
        };
        let mut rows = Vec::new();
        match body {
            Body::None => {}
            Body::Text { text, .. } => s.set_body(text.as_str().into()),
            Body::Urlencoded(r) => {
                rows = r
                    .iter()
                    .map(|r| row(&r.key, &r.value, r.enabled, false))
                    .collect()
            }
            Body::FormData(f) => {
                rows = f
                    .iter()
                    .map(|f| row(&f.key, &f.value, f.enabled, f.is_file))
                    .collect()
            }
            Body::Binary(path) => s.set_binary_path(path.as_str().into()),
            Body::Unsupported(kind) => {
                s.set_unsupported_body(kind.as_str().into());
                s.set_body_note(workspace_rules::unsupported_body_note(kind).into());
            }
        }
        rows.push(placeholder());
        self.form_rows.set_vec(rows);
        let index = Body::KINDS
            .iter()
            .position(|k| *k == body.as_str())
            .unwrap_or(0);
        s.set_body_kind_index(index as i32);
    }

    /// Selected kind's buffer -> Body. Unsupported bodies come from the file.
    pub(crate) fn form_body(&self, s: &WorkspaceState, saved: &Body) -> Body {
        if let Body::Unsupported(_) = saved {
            return saved.clone();
        }
        if s.get_graphql() {
            // invalid Variables: keep the file's body; body_error marks the form dirty
            return (self.deps.graphql_json)(&self.gql_form(s)).map_or_else(
                |_| saved.clone(),
                |text| Body::Text {
                    kind: TextKind::Json,
                    text,
                },
            );
        }
        let text = |kind| Body::Text {
            kind,
            text: s.get_body().to_string(),
        };
        // placeholder and cleared rows are not part of the body
        let rows = self
            .form_rows
            .iter()
            .filter(|r| !r.key.is_empty() || !r.value.is_empty());
        // same order as Body::KINDS
        match s.get_body_kind_index() {
            1 => text(TextKind::Json),
            2 => text(TextKind::Xml),
            3 => text(TextKind::Raw),
            4 => Body::Urlencoded(
                rows.map(|r| KeyValue {
                    key: r.key.to_string(),
                    value: r.value.to_string(),
                    enabled: r.enabled,
                })
                .collect(),
            ),
            5 => Body::FormData(
                rows.map(|r| FormField {
                    key: r.key.to_string(),
                    value: r.value.to_string(),
                    enabled: r.enabled,
                    is_file: r.file,
                })
                .collect(),
            ),
            6 => Body::Binary(s.get_binary_path().to_string()),
            _ => Body::None,
        }
    }

    fn gql_form(&self, s: &WorkspaceState) -> GraphqlBody {
        GraphqlBody {
            query: s.get_gql_query().to_string(),
            variables: s.get_gql_variables().to_string(),
            operation_name: s.get_gql_operation().to_string(),
        }
    }

    /// Some: the form can't become a body (invalid Variables).
    pub(crate) fn body_error(&self, s: &WorkspaceState) -> Option<AppError> {
        if !s.get_graphql() {
            return None;
        }
        (self.deps.graphql_json)(&self.gql_form(s)).err()
    }

    pub(crate) fn pick_file(&self, index: i32) {
        let Some(root) = self.root() else { return };
        let Some(picked) = (self.deps.pick_file)(&root) else {
            return;
        };
        // relative survives moving or cloning the collection
        let path = match picked.strip_prefix(&root) {
            Ok(rel) => rel.display().to_string(),
            Err(_) => picked.display().to_string(),
        };
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        match usize::try_from(index) {
            Err(_) => {
                s.set_binary_path(path.into());
                self.changed();
            }
            Ok(i) => {
                if let Some(row) = self.form_rows.row_data(i) {
                    self.form_rows.set_row_data(
                        i,
                        KvRow {
                            value: path.into(),
                            file: true,
                            ..row
                        },
                    );
                }
                self.kv_edited(KvTable::Form, index);
            }
        }
    }
}

/// Active rows of a form body, shown on the Body tab; 0 for other kinds.
pub(crate) fn body_count(body: &Body) -> i32 {
    match body {
        Body::Urlencoded(rows) => rows.iter().filter(|r| r.is_active()).count() as i32,
        Body::FormData(fields) => fields.iter().filter(|f| f.is_active()).count() as i32,
        _ => 0,
    }
}
