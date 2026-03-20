package tui

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"

	"github.com/kickthemoon0817/mother-terminal/internal/backend"
	"github.com/kickthemoon0817/mother-terminal/internal/history"
	"github.com/kickthemoon0817/mother-terminal/internal/remote"
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
	colorClaude   = lipgloss.Color("#e8956a") // claude terracotta
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
	recorder *history.Recorder
	remotes  *remote.Client

	mode         viewMode
	cursor       int
	sessions     []pkg.Session
	selected     *pkg.Session
	input        InputModel
	message      string // status message displayed briefly
	pendingSpawn string // CLI name waiting for directory input
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
	recorder *history.Recorder,
	remotes *remote.Client,
) Model {
	return Model{
		manager:  manager,
		registry: registry,
		windows:  windows,
		monitor:  monitor,
		recorder: recorder,
		remotes:  remotes,
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

	case spawnMsg:
		if msg.err != nil {
			m.message = fmt.Sprintf("spawn error: %v", msg.err)
			return m, nil
		}
		if msg.session != nil {
			m.manager.AddOrUpdate(*msg.session)
			if m.recorder != nil {
				m.recorder.Record(*msg.session)
			}
			m.sessions = m.manager.List()
			m.message = fmt.Sprintf("spawned: %s", msg.session.Name)
		}
		return m, nil

	case messageMsg:
		m.message = msg.text
		return m, nil

	case remoteSessionsMsg:
		if msg.err != nil {
			m.message = fmt.Sprintf("remote discover error: %v", msg.err)
			return m, nil
		}
		for _, s := range msg.sessions {
			m.manager.AddOrUpdate(s)
			if m.recorder != nil {
				m.recorder.Record(s)
			}
		}
		m.sessions = m.manager.List()
		m.message = fmt.Sprintf("discovered %d remote sessions", len(msg.sessions))
		return m, nil
	}

	return m, nil
}

func (m Model) handleKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	key := msg.String()

	switch key {
	case "ctrl+c":
		return m, tea.Quit
	case "esc":
		if m.mode == viewDetail {
			m.mode = viewDashboard
			m.selected = nil
		}
		m.input.value = ""
		m.pendingSpawn = ""
		m.input.dirMode = false
		m.message = ""
		return m, nil
	case "up":
		if m.cursor > 0 {
			m.cursor--
		}
		return m, nil
	case "down":
		if m.cursor < len(m.sessions)-1 {
			m.cursor++
		}
		return m, nil
	case "enter":
		if m.input.value != "" {
			return m.handleInputSubmit()
		}
		// No input — select session
		if m.mode == viewDashboard && len(m.sessions) > 0 && m.cursor < len(m.sessions) {
			s := m.sessions[m.cursor]
			m.selected = &s
			m.mode = viewDetail
		}
		return m, nil
	default:
		// All other keys go to the input bar
		var cmd tea.Cmd
		m.input, cmd = m.input.Update(msg)
		return m, cmd
	}
}

func (m Model) handleInputSubmit() (tea.Model, tea.Cmd) {
	value := m.input.value
	m.input.value = ""

	if value == "" {
		if m.pendingSpawn != "" {
			// Empty directory = use current directory
			cli := m.pendingSpawn
			m.pendingSpawn = ""
			m.input.dirMode = false
			m.message = ""
			return m, m.spawnSession(cli + " .")
		}
		return m, nil
	}

	// If we're waiting for a directory for /spawn
	if m.pendingSpawn != "" {
		cli := m.pendingSpawn
		m.pendingSpawn = ""
		m.input.dirMode = false
		m.message = ""
		return m, m.spawnSession(cli + " " + value)
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
		m.message = "commands: /spawn /attach /history /connect /discover /ping /hosts /refresh /back /quit"
		return m, nil

	case value == "/attach" || value == "/a":
		// Attach to the selected session's tmux pane
		if m.selected != nil && m.selected.Backend == pkg.BackendTmux {
			target := m.selected.Target
			return m, tea.ExecProcess(exec.Command("tmux", "attach", "-t", target), func(err error) tea.Msg {
				return refreshMsg{}
			})
		}
		// If in dashboard, attach to session under cursor
		if len(m.sessions) > 0 && m.cursor < len(m.sessions) {
			s := m.sessions[m.cursor]
			if s.Backend == pkg.BackendTmux {
				return m, tea.ExecProcess(exec.Command("tmux", "attach", "-t", s.Target), func(err error) tea.Msg {
					return refreshMsg{}
				})
			}
			m.message = "cannot attach — session is not in tmux"
		}
		return m, nil

	case strings.HasPrefix(value, "/spawn "):
		args := strings.TrimSpace(strings.TrimPrefix(value, "/spawn"))
		// /spawn claude --remote myserver — immediate remote spawn
		if strings.Contains(args, "--remote ") {
			parts := strings.SplitN(args, "--remote ", 2)
			cliName := strings.ToLower(strings.TrimSpace(parts[0]))
			if _, ok := pkg.KnownCLIs[cliName]; !ok {
				m.message = fmt.Sprintf("unknown CLI %q — use: claude, codex, gemini, opencode", cliName)
				return m, nil
			}
			hostName := strings.TrimSpace(parts[1])
			return m, m.spawnRemoteSession(cliName, hostName)
		}
		// /spawn claude — prompt for directory
		cliName := strings.ToLower(strings.Fields(args)[0])
		if _, ok := pkg.KnownCLIs[cliName]; !ok {
			m.message = fmt.Sprintf("unknown CLI %q — use: claude, codex, gemini, opencode", cliName)
			return m, nil
		}
		m.pendingSpawn = cliName
		m.input.dirMode = true
		m.message = fmt.Sprintf("spawn %s — enter directory (or Enter for current):", cliName)
		return m, nil

	case strings.HasPrefix(value, "/history"):
		// /history — show history for selected session
		// /history search <query> — search across all sessions
		arg := strings.TrimSpace(strings.TrimPrefix(value, "/history"))
		if strings.HasPrefix(arg, "search ") {
			query := strings.TrimSpace(strings.TrimPrefix(arg, "search"))
			return m, m.searchHistory(query)
		}
		if m.selected != nil && m.recorder != nil {
			hist, err := m.recorder.GetHistory(m.selected.Name, 50)
			if err != nil {
				m.message = err.Error()
			} else {
				m.message = hist
			}
		}
		return m, nil

	case strings.HasPrefix(value, "/connect "):
		// /connect user@host — register a remote host
		address := strings.TrimSpace(strings.TrimPrefix(value, "/connect"))
		parts := strings.SplitN(address, "@", 2)
		name := address
		if len(parts) == 2 {
			name = parts[1]
		}
		if m.remotes != nil {
			if err := m.remotes.AddHost(name, address); err != nil {
				m.message = fmt.Sprintf("invalid address: %v", err)
				return m, nil
			}
			m.message = fmt.Sprintf("connected: %s", address)
		}
		return m, nil

	case strings.HasPrefix(value, "/discover "):
		// /discover hostname — discover sessions on remote host
		hostName := strings.TrimSpace(strings.TrimPrefix(value, "/discover"))
		return m, m.discoverRemote(hostName)

	case value == "/hosts":
		// List registered remote hosts
		if m.remotes != nil {
			hosts := m.remotes.ListHosts()
			if len(hosts) == 0 {
				m.message = "no remote hosts registered — use /connect user@host"
			} else {
				var lines []string
				for _, h := range hosts {
					lines = append(lines, fmt.Sprintf("  %s (%s)", h.Name, h.Address))
				}
				m.message = "remote hosts:\n" + strings.Join(lines, "\n")
			}
		}
		return m, nil

	case strings.HasPrefix(value, "/ping "):
		// /ping hostname — ping a remote host
		hostName := strings.TrimSpace(strings.TrimPrefix(value, "/ping"))
		return m, m.pingRemote(hostName)

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

// spawnMsg is sent after a new session is spawned.
type spawnMsg struct {
	session *pkg.Session
	err     error
}

func (m Model) spawnSession(args string) tea.Cmd {
	return func() tea.Msg {
		// Parse: /spawn claude [directory]
		parts := strings.Fields(args)
		if len(parts) == 0 {
			return spawnMsg{err: fmt.Errorf("usage: /spawn <cli> [directory]")}
		}

		cliName := strings.ToLower(parts[0])
		if _, ok := pkg.KnownCLIs[cliName]; !ok {
			return spawnMsg{err: fmt.Errorf("unknown CLI %q — use: claude, codex, gemini, opencode", cliName)}
		}

		// Resolve working directory
		dir := ""
		if len(parts) > 1 {
			dir = strings.Join(parts[1:], " ")
			// Expand ~
			if strings.HasPrefix(dir, "~/") {
				home, _ := os.UserHomeDir()
				dir = filepath.Join(home, dir[2:])
			}
		}

		// Create a unique tmux session name
		sessionName := fmt.Sprintf("mtt-%s-%d", cliName, time.Now().Unix())

		// Spawn in tmux with optional directory
		tmuxArgs := []string{"new-session", "-d", "-s", sessionName}
		if dir != "" {
			tmuxArgs = append(tmuxArgs, "-c", dir)
		}
		tmuxArgs = append(tmuxArgs, cliName)
		cmd := exec.Command("tmux", tmuxArgs...)
		if err := cmd.Run(); err != nil {
			return spawnMsg{err: fmt.Errorf("failed to spawn %s in tmux: %v", cliName, err)}
		}

		// Wait briefly for process to start
		time.Sleep(500 * time.Millisecond)

		// Get the pane PID
		out, err := exec.Command("tmux", "list-panes", "-t", sessionName, "-F", "#{pane_pid}").Output()
		if err != nil {
			return spawnMsg{err: fmt.Errorf("spawned but couldn't get pane info: %v", err)}
		}
		panePID := strings.TrimSpace(string(out))

		// Resolve display name from directory
		displayDir := dir
		if displayDir == "" {
			displayDir, _ = os.Getwd()
		}

		sess := &pkg.Session{
			ID:        fmt.Sprintf("tmux-%s-%s", cliName, sessionName),
			Name:      fmt.Sprintf("%s@%s", cliName, sessionName),
			CLI:       pkg.CLIType(cliName),
			Backend:   pkg.BackendTmux,
			Target:    sessionName + ":0.0",
			Status:    pkg.StatusActive,
			Policy:    pkg.PolicyNotify,
			PID:       panePID,
			CWD:       displayDir,
			Args:      fmt.Sprintf("tmux attach -t %s", sessionName),
			ParentApp: "tmux",
		}

		return spawnMsg{session: sess}
	}
}

// messageMsg carries a status message to display.
type messageMsg struct{ text string }

// remoteSessionsMsg carries discovered remote sessions.
type remoteSessionsMsg struct {
	sessions []pkg.Session
	err      error
}

func (m Model) spawnRemoteSession(cliName, hostName string) tea.Cmd {
	return func() tea.Msg {
		if m.remotes == nil {
			return spawnMsg{err: fmt.Errorf("remote client not initialized")}
		}
		sess, err := m.remotes.Spawn(hostName, cliName)
		if err != nil {
			return spawnMsg{err: err}
		}
		return spawnMsg{session: sess}
	}
}

func (m Model) searchHistory(query string) tea.Cmd {
	return func() tea.Msg {
		if m.recorder == nil {
			return messageMsg{text: "history not available"}
		}
		results, err := m.recorder.Search(query)
		if err != nil {
			return messageMsg{text: fmt.Sprintf("search error: %v", err)}
		}
		if len(results) == 0 {
			return messageMsg{text: fmt.Sprintf("no results for %q", query)}
		}
		var lines []string
		for _, r := range results {
			if len(lines) >= 20 {
				lines = append(lines, fmt.Sprintf("  ... and %d more", len(results)-20))
				break
			}
			lines = append(lines, fmt.Sprintf("  [%s:%d] %s", r.Session, r.Line, r.Content))
		}
		return messageMsg{text: strings.Join(lines, "\n")}
	}
}

func (m Model) discoverRemote(hostName string) tea.Cmd {
	return func() tea.Msg {
		if m.remotes == nil {
			return messageMsg{text: "remote client not initialized"}
		}
		sessions, err := m.remotes.DiscoverRemote(hostName)
		return remoteSessionsMsg{sessions: sessions, err: err}
	}
}

func (m Model) pingRemote(hostName string) tea.Cmd {
	return func() tea.Msg {
		if m.remotes == nil {
			return messageMsg{text: "remote client not initialized"}
		}
		ok, latency, err := m.remotes.Ping(hostName)
		if err != nil {
			return messageMsg{text: fmt.Sprintf("ping %s: error %v", hostName, err)}
		}
		if ok {
			return messageMsg{text: fmt.Sprintf("ping %s: ok (%v)", hostName, latency)}
		}
		return messageMsg{text: fmt.Sprintf("ping %s: unreachable (%v)", hostName, latency)}
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
