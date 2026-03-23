#![deny(unsafe_code)]

mod history;
mod monitor;
mod pane;
mod persist;
mod ui;
mod usage;

use crate::ui::App;
use log::info;
use simplelog::{LevelFilter, WriteLogger};
use std::fs::File;

fn main() -> anyhow::Result<()> {
    // Init file logger → ~/.mtt/mtt.log
    let home = dirs::home_dir().unwrap_or_default();
    let log_dir = home.join(".mtt");
    std::fs::create_dir_all(&log_dir)?;
    let log_file = File::create(log_dir.join("mtt.log"))?;
    let config = simplelog::ConfigBuilder::new()
        .add_filter_ignore_str("vt100")
        .build();
    let _ = WriteLogger::init(LevelFilter::Debug, config, log_file);

    info!("mtt starting");

    let mut app = App::new();
    app.restore_sessions();
    app.run()
}
