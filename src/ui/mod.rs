use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
        MouseButton, MouseEvent, MouseEventKind,
    },
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

const DEFAULT_SIDEBAR_WIDTH: u16 = 20;
const MIN_SIDEBAR_WIDTH: u16 = 12;
const MAX_SIDEBAR_WIDTH: u16 = 40;
const BOTTOM_PANEL_HEIGHT: u16 = 5;

/// Known commands for tab autocomplete.
const COMMANDS: &[&str] = &[
    "spawn", "kill", "broadcast", "quit", "help", "history", "scroll",
];

/// Known CLI names for tab autocomplete.
const CLI_NAMES: &[&str] = &["claude", "codex", "gemini", "opencode"];

/// UI mode.
enum Mode {
    Normal,   // pane focused, keys go to AI CLI
    Command,  // typing a command in the command bar
    Scroll,   // scrolling through pane scrollback
}

/// The main application state.
pub struct App {
    pub panes: Vec<Pane>,
    pub focused: usize,
    mode: Mode,
    command_input: String,
    command_history: Vec<String>,
    history_cursor: usize,
    tab_matches: Vec<String>,
    tab_index: usize,
    message: String,
    should_quit: bool,
    last_ctrl_c: Option<Instant>,
    scroll_offset: u16,
    terminal_size: (u16, u16),
    sidebar_width: u16,
    sidebar_dragging: bool,
    show_bottom_panel: bool,
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
            tab_matches: Vec::new(),
            tab_index: 0,
            message: String::new(),
            should_quit: false,
            last_ctrl_c: None,
            scroll_offset: 0,
            terminal_size: (80, 24),
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            sidebar_dragging: false,
            show_bottom_panel: false,
        }
    }

    /// Run the main event loop.
    pub fn run(&mut self) -> anyhow::Result<()> {
        // Setup terminal
        terminal::enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        stdout().execute(EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout());
        let mut terminal = Terminal::new(backend)?;

        // Get initial size and resize panes
        let size = terminal.size()?;
        self.terminal_size = (size.width, size.height);

        // Main loop
        loop {
            // Clear stale Ctrl+C hint after timeout
            if let Some(t) = self.last_ctrl_c
                && t.elapsed() > Duration::from_millis(500) {
                    self.last_ctrl_c = None;
                    self.message.clear();
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
                    Event::Mouse(mouse) => self.handle_mouse(mouse),
                    _ => {}
                }
            }

            if self.should_quit {
                break;
            }
        }

        // Cleanup
        stdout().execute(DisableMouseCapture)?;
        terminal::disable_raw_mode()?;
        stdout().execute(LeaveAlternateScreen)?;
        Ok(())
    }

    // ── Drawing ──────────────────────────────────────────────────────────

    fn draw(&self, frame: &mut Frame) {
        let size = frame.area();

        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // status bar
                Constraint::Min(1),   // main area
                Constraint::Length(1), // command bar
            ])
            .split(size);

        self.draw_status_bar(frame, outer[0]);

        if self.panes.is_empty() {
            let msg = Paragraph::new("  No sessions. Type :spawn claude <dir> to start.")
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(msg, outer[1]);
        } else {
            // Split main area: optional bottom panel
            let main_area = if self.show_bottom_panel {
                let vsplit = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Min(1),
                        Constraint::Length(BOTTOM_PANEL_HEIGHT),
                    ])
                    .split(outer[1]);
                self.draw_bottom_panel(frame, vsplit[1]);
                vsplit[0]
            } else {
                outer[1]
            };

            let main = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(self.sidebar_width),
                    Constraint::Min(1),
                ])
                .split(main_area);

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
            Span::styled(
                " mtt ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{total} sessions"),
                Style::default().fg(Color::DarkGray),
            ),
        ];

        if active > 0 {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!("● {active}"),
                Style::default().fg(Color::Green),
            ));
        }
        if stalled > 0 {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!("◐ {stalled}"),
                Style::default().fg(Color::Yellow),
            ));
        }

        if let Some(pane) = self.panes.get(self.focused) {
            spans.push(Span::raw("  │  "));
            spans.push(Span::styled(
                format!("{} ", pane.cli.name()),
                Style::default()
                    .fg(cli_color(pane.cli))
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                short_path(&pane.cwd),
                Style::default().fg(Color::Gray),
            ));
        }

        // Scroll mode indicator
        if matches!(self.mode, Mode::Scroll) {
            spans.push(Span::raw("  │  "));
            spans.push(Span::styled(
                format!("SCROLL ↑{}", self.scroll_offset),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        if !self.message.is_empty() {
            spans.push(Span::raw("  │  "));
            spans.push(Span::styled(
                &self.message,
                Style::default().fg(Color::Cyan),
            ));
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
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::BOLD),
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

            let project = std::path::Path::new(&pane.cwd)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| pane.cwd.clone());

            let max_name = (self.sidebar_width as usize).saturating_sub(7);

            let line = Line::from(vec![
                Span::styled(
                    format!(" {}", i + 1),
                    if is_focused {
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                ),
                Span::styled(
                    format!(" {status_icon}"),
                    Style::default().fg(match pane.status {
                        Status::Active => Color::Green,
                        Status::Stalled => Color::Yellow,
                        Status::Dead => Color::Red,
                    }),
                ),
                Span::styled(
                    format!(" {}", truncate_str(&project, max_name)),
                    if is_focused {
                        Style::default()
                            .fg(cli_color(pane.cli))
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Gray)
                    },
                ),
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

            frame.render_widget(Paragraph::new(line).style(bg), row_area);
        }
    }

    fn draw_bottom_panel(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(
                " usage & sessions ",
                Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD),
            ));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Row 1: Usage limits per CLI
        let mut limit_spans = vec![
            Span::styled("  limits  ", Style::default().fg(Color::DarkGray)),
        ];
        for (cli, name, limit) in [
            (CLIType::Claude, "claude", "5h"),
            (CLIType::Codex, "codex", "5h"),
            (CLIType::Gemini, "gemini", "4h"),
            (CLIType::OpenCode, "opencode", "—"),
        ] {
            let count = self.panes.iter().filter(|p| p.cli == cli && p.status == Status::Active).count();
            limit_spans.push(Span::styled(
                format!("● {name}:{limit}"),
                Style::default().fg(cli_color(cli)),
            ));
            if count > 0 {
                limit_spans.push(Span::styled(
                    format!("({count}) "),
                    Style::default().fg(Color::Gray),
                ));
            }
            limit_spans.push(Span::raw("  "));
        }
        if inner.height > 0 {
            let row = Rect { x: inner.x, y: inner.y, width: inner.width, height: 1 };
            frame.render_widget(Paragraph::new(Line::from(limit_spans)), row);
        }

        // Row 2+: Session list with numbers
        for (i, pane) in self.panes.iter().enumerate() {
            let row_y = inner.y + 1 + i as u16;
            if row_y >= inner.y + inner.height {
                break;
            }

            let project = std::path::Path::new(&pane.cwd)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "?".to_string());

            let is_focused = i == self.focused;
            let marker = if is_focused { "▶" } else { " " };

            let line = Line::from(vec![
                Span::styled(
                    format!("  {marker} #{} ", i + 1),
                    if is_focused {
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                ),
                Span::styled("●", Style::default().fg(cli_color(pane.cli))),
                Span::styled(
                    format!(" {}", project),
                    Style::default().fg(if is_focused { Color::White } else { Color::Gray }),
                ),
            ]);

            let row = Rect { x: inner.x, y: row_y, width: inner.width, height: 1 };
            frame.render_widget(Paragraph::new(line), row);
        }
    }

    fn draw_pane_content(&self, frame: &mut Frame, area: Rect, pane_idx: usize) {
        if pane_idx >= self.panes.len() {
            return;
        }
        let pane = &self.panes[pane_idx];
        let is_focused = pane_idx == self.focused;

        let border_color = if is_focused {
            cli_color(pane.cli)
        } else {
            Color::DarkGray
        };

        let status_indicator = match pane.status {
            Status::Active => "●",
            Status::Stalled => "◐",
            Status::Dead => "✕",
        };

        let title = format!(
            " {} [{}] ",
            pane.cli.name(),
            short_path(&pane.cwd)
        );

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(Span::styled(
                format!("{status_indicator} {title}"),
                Style::default()
                    .fg(border_color)
                    .add_modifier(Modifier::BOLD),
            ));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Render the virtual terminal screen
        let screen = pane.screen();
        let (screen_rows, screen_cols) = screen.size();

        // Render screen content (visible screen only — scrollback TBD)
        for row in 0..inner.height.min(screen_rows) {
            for col in 0..inner.width.min(screen_cols) {
                if let Some(cell) = screen.cell(row, col) {
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
                    if cell.underline() {
                        style = style.add_modifier(Modifier::UNDERLINED);
                    }
                    if cell.inverse() {
                        style = style.add_modifier(Modifier::REVERSED);
                    }

                    let buf = frame.buffer_mut();
                    if let Some(buf_cell) = buf.cell_mut((inner.x + col, inner.y + row)) {
                        buf_cell.set_symbol(&ch);
                        buf_cell.set_style(style);
                    }
                }
            }
        }

        // Hide the hardware cursor — let the AI CLI render its own cursor
        // via the vt100 screen content (it draws its own cursor character)
    }

    fn draw_command_bar(&self, frame: &mut Frame, area: Rect) {
        let content = match &self.mode {
            Mode::Normal => Line::from(vec![
                Span::styled(
                    " : ",
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    "command  ",
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    "Ctrl-N/P switch  Alt-1..9 jump",
                    Style::default().fg(Color::Rgb(50, 50, 50)),
                ),
            ]),
            Mode::Command => {
                let mut spans = vec![
                    Span::styled(
                        " ❯ ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(&self.command_input),
                ];

                // Show tab completion hint
                if !self.tab_matches.is_empty() {
                    let hint = &self.tab_matches[self.tab_index % self.tab_matches.len()];
                    if hint.len() > self.command_input.len() {
                        spans.push(Span::styled(
                            &hint[self.command_input.len()..],
                            Style::default().fg(Color::DarkGray),
                        ));
                    }
                }

                spans.push(Span::styled("█", Style::default().fg(Color::Cyan)));
                Line::from(spans)
            }
            Mode::Scroll => Line::from(vec![
                Span::styled(
                    " SCROLL ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "  ↑↓ scroll  Esc exit  ",
                    Style::default().fg(Color::DarkGray),
                ),
            ]),
        };

        let bar = Paragraph::new(content)
            .style(Style::default().bg(Color::Rgb(25, 25, 25)));
        frame.render_widget(bar, area);
    }

    // ── Input handling ───────────────────────────────────────────────────

    fn handle_key(&mut self, key: KeyEvent) {
        match &self.mode {
            Mode::Normal => self.handle_normal_key(key),
            Mode::Command => self.handle_command_key(key),
            Mode::Scroll => self.handle_scroll_key(key),
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) {
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

                if !self.panes.is_empty() && self.focused < self.panes.len() {
                    if self.panes[self.focused].status == Status::Active {
                        if double {
                            self.panes[self.focused].kill();
                            self.panes.remove(self.focused);
                            if self.focused >= self.panes.len() && !self.panes.is_empty() {
                                self.focused = self.panes.len() - 1;
                            }
                            self.message = "session killed and removed".to_string();
                            self.last_ctrl_c = None;
                        } else {
                            let _ = self.panes[self.focused].send_keys(&[0x03]);
                            self.message = "interrupt sent (Ctrl+C again to kill)".to_string();
                            self.last_ctrl_c = Some(Instant::now());
                        }
                    } else if double {
                        self.panes[self.focused].kill();
                        self.panes.remove(self.focused);
                        if self.focused >= self.panes.len() && !self.panes.is_empty() {
                            self.focused = self.panes.len() - 1;
                        }
                        self.message = "session removed".to_string();
                        self.last_ctrl_c = None;
                    } else {
                        self.message = "Ctrl+C again to kill session".to_string();
                        self.last_ctrl_c = Some(Instant::now());
                    }
                } else if double {
                    self.should_quit = true;
                } else {
                    self.message = "Ctrl+C again to quit mtt".to_string();
                    self.last_ctrl_c = Some(Instant::now());
                }
            }
            // : or / enters command mode
            (_, KeyCode::Char(':')) | (_, KeyCode::Char('/')) => {
                self.mode = Mode::Command;
                self.command_input.clear();
                self.tab_matches.clear();
            }
            // Ctrl-S enters scroll mode
            (KeyModifiers::CONTROL, KeyCode::Char('s')) => {
                self.mode = Mode::Scroll;
                self.scroll_offset = 0;
                self.message = "scroll mode".to_string();
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
                let idx = c.to_digit(10).unwrap_or(0) as usize;
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
                self.tab_matches.clear();
            }
            KeyCode::Enter => {
                let cmd = self.command_input.clone();
                self.command_input.clear();
                self.mode = Mode::Normal;
                self.tab_matches.clear();
                if !cmd.is_empty() {
                    self.command_history.push(cmd.clone());
                }
                self.history_cursor = self.command_history.len();
                self.execute_command(&cmd);
            }
            KeyCode::Backspace => {
                self.command_input.pop();
                self.update_tab_matches();
            }
            KeyCode::Up => {
                if !self.command_history.is_empty() && self.history_cursor > 0 {
                    self.history_cursor -= 1;
                    self.command_input = self.command_history[self.history_cursor].clone();
                    self.update_tab_matches();
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
                    self.update_tab_matches();
                }
            }
            KeyCode::Tab => {
                self.apply_tab_completion();
            }
            KeyCode::Char(c) => {
                self.command_input.push(c);
                self.update_tab_matches();
            }
            _ => {}
        }
    }

    fn handle_scroll_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.mode = Mode::Normal;
                self.scroll_offset = 0;
                self.message.clear();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
            }
            KeyCode::PageUp => {
                self.scroll_offset = self.scroll_offset.saturating_add(20);
            }
            KeyCode::PageDown => {
                self.scroll_offset = self.scroll_offset.saturating_sub(20);
            }
            // Any other key exits scroll mode
            _ => {
                self.mode = Mode::Normal;
                self.scroll_offset = 0;
                self.message.clear();
            }
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let (_, rows) = self.terminal_size;

                // Click on bottom row — toggle bottom panel
                if mouse.row >= rows.saturating_sub(2) {
                    self.show_bottom_panel = !self.show_bottom_panel;
                    return;
                }

                // Click on sidebar border — start drag
                if mouse.column == self.sidebar_width || mouse.column == self.sidebar_width.saturating_sub(1) {
                    self.sidebar_dragging = true;
                    return;
                }

                // Click in sidebar — switch pane
                if mouse.column < self.sidebar_width {
                    let sidebar_row = mouse.row.saturating_sub(2);
                    let idx = sidebar_row as usize;
                    if idx < self.panes.len() {
                        self.focused = idx;
                    }
                    return;
                }

                // Click in bottom panel session row — switch pane
                if self.show_bottom_panel {
                    let panel_start = rows.saturating_sub(2 + BOTTOM_PANEL_HEIGHT);
                    if mouse.row > panel_start {
                        let panel_row = mouse.row.saturating_sub(panel_start + 2);
                        let idx = panel_row as usize;
                        if idx < self.panes.len() {
                            self.focused = idx;
                        }
                    }
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if self.sidebar_dragging {
                    let new_width = mouse.column.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
                    self.sidebar_width = new_width;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.sidebar_dragging = false;
            }
            MouseEventKind::ScrollUp => {
                if matches!(self.mode, Mode::Normal) {
                    self.mode = Mode::Scroll;
                    self.scroll_offset = 1;
                } else if matches!(self.mode, Mode::Scroll) {
                    self.scroll_offset = self.scroll_offset.saturating_add(3);
                }
            }
            MouseEventKind::ScrollDown => {
                if matches!(self.mode, Mode::Scroll) {
                    if self.scroll_offset <= 3 {
                        self.scroll_offset = 0;
                        self.mode = Mode::Normal;
                    } else {
                        self.scroll_offset = self.scroll_offset.saturating_sub(3);
                    }
                }
            }
            _ => {}
        }
    }

    // ── Tab completion ───────────────────────────────────────────────────

    fn update_tab_matches(&mut self) {
        self.tab_matches.clear();
        self.tab_index = 0;

        let input = &self.command_input;
        if input.is_empty() {
            return;
        }

        let parts: Vec<&str> = input.split_whitespace().collect();

        if parts.len() <= 1 && !input.ends_with(' ') {
            // Completing command name
            let prefix = parts.first().copied().unwrap_or("");
            for cmd in COMMANDS {
                if cmd.starts_with(prefix) && *cmd != prefix {
                    self.tab_matches.push(cmd.to_string());
                }
            }
        } else if parts.first() == Some(&"spawn") {
            if parts.len() == 2 && !input.ends_with(' ') {
                // Completing CLI name
                let prefix = parts[1];
                for name in CLI_NAMES {
                    if name.starts_with(prefix) && *name != prefix {
                        self.tab_matches
                            .push(format!("spawn {name}"));
                    }
                }
            } else if parts.len() >= 2 {
                // Completing directory path
                let dir_part = if input.ends_with(' ') && parts.len() == 2 {
                    ""
                } else if parts.len() > 2 {
                    parts[2]
                } else {
                    return;
                };
                self.tab_matches = get_dir_completions(dir_part)
                    .into_iter()
                    .map(|d| format!("spawn {} {d}", parts[1]))
                    .collect();
            }
        }
    }

    fn apply_tab_completion(&mut self) {
        if self.tab_matches.is_empty() {
            self.update_tab_matches();
        }

        if !self.tab_matches.is_empty() {
            let completion = self.tab_matches[self.tab_index % self.tab_matches.len()].clone();
            self.command_input = completion;
            self.tab_index += 1;
            // Re-check for deeper completions (e.g., after accepting a dir)
            if self.command_input.ends_with('/') || self.command_input.ends_with(' ') {
                self.update_tab_matches();
            }
        }
    }

    // ── Commands ─────────────────────────────────────────────────────────

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
                    expand_path(&dir)
                } else {
                    std::env::current_dir()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| ".".to_string())
                };

                // Calculate pane size from current terminal size
                let (cols, rows) = self.terminal_size;
                let pane_rows = rows.saturating_sub(4); // status + border + command
                let pane_cols = cols.saturating_sub(self.sidebar_width + 2); // sidebar + borders

                let id = self.panes.len();
                match Pane::spawn(id, cli, &cwd, pane_rows.max(10), pane_cols.max(20)) {
                    Ok(pane) => {
                        self.message =
                            format!("spawned {} in {}", cli.name(), short_path(&cwd));
                        self.panes.push(pane);
                        self.focused = self.panes.len() - 1;
                    }
                    Err(e) => {
                        self.message = format!("spawn failed: {e}");
                    }
                }
            }

            "kill" | "k" => {
                let target = if let Some(idx) = parts.get(1).and_then(|s| s.parse::<usize>().ok()) {
                    if idx > 0 && idx <= self.panes.len() { Some(idx - 1) } else { None }
                } else if !self.panes.is_empty() {
                    Some(self.focused)
                } else {
                    None
                };
                if let Some(idx) = target {
                    self.panes[idx].kill();
                    self.panes.remove(idx);
                    if self.focused >= self.panes.len() && !self.panes.is_empty() {
                        self.focused = self.panes.len() - 1;
                    }
                    self.message = format!("killed session {}", idx + 1);
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
                    if pane.status == Status::Active && pane.send_text(&text).is_ok() {
                        sent += 1;
                    }
                }
                self.message = format!("broadcast to {sent} sessions");
            }

            "scroll" => {
                self.mode = Mode::Scroll;
                self.scroll_offset = 0;
            }

            "quit" | "q" => {
                self.should_quit = true;
            }

            "help" | "h" => {
                self.message =
                    "spawn kill broadcast scroll quit | Ctrl-N/P switch | Ctrl-S scroll"
                        .to_string();
            }

            _ => {
                self.message = format!("unknown command: {}", parts[0]);
            }
        }
    }

    // ── Resize ───────────────────────────────────────────────────────────

    fn handle_resize(&mut self, cols: u16, rows: u16) {
        self.terminal_size = (cols, rows);

        // Calculate pane size: full height minus status+command, width minus sidebar
        let pane_rows = rows.saturating_sub(4); // status + border top + border bottom + command
        let pane_cols = cols.saturating_sub(self.sidebar_width + 2); // sidebar + border

        for pane in &mut self.panes {
            let _ = pane.resize(pane_rows.max(1), pane_cols.max(1));
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn convert_vt100_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

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
        (_, KeyCode::F(n)) => format!("\x1b[{n}~").into_bytes(),
        _ => vec![],
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        format!("{}…", chars[..max.saturating_sub(1)].iter().collect::<String>())
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

fn expand_path(path: &str) -> String {
    if path.starts_with("~/") {
        dirs::home_dir()
            .map(|h| h.join(&path[2..]).to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string())
    } else {
        path.to_string()
    }
}

fn get_dir_completions(prefix: &str) -> Vec<String> {
    let expanded = expand_path(prefix);

    let (dir, file_prefix) = if expanded.ends_with('/') || prefix.ends_with('/') {
        (expanded.as_str(), "")
    } else {
        let p = std::path::Path::new(&expanded);
        let dir = p.parent().map(|d| d.to_string_lossy().to_string());
        let file = p
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        // Need to return owned values
        return get_dir_completions_inner(
            &dir.unwrap_or_else(|| ".".to_string()),
            &file,
            prefix,
        );
    };

    get_dir_completions_inner(dir, file_prefix, prefix)
}

fn get_dir_completions_inner(dir: &str, file_prefix: &str, original: &str) -> Vec<String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return vec![],
    };

    let mut results = Vec::new();
    let prefix_lower = file_prefix.to_lowercase();

    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden dirs unless user typed a dot
        if name.starts_with('.') && !file_prefix.starts_with('.') {
            continue;
        }

        if name.to_lowercase().starts_with(&prefix_lower) {
            // Build completion using original prefix style
            let completion = if original.ends_with('/') {
                format!("{original}{name}")
            } else if original.contains('/') {
                let parent = &original[..original.rfind('/').unwrap_or(0) + 1];
                format!("{parent}{name}")
            } else {
                name.clone()
            };
            results.push(completion);
        }
    }

    results.sort();
    results
}

fn cli_color(cli: CLIType) -> Color {
    match cli {
        CLIType::Claude => Color::Rgb(232, 149, 106),
        CLIType::Codex => Color::Rgb(52, 211, 153),
        CLIType::Gemini => Color::Rgb(251, 146, 60),
        CLIType::OpenCode => Color::Rgb(56, 189, 248),
    }
}
