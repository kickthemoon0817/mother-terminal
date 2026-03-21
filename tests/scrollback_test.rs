use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

#[test]
fn test_scrollback_replay() {
    // Spawn a PTY, generate some output, then test scrollback replay
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 10,
            cols: 40,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let cmd = CommandBuilder::new("bash");
    let _child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("reader");
    let mut writer = pair.master.take_writer().expect("writer");

    let raw: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let raw2 = Arc::clone(&raw);
    std::thread::spawn(move || {
        let mut tmp = [0u8; 4096];
        loop {
            match reader.read(&mut tmp) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    raw2.lock().unwrap().extend_from_slice(&tmp[..n]);
                }
            }
        }
    });

    std::thread::sleep(Duration::from_millis(300));

    // Generate 30 lines of output (more than 10 visible rows)
    for i in 1..=30 {
        writer.write_all(format!("echo LINE_{i}\r").as_bytes()).unwrap();
        writer.flush().unwrap();
        std::thread::sleep(Duration::from_millis(50));
    }

    std::thread::sleep(Duration::from_millis(500));

    // Replay into a tall parser
    let data = raw.lock().unwrap().clone();
    let tall_rows = 200u16;
    let cols = 40u16;
    let mut tall_parser = vt100::Parser::new(tall_rows, cols, 0);
    tall_parser.process(&data);

    let screen = tall_parser.screen();
    let (total_rows, _) = screen.size();

    // Find the last non-empty row
    let mut last_content_row = 0u16;
    for row in 0..total_rows {
        let mut has_content = false;
        for col in 0..cols {
            if let Some(cell) = screen.cell(row, col) {
                let ch = cell.contents();
                if !ch.is_empty() && ch != " " {
                    has_content = true;
                    break;
                }
            }
        }
        if has_content {
            last_content_row = row;
        }
    }

    println!("Total rows: {total_rows}");
    println!("Last content row: {last_content_row}");
    println!("Visible window: 10 rows");

    // The content should be somewhere in the tall screen
    assert!(last_content_row > 0, "No content found in replayed screen");

    // Simulate scroll: show last 10 rows of content
    let visible = 10u16;
    let scroll_offset = 0usize; // 0 = bottom (live view)

    // The correct start_row should be: last_content_row - visible + 1 - scroll_offset
    let start = last_content_row.saturating_sub(visible - 1).saturating_sub(scroll_offset as u16);
    println!("Start row for scroll_offset 0: {start}");

    // Verify content is visible at this position
    let mut found_line = false;
    for row in start..start + visible {
        let mut line = String::new();
        for col in 0..cols {
            if let Some(cell) = screen.cell(row, col) {
                line.push(cell.contents().chars().next().unwrap_or(' '));
            }
        }
        let trimmed = line.trim();
        if trimmed.contains("LINE_") {
            found_line = true;
        }
    }

    assert!(found_line, "LINE_ not found in visible area at offset 0");

    // Now test scroll up by 10
    let scroll_offset = 10usize;
    let start = last_content_row.saturating_sub(visible - 1).saturating_sub(scroll_offset as u16);
    println!("Start row for scroll_offset 10: {start}");

    let mut found_earlier_line = false;
    for row in start..start + visible {
        let mut line = String::new();
        for col in 0..cols {
            if let Some(cell) = screen.cell(row, col) {
                line.push(cell.contents().chars().next().unwrap_or(' '));
            }
        }
        let trimmed = line.trim();
        if trimmed.contains("LINE_") {
            found_earlier_line = true;
        }
    }

    println!("Found content at offset 10: {found_earlier_line}");
}
