//! Own binary: the event-loop testing backend can be set once per process.
mod support;

use std::sync::atomic::Ordering;

use app::ui::{SendStatus, WorkspaceState};
use slint::ComponentHandle;
use support::{build, fakes, open_with_env, row_of, run_until, state};

#[test]
fn late_reply_after_cancel_is_dropped() {
    i_slint_backend_testing::init_integration_test_with_system_time();
    let f = fakes();
    let (ui, c) = build(&f);
    open_with_env(&ui, &c);
    let s = state(&ui);
    s.invoke_row_clicked(row_of(&ui, "List users"));

    f.sender.hold.store(true, Ordering::SeqCst);
    s.invoke_send();
    s.invoke_cancel();
    let before = s.get_history();
    f.sender.hold.store(false, Ordering::SeqCst);
    // worker now returns 200 OK: too late. every delivery swaps the history model
    let w = ui.clone_strong();
    run_until(move || w.global::<WorkspaceState>().get_history() != before);
    assert_eq!(s.get_send_status(), SendStatus::Cancelled);
    assert_eq!(s.get_status_text(), "");

    s.invoke_send();
    let w = ui.clone_strong();
    run_until(move || w.global::<WorkspaceState>().get_send_status() == SendStatus::Done);
    assert_eq!(s.get_send_status(), SendStatus::Done);
    assert_eq!(s.get_status_text(), "200 OK");
}
