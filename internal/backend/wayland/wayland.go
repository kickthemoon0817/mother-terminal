//go:build linux

package wayland

import (
	"fmt"
	"os"
	"os/exec"
	"strings"
	"time"

	"github.com/kickthemoon0817/mother-terminal/pkg"
)

// Backend implements the Injector interface for Wayland terminals.
type Backend struct{}

func (b *Backend) Name() pkg.BackendType {
	return pkg.BackendWayland
}

func (b *Backend) IsAvailable() bool {
	// Check for Wayland session
	if os.Getenv("WAYLAND_DISPLAY") == "" {
		return false
	}
	_, err := exec.LookPath("ydotool")
	return err == nil
}

func (b *Backend) Discover() ([]pkg.Session, error) {
	// Wayland has no universal window enumeration API.
	// Fall back to process scanning — discovery/scanner.go handles this.
	// We can only verify sessions that were registered manually or found by process scan.
	knownCLIs := map[string]pkg.CLIType{
		"claude":   pkg.CLIClaude,
		"codex":    pkg.CLICodex,
		"gemini":   pkg.CLIGemini,
		"opencode": pkg.CLIOpenCode,
	}

	var sessions []pkg.Session

	// Scan /proc for known CLI processes
	out, err := exec.Command("ps", "-eo", "pid,comm").Output()
	if err != nil {
		return nil, nil
	}

	for _, line := range strings.Split(string(out), "\n") {
		fields := strings.Fields(line)
		if len(fields) < 2 {
			continue
		}
		pid := fields[0]
		comm := fields[1]

		for name, cliType := range knownCLIs {
			if strings.Contains(strings.ToLower(comm), name) {
				sessions = append(sessions, pkg.Session{
					ID:      fmt.Sprintf("wayland-pid-%s", pid),
					Name:    fmt.Sprintf("%s-wayland-%s", name, pid),
					CLI:     cliType,
					Backend: pkg.BackendWayland,
					Target:  pid,
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
	// ydotool types text globally to the focused window.
	// The caller must ensure the correct window is focused.
	if err := exec.Command("ydotool", "type", "--key-delay", "10", text).Run(); err != nil {
		return fmt.Errorf("%w: ydotool type: %v", pkg.ErrSendKeysFailed, err)
	}

	// Press Enter (key code 28 = Enter in ydotool)
	if err := exec.Command("ydotool", "key", "28:1", "28:0").Run(); err != nil {
		return fmt.Errorf("%w: ydotool key Enter: %v", pkg.ErrSendKeysFailed, err)
	}

	return nil
}

func (b *Backend) ReadOutput(session pkg.Session, lines int) (string, error) {
	// Wayland has no universal terminal output capture mechanism.
	// This is a known limitation of the Wayland protocol — compositors
	// do not expose window content to other applications.
	return "", fmt.Errorf("%w: Wayland backend does not support output capture — use tmux backend for output reading", pkg.ErrReadOutputFailed)
}

func (b *Backend) Ping(session pkg.Session) (pkg.PingResult, error) {
	start := time.Now()

	// Check if the process is still alive
	alive := false
	if session.Target != "" {
		err := exec.Command("kill", "-0", session.Target).Run()
		alive = err == nil
	}

	return pkg.PingResult{
		Alive:      alive,
		Responsive: alive, // Best effort — we can't verify terminal responsiveness on Wayland
		Latency:    time.Since(start),
	}, nil
}
