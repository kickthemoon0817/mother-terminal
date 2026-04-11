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

    // Background version check (non-blocking) — result passed to App via shared state
    let update_notice: std::sync::Arc<std::sync::Mutex<Option<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let notice_clone = std::sync::Arc::clone(&update_notice);
    std::thread::spawn(move || {
        if let Some(latest) = update::check_for_update_cached()
            && let Ok(mut n) = notice_clone.lock() {
                *n = Some(format!("mtt v{latest} available — run `mtt update`"));
            }
    });

    let mut app = App::new();
    app.update_notice = update_notice;
    app.restore_sessions();
    app.run()
}
