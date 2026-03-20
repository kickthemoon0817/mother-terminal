package tui

import (
	"os"
	"path/filepath"
	"strings"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

// ── Slash commands registry ──────────────────────────────────────────────────

var slashCommands = []slashCmd{
	{"/spawn", "spawn <cli> — start an AI CLI session"},
	{"/history", "history — view session history"},
	{"/history search", "history search <query> — search all history"},
	{"/connect", "connect <user@host> — register remote host"},
	{"/discover", "discover <host> — find remote sessions"},
	{"/ping", "ping <host> — check remote host"},
	{"/hosts", "hosts — list remote hosts"},
	{"/refresh", "refresh — re-scan sessions"},
	{"/sessions", "sessions — show dashboard"},
	{"/back", "back — return to dashboard"},
	{"/quit", "quit — exit mtt"},
}

type slashCmd struct {
	command string
	desc    string
}

// ── InputModel ────────────────────────────────────────────────────────────────

// InputModel handles text input in the TUI.
type InputModel struct {
	value      string
	focused    bool
	suggestion string // current autocomplete suggestion
	dirMode    bool   // true when expecting a directory path
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
		case "tab", "\t":
			// Accept autocomplete suggestion
			if im.suggestion != "" {
				if im.dirMode {
					im.value = im.suggestion + "/"
				} else {
					im.value = im.suggestion + " "
				}
				// Recompute suggestion for the new value
				if im.dirMode {
					im.suggestion = im.getDirSuggestion()
				} else {
					im.suggestion = im.getSuggestion()
				}
				return im, nil
			}
		case "esc":
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

	// Update autocomplete suggestion
	if im.dirMode {
		im.suggestion = im.getDirSuggestion()
	} else {
		im.suggestion = im.getSuggestion()
	}

	return im, nil
}

// Known CLI names for autocomplete.
var knownCLINames = []string{"claude", "codex", "gemini", "opencode"}

// getSuggestion finds the best matching slash command or argument.
func (im InputModel) getSuggestion() string {
	val := im.value
	if val == "" || !strings.HasPrefix(val, "/") {
		return ""
	}

	// Check if we're typing a CLI name after /spawn
	if strings.HasPrefix(val, "/spawn ") {
		arg := strings.TrimPrefix(val, "/spawn ")
		if arg != "" && !strings.Contains(arg, " ") {
			for _, cli := range knownCLINames {
				if strings.HasPrefix(cli, strings.ToLower(arg)) && cli != arg {
					return "/spawn " + cli
				}
			}
		}
		return ""
	}

	// Don't suggest if already a complete command with args
	for _, cmd := range slashCommands {
		if val == cmd.command+" " || strings.HasPrefix(val, cmd.command+" ") {
			return ""
		}
	}

	// Find first matching command
	for _, cmd := range slashCommands {
		if strings.HasPrefix(cmd.command, val) && cmd.command != val {
			return cmd.command
		}
	}
	return ""
}

// getDirSuggestion finds the best matching directory path for current input.
func (im InputModel) getDirSuggestion() string {
	val := im.value
	if val == "" {
		return ""
	}

	// Expand ~
	expanded := val
	if strings.HasPrefix(expanded, "~/") {
		home, _ := os.UserHomeDir()
		expanded = filepath.Join(home, expanded[2:])
	} else if expanded == "~" {
		home, _ := os.UserHomeDir()
		expanded = home + "/"
	}

	// Get the directory and prefix to match
	dir := filepath.Dir(expanded)
	prefix := filepath.Base(expanded)

	// If input ends with /, list contents of that directory
	if strings.HasSuffix(val, "/") {
		dir = expanded
		prefix = ""
	}

	entries, err := os.ReadDir(dir)
	if err != nil {
		return ""
	}

	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		name := entry.Name()
		if strings.HasPrefix(strings.ToLower(name), strings.ToLower(prefix)) && name != prefix {
			// Build the full suggestion using original input prefix
			if strings.HasSuffix(val, "/") {
				return val + name
			}
			dirPart := filepath.Dir(val)
			if dirPart == "." {
				return name
			}
			return dirPart + "/" + name
		}
	}

	return ""
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
