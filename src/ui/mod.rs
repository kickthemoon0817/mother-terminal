use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use std::io::stdout;
use std::time::{Duration, Instant};

use crate::pane::{CLIType, Pane, Status};

/// UI mode.
enum Mode {
    Normal,   // pane focused, keys go to AI CLI
    Command,  // typing a command in the command bar
}

/// The main application state.
pub struct App {
    pub panes: Vec<Pane>,
    pub focused: usize,
    mode: Mode,
    command_input: String,
    command_history: Vec<String>,
    history_cursor: usize,
    message: String,
    should_quit: bool,
    last_ctrl_c: Option<std::time::Instant>,
}

impl App {
    pub fn new() -> Self {
        Self {
            panes: Vec::new(),
            focused: 0,
            mode: Mode::Normal,
            command_input: String::new(),
            command_history: Vec::new(),
            history_cursor: 0,
            message: String::new(),
            should_quit: false,
            last_ctrl_c: None,
        }
    }

    /// Run the main event loop.
    pub fn run(&mut self) -> anyhow::Result<()> {
        // Setup terminal
        terminal::enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout());
        let mut terminal = Terminal::new(backend)?;

        // Main loop
        loop {
            // Clear stale Ctrl+C hint after timeout
            if let Some(t) = self.last_ctrl_c {
                if t.elapsed() > Duration::from_millis(500) {
                    self.last_ctrl_c = None;
                    self.message.clear();
                }
            }

            // Poll PTY output from all panes
            for pane in &mut self.panes {
                pane.poll_output();
            }

            // Draw UI
            terminal.draw(|frame| self.draw(frame))?;

            // Handle input — read ONE event per poll cycle
            if event::poll(Duration::from_millis(16))? {
                match event::read()? {
                    Event::Key(key) => self.handle_key(key),
                    Event::Resize(cols, rows) => self.handle_resize(cols, rows),
                    _ => {}
                }
            }

            if self.should_quit {
                break;
            }
        }

        // Cleanup
        terminal::disable_raw_mode()?;
        stdout().execute(LeaveAlternateScreen)?;
        Ok(())
    }

    fn draw(&self, frame: &mut Frame) {
        let size = frame.area();

        // Layout: status bar (1) + main area (rest) + command bar (1)
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),     // status bar
                Constraint::Min(1),        // main area
                Constraint::Length(1),     // command bar
            ])
            .split(size);

        self.draw_status_bar(frame, outer[0]);

        if self.panes.is_empty() {
            let msg = Paragraph::new("  No sessions. Type :spawn claude <dir> to start.")
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(msg, outer[1]);
        } else {
            // Sidebar (session list) + focused pane
            let main = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(22),    // sidebar
                    Constraint::Min(1),        // focused pane
                ])
                .split(outer[1]);

            self.draw_sidebar(frame, main[0]);
            self.draw_pane_content(frame, main[1], self.focused);
        }

        self.draw_command_bar(frame, outer[2]);
    }

    fn draw_status_bar(&self, frame: &mut Frame, area: Rect) {
        let active = self.panes.iter().filter(|p| p.status == Status::Active).count();
        let stalled = self.panes.iter().filter(|p| p.status == Status::Stalled).count();
        let total = self.panes.len();

        let mut spans = vec![
            Span::styled(" mtt ", Style::default().fg(Color::Black).bg(Color::White).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(format!("{total} sessions"), Style::default().fg(Color::DarkGray)),
        ];

        if active > 0 {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(format!("● {active}"), Style::default().fg(Color::Green)));
        }
        if stalled > 0 {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(format!("◐ {stalled}"), Style::default().fg(Color::Yellow)));
        }

        // Show focused pane info
        if let Some(pane) = self.panes.get(self.focused) {
            spans.push(Span::raw("  │  "));
            spans.push(Span::styled(
                format!("{} ", pane.cli.name()),
                Style::default().fg(cli_color(pane.cli)).add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(&pane.cwd, Style::default().fg(Color::Gray)));
        }

        if !self.message.is_empty() {
            spans.push(Span::raw("  │  "));
            spans.push(Span::styled(&self.message, Style::default().fg(Color::Cyan)));
        }

        let bar = Paragraph::new(Line::from(spans))
            .style(Style::default().bg(Color::Rgb(30, 30, 30)));
        frame.render_widget(bar, area);
    }

    fn draw_sidebar(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::RIGHT)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(
                " sessions ",
                Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD),
            ));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        for (i, pane) in self.panes.iter().enumerate() {
            if i as u16 >= inner.height {
                break;
            }

            let is_focused = i == self.focused;

            let status_icon = match pane.status {
                Status::Active => "●",
                Status::Stalled => "◐",
                Status::Dead => "✕",
            };

            let status_color = match pane.status {
                Status::Active => Color::Green,
                Status::Stalled => Color::Yellow,
                Status::Dead => Color::Red,
            };

            // Short project name from CWD
            let project = std::path::Path::new(&pane.cwd)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| pane.cwd.clone());

            let label = format!(
                " {} {} {}",
                status_icon,
                pane.cli.name(),
                truncate_str(&project, 12)
            );

            let style = if is_focused {
                Style::default()
                    .fg(cli_color(pane.cli))
                    .bg(Color::Rgb(40, 40, 40))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };

            let idx_label = format!("{}", i + 1);
            let line = Line::from(vec![
                Span::styled(
                    format!(" {idx_label}"),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(label, style),
            ]);

            let row_area = Rect {
                x: inner.x,
                y: inner.y + i as u16,
                width: inner.width,
                height: 1,
            };

            let bg = if is_focused {
                Style::default().bg(Color::Rgb(40, 40, 40))
            } else {
                Style::default()
            };

            let p = Paragraph::new(line).style(bg);
            frame.render_widget(p, row_area);
        }
    }

    fn draw_pane_content(&self, frame: &mut Frame, area: Rect, pane_idx: usize) {
        let pane = &self.panes[pane_idx];
        let is_focused = pane_idx == self.focused;

        let border_color = if is_focused {
            cli_color(pane.cli)
        } else {
            Color::DarkGray
        };

        let title = format!(" {} [{}] ", pane.cli.name(), short_path(&pane.cwd));
        let status_indicator = match pane.status {
            Status::Active => "●",
            Status::Stalled => "◐",
            Status::Dead => "✕",
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(Span::styled(
                format!("{status_indicator} {title}"),
                Style::default().fg(border_color).add_modifier(Modifier::BOLD),
            ));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Render the virtual terminal screen
        let screen = pane.screen();
        let (screen_rows, screen_cols) = screen.size();

        for row in 0..inner.height.min(screen_rows) {
            for col in 0..inner.width.min(screen_cols) {
                let cell = screen.cell(row, col).unwrap();
                let ch = cell.contents();
                if ch.is_empty() {
                    continue;
                }

                let fg = convert_vt100_color(cell.fgcolor());
                let bg = convert_vt100_color(cell.bgcolor());
                let mut style = Style::default();
                if fg != Color::Reset {
                    style = style.fg(fg);
                }
                if bg != Color::Reset {
                    style = style.bg(bg);
                }
                if cell.bold() {
                    style = style.add_modifier(Modifier::BOLD);
                }

                let buf = frame.buffer_mut();
                if let Some(buf_cell) = buf.cell_mut((inner.x + col, inner.y + row)) {
                    buf_cell.set_symbol(&ch);
                    buf_cell.set_style(style);
                }
            }
        }

        // Position the real terminal cursor at the vt100 cursor location
        if is_focused {
            let (cursor_row, cursor_col) = screen.cursor_position();
            if cursor_row < inner.height && cursor_col < inner.width {
                frame.set_cursor_position((inner.x + cursor_col, inner.y + cursor_row));
            }
        }
    }

    fn draw_command_bar(&self, frame: &mut Frame, area: Rect) {
        let content = match &self.mode {
            Mode::Normal => {
                Line::from(vec![
                    Span::styled(" : ", Style::default().fg(Color::DarkGray)),
                    Span::styled("command", Style::default().fg(Color::DarkGray)),
                ])
            }
            Mode::Command => {
                Line::from(vec![
                    Span::styled(" ❯ ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::raw(&self.command_input),
                    Span::styled("█", Style::default().fg(Color::Cyan)),
                ])
            }
        };
        let bar = Paragraph::new(content)
            .style(Style::default().bg(Color::Rgb(25, 25, 25)));
        frame.render_widget(bar, area);
    }

    fn handle_key(&mut self, key: KeyEvent) {
        match &self.mode {
            Mode::Normal => self.handle_normal_key(key),
            Mode::Command => self.handle_command_key(key),
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) {
        // Reset Ctrl+C timer on any other key
        if key.code != KeyCode::Char('c') || key.modifiers != KeyModifiers::CONTROL {
            self.last_ctrl_c = None;
        }

        match (key.modifiers, key.code) {
            // Ctrl-C: context-aware
            (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                let double = self
                    .last_ctrl_c
                    .map(|t| t.elapsed() < Duration::from_millis(500))
                    .unwrap_or(false);

                if let Some(pane) = self.panes.get_mut(self.focused) {
                    if pane.status == Status::Active {
                        if double {
                            // Double Ctrl+C on active pane → kill session
                            pane.kill();
                            self.message = format!("killed session {}", self.focused + 1);
                            self.last_ctrl_c = None;
                        } else {
                            // Single Ctrl+C → send interrupt to AI CLI
                            let _ = pane.send_keys(&[0x03]); // ETX = Ctrl+C
                            self.message = "interrupt sent (Ctrl+C again to kill)".to_string();
                            self.last_ctrl_c = Some(Instant::now());
                        }
                    } else {
                        // Pane is dead/stalled — double Ctrl+C to remove
                        if double {
                            pane.kill();
                            self.message = format!("killed session {}", self.focused + 1);
                            self.last_ctrl_c = None;
                        } else {
                            self.message = "Ctrl+C again to kill session".to_string();
                            self.last_ctrl_c = Some(Instant::now());
                        }
                    }
                } else {
                    // No panes — double Ctrl+C to quit mtt
                    if double {
                        self.should_quit = true;
                    } else {
                        self.message = "Ctrl+C again to quit mtt".to_string();
                        self.last_ctrl_c = Some(Instant::now());
                    }
                }
                return;
            }
            // : or / enters command mode
            (_, KeyCode::Char(':')) | (_, KeyCode::Char('/')) => {
                self.mode = Mode::Command;
                self.command_input.clear();
            }
            // Ctrl-N / Ctrl-P: switch panes
            (KeyModifiers::CONTROL, KeyCode::Char('n')) => {
                if !self.panes.is_empty() {
                    self.focused = (self.focused + 1) % self.panes.len();
                }
            }
            (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
                if !self.panes.is_empty() {
                    self.focused = (self.focused + self.panes.len() - 1) % self.panes.len();
                }
            }
            // Alt-1..9: switch to pane by number
            (KeyModifiers::ALT, KeyCode::Char(c)) if c.is_ascii_digit() => {
                let idx = c.to_digit(10).unwrap() as usize;
                if idx > 0 && idx <= self.panes.len() {
                    self.focused = idx - 1;
                }
            }
            // Everything else goes to the focused pane
            _ => {
                if let Some(pane) = self.panes.get_mut(self.focused) {
                    let bytes = key_to_bytes(key);
                    if !bytes.is_empty() {
                        let _ = pane.send_keys(&bytes);
                    }
                }
            }
        }
    }

    fn handle_command_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.command_input.clear();
                self.history_cursor = self.command_history.len();
            }
            KeyCode::Enter => {
                let cmd = self.command_input.clone();
                self.command_input.clear();
                self.mode = Mode::Normal;
                if !cmd.is_empty() {
                    self.command_history.push(cmd.clone());
                }
                self.history_cursor = self.command_history.len();
                self.execute_command(&cmd);
            }
            KeyCode::Backspace => {
                self.command_input.pop();
            }
            KeyCode::Up => {
                if !self.command_history.is_empty() && self.history_cursor > 0 {
                    self.history_cursor -= 1;
                    self.command_input = self.command_history[self.history_cursor].clone();
                }
            }
            KeyCode::Down => {
                if self.history_cursor < self.command_history.len() {
                    self.history_cursor += 1;
                    if self.history_cursor < self.command_history.len() {
                        self.command_input = self.command_history[self.history_cursor].clone();
                    } else {
                        self.command_input.clear();
                    }
                }
            }
            KeyCode::Char(c) => {
                self.command_input.push(c);
            }
            _ => {}
        }
    }

    fn execute_command(&mut self, cmd: &str) {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if parts.is_empty() {
            return;
        }

        match parts[0] {
            "spawn" | "s" => {
                if parts.len() < 2 {
                    self.message = "usage: spawn <cli> [directory]".to_string();
                    return;
                }
                let cli = match CLIType::from_str(parts[1]) {
                    Some(c) => c,
                    None => {
                        self.message = format!("unknown CLI: {}", parts[1]);
                        return;
                    }
                };
                let cwd = if parts.len() > 2 {
                    let dir = parts[2..].join(" ");
                    if dir.starts_with("~/") {
                        dirs::home_dir()
                            .map(|h| h.join(&dir[2..]).to_string_lossy().to_string())
                            .unwrap_or(dir)
                    } else {
                        dir
                    }
                } else {
                    std::env::current_dir()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| ".".to_string())
                };

                let id = self.panes.len();
                // Use terminal inner area size (approximate)
                match Pane::spawn(id, cli, &cwd, 24, 80) {
                    Ok(pane) => {
                        self.message = format!("spawned {} in {}", cli.name(), short_path(&cwd));
                        self.panes.push(pane);
                        self.focused = self.panes.len() - 1;
                    }
                    Err(e) => {
                        self.message = format!("spawn failed: {e}");
                    }
                }
            }

            "kill" | "k" => {
                if let Some(idx) = parts.get(1).and_then(|s| s.parse::<usize>().ok()) {
                    if idx > 0 && idx <= self.panes.len() {
                        self.panes[idx - 1].kill();
                        self.message = format!("killed session {idx}");
                    }
                } else if !self.panes.is_empty() {
                    self.panes[self.focused].kill();
                    self.message = "killed focused session".to_string();
                }
            }

            "broadcast" | "bc" => {
                let text = parts[1..].join(" ");
                if text.is_empty() {
                    self.message = "usage: broadcast <message>".to_string();
                    return;
                }
                let mut sent = 0;
                for pane in &mut self.panes {
                    if pane.status == Status::Active
                        && pane.send_text(&text).is_ok()
                    {
                        sent += 1;
                    }
                }
                self.message = format!("broadcast to {sent} sessions");
            }

            "quit" | "q" => {
                self.should_quit = true;
            }

            "help" | "h" => {
                self.message = "spawn kill broadcast quit | Ctrl-N/P switch | Alt-1..9 jump".to_string();
            }

            _ => {
                self.message = format!("unknown command: {}", parts[0]);
            }
        }
    }

    fn handle_resize(&mut self, cols: u16, rows: u16) {
        // Reserve space for status bar (1) and command bar (1)
        let pane_rows = rows.saturating_sub(2);
        let pane_cols = cols;

        // For now, give each pane the full size (split is done in rendering)
        let (pr, pc) = if self.panes.len() <= 1 {
            (pane_rows, pane_cols)
        } else {
            (pane_rows / 2, pane_cols / 2)
        };

        for pane in &mut self.panes {
            let _ = pane.resize(pr.max(1), pc.max(1));
        }
    }
}

/// Convert a vt100 color to a ratatui color.
fn convert_vt100_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Convert a key event to raw bytes for the PTY.
fn key_to_bytes(key: KeyEvent) -> Vec<u8> {
    match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char(c)) => {
            vec![(c as u8) & 0x1f]
        }
        (_, KeyCode::Char(c)) => {
            let mut buf = [0u8; 4];
            let s = c.encode_utf8(&mut buf);
            s.as_bytes().to_vec()
        }
        (_, KeyCode::Enter) => vec![b'\r'],
        (_, KeyCode::Backspace) => vec![127],
        (_, KeyCode::Tab) => vec![b'\t'],
        (_, KeyCode::Esc) => vec![b'\x1b'],
        (_, KeyCode::Up) => b"\x1b[A".to_vec(),
        (_, KeyCode::Down) => b"\x1b[B".to_vec(),
        (_, KeyCode::Right) => b"\x1b[C".to_vec(),
        (_, KeyCode::Left) => b"\x1b[D".to_vec(),
        (_, KeyCode::Home) => b"\x1b[H".to_vec(),
        (_, KeyCode::End) => b"\x1b[F".to_vec(),
        (_, KeyCode::Delete) => b"\x1b[3~".to_vec(),
        (_, KeyCode::PageUp) => b"\x1b[5~".to_vec(),
        (_, KeyCode::PageDown) => b"\x1b[6~".to_vec(),
        _ => vec![],
    }
}

/// Shorten a path for display.
fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}

fn short_path(path: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        let home_str = home.to_string_lossy();
        if path.starts_with(home_str.as_ref()) {
            return format!("~{}", &path[home_str.len()..]);
        }
    }
    path.to_string()
}

fn cli_color(cli: CLIType) -> Color {
    match cli {
        CLIType::Claude => Color::Rgb(232, 149, 106),  // terracotta
        CLIType::Codex => Color::Rgb(52, 211, 153),     // emerald
        CLIType::Gemini => Color::Rgb(251, 146, 60),    // orange
        CLIType::OpenCode => Color::Rgb(56, 189, 248),  // sky
    }
}
