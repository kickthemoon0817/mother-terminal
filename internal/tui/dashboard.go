package tui

import (
	"fmt"
	"path/filepath"
	"strings"

	"github.com/kickthemoon0817/mother-terminal/pkg"
)

func (m Model) dashboardView() string {
	var b strings.Builder

	b.WriteString("╔══════════════════════════════════════════════════════════════════╗\n")
	b.WriteString("║              Mother Terminal — AI CLI Orchestrator               ║\n")
	b.WriteString("╚══════════════════════════════════════════════════════════════════╝\n\n")

	if len(m.sessions) == 0 {
		b.WriteString("  No sessions discovered.\n")
		b.WriteString("  Press 'r' to refresh or check your config.\n\n")
	} else {
		for i, sess := range m.sessions {
			cursor := "  "
			if i == m.cursor {
				cursor = "> "
			}

			status := formatStatus(sess.Status)

			// Main line: cursor + CLI + project name + status
			project := shortCWD(sess.CWD)
			if project == "" {
				project = sess.Name
			}
			b.WriteString(fmt.Sprintf("%s%-8s %-30s %s\n",
				cursor,
				string(sess.CLI),
				truncate(project, 30),
				status,
			))

			// Detail line: parent app + TTY + start time
			detail := formatSessionDetail(sess)
			if detail != "" {
				b.WriteString(fmt.Sprintf("    %s\n", detail))
			}
		}
	}

	b.WriteString("\n")

	// Window timers
	if m.windows != nil {
		hasWindows := false
		for _, sess := range m.sessions {
			remaining := m.windows.Remaining(sess.Name)
			if remaining > 0 {
				if !hasWindows {
					b.WriteString("  Timers: ")
					hasWindows = true
				}
				hours := int(remaining.Hours())
				mins := int(remaining.Minutes()) % 60
				b.WriteString(fmt.Sprintf("%s %dh%02dm  ", sess.Name, hours, mins))
			}
		}
		if hasWindows {
			b.WriteString("\n")
		}
	}

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

func formatSessionDetail(sess pkg.Session) string {
	parts := []string{}
	if sess.ParentApp != "" && sess.ParentApp != "unknown" {
		parts = append(parts, sess.ParentApp)
	}
	if sess.Target != "" {
		parts = append(parts, filepath.Base(sess.Target))
	}
	if sess.StartTime != "" {
		// Shorten "Thu Mar 19 11:37:12 2026" to "Mar 19 11:37"
		parts = append(parts, shortTime(sess.StartTime))
	}
	if len(parts) == 0 {
		return ""
	}
	return strings.Join(parts, " | ")
}

// shortCWD returns a shortened working directory path.
func shortCWD(cwd string) string {
	if cwd == "" {
		return ""
	}
	// Replace home dir prefix with ~
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

// shortTime extracts a short time from ps lstart format.
// Input: "Thu Mar 19 11:37:12 2026" → Output: "Mar 19 11:37"
func shortTime(lstart string) string {
	fields := strings.Fields(lstart)
	if len(fields) >= 4 {
		// fields: [Day, Month, Date, Time, Year]
		timeParts := strings.Split(fields[3], ":")
		shortT := fields[3]
		if len(timeParts) >= 2 {
			shortT = timeParts[0] + ":" + timeParts[1]
		}
		return fmt.Sprintf("%s %s %s", fields[1], fields[2], shortT)
	}
	return lstart
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
	r := []rune(s)
	if len(r) <= max {
		return s
	}
	return string(r[:max-3]) + "..."
}
