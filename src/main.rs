#![deny(unsafe_code)]

mod pane;
mod ui;

use crate::ui::App;

fn main() -> anyhow::Result<()> {
    let mut app = App::new();
    app.run()
}
