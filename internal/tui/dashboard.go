package tui

import (
	"fmt"
	"path/filepath"
	"strings"

	"github.com/charmbracelet/lipgloss"

	"github.com/kickthemoon0817/mother-terminal/pkg"
)

// ── Dashboard constants ───────────────────────────────────────────────────────

const (
	colCLI    = 10
	colCWD    = 34
	colStatus = 12
	colParent = 14
	colTime   = 12
)

// ── Main view ─────────────────────────────────────────────────────────────────

func (m Model) dashboardView() string {
	w := m.width
	if w == 0 {
		w = 100
	}
	h := m.height
	if h == 0 {
		h = 30
	}

	// Header
	header := renderHeader(w)

	// Session table
	table := m.renderSessionTable(w)

	// Status bar
	statusBar := m.renderStatusBar(w)

	// Help / input bar
	bottomBar := m.renderBottomBar(w)

	// Stack all sections
	return lipgloss.JoinVertical(
		lipgloss.Left,
		header,
		table,
		statusBar,
		bottomBar,
	)
}

// ── Header ────────────────────────────────────────────────────────────────────

func renderHeader(w int) string {
	title := lipgloss.NewStyle().
		Foreground(colorFgBold).
		Bold(true).
		Render("MTT")

	subtitle := lipgloss.NewStyle().
		Foreground(colorMuted).
		Render("AI CLI Orchestrator")

	dot := lipgloss.NewStyle().Foreground(colorBorder).Render("  ·  ")

	inner := title + dot + subtitle

	return lipgloss.NewStyle().
		BorderStyle(lipgloss.ThickBorder()).
		BorderBottom(true).
		BorderForeground(colorBorder).
		Width(w).
		Padding(0, 1).
		Render(inner)
}

// ── Table ─────────────────────────────────────────────────────────────────────

func (m Model) renderSessionTable(w int) string {
	if len(m.sessions) == 0 {
		empty := lipgloss.NewStyle().
			Foreground(colorMuted).
			Padding(2, 2).
			Render("No sessions discovered.  Press r to refresh.")
		return empty
	}

	var rows []string

	// Column header
	rows = append(rows, renderTableHeader(w))

	// Separator
	sep := lipgloss.NewStyle().
		Foreground(colorBorder).
		Width(w).
		Render(strings.Repeat("─", w))
	rows = append(rows, sep)

	for i, sess := range m.sessions {
		rows = append(rows, m.renderSessionRow(sess, i == m.cursor, w))
	}

	return strings.Join(rows, "\n")
}

func renderTableHeader(w int) string {
	_ = w
	style := lipgloss.NewStyle().
		Foreground(colorSubtle).
		Bold(true).
		PaddingLeft(2)

	col := func(s string, width int) string {
		return lipgloss.NewStyle().
			Foreground(colorSubtle).
			Width(width).
			Render(s)
	}

	_ = style
	line := "  " +
		col("CLI", colCLI) +
		col("DIRECTORY", colCWD) +
		col("STATUS", colStatus) +
		col("TERMINAL", colParent) +
		col("STARTED", colTime)

	return lipgloss.NewStyle().
		Foreground(colorSubtle).
		Bold(true).
		PaddingTop(1).
		Render(line)
}

func (m Model) renderSessionRow(sess pkg.Session, selected bool, w int) string {
	// Cursor indicator
	cursor := "  "
	if selected {
		cursor = lipgloss.NewStyle().
			Foreground(colorActive).
			Bold(true).
			Render("▶ ")
	}

	// CLI badge
	cliBadge := renderCLIBadge(sess.CLI)

	// Directory (primary identifier)
	cwd := shortCWD(sess.CWD)
	if cwd == "" {
		cwd = sess.Name
	}
	cwdStyle := lipgloss.NewStyle().Foreground(colorFg).Width(colCWD)
	if selected {
		cwdStyle = cwdStyle.Foreground(colorFgBold).Bold(true)
	}
	cwdCell := cwdStyle.Render(truncate(cwd, colCWD-1))

	// Status badge
	statusCell := renderStatusBadge(sess.Status)

	// Parent app
	parent := sess.ParentApp
	if parent == "" || parent == "unknown" {
		parent = "—"
	}
	parentCell := lipgloss.NewStyle().
		Foreground(colorSubtle).
		Width(colParent).
		Render(truncate(parent, colParent-1))

	// Start time
	started := shortTime(sess.StartTime)
	if started == "" {
		started = "—"
	}
	startedCell := lipgloss.NewStyle().
		Foreground(colorSubtle).
		Width(colTime).
		Render(started)

	// Window timer (appended inline if present)
	timerSuffix := ""
	if m.windows != nil {
		remaining := m.windows.Remaining(sess.Name)
		if remaining > 0 {
			timerSuffix = "  " + lipgloss.NewStyle().
				Foreground(colorStalled).
				Render("⏱ "+formatDuration(remaining))
		}
	}

	line := cursor + cliBadge + cwdCell + statusCell + parentCell + startedCell + timerSuffix

	rowStyle := lipgloss.NewStyle()
	if selected {
		rowStyle = rowStyle.Background(colorSelection)
	}

	return rowStyle.Width(w).Render(line)
}

// ── Badges ────────────────────────────────────────────────────────────────────

func renderCLIBadge(cli pkg.CLIType) string {
	label := strings.ToUpper(string(cli))
	if len(label) > 8 {
		label = label[:8]
	}
	return lipgloss.NewStyle().
		Foreground(cliColor(cli)).
		Bold(true).
		Width(colCLI).
		Render(label)
}

func renderStatusBadge(status pkg.SessionStatus) string {
	var label string
	switch status {
	case pkg.StatusActive:
		label = "● active"
	case pkg.StatusStalled:
		label = "◐ stalled"
	case pkg.StatusDead:
		label = "○ dead"
	case pkg.StatusDiscovered:
		label = "◌ found"
	default:
		label = "? unknown"
	}
	return lipgloss.NewStyle().
		Foreground(statusColor(status)).
		Width(colStatus).
		Render(label)
}

// ── Status bar ────────────────────────────────────────────────────────────────

func (m Model) renderStatusBar(w int) string {
	total := len(m.sessions)
	active, stalled, dead := 0, 0, 0
	for _, s := range m.sessions {
		switch s.Status {
		case pkg.StatusActive:
			active++
		case pkg.StatusStalled:
			stalled++
		case pkg.StatusDead:
			dead++
		}
	}

	dot := lipgloss.NewStyle().Foreground(colorBorder).Render("  │  ")

	totalStr := lipgloss.NewStyle().Foreground(colorSubtle).
		Render(fmt.Sprintf("sessions: %d", total))

	activeStr := lipgloss.NewStyle().Foreground(colorActive).
		Render(fmt.Sprintf("● %d active", active))

	stalledStr := lipgloss.NewStyle().Foreground(colorStalled).
		Render(fmt.Sprintf("◐ %d stalled", stalled))

	deadStr := lipgloss.NewStyle().Foreground(colorDead).
		Render(fmt.Sprintf("○ %d dead", dead))

	content := totalStr + dot + activeStr + dot + stalledStr + dot + deadStr

	return lipgloss.NewStyle().
		BorderStyle(lipgloss.ThickBorder()).
		BorderTop(true).
		BorderForeground(colorBorder).
		Width(w).
		Padding(0, 1).
		MarginTop(1).
		Render(content)
}

// ── Bottom bar ────────────────────────────────────────────────────────────────

func (m Model) renderBottomBar(w int) string {
	if m.input.focused {
		return renderInputBar(m.input.value, w)
	}
	return renderHelpBar([]helpItem{
		{"↑↓/jk", "navigate"},
		{"enter", "detail"},
		{"/", "command"},
		{"r", "refresh"},
		{"q", "quit"},
	}, w)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

func shortCWD(cwd string) string {
	if cwd == "" {
		return ""
	}
	home := "/Users/"
	if idx := strings.Index(cwd, home); idx == 0 {
		parts := strings.SplitN(cwd[len(home):], "/", 2)
		if len(parts) == 2 {
			return "~/" + parts[1]
		}
		return "~"
	}
	return cwd
}

func shortTime(lstart string) string {
	fields := strings.Fields(lstart)
	if len(fields) >= 4 {
		timeParts := strings.Split(fields[3], ":")
		shortT := fields[3]
		if len(timeParts) >= 2 {
			shortT = timeParts[0] + ":" + timeParts[1]
		}
		return fmt.Sprintf("%s %s %s", fields[1], fields[2], shortT)
	}
	return lstart
}

func truncate(s string, max int) string {
	r := []rune(s)
	if len(r) <= max {
		return s
	}
	return string(r[:max-3]) + "..."
}

func shortBase(path string) string {
	return filepath.Base(path)
}
