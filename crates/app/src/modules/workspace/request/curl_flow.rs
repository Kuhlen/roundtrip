//! cURL in: a pasted command fills a request.

use domain::collection::Protocol;
use domain::curl;
use domain::http::Request;
use slint::ComponentHandle;

use crate::modules::workspace::workspace_controller::WorkspaceController;
use crate::modules::workspace::workspace_rules::{self, error_text};
use crate::ui::WorkspaceState;

impl WorkspaceController {
    /// URL field edit: a pasted `curl ...` is parsed, anything else is a normal edit.
    pub(crate) fn url_edited(&self) {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        let after = s.get_url().to_string();
        let before = self.last_url.borrow().clone();
        if !workspace_rules::is_curl_paste(&before, &after) {
            self.changed();
            return;
        }
        match curl::parse(&after) {
            Ok(import) => {
                // curl text must never reach the tab it was pasted in (autosave)
                s.set_url(before.as_str().into());
                self.fill_from_curl(import.request, before.is_empty());
                if !import.skipped.is_empty() {
                    let hint = format!("Skipped: {}", import.skipped.join(", "));
                    self.notice("Imported from cURL", &hint);
                }
            }
            Err(e) => {
                // file-backed tab: autosave would write the curl text into the file
                if self.with_active(|t| t.file.is_some()).unwrap_or(false) {
                    s.set_url(before.as_str().into());
                }
                self.changed();
                let (_, why) = error_text(&e, None);
                self.notice("Couldn't read cURL command", &why);
            }
        }
    }

    /// Into the empty Untitled tab it was pasted in, else a new one; dirty either way.
    fn fill_from_curl(&self, request: Request, url_was_empty: bool) {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        let reuse = url_was_empty
            && self
                .with_active(|t| {
                    t.file.is_none()
                        && t.protocol == Protocol::Http
                        && self.form(&s, &t.saved) == Request::default()
                })
                .unwrap_or(false);
        if !reuse {
            self.new_tab();
        }
        // through `saved` so every form field is set; then forget it, nothing is on disk
        self.with_active_mut(|t| t.saved = request);
        self.fill_saved(&s);
        self.with_active_mut(|t| t.saved = Request::default());
        self.changed();
    }

    /// Copy as cURL: what Send would send, as a command. "" when it can't be said; banner tells why.
    pub(crate) fn curl_command(&self) -> String {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        if !s.get_unsupported_protocol().is_empty() {
            self.notice(
                "Can't copy as cURL",
                "Only HTTP and GraphQL requests can be copied.",
            );
            return String::new();
        }
        // files re-read, as on Send
        self.refresh_vars();
        let Some(form) = self.with_active(|t| self.outgoing(self.form(&s, &t.saved))) else {
            return String::new();
        };
        let mut request = workspace_rules::interpolate_request(&form, |n| self.lookup(n));
        if let Some(root) = self.tab_root() {
            workspace_rules::resolve_files(&mut request, &root);
        }
        match curl::to_command(&request) {
            Ok(command) => command,
            Err(e) => {
                let (_, why) = error_text(&e, None);
                self.notice("Can't copy as cURL", &why);
                String::new()
            }
        }
    }
}
