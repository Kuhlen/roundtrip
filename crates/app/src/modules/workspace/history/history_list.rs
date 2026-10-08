//! Load, search, reopen and clear history entries.

use std::time::SystemTime;

use domain::history::HistoryEntry;
use slint::{ComponentHandle, ModelRc, VecModel};

use super::history_rules::relative_time;
use crate::modules::workspace::tabs::tab::status_class;
use crate::modules::workspace::workspace_controller::WorkspaceController;
use crate::modules::workspace::workspace_rules::error_text;
use crate::ui::{DialogKind, HistoryRow, StatusClass, WorkspaceState};

const LIMIT: usize = 50;

impl WorkspaceController {
    /// Newest entries, or matches of the search box.
    pub(crate) fn load_history(&self) {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        let query = s.get_history_query().trim().to_owned();
        let result = match &self.deps.history {
            Ok(h) if query.is_empty() => h.recent(LIMIT),
            // ponytail: LIKE scan on UI thread per keystroke; debounce or FTS index when history grows
            Ok(h) => h.search(&query, LIMIT),
            Err(e) => Err(e.clone()),
        };
        let (entries, note) = match result {
            Ok(entries) if entries.is_empty() && query.is_empty() => {
                (entries, "No history yet".to_owned())
            }
            Ok(entries) if entries.is_empty() => (entries, "No matches".to_owned()),
            Ok(entries) => (entries, String::new()),
            Err(e) => {
                let (title, hint) = error_text(&e, None);
                let why = if hint.is_empty() { title } else { hint };
                (Vec::new(), format!("History is unavailable: {why}"))
            }
        };
        let now = SystemTime::now();
        let rows: Vec<HistoryRow> = entries.iter().map(|e| row(e, now)).collect();
        *self.history_ids.borrow_mut() = entries.iter().map(|e| e.id).collect();
        s.set_history(ModelRc::new(VecModel::from(rows)));
        s.set_history_note(note.into());
    }

    /// Clean Untitled tab: closing it loses nothing, history still has the request.
    pub(crate) fn open_history(&self, index: i32) {
        let id = usize::try_from(index)
            .ok()
            .and_then(|i| self.history_ids.borrow().get(i).copied());
        let (Some(id), Ok(history)) = (id, &self.deps.history) else {
            return;
        };
        match history.request(id) {
            Ok(request) => self.open_untitled(request),
            Err(e) => self.banner(&e),
        }
    }

    pub(crate) fn ask_clear_history(&self) {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        s.set_dialog_kind(DialogKind::ClearHistory);
        s.set_confirm_open(true);
    }

    pub(crate) fn confirm_clear_history(&self) {
        self.close_dialog();
        let Ok(history) = &self.deps.history else {
            return;
        };
        match history.clear() {
            Ok(()) => self.load_history(),
            Err(e) => self.banner(&e),
        }
    }
}

fn row(e: &HistoryEntry, now: SystemTime) -> HistoryRow {
    HistoryRow {
        method: e.method.as_str().into(),
        url: e.url.as_str().into(),
        status: e.status.map_or("—".to_owned(), |c| c.to_string()).into(),
        status_class: e.status.map_or(StatusClass::Err, status_class),
        failed: e.status.is_none(),
        time: relative_time(now, e.at).into(),
    }
}
