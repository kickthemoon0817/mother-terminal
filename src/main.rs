#![deny(unsafe_code)]

mod history;
mod monitor;
mod pane;
mod persist;
mod ui;

use crate::ui::App;

fn main() -> anyhow::Result<()> {
    let mut app = App::new();
    app.run()
}
