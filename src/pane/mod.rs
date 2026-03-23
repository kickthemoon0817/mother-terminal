use anyhow::Result;
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Supported AI CLI types.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CLIType {
    Claude,
    Codex,
    Gemini,
    OpenCode,
}

impl CLIType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "gemini" => Some(Self::Gemini),
            "opencode" => Some(Self::OpenCode),
            _ => None,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::OpenCode => "opencode",
        }
    }
}

/// Session status.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Status {
    Active,
    Stalled,
    Dead,
}

/// A pre-rendered scrollback line with per-cell color info.
#[derive(Clone)]
pub struct ScrollLine {
    pub cells: Vec<ScrollCell>,
}

#[derive(Clone)]
pub struct ScrollCell {
    pub ch: String,
    pub fg: vt100::Color,
    pub bg: vt100::Color,
    pub bold: bool,
}

/// A pane holding an AI CLI session with its PTY and virtual screen.
pub struct Pane {
    pub id: usize,
    pub cli: CLIType,
    pub cwd: String,
    pub status: Status,
    pub started: Instant,
    pub scroll_offset: usize,
    pub scrollback: Vec<ScrollLine>, // pre-rendered lines for instant scroll

    writer: Box<dyn Write + Send>,
    buffer: Arc<Mutex<Vec<u8>>>,
    _raw_history: Arc<Mutex<Vec<u8>>>,
    parser: vt100::Parser,
    last_top_line: String, // detect when top line scrolls off
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send>,
}

impl Pane {
    /// Spawn a new AI CLI process in a PTY.
    /// `extra_args` are passed directly to the CLI (e.g., `--dangerously-skip-permissions`).
    pub fn spawn(id: usize, cli: CLIType, cwd: &str, rows: u16, cols: u16, extra_args: &[&str]) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new(cli.name());
        for arg in extra_args {
            cmd.arg(arg);
        }
        cmd.cwd(cwd);

        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        // Shared buffers for async PTY output
        let buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::with_capacity(8192)));
        let raw_history: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::with_capacity(65536)));
        let buf_clone = Arc::clone(&buffer);
        let hist_clone = Arc::clone(&raw_history);

        // Background thread reads PTY output continuously
        std::thread::spawn(move || {
            let mut tmp = [0u8; 4096];
            loop {
                match reader.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Ok(mut buf) = buf_clone.lock() {
                            buf.extend_from_slice(&tmp[..n]);
                        }
                        if let Ok(mut hist) = hist_clone.lock() {
                            hist.extend_from_slice(&tmp[..n]);
                            // Cap at 1MB to prevent unbounded growth
                            if hist.len() > 1_048_576 {
                                let drain = hist.len() - 524_288;
                                hist.drain(..drain);
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            id,
            cli,
            cwd: cwd.to_string(),
            status: Status::Active,
            started: Instant::now(),
            scroll_offset: 0,
            scrollback: Vec::new(),
            writer,
            buffer,
            _raw_history: raw_history,
            parser: vt100::Parser::new(rows, cols, 1000),
            last_top_line: String::new(),
            master: pair.master,
            child,
        })
    }

    /// Drain buffered PTY output and update the virtual screen.
    /// Non-blocking — returns true if there was new data.
    pub fn poll_output(&mut self) -> bool {
        let data = {
            let mut buf = match self.buffer.lock() {
                Ok(b) => b,
                Err(_) => return false,
            };
            if buf.is_empty() {
                return false;
            }
            let data = buf.clone();
            buf.clear();
            data
        };

        // Capture top line before processing (to detect scroll)
        let screen = self.parser.screen();
        let cols = screen.size().1;
        let mut old_top = String::new();
        for col in 0..cols {
            if let Some(cell) = screen.cell(0, col) {
                old_top.push_str(&cell.contents());
            }
        }

        self.parser.process(&data);

        // If top line changed, the old one scrolled off — save it to scrollback
        let screen = self.parser.screen();
        let mut new_top = String::new();
        for col in 0..cols {
            if let Some(cell) = screen.cell(0, col) {
                new_top.push_str(&cell.contents());
            }
        }

        if !self.last_top_line.is_empty() && old_top != new_top && old_top != self.last_top_line {
            // Capture the old top line with colors
            // (we already lost the cells, so use last_top_line as plain text)
            let cells: Vec<ScrollCell> = self.last_top_line.chars().map(|c| ScrollCell {
                ch: c.to_string(),
                fg: vt100::Color::Default,
                bg: vt100::Color::Default,
                bold: false,
            }).collect();
            self.scrollback.push(ScrollLine { cells });

            // Cap scrollback at 5000 lines
            if self.scrollback.len() > 5000 {
                self.scrollback.drain(..1000);
            }
        }
        self.last_top_line = new_top;

        true
    }

    /// Send keystrokes to the PTY.
    pub fn send_keys(&mut self, data: &[u8]) -> Result<()> {
        self.writer.write_all(data)?;
        self.writer.flush()?;
        Ok(())
    }

    /// Send a text string followed by Enter.
    pub fn send_text(&mut self, text: &str) -> Result<()> {
        self.writer.write_all(text.as_bytes())?;
        self.writer.write_all(b"\r")?;
        self.writer.flush()?;
        Ok(())
    }

    /// Get the current virtual screen.
    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    /// Resize the PTY and virtual screen.
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        self.parser.set_size(rows, cols);
        Ok(())
    }

    /// Check if the child process is still running.
    pub fn is_alive(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(Some(_)) => {
                self.status = Status::Dead;
                false
            }
            Ok(None) => true,
            Err(_) => {
                self.status = Status::Dead;
                false
            }
        }
    }

    /// Get the maximum scroll offset (number of scrollback lines).
    pub fn max_scroll(&self) -> usize {
        self.scrollback.len()
    }


    /// Kill the child process and reap it to prevent zombies.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        // Wait to reap the child process (prevent zombie)
        let _ = self.child.wait();
        self.status = Status::Dead;
    }

    /// Get the screen contents as a string (for history recording).
    pub fn screen_text(&self) -> String {
        let screen = self.parser.screen();
        let mut lines = Vec::new();
        for row in 0..screen.size().0 {
            let mut line = String::new();
            for col in 0..screen.size().1 {
                if let Some(cell) = screen.cell(row, col) {
                    line.push(cell.contents().chars().next().unwrap_or(' '));
                }
            }
            lines.push(line.trim_end().to_string());
        }
        lines.join("\n")
    }
}
