package tui

import (
	"fmt"

	tea "github.com/charmbracelet/bubbletea"

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

// refreshMsg triggers a session list refresh.
type refreshMsg struct{}

// monitorEventMsg wraps a monitor event for the TUI.
type monitorEventMsg struct {
	event session.Event
}

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

	// Update input model
	if m.input.focused {
		var cmd tea.Cmd
		m.input, cmd = m.input.Update(msg)
		return m, cmd
	}

	return m, nil
}

func (m Model) handleKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	key := msg.String()

	// Global keys
	switch key {
	case "ctrl+c", "q":
		if !m.input.focused {
			return m, tea.Quit
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
	case "tab":
		m.input.focused = !m.input.focused
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
		m.input.focused = false
	case "tab":
		m.input.focused = !m.input.focused
	case "enter":
		if m.input.focused && m.input.value != "" && m.selected != nil {
			query := m.input.value
			m.input.value = ""
			return m, m.sendQuery(m.selected, query)
		}
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
		return "Unknown view"
	}
}

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
		event := <-m.monitor.Events()
		return monitorEventMsg{event: event}
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

// Run starts the TUI application.
func Run(model Model) error {
	p := tea.NewProgram(model)
	_, err := p.Run()
	if err != nil {
		return fmt.Errorf("TUI error: %w", err)
	}
	return nil
}
