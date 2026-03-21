#![deny(unsafe_code)]

mod history;
mod monitor;
mod pane;
mod persist;
mod ui;

use crate::ui::App;

fn main() -> anyhow::Result<()> {
    let mut app = App::new();

    // Restore previous sessions if any were saved
    app.restore_sessions();

    app.run()
}
