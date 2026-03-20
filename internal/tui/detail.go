package tui

import (
	"fmt"
	"strings"
)

func (m Model) detailView() string {
	if m.selected == nil {
		return "No session selected"
	}

	sess := m.selected
	var b strings.Builder

	b.WriteString("╔══════════════════════════════════════════════════════════════════╗\n")
	b.WriteString(fmt.Sprintf("║  Session: %-55s ║\n", truncate(sess.Name, 55)))
	b.WriteString("╚══════════════════════════════════════════════════════════════════╝\n\n")

	// Session identity
	b.WriteString(fmt.Sprintf("  CLI:        %s\n", sess.CLI))
	if sess.CWD != "" {
		b.WriteString(fmt.Sprintf("  Directory:  %s\n", sess.CWD))
	}
	if sess.Args != "" {
		b.WriteString(fmt.Sprintf("  Command:    %s\n", sess.Args))
	}
	if sess.ParentApp != "" && sess.ParentApp != "unknown" {
		b.WriteString(fmt.Sprintf("  Terminal:   %s\n", sess.ParentApp))
	}
	if sess.StartTime != "" {
		b.WriteString(fmt.Sprintf("  Started:    %s\n", shortTime(sess.StartTime)))
	}

	b.WriteString("\n")

	// Session state
	b.WriteString(fmt.Sprintf("  Status:     %s\n", formatStatus(sess.Status)))
	b.WriteString(fmt.Sprintf("  Backend:    %s\n", sess.Backend))
	b.WriteString(fmt.Sprintf("  Target:     %s\n", sess.Target))
	if sess.PID != "" {
		b.WriteString(fmt.Sprintf("  PID:        %s\n", sess.PID))
	}
	if sess.Policy != "" {
		b.WriteString(fmt.Sprintf("  Policy:     %s\n", sess.Policy))
	}
	if sess.ResumeMessage != "" {
		b.WriteString(fmt.Sprintf("  Resume:     %q\n", sess.ResumeMessage))
	}

	// Usage window
	if m.windows != nil {
		remaining := m.windows.Remaining(sess.Name)
		if remaining > 0 {
			hours := int(remaining.Hours())
			mins := int(remaining.Minutes()) % 60
			b.WriteString(fmt.Sprintf("  Window:     %dh%02dm remaining\n", hours, mins))
		} else {
			w := m.windows.GetWindow(sess.Name)
			if w != nil {
				b.WriteString("  Window:     expired\n")
			}
		}
	}

	b.WriteString("\n")

	// Recent output (if supported)
	b.WriteString("  ─── Recent Output ─────────────────────────────────────────\n")
	output := m.readSessionOutput(sess.Name)
	if output == "" {
		b.WriteString("  (output capture not available for this terminal)\n")
	} else {
		for _, line := range strings.Split(output, "\n") {
			b.WriteString("  " + line + "\n")
		}
	}

	b.WriteString("\n")

	// Input bar
	if m.input.focused {
		b.WriteString("  Query: " + m.input.value + "█\n")
	} else {
		b.WriteString("  [Tab] Input  [Esc] Back  [q] Quit\n")
	}

	return b.String()
}

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
