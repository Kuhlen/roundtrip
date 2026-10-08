//! Send on a worker thread; the result lands in the tab that sent it, unless cancelled.

use std::time::{Duration, Instant, SystemTime};

use domain::AppError;
use domain::history::{HistoryEntry, redact};
use domain::http::{CancelFlag, Response};
use slint::ComponentHandle;

use super::tabs::tab::ResponseView;
use super::workspace_controller::WorkspaceController;
use super::workspace_rules;
use crate::ui::WorkspaceState;

/// (tab id, send number, result, elapsed, pretty body)
pub(super) type Delivery = (
    u64,
    u64,
    Result<Response, AppError>,
    Duration,
    Option<String>,
);

impl WorkspaceController {
    pub(super) fn send(&self) {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        if !s.get_can_send() {
            return;
        }
        let Some(id) = self.active.get() else { return };
        // files re-read on each Send
        self.refresh_vars();
        let Some(form) = self.with_active(|t| self.outgoing(self.form(&s, &t.saved))) else {
            return;
        };
        let root = self.tab_root();
        let mut request = workspace_rules::interpolate_request(&form, |n| self.lookup(n));
        // stored before interpolation: the reopened tab keeps {{vars}}
        let mut stored = form;
        if let Some(root) = &root {
            workspace_rules::resolve_files(&mut request, root);
            // a reopened Untitled tab has no collection to resolve against
            workspace_rules::resolve_files(&mut stored, root);
        }
        let entry = HistoryEntry {
            id: 0,
            method: request.method,
            url: request.url.clone(),
            status: None,
            status_text: None,
            time_ms: None,
            size_bytes: None,
            at: SystemTime::now(),
            collection: root,
            name: self
                .with_active(|t| t.file.as_ref().map(|_| t.title.clone()))
                .flatten(),
        };
        let number = self.send_seq.get() + 1;
        self.send_seq.set(number);
        let cancel = CancelFlag::new();
        let sending = ResponseView::sending();
        sending.show(&s);
        self.with_active_mut(|t| {
            t.response = sending;
            t.sending = Some((number, cancel.clone()));
        });
        let sender = self.deps.sender.clone();
        let history = self.deps.history.as_ref().ok().cloned();
        let pretty_json = self.deps.pretty_json;
        let tx = self.responses.0.clone();
        let weak = self.ui.clone();
        // Rc controller stays on the UI thread: result goes by channel, then a wake-up
        std::thread::spawn(move || {
            let start = Instant::now();
            let result = sender.send(&request, &cancel);
            let elapsed = start.elapsed();
            // cancelled: no outcome worth keeping
            if !cancel.is_cancelled()
                && let Some(history) = history
            {
                // a failed write must not cost the response
                let _ = history.record(&finished(entry, &result), &redact(&stored));
            }
            let pretty = result.as_ref().ok().and_then(|r| pretty_json(&r.body));
            if tx.send((id, number, result, elapsed, pretty)).is_ok() {
                let _ = weak.upgrade_in_event_loop(|ui| {
                    ui.global::<WorkspaceState>().invoke_response_ready();
                });
            }
        });
    }

    /// Esc / Cancel: the worker drops the connection; its late result is ignored by number.
    pub(super) fn cancel(&self) {
        let Some((_, cancel)) = self.with_active_mut(|t| t.sending.take()).flatten() else {
            return;
        };
        cancel.cancel();
        let view = ResponseView::cancelled();
        view.show(&self.ui().global::<WorkspaceState>());
        self.with_active_mut(|t| t.response = view);
    }

    /// Route queued results: shown on the active tab, stored on others; stale or closed dropped.
    pub(super) fn deliver_responses(&self) {
        let deliveries: Vec<Delivery> = self.responses.1.borrow().try_iter().collect();
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        for (id, number, result, elapsed, pretty) in deliveries {
            let view = ResponseView::from_result(result, elapsed, pretty);
            let landed = self
                .tabs
                .borrow_mut()
                .iter_mut()
                .find(|t| t.id == id && t.sending.as_ref().is_some_and(|(n, _)| *n == number))
                .map(|t| {
                    t.sending = None;
                    t.response = view.clone();
                })
                .is_some();
            if landed && self.active.get() == Some(id) {
                view.show(&s);
            }
        }
        // new entries were written by the workers before queueing
        self.load_history();
    }
}

fn finished(entry: HistoryEntry, result: &Result<Response, AppError>) -> HistoryEntry {
    match result {
        Ok(r) => HistoryEntry {
            status: Some(r.status),
            status_text: Some(r.status_text.clone()),
            time_ms: Some(u64::try_from(r.elapsed.as_millis()).unwrap_or(u64::MAX)),
            size_bytes: Some(r.size_bytes),
            ..entry
        },
        Err(_) => entry,
    }
}
