#![deny(unsafe_code)]

mod branch;
mod history;
mod monitor;
mod pane;
mod persist;
mod ui;
mod update;
mod usage;

use crate::ui::App;
use log::info;
use simplelog::{LevelFilter, WriteLogger};
use std::fs::File;

fn main() -> anyhow::Result<()> {
    // Handle subcommands before TUI setup
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        match args[1].as_str() {
            "update" => return update::self_update(),
            "version" | "--version" | "-V" => {
                println!("mtt v{}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            _ => {}
        }
    }

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

    // Background version check (non-blocking)
    std::thread::spawn(|| {
        if let Some(latest) = update::check_for_update_cached() {
            eprintln!("\x1b[33mmtt v{latest} available — run `mtt update`\x1b[0m");
        }
    });

    let mut app = App::new();
    app.restore_sessions();
    app.run()
}
