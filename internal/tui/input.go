package tui

import (
	tea "github.com/charmbracelet/bubbletea"
)

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
				im.value = im.value[:len(im.value)-1]
			}
		case "escape":
			im.focused = false
			im.value = ""
		case "enter":
			// Handled by parent
		default:
			// Only add printable characters
			if len(msg.String()) == 1 || msg.String() == "space" {
				if msg.String() == "space" {
					im.value += " "
				} else {
					im.value += msg.String()
				}
			}
		}
	}
	return im, nil
}
