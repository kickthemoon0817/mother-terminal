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

	b.WriteString(fmt.Sprintf("╔══════════════════════════════════════════════════════════╗\n"))
	b.WriteString(fmt.Sprintf("║  Session: %-47s ║\n", sess.Name))
	b.WriteString(fmt.Sprintf("╚══════════════════════════════════════════════════════════╝\n\n"))

	// Metadata
	b.WriteString(fmt.Sprintf("  CLI:      %s\n", sess.CLI))
	b.WriteString(fmt.Sprintf("  Backend:  %s\n", sess.Backend))
	b.WriteString(fmt.Sprintf("  Target:   %s\n", sess.Target))
	b.WriteString(fmt.Sprintf("  Status:   %s\n", formatStatus(sess.Status)))
	b.WriteString(fmt.Sprintf("  Policy:   %s\n", sess.Policy))

	if sess.ResumeMessage != "" {
		b.WriteString(fmt.Sprintf("  Resume:   %q\n", sess.ResumeMessage))
	}

	// Usage window
	if m.windows != nil {
		remaining := m.windows.Remaining(sess.Name)
		if remaining > 0 {
			hours := int(remaining.Hours())
			mins := int(remaining.Minutes()) % 60
			b.WriteString(fmt.Sprintf("  Window:   %dh%02dm remaining\n", hours, mins))
		} else {
			w := m.windows.GetWindow(sess.Name)
			if w != nil {
				b.WriteString("  Window:   expired\n")
			} else {
				b.WriteString("  Window:   not tracked\n")
			}
		}
	}

	b.WriteString("\n")

	// Recent output
	b.WriteString("  ─── Recent Output ─────────────────────────────────────\n")
	output := m.readSessionOutput(sess.Name)
	if output == "" {
		b.WriteString("  (no output captured)\n")
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
		return "(output capture not supported for this backend)"
	}

	return output
}
