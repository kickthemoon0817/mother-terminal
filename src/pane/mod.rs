use anyhow::Result;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
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

/// A pane holding an AI CLI session with its PTY and virtual screen.
pub struct Pane {
    pub id: usize,
    pub cli: CLIType,
    pub cwd: String,
    pub status: Status,
    pub started: Instant,

    writer: Box<dyn Write + Send>,
    reader: Arc<Mutex<Box<dyn Read + Send>>>,
    parser: vt100::Parser,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send>,
}

impl Pane {
    /// Spawn a new AI CLI process in a PTY.
    pub fn spawn(id: usize, cli: CLIType, cwd: &str, rows: u16, cols: u16) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new(cli.name());
        cmd.cwd(cwd);

        let child = pair.slave.spawn_command(cmd)?;
        // Drop slave — the child process holds the only reference
        drop(pair.slave);

        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        Ok(Self {
            id,
            cli,
            cwd: cwd.to_string(),
            status: Status::Active,
            started: Instant::now(),
            writer,
            reader: Arc::new(Mutex::new(reader)),
            master: pair.master,
            parser: vt100::Parser::new(rows, cols, 1000), // 1000 lines scrollback
            child,
        })
    }

    /// Read new output from the PTY and update the virtual screen.
    /// Returns true if there was new data.
    pub fn poll_output(&mut self) -> bool {
        let mut buf = [0u8; 4096];
        let reader = self.reader.clone();
        let mut reader = match reader.try_lock() {
            Ok(r) => r,
            Err(_) => return false,
        };

        // Non-blocking read — set_non_blocking isn't available on all platforms,
        // so we rely on the caller to poll at intervals
        match reader.read(&mut buf) {
            Ok(0) => {
                // EOF — process exited
                self.status = Status::Dead;
                false
            }
            Ok(n) => {
                self.parser.process(&buf[..n]);
                true
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => false,
            Err(_) => {
                self.status = Status::Dead;
                false
            }
        }
    }

    /// Send keystrokes to the PTY.
    pub fn send_keys(&mut self, data: &[u8]) -> Result<()> {
        self.writer.write_all(data)?;
        Ok(())
    }

    /// Send a text string followed by Enter.
    pub fn send_text(&mut self, text: &str) -> Result<()> {
        self.writer.write_all(text.as_bytes())?;
        self.writer.write_all(b"\r")?;
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

    /// Kill the child process.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        self.status = Status::Dead;
    }

    /// Get the screen contents as a string (for history recording).
    pub fn screen_text(&self) -> String {
        let screen = self.parser.screen();
        let mut lines = Vec::new();
        for row in 0..screen.size().0 {
            let mut line = String::new();
            for col in 0..screen.size().1 {
                let cell = screen.cell(row, col).unwrap();
                line.push(cell.contents().chars().next().unwrap_or(' '));
            }
            lines.push(line.trim_end().to_string());
        }
        lines.join("\n")
    }
}
