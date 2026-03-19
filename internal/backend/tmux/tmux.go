package tmux

import (
	"fmt"
	"os/exec"
	"strings"
	"time"

	"github.com/kickthemoon0817/mother-terminal/pkg"
)

// Backend implements the Injector interface for tmux.
type Backend struct{}

func (b *Backend) Name() pkg.BackendType {
	return pkg.BackendTmux
}

func (b *Backend) IsAvailable() bool {
	_, err := exec.LookPath("tmux")
	return err == nil
}

func (b *Backend) Discover() ([]pkg.Session, error) {
	out, err := exec.Command("tmux", "list-panes", "-a", "-F", "#{session_name}:#{window_index}.#{pane_index} #{pane_pid} #{pane_current_command}").Output()
	if err != nil {
		return nil, nil // tmux not running is not an error
	}

	var sessions []pkg.Session
	knownCLIs := map[string]pkg.CLIType{
		"claude":   pkg.CLIClaude,
		"codex":    pkg.CLICodex,
		"gemini":   pkg.CLIGemini,
		"opencode": pkg.CLIOpenCode,
	}

	for _, line := range strings.Split(strings.TrimSpace(string(out)), "\n") {
		if line == "" {
			continue
		}
		parts := strings.Fields(line)
		if len(parts) < 3 {
			continue
		}
		paneID := parts[0]
		cmd := parts[2]

		for name, cliType := range knownCLIs {
			if strings.Contains(strings.ToLower(cmd), name) {
				sessions = append(sessions, pkg.Session{
					ID:      fmt.Sprintf("tmux-%s", paneID),
					Name:    fmt.Sprintf("%s-%s", name, paneID),
					CLI:     cliType,
					Backend: pkg.BackendTmux,
					Target:  paneID,
					Status:  pkg.StatusDiscovered,
					Policy:  pkg.PolicyNotify,
				})
				break
			}
		}
	}

	return sessions, nil
}

func (b *Backend) SendKeys(session pkg.Session, text string) error {
	// Use tmux send-keys to inject the text, then press Enter
	cmd := exec.Command("tmux", "send-keys", "-t", session.Target, text, "Enter")
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("%w: tmux send-keys to %s: %v", pkg.ErrSendKeysFailed, session.Target, err)
	}
	return nil
}

func (b *Backend) ReadOutput(session pkg.Session, lines int) (string, error) {
	if lines <= 0 {
		lines = 50
	}
	cmd := exec.Command("tmux", "capture-pane", "-t", session.Target, "-p", "-S", fmt.Sprintf("-%d", lines))
	out, err := cmd.Output()
	if err != nil {
		return "", fmt.Errorf("%w: tmux capture-pane from %s: %v", pkg.ErrReadOutputFailed, session.Target, err)
	}
	return string(out), nil
}

func (b *Backend) Ping(session pkg.Session) (pkg.PingResult, error) {
	start := time.Now()

	// Liveness: check if the pane exists and has a running process
	cmd := exec.Command("tmux", "list-panes", "-t", session.Target, "-F", "#{pane_pid}")
	out, err := cmd.Output()
	if err != nil {
		return pkg.PingResult{Alive: false}, nil
	}

	pid := strings.TrimSpace(string(out))
	alive := pid != ""

	// Responsiveness: try to capture the pane (if this works, the terminal is responsive)
	responsive := false
	if alive {
		cmd = exec.Command("tmux", "capture-pane", "-t", session.Target, "-p", "-S", "-1")
		if err := cmd.Run(); err == nil {
			responsive = true
		}
	}

	return pkg.PingResult{
		Alive:      alive,
		Responsive: responsive,
		Latency:    time.Since(start),
	}, nil
}
