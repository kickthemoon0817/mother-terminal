package tui

import (
	"strings"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

// ── InputModel ────────────────────────────────────────────────────────────────

// InputModel handles text input in the TUI.
type InputModel struct {
	value   string
	focused bool
}

// NewInputModel creates a new input model.
func NewInputModel() InputModel {
	return InputModel{}
}

// Update handles key events for the input.
func (im InputModel) Update(msg tea.Msg) (InputModel, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyMsg:
		switch msg.String() {
		case "backspace":
			if len(im.value) > 0 {
				r := []rune(im.value)
				im.value = string(r[:len(r)-1])
			}
		case "escape":
			im.focused = false
			im.value = ""
		case "enter":
			// Handled by parent model
		default:
			s := msg.String()
			if s == "space" {
				im.value += " "
			} else if len([]rune(s)) == 1 {
				im.value += s
			}
		}
	}
	return im, nil
}

// ── Shared bar renderers ──────────────────────────────────────────────────────

// helpItem is a key + description pair for the help bar.
type helpItem struct {
	key  string
	desc string
}

// renderHelpBar renders a horizontal key-binding hint strip.
func renderHelpBar(items []helpItem, w int) string {
	var parts []string
	for _, item := range items {
		key := lipgloss.NewStyle().
			Foreground(colorFgBold).
			Bold(true).
			Render(item.key)
		desc := lipgloss.NewStyle().
			Foreground(colorMuted).
			Render(" " + item.desc)
		parts = append(parts, key+desc)
	}

	sep := lipgloss.NewStyle().Foreground(colorBorder).Render("  ·  ")
	content := strings.Join(parts, sep)

	return lipgloss.NewStyle().
		BorderStyle(lipgloss.ThickBorder()).
		BorderTop(true).
		BorderForeground(colorBorder).
		Width(w).
		Padding(0, 1).
		Render(content)
}

// renderInputBar renders the active query input bar.
func renderInputBar(value string, w int) string {
	prompt := lipgloss.NewStyle().
		Foreground(colorInputFg).
		Bold(true).
		Render("query")

	colon := lipgloss.NewStyle().
		Foreground(colorBorder).
		Render("  ›  ")

	cursor := lipgloss.NewStyle().
		Foreground(colorActive).
		Bold(true).
		Render("█")

	text := lipgloss.NewStyle().
		Foreground(colorFg).
		Render(value)

	inner := prompt + colon + text + cursor

	return lipgloss.NewStyle().
		BorderStyle(lipgloss.ThickBorder()).
		BorderTop(true).
		BorderForeground(colorInputFg).
		Background(colorInputBg).
		Width(w).
		Padding(0, 1).
		Render(inner)
}
