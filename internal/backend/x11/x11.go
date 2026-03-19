//go:build linux

package x11

import (
	"fmt"
	"os"
	"os/exec"
	"strings"
	"time"

	"github.com/kickthemoon0817/mother-terminal/pkg"
)

// Backend implements the Injector interface for X11 terminals.
type Backend struct{}

func (b *Backend) Name() pkg.BackendType {
	return pkg.BackendX11
}

func (b *Backend) IsAvailable() bool {
	if os.Getenv("DISPLAY") == "" {
		return false
	}
	_, err := exec.LookPath("xdotool")
	return err == nil
}

func (b *Backend) Discover() ([]pkg.Session, error) {
	// Search for terminal windows using xdotool
	out, err := exec.Command("xdotool", "search", "--name", "").Output()
	if err != nil {
		return nil, nil
	}

	knownCLIs := map[string]pkg.CLIType{
		"claude":   pkg.CLIClaude,
		"codex":    pkg.CLICodex,
		"gemini":   pkg.CLIGemini,
		"opencode": pkg.CLIOpenCode,
	}

	var sessions []pkg.Session
	for _, winID := range strings.Split(strings.TrimSpace(string(out)), "\n") {
		if winID == "" {
			continue
		}

		// Get window name
		nameOut, err := exec.Command("xdotool", "getwindowname", winID).Output()
		if err != nil {
			continue
		}
		winName := strings.TrimSpace(string(nameOut))

		for name, cliType := range knownCLIs {
			if strings.Contains(strings.ToLower(winName), name) {
				sessions = append(sessions, pkg.Session{
					ID:      fmt.Sprintf("x11-%s", winID),
					Name:    fmt.Sprintf("%s-x11-%s", name, winID),
					CLI:     cliType,
					Backend: pkg.BackendX11,
					Target:  winID,
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
	// Focus the window first
	if err := exec.Command("xdotool", "windowactivate", session.Target).Run(); err != nil {
		return fmt.Errorf("%w: xdotool windowactivate %s: %v", pkg.ErrSendKeysFailed, session.Target, err)
	}

	// Type the text
	if err := exec.Command("xdotool", "type", "--clearmodifiers", "--delay", "10", text).Run(); err != nil {
		return fmt.Errorf("%w: xdotool type to %s: %v", pkg.ErrSendKeysFailed, session.Target, err)
	}

	// Press Enter
	if err := exec.Command("xdotool", "key", "Return").Run(); err != nil {
		return fmt.Errorf("%w: xdotool key Return to %s: %v", pkg.ErrSendKeysFailed, session.Target, err)
	}

	return nil
}

func (b *Backend) ReadOutput(session pkg.Session, lines int) (string, error) {
	// X11 has no universal terminal output capture mechanism.
	// We can only detect that the window exists and is responsive.
	// For actual output reading, users should use tmux backend instead.
	return "", fmt.Errorf("%w: X11 backend does not support direct output capture — use tmux backend for output reading", pkg.ErrReadOutputFailed)
}

func (b *Backend) Ping(session pkg.Session) (pkg.PingResult, error) {
	start := time.Now()

	// Check if window still exists
	err := exec.Command("xdotool", "getwindowname", session.Target).Run()
	alive := err == nil

	// Responsiveness: try to get window geometry (proves window is mapped)
	responsive := false
	if alive {
		err = exec.Command("xdotool", "getwindowgeometry", session.Target).Run()
		responsive = err == nil
	}

	return pkg.PingResult{
		Alive:      alive,
		Responsive: responsive,
		Latency:    time.Since(start),
	}, nil
}
