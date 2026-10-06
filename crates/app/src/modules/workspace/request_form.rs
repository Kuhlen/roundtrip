//! Form <-> Request: fill, read back, dirty flag, key/value tables, save.

use domain::auth::effective;
use domain::http::{BodyKind, KeyValue, Method, Request};
use slint::{ComponentHandle, Model, VecModel};

use super::auth_fields::{auth_fields, auth_from_fields};
use super::tree_actions::set_method;
use super::workspace_controller::{Active, WorkspaceController};
use super::workspace_rules;
use crate::ui::{AuthFields, KvRow, KvTable, SendStatus, WorkspaceState};

impl WorkspaceController {
    pub(super) fn clear_request(&self, s: &WorkspaceState) {
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
        s.set_request_auth(AuthFields::default());
        s.set_inherited_auth("".into());
        s.set_auth_blocked(false);
        self.params.set_vec(vec![placeholder()]);
        self.headers.set_vec(vec![placeholder()]);
    }

    /// Form ← saved request. Last response stays: a late reply never lands on a reset panel.
    pub(super) fn fill(&self, active: Active) {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        let r = &active.saved;
        s.set_has_request(true);
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
        s.set_request_auth(auth_fields(r.auth.as_ref()));
        self.params.set_vec(with_placeholder(&r.params));
        self.headers.set_vec(with_placeholder(&r.headers));
        *self.active.borrow_mut() = Some(active);
        self.show_header();
        s.set_dirty(false);
        self.refresh_tree();
        self.changed();
    }

    /// Name, breadcrumb and file of the active request.
    pub(super) fn show_header(&self) {
        let Some(active) = self.active.borrow().clone() else {
            return;
        };
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        let root = self.root().unwrap_or_default();
        let rel = active.path.strip_prefix(&root).unwrap_or(&active.path);
        let crumb = rel
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| format!("{} /", p.display()));
        s.set_request_name(active.name.as_str().into());
        s.set_crumb(crumb.unwrap_or_default().into());
        s.set_request_file(rel.display().to_string().into());
    }

    /// Form → Request. Unsupported bodies come from the file, never from the form.
    pub(super) fn form(&self, s: &WorkspaceState, a: &Active) -> Request {
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
            auth: auth_from_fields(&s.get_request_auth(), a.saved.auth.as_ref()),
        }
    }

    pub(super) fn is_dirty(&self) -> bool {
        self.ui().global::<WorkspaceState>().get_dirty()
    }

    /// Form request with the auth Send will use: its own, else the collection's.
    pub(super) fn outgoing(&self, form: Request) -> Request {
        let collection = self.collection.borrow();
        let auth = effective(
            form.auth.as_ref(),
            collection.as_ref().and_then(|c| c.auth.as_ref()),
        )
        .cloned();
        Request { auth, ..form }
    }

    /// Every form edit: dirty flag, tab counts, auth note, resolved-URL line.
    pub(super) fn changed(&self) {
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
        let collection_auth = self
            .collection
            .borrow()
            .as_ref()
            .and_then(|c| c.auth.clone());
        s.set_inherited_auth(workspace_rules::inherit_note(collection_auth.as_ref()).into());
        let out = self.outgoing(form);
        s.set_auth_blocked(out.auth.as_ref().is_some_and(|a| !a.is_sendable()));
        s.set_resolved_url(workspace_rules::resolved_url(&out, |n| self.lookup(n)).into());
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
    pub(super) fn kv_edited(&self, table: KvTable, index: i32) {
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

    pub(super) fn kv_removed(&self, table: KvTable, index: i32) {
        let model = self.kv_model(table);
        if let Ok(i) = usize::try_from(index)
            && i + 1 < model.row_count()
        {
            model.remove(i);
        }
        self.changed();
    }

    pub(super) fn save(&self) -> bool {
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
}

pub(super) fn placeholder() -> KvRow {
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
