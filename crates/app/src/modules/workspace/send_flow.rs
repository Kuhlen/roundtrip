//! Send on a worker thread; the result lands back on the UI thread.

use std::time::{Duration, Instant};

use domain::AppError;
use domain::http::Response;
use slint::{ComponentHandle, ModelRc, VecModel};

use super::workspace_controller::WorkspaceController;
use super::workspace_rules;
use crate::ui::{AppWindow, KvRow, SendStatus, StatusClass, WorkspaceState};

impl WorkspaceController {
    pub(super) fn send(&self) {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        if !s.get_can_send() {
            return;
        }
        // files re-read on each Send
        self.refresh_vars();
        let request = self.active.borrow().as_ref().map(|a| {
            let out = self.outgoing(self.form(&s, a));
            workspace_rules::interpolate_request(&out, |n| self.lookup(n))
        });
        let Some(request) = request else { return };
        s.set_send_status(SendStatus::Sending);
        let sender = self.deps.sender.clone();
        let pretty_json = self.deps.pretty_json;
        let weak = self.ui.clone();
        std::thread::spawn(move || {
            let start = Instant::now();
            let result = sender.send(&request);
            let elapsed = start.elapsed();
            let pretty = result.as_ref().ok().and_then(|r| pretty_json(&r.body));
            // Send is disabled while sending: no newer response can be overwritten
            let _ = weak.upgrade_in_event_loop(move |ui| show_result(&ui, result, elapsed, pretty));
        });
    }
}

pub(super) fn reset_response(s: &WorkspaceState) {
    s.set_send_status(SendStatus::Idle);
    s.set_status_text("".into());
    s.set_status_class(StatusClass::Ok);
    s.set_elapsed("".into());
    s.set_size("".into());
    s.set_truncated(false);
    s.set_response_raw("".into());
    s.set_response_pretty("".into());
    s.set_response_headers(ModelRc::default());
    s.set_fail_title("".into());
    s.set_fail_hint("".into());
    s.set_fail_detail("".into());
}

/// Runs on the UI thread with the worker's result.
fn show_result(
    ui: &AppWindow,
    result: Result<Response, AppError>,
    elapsed: Duration,
    pretty: Option<String>,
) {
    let s = ui.global::<WorkspaceState>();
    match result {
        Ok(r) => {
            s.set_status_text(format!("{} {}", r.status, r.status_text).into());
            s.set_status_class(match r.status {
                500.. => StatusClass::Err,
                300.. => StatusClass::Warn,
                _ => StatusClass::Ok,
            });
            s.set_elapsed(format!("{} ms", r.elapsed.as_millis()).into());
            s.set_size(workspace_rules::format_size(r.size_bytes).into());
            s.set_truncated(r.truncated);
            s.set_response_raw(r.body.into());
            s.set_response_pretty(pretty.unwrap_or_default().into());
            let headers: Vec<KvRow> = r
                .headers
                .into_iter()
                .map(|h| KvRow {
                    enabled: true,
                    key: h.key.into(),
                    value: h.value.into(),
                })
                .collect();
            s.set_response_headers(ModelRc::new(VecModel::from(headers)));
            s.set_send_status(SendStatus::Done);
        }
        Err(e) => {
            let (title, hint) = workspace_rules::error_text(&e, None);
            s.set_elapsed(format!("{} ms", elapsed.as_millis()).into());
            s.set_fail_title(title.into());
            s.set_fail_hint(hint.into());
            s.set_fail_detail(e.to_string().into());
            s.set_send_status(SendStatus::Failed);
        }
    }
}
