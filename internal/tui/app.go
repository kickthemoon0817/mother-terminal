package tui

import (
	"fmt"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"

	"github.com/kickthemoon0817/mother-terminal/internal/backend"
	"github.com/kickthemoon0817/mother-terminal/internal/scheduler"
	"github.com/kickthemoon0817/mother-terminal/internal/session"
	"github.com/kickthemoon0817/mother-terminal/pkg"
)

// View mode for the TUI.
type viewMode int

const (
	viewDashboard viewMode = iota
	viewDetail
)

// ── Palette ──────────────────────────────────────────────────────────────────

var (
	// Base chrome
	colorBorder = lipgloss.Color("#3a3a3a")
	colorMuted   = lipgloss.Color("#5a5a5a")
	colorSubtle  = lipgloss.Color("#888888")
	colorFg      = lipgloss.Color("#d4d4d4")
	colorFgBold  = lipgloss.Color("#f0f0f0")

	// Status
	colorActive     = lipgloss.Color("#4ade80") // green-400
	colorStalled    = lipgloss.Color("#facc15") // yellow-400
	colorDead       = lipgloss.Color("#f87171") // red-400
	colorDiscovered = lipgloss.Color("#60a5fa") // blue-400

	// CLI type accents
	colorClaude   = lipgloss.Color("#a78bfa") // violet-400
	colorCodex    = lipgloss.Color("#34d399") // emerald-400
	colorGemini   = lipgloss.Color("#fb923c") // orange-400
	colorOpenCode = lipgloss.Color("#38bdf8") // sky-400

	// Highlights
	colorSelection = lipgloss.Color("#2d3748") // dark blue-gray
	colorInputBg   = lipgloss.Color("#1e2433")
	colorInputFg   = lipgloss.Color("#93c5fd")
)

// ── Shared style primitives ───────────────────────────────────────────────────
// Styles are created inline where used to avoid unused variable warnings.

// cliColor returns the accent color for a CLI type.
func cliColor(cli pkg.CLIType) lipgloss.Color {
	switch cli {
	case pkg.CLIClaude:
		return colorClaude
	case pkg.CLICodex:
		return colorCodex
	case pkg.CLIGemini:
		return colorGemini
	case pkg.CLIOpenCode:
		return colorOpenCode
	default:
		return colorSubtle
	}
}

// statusColor returns the color for a session status.
func statusColor(status pkg.SessionStatus) lipgloss.Color {
	switch status {
	case pkg.StatusActive:
		return colorActive
	case pkg.StatusStalled:
		return colorStalled
	case pkg.StatusDead:
		return colorDead
	case pkg.StatusDiscovered:
		return colorDiscovered
	default:
		return colorMuted
	}
}

// ── Model ────────────────────────────────────────────────────────────────────

// Model is the root bubbletea model for Mother Terminal.
type Model struct {
	manager  *session.Manager
	registry *backend.Registry
	windows  *scheduler.WindowTracker
	monitor  *session.Monitor

	mode     viewMode
	cursor   int
	sessions []pkg.Session
	selected *pkg.Session
	input    InputModel
	width    int
	height   int
	err      error
}

// NewModel creates a new TUI model.
func NewModel(
	manager *session.Manager,
	registry *backend.Registry,
	windows *scheduler.WindowTracker,
	monitor *session.Monitor,
) Model {
	return Model{
		manager:  manager,
		registry: registry,
		windows:  windows,
		monitor:  monitor,
		mode:     viewDashboard,
		input:    NewInputModel(),
	}
}

// ── Messages ─────────────────────────────────────────────────────────────────

// refreshMsg triggers a session list refresh.
type refreshMsg struct{}

// monitorEventMsg wraps a monitor event for the TUI.
type monitorEventMsg struct {
	event session.Event
}

// ── Lifecycle ─────────────────────────────────────────────────────────────────

func (m Model) Init() tea.Cmd {
	return tea.Batch(
		m.refreshSessions(),
		m.listenMonitorEvents(),
		tickCmd(),
	)
}

func (m Model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyMsg:
		return m.handleKey(msg)

	case tea.WindowSizeMsg:
		m.width = msg.Width
		m.height = msg.Height
		return m, nil

	case refreshMsg:
		m.sessions = m.manager.List()
		return m, nil

	case monitorEventMsg:
		m.sessions = m.manager.List()
		return m, m.listenMonitorEvents()

	case tickMsg:
		return m, tickCmd()
	}

	// Delegate to input model when focused
	if m.input.focused {
		var cmd tea.Cmd
		m.input, cmd = m.input.Update(msg)
		return m, cmd
	}

	return m, nil
}

func (m Model) handleKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	key := msg.String()

	// Global quit (only when input not focused)
	if !m.input.focused {
		switch key {
		case "ctrl+c", "q":
			return m, tea.Quit
		}
	} else {
		// When input is focused, only handle control keys here.
		// All other keys go to the InputModel for character capture.
		switch key {
		case "ctrl+c":
			return m, tea.Quit
		case "escape":
			m.input.focused = false
			m.input.value = ""
			return m, nil
		case "tab":
			m.input.focused = false
			return m, nil
		case "enter":
			return m.handleInputSubmit()
		default:
			// Pass to InputModel for character handling
			var cmd tea.Cmd
			m.input, cmd = m.input.Update(msg)
			return m, cmd
		}
	}

	switch m.mode {
	case viewDashboard:
		return m.handleDashboardKey(key)
	case viewDetail:
		return m.handleDetailKey(key)
	}

	return m, nil
}

func (m Model) handleInputSubmit() (tea.Model, tea.Cmd) {
	value := m.input.value
	m.input.value = ""

	if value == "" {
		return m, nil
	}

	// Slash commands
	switch {
	case value == "/quit" || value == "/q":
		return m, tea.Quit

	case value == "/back" || value == "/b":
		if m.mode == viewDetail {
			m.mode = viewDashboard
			m.selected = nil
		}
		m.input.focused = false
		return m, nil

	case value == "/refresh" || value == "/r":
		m.sessions = m.manager.List()
		return m, nil

	case value == "/sessions" || value == "/ls":
		m.mode = viewDashboard
		m.selected = nil
		m.sessions = m.manager.List()
		return m, nil

	case value == "/help" || value == "/h":
		// Stay in current view, help is shown in the help bar
		return m, nil

	default:
		// If in detail view with a selected session, send as query
		if m.selected != nil {
			return m, m.sendQuery(m.selected, value)
		}

		// If in dashboard, select the session under cursor and send
		if len(m.sessions) > 0 && m.cursor < len(m.sessions) {
			s := m.sessions[m.cursor]
			m.selected = &s
			m.mode = viewDetail
			return m, m.sendQuery(m.selected, value)
		}
	}

	return m, nil
}

func (m Model) handleDashboardKey(key string) (tea.Model, tea.Cmd) {
	switch key {
	case "up", "k":
		if m.cursor > 0 {
			m.cursor--
		}
	case "down", "j":
		if m.cursor < len(m.sessions)-1 {
			m.cursor++
		}
	case "enter":
		if len(m.sessions) > 0 && m.cursor < len(m.sessions) {
			s := m.sessions[m.cursor]
			m.selected = &s
			m.mode = viewDetail
		}
	case "/", "tab":
		m.input.focused = true
	case "r":
		m.sessions = m.manager.List()
	}
	return m, nil
}

func (m Model) handleDetailKey(key string) (tea.Model, tea.Cmd) {
	switch key {
	case "escape":
		m.mode = viewDashboard
		m.selected = nil
	case "/", "tab":
		m.input.focused = true
	}
	return m, nil
}

func (m Model) View() string {
	switch m.mode {
	case viewDashboard:
		return m.dashboardView()
	case viewDetail:
		return m.detailView()
	default:
		return "unknown view"
	}
}

// ── Commands ──────────────────────────────────────────────────────────────────

func (m Model) refreshSessions() tea.Cmd {
	return func() tea.Msg {
		return refreshMsg{}
	}
}

func (m Model) listenMonitorEvents() tea.Cmd {
	if m.monitor == nil {
		return nil
	}
	return func() tea.Msg {
		select {
		case event := <-m.monitor.Events():
			return monitorEventMsg{event: event}
		case <-m.monitor.Done():
			return nil
		}
	}
}

func (m Model) sendQuery(sess *pkg.Session, query string) tea.Cmd {
	return func() tea.Msg {
		inj, err := m.registry.Get(sess.Backend)
		if err != nil {
			return refreshMsg{}
		}
		inj.SendKeys(*sess, query)
		return refreshMsg{}
	}
}

// ── Run ───────────────────────────────────────────────────────────────────────

// Run starts the TUI application.
func Run(model Model) error {
	p := tea.NewProgram(model)

	_, err := p.Run()
	if err != nil {
		return fmt.Errorf("TUI error: %w", err)
	}
	return nil
}
