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
    "spawn", "kill", "broadcast", "quit", "help", "history", "scroll", "layout",
];

/// Known CLI names for tab autocomplete.
const CLI_NAMES: &[&str] = &["claude", "codex", "gemini", "opencode"];

/// UI mode.
enum Mode {
    Normal,   // pane focused, keys go to AI CLI
    Command,  // typing a command in the command bar
    Scroll,   // scrolling through pane scrollback
}

/// Session panel position.
#[derive(Clone, Copy, PartialEq)]
enum PanelPosition {
    Left,
    Bottom,
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
    terminal_size: (u16, u16),
    sidebar_width: u16,
    sidebar_dragging: bool,
    show_bottom_panel: bool,
    panel_position: PanelPosition,
    show_help: bool,
    show_session_picker: bool,
    picker_cursor: usize,
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
            terminal_size: (80, 24),
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            sidebar_dragging: false,
            show_bottom_panel: false,
            panel_position: PanelPosition::Left,
            show_help: false,
            show_session_picker: false,
            picker_cursor: 0,
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

        // Kill all panes to unblock reader threads
        for pane in &mut self.panes {
            pane.kill();
        }
        self.panes.clear();

        // Cleanup terminal
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
                Constraint::Length(1), // usage limits bar
            ])
            .split(size);

        self.draw_status_bar(frame, outer[0]);

        self.draw_usage_bar(frame, outer[2]);

        if self.panes.is_empty() {
            let msg = Paragraph::new("  No sessions. Type :spawn claude <dir> to start.")
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(msg, outer[1]);
        } else {
            // Optional extra bottom panel (usage/limits)
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

            match self.panel_position {
                PanelPosition::Left => {
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
                PanelPosition::Bottom => {
                    let main = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Min(1),
                            Constraint::Length(1),
                        ])
                        .split(main_area);

                    self.draw_pane_content(frame, main[0], self.focused);
                    self.draw_session_tab_bar(frame, main[1]);
                }
            }
        }

        // Floating command bar (only when in command mode)
        if matches!(self.mode, Mode::Command) {
            self.draw_floating_command(frame, size);
        }

        // Session picker overlay
        if self.show_session_picker {
            self.draw_session_picker(frame, size);
        }

        // Help overlay (drawn last, on top of everything)
        if self.show_help {
            self.draw_help_overlay(frame, size);
        }
    }

    fn draw_status_bar(&self, frame: &mut Frame, area: Rect) {
        let active = self.panes.iter().filter(|p| p.status == Status::Active).count();
        let stalled = self.panes.iter().filter(|p| p.status == Status::Stalled).count();
        let total = self.panes.len();

        let mut spans = vec![
            Span::styled(
                " mtt ",
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Rgb(50, 23, 77))
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
            let offset = self.panes.get(self.focused).map(|p| p.scroll_offset).unwrap_or(0);
            spans.push(Span::raw("  │  "));
            spans.push(Span::styled(
                format!("SCROLL ↑{offset}"),
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
                Style::default().fg(Color::Rgb(120, 80, 170)),
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

    fn draw_usage_bar(&self, frame: &mut Frame, area: Rect) {
        let sep = Span::styled(" │ ", Style::default().fg(Color::Rgb(40, 40, 40)));

        let mut spans: Vec<Span> = vec![Span::raw(" ")];

        for (cli, name, limit_h) in [
            (CLIType::Claude, "claude", 5u32),
            (CLIType::Codex, "codex", 5u32),
            (CLIType::Gemini, "gemini", 4u32),
        ] {
            let count = self.panes.iter().filter(|p| p.cli == cli).count();
            // Simple usage estimate based on session age
            let total_mins: u64 = self.panes
                .iter()
                .filter(|p| p.cli == cli && p.status == Status::Active)
                .map(|p| p.started.elapsed().as_secs() / 60)
                .sum();
            let limit_mins = limit_h as u64 * 60;
            let remaining_mins = limit_mins.saturating_sub(total_mins);
            let pct = if limit_mins > 0 {
                ((limit_mins - remaining_mins) * 100 / limit_mins) as u32
            } else {
                0
            };
            let rem_h = remaining_mins / 60;
            let rem_m = remaining_mins % 60;

            spans.push(Span::styled(
                format!("●"),
                Style::default().fg(cli_color(cli)),
            ));
            spans.push(Span::styled(
                format!("{name} {limit_h}h:{pct}%({rem_h}h{rem_m:02}m)"),
                Style::default().fg(Color::DarkGray),
            ));
            if count > 0 {
                spans.push(Span::styled(
                    format!(" [{count}]"),
                    Style::default().fg(Color::Gray),
                ));
            }
            spans.push(sep.clone());
        }

        // Session count
        let total = self.panes.len();
        spans.push(Span::styled(
            format!("sessions:{total}"),
            Style::default().fg(Color::DarkGray),
        ));

        if !self.panes.is_empty() {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                "Ctrl-J picker",
                Style::default().fg(Color::Rgb(50, 50, 50)),
            ));
        }

        let bar = Paragraph::new(Line::from(spans))
            .style(Style::default().bg(Color::Rgb(18, 18, 18)));
        frame.render_widget(bar, area);
    }

    fn draw_session_tab_bar(&self, frame: &mut Frame, area: Rect) {
        let sep = Span::styled(" │ ", Style::default().fg(Color::DarkGray));

        let mut spans: Vec<Span> = vec![Span::raw(" ")];

        // Per-CLI usage limits
        for (cli, name) in [
            (CLIType::Claude, "claude"),
            (CLIType::Codex, "codex"),
            (CLIType::Gemini, "gemini"),
        ] {
            let count = self.panes.iter().filter(|p| p.cli == cli && p.status == Status::Active).count();
            if count > 0 {
                spans.push(Span::styled(
                    format!("●{name}({count})"),
                    Style::default().fg(cli_color(cli)),
                ));
                spans.push(sep.clone());
            }
        }

        // Session count + hint
        let active = self.panes.iter().filter(|p| p.status == Status::Active).count();
        spans.push(Span::styled(
            format!("{active} active"),
            Style::default().fg(Color::Green),
        ));
        spans.push(sep.clone());

        // Focused session indicator
        if let Some(pane) = self.panes.get(self.focused) {
            let project = std::path::Path::new(&pane.cwd)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "?".to_string());
            spans.push(Span::styled(
                format!("▶ {}", project),
                Style::default().fg(cli_color(pane.cli)).add_modifier(Modifier::BOLD),
            ));
        }

        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            "sessions ↓",
            Style::default().fg(Color::DarkGray),
        ));

        let bar = Paragraph::new(Line::from(spans))
            .style(Style::default().bg(Color::Rgb(25, 25, 25)));
        frame.render_widget(bar, area);
    }

    fn draw_session_picker(&self, frame: &mut Frame, area: Rect) {
        if self.panes.is_empty() {
            return;
        }

        let width = 40u16.min(area.width.saturating_sub(4));
        let height = (self.panes.len() as u16 + 2).min(area.height.saturating_sub(4));
        let x = (area.width.saturating_sub(width)) / 2;
        let y = area.height.saturating_sub(height + 3);

        let picker_area = Rect { x, y, width, height };

        let block = Block::default()
            .borders(Borders::TOP | Borders::BOTTOM)
            .border_style(Style::default().fg(Color::DarkGray))
            .style(Style::default().bg(Color::Rgb(20, 20, 25)));

        let inner = block.inner(picker_area);
        frame.render_widget(block, picker_area);

        for (i, pane) in self.panes.iter().enumerate() {
            if i as u16 >= inner.height {
                break;
            }

            let is_selected = i == self.picker_cursor;
            let project = std::path::Path::new(&pane.cwd)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "?".to_string());

            let status_icon = match pane.status {
                Status::Active => "●",
                Status::Stalled => "◐",
                Status::Dead => "✕",
            };

            let marker = if is_selected { "▶" } else { " " };

            let line = Line::from(vec![
                Span::styled(
                    format!(" {marker} {}", i + 1),
                    if is_selected {
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
                    format!(" {} ", pane.cli.name()),
                    Style::default().fg(cli_color(pane.cli)),
                ),
                Span::styled(
                    truncate_str(&project, 20),
                    if is_selected {
                        Style::default().fg(Color::White)
                    } else {
                        Style::default().fg(Color::Gray)
                    },
                ),
            ]);

            let row = Rect {
                x: inner.x,
                y: inner.y + i as u16,
                width: inner.width,
                height: 1,
            };
            let bg = if is_selected {
                Style::default().bg(Color::Rgb(40, 40, 50))
            } else {
                Style::default()
            };
            frame.render_widget(Paragraph::new(line).style(bg), row);
        }
    }

    fn draw_help_overlay(&self, frame: &mut Frame, area: Rect) {
        let help_text = vec![
            "",
            "  mtt — AI-Native Terminal Multiplexer",
            "",
            "  Commands (press : to enter)",
            "  ─────────────────────────────────────",
            "  :spawn <cli> [--flags] [dir]  start session",
            "  :kill [n]                     kill session",
            "  :broadcast <msg>              send to all",
            "  :layout                       toggle side/bottom",
            "  :scroll                       enter scroll mode",
            "  :help                         show this help",
            "  :quit                         exit mtt",
            "",
            "  Keys",
            "  ─────────────────────────────────────",
            "  Ctrl-N / Ctrl-P               switch pane",
            "  Alt-1..9                      jump to pane",
            "  Ctrl-S                        scroll mode",
            "  Ctrl-C                        interrupt / kill",
            "  : or /                        command / AI slash",
            "",
            "  Press any key to close",
        ];

        let width = 45u16;
        let height = help_text.len() as u16 + 2;
        let x = area.width.saturating_sub(width) / 2;
        let y = area.height.saturating_sub(height) / 2;

        let help_area = Rect { x, y, width, height };

        let lines: Vec<Line> = help_text
            .iter()
            .map(|s| Line::from(Span::styled(*s, Style::default().fg(Color::Gray))))
            .collect();

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(120, 80, 170)))
            .style(Style::default().bg(Color::Rgb(20, 20, 20)));

        let para = Paragraph::new(lines).block(block);
        frame.render_widget(para, help_area);
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

        // In scroll mode, replay raw history into a tall virtual terminal
        let scroll_parser;
        let render_screen = if matches!(self.mode, Mode::Scroll) && is_focused && pane.scroll_offset > 0 {
            scroll_parser = pane.scrollback_screen(screen_rows, screen_cols);
            if let Some(ref sp) = scroll_parser {
                sp.screen()
            } else {
                screen
            }
        } else {
            screen
        };

        // Determine which rows to render when scrolling
        let (render_rows, render_cols) = render_screen.size();
        let start_row = if matches!(self.mode, Mode::Scroll) && is_focused && pane.scroll_offset > 0 {
            // Find the cursor row (last content row) in the tall screen
            let (cursor_row, _) = render_screen.cursor_position();
            let visible = inner.height.min(render_rows);
            // Base = cursor at bottom of view, scroll_offset moves up
            cursor_row
                .saturating_sub(visible)
                .saturating_add(1)
                .saturating_sub(pane.scroll_offset as u16)
        } else {
            0
        };

        for display_row in 0..inner.height.min(render_rows) {
            let src_row = start_row + display_row;
            if src_row >= render_rows {
                break;
            }
            for col in 0..inner.width.min(render_cols) {
                if let Some(cell) = render_screen.cell(src_row, col) {
                    let ch = cell.contents();
                    let fg = convert_vt100_color(cell.fgcolor());
                    let bg = convert_vt100_color(cell.bgcolor());

                    // Skip truly empty cells (no content AND no background)
                    if ch.is_empty() && bg == Color::Reset {
                        continue;
                    }
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
                    if let Some(buf_cell) = buf.cell_mut((inner.x + col, inner.y + display_row)) {
                        buf_cell.set_symbol(&ch);
                        buf_cell.set_style(style);
                    }
                }
            }
        }

        // Hide the hardware cursor — let the AI CLI render its own cursor
        // via the vt100 screen content (it draws its own cursor character)
    }

    fn draw_floating_command(&self, frame: &mut Frame, area: Rect) {
        let width = (area.width / 2).max(40).min(area.width.saturating_sub(4));
        let x = (area.width.saturating_sub(width)) / 2;
        let y = area.height / 2;

        let cmd_area = Rect {
            x,
            y,
            width,
            height: 3,
        };

        let mut spans = vec![
            Span::styled(
                " ❯ ",
                Style::default()
                    .fg(Color::Rgb(120, 80, 170))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(&self.command_input),
        ];

        // Tab completion hint
        if !self.tab_matches.is_empty() {
            let hint = &self.tab_matches[self.tab_index % self.tab_matches.len()];
            if hint.len() > self.command_input.len() {
                spans.push(Span::styled(
                    &hint[self.command_input.len()..],
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }

        spans.push(Span::styled("█", Style::default().fg(Color::Rgb(120, 80, 170))));

        let block = Block::default()
            .borders(Borders::TOP | Borders::BOTTOM)
            .border_style(Style::default().fg(Color::DarkGray))
            .style(Style::default().bg(Color::Rgb(20, 20, 25)));

        let para = Paragraph::new(Line::from(spans)).block(block);
        frame.render_widget(para, cmd_area);
    }

    // ── Input handling ───────────────────────────────────────────────────

    fn handle_key(&mut self, key: KeyEvent) {
        // Dismiss help overlay on any key
        if self.show_help {
            self.show_help = false;
            return;
        }

        // Session picker navigation
        if self.show_session_picker {
            match key.code {
                KeyCode::Esc => {
                    self.show_session_picker = false;
                }
                KeyCode::Up => {
                    if self.picker_cursor > 0 {
                        self.picker_cursor -= 1;
                    } else {
                        self.show_session_picker = false;
                    }
                }
                KeyCode::Down => {
                    if self.picker_cursor + 1 < self.panes.len() {
                        self.picker_cursor += 1;
                    }
                }
                KeyCode::Enter => {
                    self.focused = self.picker_cursor;
                    self.show_session_picker = false;
                }
                _ => {
                    self.show_session_picker = false;
                }
            }
            return;
        }

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
            // Down arrow opens session picker (bottom layout only)
            (KeyModifiers::CONTROL, KeyCode::Char('j')) => {
                if !self.panes.is_empty() {
                    self.picker_cursor = self.focused;
                    self.show_session_picker = true;
                }
            }
            // : enters mtt command mode (/ passes through to the pane for AI CLI slash commands)
            (_, KeyCode::Char(':')) => {
                self.mode = Mode::Command;
                self.command_input.clear();
                self.tab_matches.clear();
            }
            // Ctrl-S enters scroll mode
            (KeyModifiers::CONTROL, KeyCode::Char('s')) => {
                self.mode = Mode::Scroll;
                if let Some(pane) = self.panes.get_mut(self.focused) {
                    pane.scroll_offset = 0;
                }
                self.message = "scroll mode — ↑↓ scroll, Esc exit".to_string();
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
                if let Some(pane) = self.panes.get_mut(self.focused) {
                    pane.scroll_offset = 0;
                }
                self.message.clear();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(pane) = self.panes.get_mut(self.focused) {
                    let (cols, rows) = self.terminal_size;
                    let visible = rows.saturating_sub(4);
                    let max = pane.max_scroll(visible, cols.saturating_sub(self.sidebar_width + 2));
                    pane.scroll_offset = (pane.scroll_offset + 1).min(max);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(pane) = self.panes.get_mut(self.focused) {
                    if pane.scroll_offset == 0 {
                        self.mode = Mode::Normal;
                        self.message.clear();
                    } else {
                        pane.scroll_offset = pane.scroll_offset.saturating_sub(1);
                    }
                }
            }
            KeyCode::PageUp => {
                if let Some(pane) = self.panes.get_mut(self.focused) {
                    let (cols, rows) = self.terminal_size;
                    let visible = rows.saturating_sub(4);
                    let max = pane.max_scroll(visible, cols.saturating_sub(self.sidebar_width + 2));
                    pane.scroll_offset = (pane.scroll_offset + 20).min(max);
                }
            }
            KeyCode::PageDown => {
                if let Some(pane) = self.panes.get_mut(self.focused) {
                    if pane.scroll_offset <= 20 {
                        pane.scroll_offset = 0;
                        self.mode = Mode::Normal;
                        self.message.clear();
                    } else {
                        pane.scroll_offset = pane.scroll_offset.saturating_sub(20);
                    }
                }
            }
            _ => {
                self.mode = Mode::Normal;
                if let Some(pane) = self.panes.get_mut(self.focused) {
                    pane.scroll_offset = 0;
                }
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
                }
                if let Some(pane) = self.panes.get_mut(self.focused) {
                    let (_, rows) = self.terminal_size;
                    let visible = rows.saturating_sub(4);
                    let max = pane.max_scroll(visible, self.terminal_size.0.saturating_sub(self.sidebar_width + 2));
                    pane.scroll_offset = (pane.scroll_offset + 1).min(max);
                }
            }
            MouseEventKind::ScrollDown => {
                if let Some(pane) = self.panes.get_mut(self.focused) {
                    if pane.scroll_offset == 0 {
                        self.mode = Mode::Normal;
                    } else {
                        pane.scroll_offset = pane.scroll_offset.saturating_sub(1);
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
                // Parse remaining args: flags (--xxx) and directory (last non-flag arg)
                let mut extra_args: Vec<&str> = Vec::new();
                let mut cwd = String::new();
                for part in &parts[2..] {
                    if part.starts_with("--") {
                        extra_args.push(part);
                    } else {
                        // Last non-flag arg is the directory
                        cwd = expand_path(part);
                    }
                }
                if cwd.is_empty() {
                    cwd = std::env::current_dir()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| ".".to_string());
                }

                // Calculate pane size from current terminal size
                let (cols, rows) = self.terminal_size;
                let pane_rows = rows.saturating_sub(4);
                let pane_cols = cols.saturating_sub(self.sidebar_width + 2);

                let id = self.panes.len();
                match Pane::spawn(id, cli, &cwd, pane_rows.max(10), pane_cols.max(20), &extra_args) {
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
                if let Some(pane) = self.panes.get_mut(self.focused) {
                    pane.scroll_offset = 0;
                }
            }

            "quit" | "q" => {
                self.should_quit = true;
            }

            "layout" | "l" => {
                self.panel_position = match self.panel_position {
                    PanelPosition::Left => PanelPosition::Bottom,
                    PanelPosition::Bottom => PanelPosition::Left,
                };
                let pos = match self.panel_position {
                    PanelPosition::Left => "left",
                    PanelPosition::Bottom => "bottom",
                };
                self.message = format!("panel: {pos}");
            }

            "help" | "h" => {
                self.show_help = true;
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
