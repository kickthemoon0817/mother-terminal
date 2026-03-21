use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

#[test]
fn test_pty_spawn_send_read() {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let cmd = CommandBuilder::new("bash");
    let _child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("reader");
    let mut writer = pair.master.take_writer().expect("writer");

    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let buf2 = Arc::clone(&buf);
    std::thread::spawn(move || {
        let mut tmp = [0u8; 4096];
        loop {
            match reader.read(&mut tmp) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    buf2.lock().unwrap().extend_from_slice(&tmp[..n]);
                }
            }
        }
    });

    std::thread::sleep(Duration::from_millis(500));
    writer.write_all(b"echo PTY_TEST_OK\r").unwrap();
    writer.flush().unwrap();
    std::thread::sleep(Duration::from_millis(500));

    let data = buf.lock().unwrap().clone();
    let output = String::from_utf8_lossy(&data);
    assert!(
        output.contains("PTY_TEST_OK"),
        "PTY output missing. Got: {}",
        &output[..output.len().min(200)]
    );
}

#[test]
fn test_vt100_parser_renders_output() {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let cmd = CommandBuilder::new("bash");
    let _child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("reader");
    let mut writer = pair.master.take_writer().expect("writer");

    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let buf2 = Arc::clone(&buf);
    std::thread::spawn(move || {
        let mut tmp = [0u8; 4096];
        loop {
            match reader.read(&mut tmp) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    buf2.lock().unwrap().extend_from_slice(&tmp[..n]);
                }
            }
        }
    });

    std::thread::sleep(Duration::from_millis(500));
    writer.write_all(b"echo VT100_RENDER_OK\r").unwrap();
    writer.flush().unwrap();
    std::thread::sleep(Duration::from_millis(500));

    let data = buf.lock().unwrap().clone();
    let mut parser = vt100::Parser::new(24, 80, 100);
    parser.process(&data);

    let screen = parser.screen();
    let mut found = false;
    for row in 0..24u16 {
        let mut line = String::new();
        for col in 0..80u16 {
            if let Some(cell) = screen.cell(row, col) {
                line.push(cell.contents().chars().next().unwrap_or(' '));
            }
        }
        if line.contains("VT100_RENDER_OK") {
            found = true;
            break;
        }
    }
    assert!(found, "vt100 screen doesn't contain VT100_RENDER_OK");
}
