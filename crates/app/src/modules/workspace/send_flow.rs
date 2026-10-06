//! Send on a worker thread; the result lands in the tab that sent it.

use std::time::{Duration, Instant};

use domain::AppError;
use domain::http::Response;
use slint::ComponentHandle;

use super::tabs::tab::ResponseView;
use super::workspace_controller::WorkspaceController;
use super::workspace_rules;
use crate::ui::WorkspaceState;

/// (tab id, result, elapsed, pretty body)
pub(super) type Delivery = (u64, Result<Response, AppError>, Duration, Option<String>);

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
        let request = self.with_active(|t| {
            let out = self.outgoing(self.form(&s, &t.saved));
            workspace_rules::interpolate_request(&out, |n| self.lookup(n))
        });
        let Some(mut request) = request else { return };
        if let Some(root) = self.tab_root() {
            workspace_rules::resolve_files(&mut request, &root);
        }
        let sending = ResponseView::sending();
        sending.show(&s);
        self.with_active_mut(|t| t.response = sending);
        let sender = self.deps.sender.clone();
        let pretty_json = self.deps.pretty_json;
        let tx = self.responses.0.clone();
        let weak = self.ui.clone();
        // Rc controller stays on the UI thread: result goes by channel, then a wake-up
        std::thread::spawn(move || {
            let start = Instant::now();
            let result = sender.send(&request);
            let elapsed = start.elapsed();
            let pretty = result.as_ref().ok().and_then(|r| pretty_json(&r.body));
            if tx.send((id, result, elapsed, pretty)).is_ok() {
                let _ = weak.upgrade_in_event_loop(|ui| {
                    ui.global::<WorkspaceState>().invoke_response_ready();
                });
            }
        });
    }

    /// Route queued results: shown on the active tab, stored on others, dropped for closed ones.
    pub(super) fn deliver_responses(&self) {
        let deliveries: Vec<Delivery> = self.responses.1.borrow().try_iter().collect();
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        for (id, result, elapsed, pretty) in deliveries {
            let view = ResponseView::from_result(result, elapsed, pretty);
            if self.active.get() == Some(id) {
                view.show(&s);
            }
            if let Some(t) = self.tabs.borrow_mut().iter_mut().find(|t| t.id == id) {
                t.response = view;
            }
        }
    }
}
