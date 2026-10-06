#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use slint::ComponentHandle;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ui = app::ui::AppWindow::new()?;
    // dropped after run(): callbacks hold only weak refs to it
    let _workspace = app::di::build(&ui)?;
    ui.run()?;
    Ok(())
}
