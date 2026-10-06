//! Close tabs or quit without losing edits: ask per dirty scratch tab, stop on a failed save.

use slint::ComponentHandle;

use crate::modules::workspace::workspace_controller::{PendingAction, WorkspaceController};
use crate::ui::{DialogKind, WorkspaceState};

impl WorkspaceController {
    /// Close `ids` in order; stops at a dialog or a file tab that will not save.
    pub(crate) fn close_tabs(&self, ids: Vec<u64>) {
        self.advance(PendingAction::CloseTabs(ids));
    }

    /// Run a close sequence; true once every tab passed, false while asking or stopped.
    pub(crate) fn advance(&self, action: PendingAction) -> bool {
        let (mut ids, window) = match action {
            PendingAction::CloseTabs(ids) => (ids, false),
            PendingAction::CloseWindow(ids) => (ids, true),
            PendingAction::Delete(_) => return true,
        };
        while let Some(&id) = ids.first() {
            let tab = self
                .tabs
                .borrow()
                .iter()
                .find(|t| t.id == id)
                .map(|t| (t.file.is_some(), t.dirty));
            match tab {
                Some((false, true)) => {
                    self.ask_untitled(id, ids, window);
                    return false;
                }
                // save() banners why: failed write or invalid Variables
                Some((true, true)) => {
                    self.switch_to(id);
                    if !self.save() {
                        return false;
                    }
                }
                _ => {}
            }
            ids.remove(0);
            // quit keeps tabs: the session restores them
            if !window {
                self.remove_id(id, true);
            }
        }
        true
    }

    fn ask_untitled(&self, id: u64, ids: Vec<u64>, window: bool) {
        self.switch_to(id);
        let title = self
            .tabs
            .borrow()
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.title.clone())
            .unwrap_or_default();
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        s.set_dialog_kind(DialogKind::Untitled);
        s.set_confirm_name(title.into());
        s.set_confirm_file("".into());
        s.set_confirm_folder(false);
        s.set_confirm_open(true);
        *self.pending.borrow_mut() = Some(if window {
            PendingAction::CloseWindow(ids)
        } else {
            PendingAction::CloseTabs(ids)
        });
    }

    /// `flush`: false drops the edits (Discard).
    fn remove_id(&self, id: u64, flush: bool) {
        let index = self.tabs.borrow().iter().position(|t| t.id == id);
        match index {
            Some(i) if flush => self.close_tab(i),
            Some(i) => self.remove_tab(i),
            None => {}
        }
    }

    /// Continue after the asked tab was saved or discarded; quit hides when done.
    pub(crate) fn resume_pending(&self) {
        let action = self.pending.borrow_mut().take();
        let Some(action) = action else { return };
        let window = matches!(action, PendingAction::CloseWindow(_));
        if self.advance(action) && window {
            let _ = self.ui().hide();
        }
    }

    pub(crate) fn confirm_discard(&self) {
        let mut action = self.close_dialog();
        if let Some(PendingAction::CloseTabs(ids) | PendingAction::CloseWindow(ids)) = &mut action
            && !ids.is_empty()
        {
            let id = ids.remove(0);
            self.remove_id(id, false);
        }
        *self.pending.borrow_mut() = action;
        self.resume_pending();
    }

    /// The pending close resumes once Save as succeeds.
    pub(crate) fn confirm_save_as(&self) {
        self.ui().global::<WorkspaceState>().set_confirm_open(false);
        let id = match &*self.pending.borrow() {
            Some(PendingAction::CloseTabs(ids) | PendingAction::CloseWindow(ids)) => {
                ids.first().copied()
            }
            _ => None,
        };
        if let Some(id) = id {
            self.switch_to(id);
            self.open_save_as();
        }
    }
}
