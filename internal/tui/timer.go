package tui

import (
	"fmt"
	"time"

	tea "github.com/charmbracelet/bubbletea"
)

// tickMsg is sent periodically to update timer displays.
type tickMsg time.Time

// tickCmd returns a command that ticks every second for timer updates.
func tickCmd() tea.Cmd {
	return tea.Tick(time.Second, func(t time.Time) tea.Msg {
		return tickMsg(t)
	})
}

// formatDuration formats a duration for display.
func formatDuration(d time.Duration) string {
	if d <= 0 {
		return "expired"
	}

	hours := int(d.Hours())
	mins := int(d.Minutes()) % 60
	secs := int(d.Seconds()) % 60

	if hours > 0 {
		return fmt.Sprintf("%dh%02dm%02ds", hours, mins, secs)
	}
	if mins > 0 {
		return fmt.Sprintf("%dm%02ds", mins, secs)
	}
	return fmt.Sprintf("%ds", secs)
}
