package tui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"

	"github.com/kickthemoon0817/mother-terminal/pkg"
)

// ── Detail view ───────────────────────────────────────────────────────────────

func (m Model) detailView() string {
	if m.selected == nil {
		return "no session selected"
	}

	w := m.width
	if w == 0 {
		w = 100
	}

	sess := m.selected

	// Header strip
	header := m.renderDetailHeader(sess, w)

	// Two-column metadata panel
	meta := m.renderMetaPanel(sess, w)

	// Output panel
	output := m.renderOutputPanel(sess, w)

	// Bottom bar
	bottom := m.renderDetailBottomBar(w)

	return lipgloss.JoinVertical(lipgloss.Left,
		header,
		meta,
		output,
		bottom,
	)
}

// ── Detail header ─────────────────────────────────────────────────────────────

func (m Model) renderDetailHeader(sess *pkg.Session, w int) string {
	cliBadge := lipgloss.NewStyle().
		Foreground(cliColor(sess.CLI)).
		Bold(true).
		Render(strings.ToUpper(string(sess.CLI)))

	sep := lipgloss.NewStyle().Foreground(colorBorder).Render("  /  ")

	cwd := shortCWD(sess.CWD)
	if cwd == "" {
		cwd = sess.Name
	}
	cwdStyle := lipgloss.NewStyle().Foreground(colorFgBold).Bold(true)
	cwdStr := cwdStyle.Render(truncate(cwd, w-40))

	statusBadge := renderStatusBadge(sess.Status)

	left := cliBadge + sep + cwdStr
	right := statusBadge

	// Pad right to align
	leftWidth := lipgloss.Width(left)
	rightWidth := lipgloss.Width(right)
	gap := w - leftWidth - rightWidth - 4
	if gap < 1 {
		gap = 1
	}
	inner := left + strings.Repeat(" ", gap) + right

	return lipgloss.NewStyle().
		BorderStyle(lipgloss.ThickBorder()).
		BorderBottom(true).
		BorderForeground(colorBorder).
		Width(w).
		Padding(0, 1).
		Render(inner)
}

// ── Metadata panel ────────────────────────────────────────────────────────────

func (m Model) renderMetaPanel(sess *pkg.Session, w int) string {
	// Left column: identity
	idRows := []metaRow{
		{"CLI", string(sess.CLI), cliColor(sess.CLI)},
		{"DIRECTORY", sess.CWD, colorFg},
		{"COMMAND", sess.Args, colorSubtle},
		{"TERMINAL", sess.ParentApp, colorSubtle},
		{"STARTED", shortTime(sess.StartTime), colorSubtle},
	}

	// Right column: runtime state
	policy := string(sess.Policy)
	if policy == "" {
		policy = "notify"
	}

	stateRows := []metaRow{
		{"STATUS", statusLabel(sess.Status), statusColor(sess.Status)},
		{"BACKEND", string(sess.Backend), colorSubtle},
		{"TARGET", shortBase(sess.Target), colorSubtle},
		{"PID", sess.PID, colorSubtle},
		{"POLICY", policy, colorSubtle},
	}

	// Window timer row
	if m.windows != nil {
		remaining := m.windows.Remaining(sess.Name)
		if remaining > 0 {
			stateRows = append(stateRows, metaRow{"WINDOW", formatDuration(remaining) + " left", colorStalled})
		} else if w2 := m.windows.GetWindow(sess.Name); w2 != nil {
			stateRows = append(stateRows, metaRow{"WINDOW", "expired", colorDead})
		}
	}

	// Resume message
	if sess.ResumeMessage != "" {
		stateRows = append(stateRows, metaRow{"RESUME", fmt.Sprintf("%q", sess.ResumeMessage), colorSubtle})
	}

	halfW := w/2 - 2
	leftPanel := renderMetaColumn(idRows, halfW)
	rightPanel := renderMetaColumn(stateRows, halfW)

	combined := lipgloss.JoinHorizontal(lipgloss.Top, leftPanel, rightPanel)

	return lipgloss.NewStyle().
		BorderStyle(lipgloss.NormalBorder()).
		BorderBottom(true).
		BorderForeground(colorBorder).
		Width(w).
		Padding(1, 1).
		Render(combined)
}

type metaRow struct {
	label string
	value string
	color lipgloss.Color
}

func renderMetaColumn(rows []metaRow, w int) string {
	labelStyle := lipgloss.NewStyle().
		Foreground(colorMuted).
		Bold(true).
		Width(10)

	var lines []string
	for _, row := range rows {
		if row.value == "" || row.value == "unknown" {
			continue
		}
		label := labelStyle.Render(row.label)
		val := lipgloss.NewStyle().
			Foreground(row.color).
			Width(w - 11).
			Render(truncate(row.value, w-11))
		lines = append(lines, label+" "+val)
	}
	if len(lines) == 0 {
		lines = []string{""}
	}
	return lipgloss.NewStyle().Width(w).Render(strings.Join(lines, "\n"))
}

func statusLabel(status pkg.SessionStatus) string {
	switch status {
	case pkg.StatusActive:
		return "active"
	case pkg.StatusStalled:
		return "stalled"
	case pkg.StatusDead:
		return "dead"
	case pkg.StatusDiscovered:
		return "discovered"
	default:
		return string(status)
	}
}

// ── Output panel ──────────────────────────────────────────────────────────────

func (m Model) renderOutputPanel(sess *pkg.Session, w int) string {
	titleStyle := lipgloss.NewStyle().
		Foreground(colorSubtle).
		Bold(true)

	title := titleStyle.Render("RECENT OUTPUT")

	output := m.readSessionOutput(sess.Name)

	var content string
	if output == "" {
		content = lipgloss.NewStyle().
			Foreground(colorMuted).
			Italic(true).
			Render("output capture not available for this terminal")
	} else {
		lines := strings.Split(strings.TrimRight(output, "\n"), "\n")
		var styled []string
		for _, line := range lines {
			styled = append(styled, lipgloss.NewStyle().Foreground(colorFg).Render(line))
		}
		content = strings.Join(styled, "\n")
	}

	inner := title + "\n" + strings.Repeat("─", lipgloss.Width(title)+20) + "\n" + content

	return lipgloss.NewStyle().
		Padding(1, 1).
		Width(w).
		Render(inner)
}

// ── Bottom bar ────────────────────────────────────────────────────────────────

func (m Model) renderDetailBottomBar(w int) string {
	if m.input.focused {
		return renderInputBar(m.input.value, w)
	}
	return renderHelpBar([]helpItem{
		{"/", "command"},
		{"esc", "back"},
		{"q", "quit"},
	}, w)
}

// ── Session output reader ─────────────────────────────────────────────────────

func (m Model) readSessionOutput(name string) string {
	sess, err := m.manager.Get(name)
	if err != nil {
		return ""
	}
	inj, err := m.registry.Get(sess.Backend)
	if err != nil {
		return ""
	}
	output, err := inj.ReadOutput(*sess, 15)
	if err != nil {
		return ""
	}
	return output
}
