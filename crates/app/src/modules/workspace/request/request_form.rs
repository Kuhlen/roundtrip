//! Form <-> Request: fill, read back, dirty flag, key/value tables, save.

use domain::auth::effective;
use domain::collection::Protocol;
use domain::http::{Body, KeyValue, Method, Request, TextKind};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use super::body_fields::body_count;
use crate::modules::workspace::auth_fields::{auth_fields, auth_from_fields};
use crate::modules::workspace::sidebar::set_method;
use crate::modules::workspace::tabs::tab::ResponseView;
use crate::modules::workspace::workspace_controller::WorkspaceController;
use crate::modules::workspace::workspace_rules;
use crate::ui::{AuthFields, KvRow, KvTable, WorkspaceState};

impl WorkspaceController {
    pub(crate) fn clear_request(&self, s: &WorkspaceState) {
        self.active.set(None);
        *self.pending.borrow_mut() = None;
        s.set_has_request(false);
        s.set_crumb("".into());
        s.set_request_name("".into());
        s.set_request_file("".into());
        s.set_method_index(0);
        s.set_url("".into());
        s.set_resolved_url("".into());
        s.set_url_segments(ModelRc::default());
        self.fill_body(s, &Body::None, None);
        s.set_body_count(0);
        s.set_unsupported_protocol("".into());
        s.set_param_count(0);
        s.set_header_count(0);
        s.set_dirty(false);
        // a late reply goes to its tab, never to this panel
        ResponseView::default().show(s);
        s.set_confirm_open(false);
        s.set_request_auth(AuthFields::default());
        s.set_inherited_auth("".into());
        s.set_auth_blocked(false);
        self.params.set_vec(vec![placeholder()]);
        self.headers.set_vec(vec![placeholder()]);
    }

    /// Form ← the active tab's saved request.
    pub(in crate::modules::workspace) fn fill_saved(&self, s: &WorkspaceState) {
        let Some((mut saved, protocol)) = self.with_active(|t| (t.saved.clone(), t.protocol))
        else {
            return;
        };
        let gql = match &saved.body {
            Body::Text {
                kind: TextKind::Json,
                text,
            } if protocol == Protocol::Graphql => (self.deps.graphql_parse)(text),
            _ => None,
        };
        // compare against the layout save writes, so opening is never dirty
        if let Some(text) = gql.as_ref().and_then(|g| (self.deps.graphql_json)(g).ok()) {
            saved.body = Body::Text {
                kind: TextKind::Json,
                text,
            };
            self.with_active_mut(|t| t.saved = saved.clone());
        }
        let r = &saved;
        s.set_method_index(Method::ALL.iter().position(|m| *m == r.method).unwrap_or(0) as i32);
        s.set_url(r.url.as_str().into());
        self.fill_body(s, &r.body, gql.as_ref());
        let protocol = if protocol.is_sendable() {
            ""
        } else {
            workspace_rules::protocol_name(protocol)
        };
        s.set_unsupported_protocol(protocol.into());
        s.set_request_auth(auth_fields(r.auth.as_ref()));
        self.params.set_vec(with_placeholder(&r.params));
        self.headers.set_vec(with_placeholder(&r.headers));
    }

    /// Name, breadcrumb and file of the active request.
    pub(crate) fn show_header(&self) {
        let Some((title, path)) =
            self.with_active(|t| (t.title.clone(), t.file.as_ref().map(|f| f.path.clone())))
        else {
            return;
        };
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        let Some(path) = path else {
            s.set_request_name(title.as_str().into());
            s.set_crumb("".into());
            s.set_request_file("not saved".into());
            return;
        };
        let root = self.tab_root().unwrap_or_default();
        let rel = path.strip_prefix(&root).unwrap_or(&path);
        let crumb = rel
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| format!("{} /", p.display()));
        s.set_request_name(title.as_str().into());
        s.set_crumb(crumb.unwrap_or_default().into());
        s.set_request_file(rel.display().to_string().into());
    }

    /// Form → Request. Unsupported bodies come from the file, never from the form.
    pub(in crate::modules::workspace) fn form(
        &self,
        s: &WorkspaceState,
        saved: &Request,
    ) -> Request {
        let body = self.form_body(s, &saved.body);
        Request {
            method: usize::try_from(s.get_method_index())
                .ok()
                .and_then(|i| Method::ALL.get(i))
                .copied()
                .unwrap_or_default(),
            url: s.get_url().to_string(),
            params: kv_rows(&self.params),
            headers: kv_rows(&self.headers),
            body,
            auth: auth_from_fields(&s.get_request_auth(), saved.auth.as_ref()),
        }
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.ui().global::<WorkspaceState>().get_dirty()
    }

    /// Form request with the auth Send will use: its own, else its collection's.
    pub(crate) fn outgoing(&self, form: Request) -> Request {
        let inherited = self.tab_root().and_then(|r| self.auth_of(&r));
        let auth = effective(form.auth.as_ref(), inherited.as_ref()).cloned();
        Request { auth, ..form }
    }

    /// Every form edit: dirty flag, tab counts, auth note, resolved-URL line.
    pub(crate) fn changed(&self) {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        *self.last_url.borrow_mut() = s.get_url().to_string();
        let current = self.with_active(|t| {
            let form = self.form(&s, &t.saved);
            let error = self.body_error(&s);
            let dirty = form != t.saved || error.is_some();
            (form, dirty, error)
        });
        let Some((form, dirty, error)) = current else {
            s.set_resolved_url("".into());
            s.set_url_segments(ModelRc::default());
            return;
        };
        let flipped = self
            .with_active_mut(|t| std::mem::replace(&mut t.dirty, dirty) != dirty)
            .unwrap_or(false);
        if flipped {
            self.push_tabs();
        }
        s.set_body_error(
            error
                .map(|e| format!("Variables: {e}"))
                .unwrap_or_default()
                .into(),
        );
        s.set_param_count(active_count(&form.params));
        s.set_header_count(active_count(&form.headers));
        s.set_body_count(body_count(&form.body));
        let note = match self.tab_root() {
            Some(root) => workspace_rules::inherit_note(self.auth_of(&root).as_ref()),
            None => workspace_rules::inherit_note_untitled(),
        };
        s.set_inherited_auth(note.into());
        let out = self.outgoing(form);
        s.set_auth_blocked(out.auth.as_ref().is_some_and(|a| !a.is_sendable()));
        s.set_resolved_url(workspace_rules::resolved_url(&out, |n| self.lookup(n)).into());
        s.set_url_segments(self.url_segments(&s.get_url()));
        if s.get_dirty() != dirty {
            s.set_dirty(dirty);
            self.refresh_tree();
        }
        self.schedule_autosave();
    }

    fn kv_model(&self, table: KvTable) -> &VecModel<KvRow> {
        match table {
            KvTable::Params => self.params.as_ref(),
            KvTable::Headers => self.headers.as_ref(),
            KvTable::Form => self.form_rows.as_ref(),
        }
    }

    /// Typing into the placeholder row turns it into a real row.
    pub(crate) fn kv_edited(&self, table: KvTable, index: i32) {
        grow_rows(self.kv_model(table), index);
        self.changed();
    }

    pub(crate) fn kv_removed(&self, table: KvTable, index: i32) {
        remove_row(self.kv_model(table), index);
        self.changed();
    }

    /// false when not written: failed, invalid body, or Save as opened instead.
    pub(crate) fn save(&self) -> bool {
        let untitled = self.with_active(|t| t.file.is_none()).unwrap_or(false);
        // rewriting an unchanged file clobbers outside edits and YAML comments
        if !untitled && !self.is_dirty() {
            return true;
        }
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        if let Some(e) = self.body_error(&s) {
            self.banner(&e);
            return false;
        }
        if untitled {
            self.open_save_as();
            return false;
        }
        let target = self
            .with_active(|t| Some((t.file.as_ref()?.path.clone(), self.form(&s, &t.saved))))
            .flatten();
        let Some((path, form)) = target else {
            return false;
        };
        match self.deps.collections.save_request(&path, &form) {
            Ok(()) => {
                // paths are unique across collections
                for c in self.collections.borrow_mut().iter_mut() {
                    set_method(&mut c.children, &path, form.method);
                }
                self.with_active_mut(|t| t.saved = form);
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

pub(crate) fn placeholder() -> KvRow {
    KvRow {
        enabled: true,
        ..KvRow::default()
    }
}

/// Typing into the placeholder row turns it into a real row.
pub(crate) fn grow_rows(model: &VecModel<KvRow>, index: i32) {
    let last = model.row_count().saturating_sub(1);
    let filled_last = usize::try_from(index).is_ok_and(|i| i == last)
        && model
            .row_data(last)
            .is_some_and(|r| !r.key.is_empty() || !r.value.is_empty());
    if filled_last {
        model.push(placeholder());
    }
}

/// The placeholder row stays.
pub(crate) fn remove_row(model: &VecModel<KvRow>, index: i32) {
    if let Ok(i) = usize::try_from(index)
        && i + 1 < model.row_count()
    {
        model.remove(i);
    }
}

fn with_placeholder(rows: &[KeyValue]) -> Vec<KvRow> {
    rows.iter()
        .map(|r| KvRow {
            enabled: r.enabled,
            key: r.key.as_str().into(),
            value: r.value.as_str().into(),
            file: false,
            secret: false,
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
