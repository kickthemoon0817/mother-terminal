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
use std::collections::HashMap;
use std::io::stdout;
use std::time::{Duration, Instant};

use crate::history::Recorder as HistoryRecorder;
use crate::monitor::StallDetector;
use crate::pane::{CLIType, Pane, Status};
use crate::persist;
const DEFAULT_SIDEBAR_WIDTH: u16 = 20;
const MIN_SIDEBAR_WIDTH: u16 = 12;
const MAX_SIDEBAR_WIDTH: u16 = 40;
const BOTTOM_PANEL_HEIGHT: u16 = 5;

/// Known commands for tab autocomplete.
const COMMANDS: &[&str] = &[
    "spawn", "kill", "broadcast", "quit", "help", "history", "scroll", "layout", "alias", "sessions", "!",
];

/// Known CLI names for tab autocomplete.
const CLI_NAMES: &[&str] = &["claude", "codex", "gemini", "opencode", "shell", "bash", "zsh"];

/// Known CLI flags for tab autocomplete after CLI name.
const CLI_FLAGS: &[&str] = &[
    "--dangerously-skip-permissions",
    "--verbose",
    "--model",
    "--resume",
    "--continue",
    "--print",
];

/// Load aliases from ~/.mtt/aliases.json
fn load_aliases() -> HashMap<String, String> {
    let path = match dirs::home_dir() {
        Some(h) => h.join(".mtt/aliases.json"),
        None => return HashMap::new(),
    };
    let data = match std::fs::read_to_string(&path) {
        Ok(d) => d,
        Err(_) => return HashMap::new(),
    };
    serde_json::from_str(&data).unwrap_or_default()
}

/// Save aliases to ~/.mtt/aliases.json
fn save_aliases(aliases: &HashMap<String, String>) {
    if let Some(home) = dirs::home_dir() {
        let dir = home.join(".mtt");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("aliases.json");
        if let Ok(json) = serde_json::to_string_pretty(aliases) {
            let _ = std::fs::write(&path, json);
        }
    }
}

/// UI mode.
enum Mode {
    Normal,      // pane focused, keys go to AI CLI
    Command,     // typing a command in the command bar
    Scroll,      // scrolling through pane scrollback
    SidebarNav,  // navigating the sidebar to switch sessions
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
    aliases: HashMap<String, String>, // e.g. "cla" -> "claude --dangerously-skip-permissions"
    history_cursor: usize,
    tab_matches: Vec<String>,
    tab_index: usize,
    message: String,
    should_quit: bool,
    quit_with_save: bool, // true only when Ctrl+Q / :quit saves sessions
    last_ctrl_c: Option<Instant>,
    terminal_size: (u16, u16),
    sidebar_width: u16,
    sidebar_dragging: bool,
    show_bottom_panel: bool,
    panel_position: PanelPosition,
    show_help: bool,
    show_session_picker: bool,
    picker_cursor: usize,
    cached_usage: std::sync::Arc<std::sync::Mutex<HashMap<String, String>>>,
    last_usage_fetch: Instant,
    last_slow_tick: Instant, // rate-limit history/stall/usage-parse to every 2s
    last_scroll: Instant,    // throttle scroll events
    home_cursor: usize,
    saved_sessions: Vec<persist::SessionInfo>,
    history: Option<HistoryRecorder>,
    stall: StallDetector,
}

impl App {
    pub fn new() -> Self {
        Self {
            panes: Vec::new(),
            focused: 0,
            mode: Mode::Normal,
            command_input: String::new(),
            command_history: Vec::new(),
            aliases: load_aliases(),
            history_cursor: 0,
            tab_matches: Vec::new(),
            tab_index: 0,
            message: String::new(),
            should_quit: false,
            quit_with_save: false,
            last_ctrl_c: None,
            terminal_size: (80, 24),
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            sidebar_dragging: false,
            show_bottom_panel: false,
            panel_position: PanelPosition::Left,
            show_help: false,
            show_session_picker: false,
            picker_cursor: 0,
            cached_usage: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
            last_usage_fetch: Instant::now() - Duration::from_secs(120), // trigger immediately
            last_slow_tick: Instant::now(),
            last_scroll: Instant::now(),
            home_cursor: 0,
            saved_sessions: persist::load_sessions(),
            history: HistoryRecorder::new().ok(),
            stall: StallDetector::new(),
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

            // Refresh usage data in background thread every 60s
            if self.last_usage_fetch.elapsed() > Duration::from_secs(60) {
                self.last_usage_fetch = Instant::now();
                let cache = std::sync::Arc::clone(&self.cached_usage);
                std::thread::spawn(move || {
                    for name in ["claude", "codex", "gemini"] {
                        let result = crate::usage::format_cli_usage(name);
                        if result != "—"
                            && let Ok(mut c) = cache.lock() {
                                c.insert(name.to_string(), result);
                            }
                    }
                });
            }

            // Poll PTY output (fast — just drain buffer, O(1) swap)
            for pane in &mut self.panes {
                pane.poll_output();
                if !pane.is_alive() && pane.status != Status::Dead {
                    pane.status = Status::Dead;
                }
            }

            // Slow tick: history, stall detection, usage parsing (every 2s, not 60fps)
            if self.last_slow_tick.elapsed() > Duration::from_secs(2) {
                self.last_slow_tick = Instant::now();
                for pane in &mut self.panes {
                    if pane.status == Status::Dead { continue; }
                    let text = pane.screen_text();
                    if let Some(ref recorder) = self.history {
                        let name = format!("{}_{}", pane.cli.name(), pane.id);
                        let _ = recorder.record(&name, &text);
                    }
                    if let Some(usage) = crate::usage::parse_usage_from_screen(pane.cli.name(), &text)
                        && let Ok(mut c) = self.cached_usage.lock() {
                            c.insert(pane.cli.name().to_string(), usage.format());
                        }
                    let stall = self.stall.check(&format!("{}", pane.id), &text);
                    if let crate::monitor::StallStatus::Stalled { .. } = stall {
                        pane.status = Status::Stalled;
                    } else if pane.status == Status::Stalled {
                        pane.status = Status::Active;
                    }
                }
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

        // Clear sessions.json unless Ctrl+Q saved them for restore
        if !self.quit_with_save {
            let _ = persist::save_sessions(&[]);
        }

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
            self.draw_empty_state(frame, outer[1]);
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

        let mut spans = vec![
            Span::styled(
                " mtt",
                Style::default()
                    .fg(Color::Rgb(120, 80, 170))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " 0.0.19 ",
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
            // Session uptime
            let uptime = pane.started.elapsed();
            let mins = uptime.as_secs() / 60;
            let hrs = mins / 60;
            let m = mins % 60;
            spans.push(Span::styled(
                if hrs > 0 { format!("  {hrs}h{m:02}m") } else { format!("  {m}m") },
                Style::default().fg(Color::DarkGray),
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

        let bar = Paragraph::new(Line::from(spans));
        frame.render_widget(bar, area);
    }

    fn draw_empty_state(&self, frame: &mut Frame, area: Rect) {
        let w = area.width as usize;
        let mut lines: Vec<Line> = Vec::new();

        // Horizontal bar
        lines.push(Line::from(Span::styled(
            "─".repeat(w),
            Style::default().fg(Color::DarkGray),
        )));

        lines.push(Line::from(Span::styled(
            "  Recent sessions",
            Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));

        let sessions = &self.saved_sessions;
        if sessions.is_empty() {
            lines.push(Line::from(Span::styled(
                "  No session history yet.",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            for (i, info) in sessions.iter().take(15).enumerate() {
                let is_selected = i == self.home_cursor;
                let color = match info.cli_type.as_str() {
                    "claude" => Color::Rgb(232, 149, 106),
                    "codex" => Color::Rgb(52, 211, 153),
                    "gemini" => Color::Rgb(251, 146, 60),
                    _ => Color::Gray,
                };

                let dir = short_path(&info.cwd);
                let date = format_epoch(info.last_active);

                let marker = if is_selected { "▶" } else { " " };

                lines.push(Line::from(vec![
                    Span::styled(
                        format!(" {marker} "),
                        if is_selected {
                            Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        },
                    ),
                    Span::styled("● ", Style::default().fg(color)),
                    Span::styled(
                        format!("{:<8}", info.cli_type),
                        Style::default().fg(color),
                    ),
                    Span::styled(
                        dir.to_string(),
                        if is_selected {
                            Style::default().fg(Color::White)
                        } else {
                            Style::default().fg(Color::Gray)
                        },
                    ),
                    Span::styled(
                        format!("  {date}"),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  ↑↓ select  Enter spawn  :spawn <cli> <dir> for new",
            Style::default().fg(Color::DarkGray),
        )));

        let para = Paragraph::new(lines);
        frame.render_widget(para, area);
    }

    fn draw_sidebar(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::RIGHT)
            .border_style(Style::default().fg(Color::DarkGray));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        for (i, pane) in self.panes.iter().enumerate() {
            if i as u16 >= inner.height {
                break;
            }

            let is_focused = i == self.focused;
            let is_nav_cursor = matches!(self.mode, Mode::SidebarNav) && i == self.picker_cursor;

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
                    if is_nav_cursor {
                        Style::default().fg(Color::Rgb(120, 80, 170)).add_modifier(Modifier::BOLD)
                    } else if is_focused {
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

            let bg = if is_nav_cursor {
                Style::default().bg(Color::Rgb(50, 30, 70))
            } else if is_focused {
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
                " sessions ",
                Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD),
            ));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Session list
        for (i, pane) in self.panes.iter().enumerate() {
            let row_y = inner.y + i as u16;
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

        // All CLIs: show cached API usage, fallback to session time
        for (cli, name) in [
            (CLIType::Claude, "claude"),
            (CLIType::Codex, "codex"),
            (CLIType::Gemini, "gemini"),
        ] {
            let count = self.panes.iter().filter(|p| p.cli == cli).count();
            let display = self.cached_usage
                .lock().ok()
                .and_then(|c| c.get(name).cloned())
                .unwrap_or_else(|| "—".to_string());
            spans.push(Span::styled("● ", Style::default().fg(cli_color(cli))));
            spans.push(Span::styled(format!("{name} {display}"), Style::default().fg(Color::Gray)));
            if count > 0 {
                spans.push(Span::styled(format!(" [{count}]"), Style::default().fg(Color::White)));
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
                    format!("● {name}({count})"),
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
            "  :!<cmd>                      run shell command",
            "  :quit                         exit mtt",
            "",
            "  Keys",
            "  ─────────────────────────────────────",
            "  Ctrl-N / Ctrl-P               switch pane",
            "  Alt-1..9                      jump to pane",
            "  Ctrl-B                        scroll mode",
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

        let title_style = if is_focused {
            Style::default()
                .fg(Color::Black)
                .bg(border_color)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(border_color)
                .add_modifier(Modifier::BOLD)
        };

        let block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(border_color))
            .title(Span::styled(
                format!("{status_indicator} {title}"),
                title_style,
            ));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Render the virtual terminal screen
        let screen = pane.screen();
        let (screen_rows, screen_cols) = screen.size();
        let in_scroll = matches!(self.mode, Mode::Scroll) && is_focused && pane.scroll_offset > 0;

        // In scroll mode, render scrollback lines above the live screen
        if in_scroll {
            let sb_len = pane.scrollback.len();
            let offset = pane.scroll_offset;
            let visible = inner.height as usize;

            for display_row in 0..inner.height {
                // Which scrollback line to show
                // offset=1 means show 1 line of scrollback at top, rest is live screen
                let sb_idx = if offset >= visible {
                    // All scrollback
                    sb_len.saturating_sub(offset) + display_row as usize
                } else {
                    // Mix: top rows are scrollback, bottom rows are live screen
                    let sb_rows = offset.min(visible);
                    if (display_row as usize) < sb_rows {
                        sb_len.saturating_sub(offset) + display_row as usize
                    } else {
                        // Render live screen row
                        let live_row = display_row as usize - sb_rows;
                        let buf = frame.buffer_mut();
                        let mut col: u16 = 0;
                        while col < inner.width.min(screen_cols) {
                            if let Some(cell) = screen.cell(live_row as u16, col) {
                                let ch = cell.contents();
                                let fg = convert_vt100_color(cell.fgcolor());
                                let bg = convert_vt100_color(cell.bgcolor());
                                if !ch.is_empty() || bg != Color::Reset {
                                    let mut style = Style::default();
                                    if fg != Color::Reset { style = style.fg(fg); }
                                    if bg != Color::Reset { style = style.bg(bg); }
                                    if cell.bold() { style = style.add_modifier(Modifier::BOLD); }
                                    if let Some(buf_cell) = buf.cell_mut((inner.x + col, inner.y + display_row)) {
                                        buf_cell.set_symbol(&ch);
                                        buf_cell.set_style(style);
                                    }
                                }
                                col += if ch.chars().any(is_wide_char) { 2 } else { 1 };
                            } else { col += 1; }
                        }
                        continue;
                    }
                };

                if sb_idx < sb_len {
                    let line = &pane.scrollback[sb_idx];
                    let buf = frame.buffer_mut();
                    for (c, sc) in line.cells.iter().enumerate() {
                        if c as u16 >= inner.width { break; }
                        let mut style = Style::default();
                        let fg = convert_vt100_color(sc.fg);
                        let bg = convert_vt100_color(sc.bg);
                        if fg != Color::Reset { style = style.fg(fg); }
                        if bg != Color::Reset { style = style.bg(bg); }
                        if sc.bold { style = style.add_modifier(Modifier::BOLD); }
                        if let Some(buf_cell) = buf.cell_mut((inner.x + c as u16, inner.y + display_row)) {
                            let mut tmp = [0u8; 4];
                            buf_cell.set_symbol(sc.ch.encode_utf8(&mut tmp));
                            buf_cell.set_style(style);
                        }
                    }
                }
            }
        } else {
            // Normal mode: render live screen directly
            for display_row in 0..inner.height.min(screen_rows) {
                let mut col: u16 = 0;
                while col < inner.width.min(screen_cols) {
                    if let Some(cell) = screen.cell(display_row, col) {
                    let ch = cell.contents();
                    let fg = convert_vt100_color(cell.fgcolor());
                    let bg = convert_vt100_color(cell.bgcolor());

                    if ch.is_empty() && bg == Color::Reset {
                        col += 1;
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

                    // Wide characters (Korean/CJK) take 2 columns
                    let w = if ch.chars().any(is_wide_char) { 2 } else { 1 };
                    col += w;
                } else {
                    col += 1;
                }
            }
        }
        } // end else (normal mode)

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
            Span::styled(
                &self.command_input,
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
        ];

        // Tab completion hint (dimmer than input)
        if !self.tab_matches.is_empty() {
            let hint = &self.tab_matches[self.tab_index % self.tab_matches.len()];
            if hint.len() > self.command_input.len() {
                spans.push(Span::styled(
                    &hint[self.command_input.len()..],
                    Style::default().fg(Color::Rgb(70, 70, 70)),
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
            Mode::SidebarNav => self.handle_sidebar_nav_key(key),
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) {
        if key.code != KeyCode::Char('c') || key.modifiers != KeyModifiers::CONTROL {
            self.last_ctrl_c = None;
        }

        match (key.modifiers, key.code) {
            // Ctrl-Q: save sessions and quit
            (KeyModifiers::CONTROL, KeyCode::Char('q')) => {
                self.save_and_quit();
            }
            // Ctrl-C: context-aware
            (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                let double = self
                    .last_ctrl_c
                    .map(|t| t.elapsed() < Duration::from_millis(500))
                    .unwrap_or(false);

                if !self.panes.is_empty() && self.focused < self.panes.len() {
                    if self.panes[self.focused].status == Status::Active {
                        if double {
                            self.save_pane_to_history(self.focused);

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
                        self.save_pane_to_history(self.focused);

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
            // Ctrl-J removed — session picker only available from command mode
            // : enters mtt command mode (/ passes through to the pane for AI CLI slash commands)
            (_, KeyCode::Char(':')) => {
                self.mode = Mode::Command;
                self.command_input.clear();
                self.tab_matches.clear();
            }
            // Ctrl-B enters scroll mode (Ctrl-S conflicts with flow control)
            (KeyModifiers::CONTROL, KeyCode::Char('b')) => {
                if let Some(pane) = self.panes.get(self.focused) {
                    if pane.scrollback.is_empty() {
                        self.message = "no scrollback (TUI apps manage their own scroll)".to_string();
                    } else {
                        self.mode = Mode::Scroll;
                        if let Some(pane) = self.panes.get_mut(self.focused) {
                            pane.scroll_offset = 0;
                        }
                        self.message = "scroll mode — ↑↓/PgUp/PgDn scroll, Esc exit".to_string();
                    }
                }
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
            // Home screen navigation when no panes
            _ if self.panes.is_empty() && !self.saved_sessions.is_empty() => {
                match key.code {
                    KeyCode::Up => {
                        if self.home_cursor > 0 {
                            self.home_cursor -= 1;
                        }
                    }
                    KeyCode::Down => {
                        if self.home_cursor + 1 < self.saved_sessions.len() {
                            self.home_cursor += 1;
                        }
                    }
                    KeyCode::Enter => {
                        let info = self.saved_sessions[self.home_cursor].clone();
                        let cmd = format!("spawn {} {}", info.cli_type, info.cwd);
                        self.execute_command(&cmd);
                    }
                    _ => {}
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
                // If input is empty and bottom layout, open session picker
                if self.command_input.is_empty()
                    && self.panel_position == PanelPosition::Bottom
                    && !self.panes.is_empty()
                {
                    self.picker_cursor = self.focused;
                    self.show_session_picker = true;
                    self.mode = Mode::Normal;
                    self.tab_matches.clear();
                } else if self.history_cursor < self.command_history.len() {
                    // Otherwise cycle command history
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
            KeyCode::Left => {
                // Left arrow enters sidebar navigation (left layout only)
                if self.panel_position == PanelPosition::Left && !self.panes.is_empty() {
                    self.mode = Mode::SidebarNav;
                    self.picker_cursor = self.focused;
                    self.command_input.clear();
                    self.tab_matches.clear();
                }
            }
            KeyCode::F(2) => {
                if !self.panes.is_empty() {
                    self.picker_cursor = self.focused;
                    self.show_session_picker = true;
                    self.mode = Mode::Normal;
                    self.command_input.clear();
                }
            }
            KeyCode::Char(c) => {
                self.command_input.push(c);
                self.update_tab_matches();
            }
            _ => {}
        }
    }

    fn handle_sidebar_nav_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.picker_cursor > 0 {
                    self.picker_cursor -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.picker_cursor + 1 < self.panes.len() {
                    self.picker_cursor += 1;
                }
            }
            KeyCode::Enter => {
                self.focused = self.picker_cursor;
                self.mode = Mode::Normal;
            }
            KeyCode::Right | KeyCode::Esc => {
                // Right arrow or Esc goes back to normal mode
                self.mode = Mode::Normal;
            }
            _ => {
                self.mode = Mode::Normal;
            }
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
                    let max = pane.max_scroll();
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
                    let max = pane.max_scroll();
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
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                // Throttle: skip if last scroll was <50ms ago
                if self.last_scroll.elapsed() < Duration::from_millis(50) {
                    return;
                }
                self.last_scroll = Instant::now();

                let is_up = matches!(mouse.kind, MouseEventKind::ScrollUp);

                if matches!(self.mode, Mode::Scroll) {
                    // mtt scroll mode — navigate scrollback
                    if let Some(pane) = self.panes.get_mut(self.focused) {
                        if is_up {
                            let max = pane.max_scroll();
                            pane.scroll_offset = (pane.scroll_offset + 1).min(max);
                        } else if pane.scroll_offset == 0 {
                            self.mode = Mode::Normal;
                        } else {
                            pane.scroll_offset = pane.scroll_offset.saturating_sub(1);
                        }
                    }
                } else if let Some(pane) = self.panes.get_mut(self.focused) {
                    // Normal mode — forward to pane as SGR mouse event
                    let col = mouse.column.saturating_sub(self.sidebar_width + 1) + 1;
                    let row = mouse.row.saturating_sub(1) + 1;
                    let btn = if is_up { 64 } else { 65 };
                    let seq = format!("\x1b[<{btn};{col};{row}M");
                    let _ = pane.send_keys(seq.as_bytes());
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
            // Completing command name + aliases
            let prefix = parts.first().copied().unwrap_or("");
            for cmd in COMMANDS {
                if cmd.starts_with(prefix) && *cmd != prefix {
                    self.tab_matches.push(cmd.to_string());
                }
            }
            for alias_name in self.aliases.keys() {
                if alias_name.starts_with(prefix) && alias_name != prefix {
                    self.tab_matches.push(alias_name.clone());
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
                // Check if last part starts with -- (completing a flag)
                let last = parts.last().copied().unwrap_or("");
                if last.starts_with("--") || (input.ends_with(' ') && parts.len() >= 2) {
                    let prefix = if last.starts_with("--") { last } else { "--" };
                    let base = if last.starts_with("--") {
                        parts[..parts.len()-1].join(" ")
                    } else {
                        parts.join(" ")
                    };
                    for flag in CLI_FLAGS {
                        if flag.starts_with(prefix) && *flag != prefix {
                            self.tab_matches.push(format!("{base} {flag}"));
                        }
                    }
                    if !self.tab_matches.is_empty() {
                        return;
                    }
                }
                // Completing directory path
                let dir_part = if input.ends_with(' ') && parts.len() == 2 {
                    ""
                } else if parts.len() > 2 {
                    parts[2..].iter().rev().find(|p| !p.starts_with("--")).copied().unwrap_or("")
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
                // Match resize calculation: chrome(4) + pane borders(2) + sidebar + sidebar border(1)
                let pane_rows = rows.saturating_sub(6);
                let pane_cols = cols.saturating_sub(self.sidebar_width + 3);

                let id = self.panes.len();
                match Pane::spawn(id, cli, &cwd, pane_rows.max(10), pane_cols.max(20), &extra_args) {
                    Ok(mut pane) => {
                        // Ensure PTY gets the correct size immediately
                        let _ = pane.resize(pane_rows.max(1), pane_cols.max(1));

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
                    self.save_pane_to_history(idx);

                    self.stall.remove(&format!("{}", self.panes[idx].id));
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
                self.save_and_quit();
            }

            "sessions" | "ss" => {
                if !self.panes.is_empty() {
                    self.picker_cursor = self.focused;
                    self.show_session_picker = true;
                } else {
                    self.message = "no sessions".to_string();
                }
            }

            "history" => {
                if let Some(ref recorder) = self.history {
                    if parts.len() > 1 && parts[1] == "search" {
                        let query = parts[2..].join(" ");
                        match recorder.search(&query) {
                            Ok(results) => {
                                if results.is_empty() {
                                    self.message = format!("no results for '{query}'");
                                } else {
                                    let lines: Vec<String> = results.iter().take(5).map(|r| {
                                        format!("[{}:{}] {}", r.session, r.line_number, &r.content[..r.content.len().min(50)])
                                    }).collect();
                                    self.message = lines.join(" | ");
                                }
                            }
                            Err(e) => self.message = format!("search error: {e}"),
                        }
                    } else if let Some(pane) = self.panes.get(self.focused) {
                        let name = format!("{}_{}", pane.cli.name(), pane.id);
                        match recorder.get_history(&name, 20) {
                            Ok(lines) => self.message = lines.join("\n"),
                            Err(e) => self.message = format!("history error: {e}"),
                        }
                    }
                } else {
                    self.message = "history not available".to_string();
                }
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

            "alias" => {
                // :alias — list all aliases
                // :alias cla claude --dangerously-skip-permissions — define alias
                // :alias cla — remove alias
                if parts.len() == 1 {
                    if self.aliases.is_empty() {
                        self.message = "no aliases. :alias <name> <expansion>".to_string();
                    } else {
                        let list: Vec<String> = self.aliases.iter()
                            .map(|(k, v)| format!("{k}={v}"))
                            .collect();
                        self.message = list.join(" | ");
                    }
                } else if parts.len() == 2 {
                    // Remove alias
                    let name = parts[1];
                    self.aliases.remove(name);
                    save_aliases(&self.aliases);
                    self.message = format!("removed alias '{name}'");
                } else {
                    // Define alias: :alias cla claude --dangerously-skip-permissions
                    let name = parts[1].to_string();
                    let expansion = parts[2..].join(" ");
                    self.aliases.insert(name.clone(), expansion.clone());
                    save_aliases(&self.aliases);
                    self.message = format!("alias {name} = {expansion}");
                }
            }

            _ => {
                // !command — run shell command
                if cmd.starts_with('!') {
                    let shell_cmd = cmd.trim_start_matches('!').trim();
                    if !shell_cmd.is_empty() {
                        match std::process::Command::new("sh")
                            .args(["-c", shell_cmd])
                            .output()
                        {
                            Ok(output) => {
                                let stdout = String::from_utf8_lossy(&output.stdout);
                                let stderr = String::from_utf8_lossy(&output.stderr);
                                let result = if !stdout.is_empty() {
                                    stdout.trim().to_string()
                                } else if !stderr.is_empty() {
                                    stderr.trim().to_string()
                                } else {
                                    format!("(exit {})", output.status.code().unwrap_or(-1))
                                };
                                // Truncate long output for status bar
                                self.message = if result.len() > 200 {
                                    format!("{}…", &result[..200])
                                } else {
                                    result
                                };
                            }
                            Err(e) => {
                                self.message = format!("shell error: {e}");
                            }
                        }
                    }
                }
                // Check aliases
                else if let Some(expansion) = self.aliases.get(parts[0]).cloned() {
                    let full = if parts.len() > 1 {
                        format!("spawn {} {}", expansion, parts[1..].join(" "))
                    } else {
                        format!("spawn {}", expansion)
                    };
                    self.execute_command(&full);
                } else {
                    self.message = format!("unknown command: {}", parts[0]);
                }
            }
        }
    }

    fn save_pane_to_history(&mut self, idx: usize) {
        if idx < self.panes.len() {
            let pane = &self.panes[idx];
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let info = persist::SessionInfo {
                cli_type: pane.cli.name().to_string(),
                cwd: pane.cwd.clone(),
                status: "killed".to_string(),
                last_active: now,
            };
            // Update in-memory list only (sessions.json written only on Ctrl+Q)
            self.saved_sessions.retain(|s| !(s.cli_type == info.cli_type && s.cwd == info.cwd));
            self.saved_sessions.insert(0, info);
            self.saved_sessions.truncate(20);
        }
    }

    fn save_and_quit(&mut self) {
        // Save session info for restore
        let sessions: Vec<persist::SessionInfo> = self
            .panes
            .iter()
            .filter(|p| p.status == Status::Active)
            .map(|p| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                persist::SessionInfo {
                    cli_type: p.cli.name().to_string(),
                    cwd: p.cwd.clone(),
                    status: "saved".to_string(),
                    last_active: now,
                }
            })
            .collect();
        let _ = persist::save_sessions(&sessions);

        self.quit_with_save = true;
        self.should_quit = true;
    }

    /// Load previous sessions and re-spawn them.
    pub fn restore_sessions(&mut self) {
        let saved = persist::load_sessions();
        if saved.is_empty() {
            return;
        }
        let (cols, rows) = self.terminal_size;
        let pane_rows = rows.saturating_sub(6);
        let pane_cols = cols.saturating_sub(self.sidebar_width + 3);

        for info in &saved {
            let cli = match CLIType::from_str(&info.cli_type) {
                Some(c) => c,
                None => continue,
            };
            let id = self.panes.len();
            match Pane::spawn(id, cli, &info.cwd, pane_rows.max(10), pane_cols.max(20), &[]) {
                Ok(mut pane) => {
                    let _ = pane.resize(pane_rows.max(1), pane_cols.max(1));

                    self.panes.push(pane);
                }
                Err(_) => continue,
            }
        }
        if !self.panes.is_empty() {
            self.focused = 0;
            self.message = format!("restored {} sessions", self.panes.len());
        }
        // Clear saved file after restore
        let _ = persist::save_sessions(&[]);
    }

    // ── Resize ───────────────────────────────────────────────────────────

    fn handle_resize(&mut self, cols: u16, rows: u16) {
        self.terminal_size = (cols, rows);

        // Account for: status bar (1) + usage bar (1) + pane border (2)
        let chrome_rows: u16 = 4;

        // Pane borders take 2 cols (left+right) and 2 rows (top+bottom)
        let border_cols: u16 = 2;
        let border_rows: u16 = 2;

        let (pane_rows, pane_cols) = match self.panel_position {
            PanelPosition::Left => {
                // sidebar + sidebar border (1) + pane borders (2)
                let pr = rows.saturating_sub(chrome_rows + border_rows);
                let pc = cols.saturating_sub(self.sidebar_width + 1 + border_cols);
                (pr, pc)
            }
            PanelPosition::Bottom => {
                let extra = if self.show_bottom_panel { BOTTOM_PANEL_HEIGHT } else { 1 };
                let pr = rows.saturating_sub(chrome_rows + extra + border_rows);
                let pc = cols.saturating_sub(border_cols);
                (pr, pc)
            }
        };

        for pane in &mut self.panes {
            let _ = pane.resize(pane_rows.max(1), pane_cols.max(1));
        }

        // Close overlays on resize to avoid rendering issues
        self.show_session_picker = false;
        self.show_help = false;
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
        // Ctrl+Backspace: delete word backward (send ESC + DEL or Ctrl+W)
        (KeyModifiers::CONTROL, KeyCode::Backspace) => vec![0x17], // Ctrl+W
        // Alt/Option+Backspace: delete word backward
        (KeyModifiers::ALT, KeyCode::Backspace) => b"\x1b\x7f".to_vec(), // ESC + DEL
        // Ctrl+Delete: delete word forward
        (KeyModifiers::CONTROL, KeyCode::Delete) => b"\x1b[3;5~".to_vec(),
        // Alt+Left/Right: word jump
        (KeyModifiers::ALT, KeyCode::Left) => b"\x1bb".to_vec(),  // ESC + b
        (KeyModifiers::ALT, KeyCode::Right) => b"\x1bf".to_vec(), // ESC + f
        // Ctrl+Left/Right: word jump (alternative)
        (KeyModifiers::CONTROL, KeyCode::Left) => b"\x1b[1;5D".to_vec(),
        (KeyModifiers::CONTROL, KeyCode::Right) => b"\x1b[1;5C".to_vec(),
        // Ctrl+char
        (KeyModifiers::CONTROL, KeyCode::Char(c)) => {
            vec![(c as u8) & 0x1f]
        }
        // Alt+char
        (KeyModifiers::ALT, KeyCode::Char(c)) => {
            let mut bytes = vec![b'\x1b'];
            let mut buf = [0u8; 4];
            let s = c.encode_utf8(&mut buf);
            bytes.extend_from_slice(s.as_bytes());
            bytes
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

fn is_wide_char(c: char) -> bool {
    let cp = c as u32;
    (0x1100..=0x115F).contains(&cp)    // Hangul Jamo
    || (0x2E80..=0x303E).contains(&cp) // CJK Radicals
    || (0x3040..=0x33BF).contains(&cp) // Hiragana, Katakana
    || (0x3400..=0x4DBF).contains(&cp) // CJK Ext A
    || (0x4E00..=0x9FFF).contains(&cp) // CJK Unified
    || (0xAC00..=0xD7AF).contains(&cp) // Hangul Syllables
    || (0xF900..=0xFAFF).contains(&cp) // CJK Compat
    || (0xFE30..=0xFE4F).contains(&cp) // CJK Forms
    || (0xFF00..=0xFF60).contains(&cp) // Fullwidth
    || (0x20000..=0x2FA1F).contains(&cp) // CJK Ext B+
}

fn format_epoch(epoch: u64) -> String {
    if epoch == 0 {
        return "—".to_string();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let diff = now.saturating_sub(epoch);
    if diff < 60 {
        "just now".to_string()
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}

fn cli_color(cli: CLIType) -> Color {
    match cli {
        CLIType::Claude => Color::Rgb(232, 149, 106),
        CLIType::Codex => Color::Rgb(52, 211, 153),
        CLIType::Gemini => Color::Rgb(251, 146, 60),
        CLIType::OpenCode => Color::Rgb(56, 189, 248),
        CLIType::Shell => Color::Rgb(180, 180, 180), // gray for plain shell
    }
}
