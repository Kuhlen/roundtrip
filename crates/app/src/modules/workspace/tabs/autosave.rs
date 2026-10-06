//! Save the active file-backed tab 1 s after its last edit, like ApiArk.

use std::time::Duration;

use slint::{ComponentHandle, TimerMode};

use crate::modules::workspace::workspace_controller::WorkspaceController;
use crate::ui::WorkspaceState;

const DELAY: Duration = Duration::from_secs(1);

impl WorkspaceController {
    /// Restart the timer; only file-backed tabs with a valid body qualify.
    pub(crate) fn schedule_autosave(&self) {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        let file_backed = self.with_active(|t| t.file.is_some()).unwrap_or(false);
        if !file_backed || !s.get_dirty() || self.body_error(&s).is_some() {
            self.autosave.stop();
            return;
        }
        let me = self.me.clone();
        self.autosave.start(TimerMode::SingleShot, DELAY, move || {
            if let Some(c) = me.upgrade() {
                c.flush();
            }
        });
    }

    /// Save now what the timer would save; quiet when the body cannot be saved.
    pub fn flush(&self) {
        self.autosave.stop();
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        let file_backed = self.with_active(|t| t.file.is_some()).unwrap_or(false);
        if file_backed && s.get_dirty() && self.body_error(&s).is_none() {
            self.save();
        }
    }
}
