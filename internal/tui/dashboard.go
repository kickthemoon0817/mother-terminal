package tui

import (
	"fmt"
	"strings"

	"github.com/kickthemoon0817/mother-terminal/pkg"
)

func (m Model) dashboardView() string {
	var b strings.Builder

	b.WriteString("╔══════════════════════════════════════════════════════════╗\n")
	b.WriteString("║           Mother Terminal — AI CLI Orchestrator          ║\n")
	b.WriteString("╚══════════════════════════════════════════════════════════╝\n\n")

	if len(m.sessions) == 0 {
		b.WriteString("  No sessions discovered.\n")
		b.WriteString("  Press 'r' to refresh or check your config.\n\n")
	} else {
		// Header
		b.WriteString(fmt.Sprintf("  %-4s %-20s %-10s %-10s %-12s %s\n",
			"", "NAME", "CLI", "BACKEND", "STATUS", "WINDOW"))
		b.WriteString(fmt.Sprintf("  %-4s %-20s %-10s %-10s %-12s %s\n",
			"", "────", "───", "───────", "──────", "──────"))

		for i, sess := range m.sessions {
			cursor := "  "
			if i == m.cursor {
				cursor = "> "
			}

			status := formatStatus(sess.Status)
			window := m.formatWindow(sess.Name)

			b.WriteString(fmt.Sprintf("%s%-20s %-10s %-10s %-12s %s\n",
				cursor,
				truncate(sess.Name, 20),
				string(sess.CLI),
				string(sess.Backend),
				status,
				window,
			))
		}
	}

	b.WriteString("\n")

	// Input bar
	if m.input.focused {
		b.WriteString("  Query: " + m.input.value + "█\n")
	} else {
		b.WriteString("  [Tab] Input  [Enter] Detail  [r] Refresh  [q] Quit\n")
	}

	return b.String()
}

func formatStatus(status pkg.SessionStatus) string {
	switch status {
	case pkg.StatusActive:
		return "[active]"
	case pkg.StatusStalled:
		return "[STALLED]"
	case pkg.StatusDead:
		return "[dead]"
	case pkg.StatusDiscovered:
		return "[found]"
	default:
		return "[?]"
	}
}

func (m Model) formatWindow(sessionName string) string {
	if m.windows == nil {
		return ""
	}
	remaining := m.windows.Remaining(sessionName)
	if remaining <= 0 {
		return "expired"
	}
	hours := int(remaining.Hours())
	mins := int(remaining.Minutes()) % 60
	return fmt.Sprintf("%dh%02dm left", hours, mins)
}

func truncate(s string, max int) string {
	if len(s) <= max {
		return s
	}
	return s[:max-3] + "..."
}
