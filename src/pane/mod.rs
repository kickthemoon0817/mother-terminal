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
    Shell, // plain bash/zsh
}

impl CLIType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "gemini" => Some(Self::Gemini),
            "opencode" => Some(Self::OpenCode),
            "bash" | "zsh" | "sh" | "shell" => Some(Self::Shell),
            _ => None,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::OpenCode => "opencode",
            Self::Shell => "sh",
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

#[derive(Clone, Copy)]
pub struct ScrollCell {
    pub ch: char,
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
    parser: vt100::Parser,
    last_snapshot: String, // for dedup — only store if screen changed
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

        let binary = if cli == CLIType::Shell {
            std::env::var("SHELL").unwrap_or_else(|_| "bash".to_string())
        } else {
            cli.name().to_string()
        };
        let mut cmd = CommandBuilder::new(&binary);
        for arg in extra_args {
            cmd.arg(arg);
        }
        cmd.cwd(cwd);

        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        // Shared buffer for async PTY output
        let buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::with_capacity(8192)));
        let buf_clone = Arc::clone(&buffer);

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
            parser: vt100::Parser::new(rows, cols, 1000),
            last_snapshot: String::new(),
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
            let mut data = Vec::with_capacity(buf.len());
            std::mem::swap(&mut *buf, &mut data);
            data
        };

        self.parser.process(&data);
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

    /// Capture current screen as scrollback snapshot (called from slow tick).
    /// Only stores if screen content changed since last snapshot.
    pub fn capture_scrollback_snapshot(&mut self) {
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();

        // Build a quick hash string to check if screen changed
        let mut check = String::with_capacity(256);
        for row in 0..rows.min(5) {
            for col in 0..cols.min(40) {
                if let Some(cell) = screen.cell(row, col) {
                    let ch = cell.contents();
                    if !ch.is_empty() {
                        check.push_str(&ch);
                    }
                }
            }
        }

        if check == self.last_snapshot {
            return; // Screen unchanged, skip
        }
        self.last_snapshot = check;

        // Capture all rows as scrollback lines with color info
        for row in 0..rows {
            let mut cells = Vec::with_capacity(cols as usize);
            let mut has_content = false;
            for col in 0..cols {
                if let Some(cell) = screen.cell(row, col) {
                    let ch = cell.contents();
                    let c = ch.chars().next().unwrap_or(' ');
                    if c != ' ' { has_content = true; }
                    cells.push(ScrollCell {
                        ch: c,
                        fg: cell.fgcolor(),
                        bg: cell.bgcolor(),
                        bold: cell.bold(),
                    });
                }
            }
            if has_content {
                self.scrollback.push(ScrollLine { cells });
            }
        }

        // Cap scrollback at 5000 lines
        if self.scrollback.len() > 5000 {
            self.scrollback.drain(..1000);
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
