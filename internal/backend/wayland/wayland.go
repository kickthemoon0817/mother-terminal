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

// Backend implements the Injector interface for Linux Wayland terminals.
// Uses process scanning + TTY identification for universal terminal support.
// ydotool is NOT required — direct TTY write bypasses the display protocol.
type Backend struct{}

func (b *Backend) Name() pkg.BackendType {
	return pkg.BackendWayland
}

func (b *Backend) IsAvailable() bool {
	// Available on any Linux with Wayland display
	return os.Getenv("WAYLAND_DISPLAY") != ""
}

func (b *Backend) Discover() ([]pkg.Session, error) {
	procs, err := b.scanProcesses()
	if err != nil || len(procs) == 0 {
		return nil, nil
	}

	var sessions []pkg.Session
	seen := make(map[string]bool)

	for _, p := range procs {
		if p.TTY == "" || p.TTY == "?" || seen[p.TTY] {
			continue
		}
		seen[p.TTY] = true

		// On Linux, TTYs from ps are like "pts/1" — map to /dev/pts/1
		ttyPath := "/dev/" + p.TTY
		if _, err := os.Stat(ttyPath); err != nil {
			continue
		}

		sessions = append(sessions, pkg.Session{
			ID:      fmt.Sprintf("wayland-%s-%s", p.CLI, p.PID),
			Name:    fmt.Sprintf("%s-%s", p.CLI, p.TTY),
			CLI:     p.CLI,
			Backend: pkg.BackendWayland,
			Target:  ttyPath, // e.g., /dev/pts/1
			Status:  pkg.StatusDiscovered,
			Policy:  pkg.PolicyNotify,
		})
	}

	return sessions, nil
}

type discoveredProcess struct {
	PID       string
	TTY       string
	CLI       pkg.CLIType
	Command   string
	ParentApp string
}

func (b *Backend) scanProcesses() ([]discoveredProcess, error) {
	out, err := exec.Command("ps", "-eo", "pid,tty,ppid,comm").Output()
	if err != nil {
		return nil, err
	}

	pidComm := make(map[string]string)
	for _, line := range strings.Split(string(out), "\n") {
		fields := strings.Fields(line)
		if len(fields) >= 4 {
			pidComm[fields[0]] = fields[3]
		}
	}

	var procs []discoveredProcess
	for _, line := range strings.Split(string(out), "\n") {
		fields := strings.Fields(line)
		if len(fields) < 4 {
			continue
		}
		pid := fields[0]
		tty := fields[1]
		ppid := fields[2]
		comm := fields[3]

		parts := strings.Split(comm, "/")
		baseName := strings.ToLower(parts[len(parts)-1])

		for name, cliType := range pkg.KnownCLIs {
			if baseName == name {
				parentApp := traceParentApp(ppid, pidComm)
				procs = append(procs, discoveredProcess{
					PID:       pid,
					TTY:       tty,
					CLI:       cliType,
					Command:   comm,
					ParentApp: parentApp,
				})
				break
			}
		}
	}

	return procs, nil
}

// traceParentApp walks up the process tree to find the terminal app.
func traceParentApp(ppid string, pidComm map[string]string) string {
	visited := make(map[string]bool)
	current := ppid

	for i := 0; i < 10; i++ {
		if current == "" || current == "0" || current == "1" || visited[current] {
			break
		}
		visited[current] = true

		comm, ok := pidComm[current]
		if !ok {
			break
		}

		lower := strings.ToLower(comm)
		switch {
		case strings.Contains(lower, "gnome-terminal"):
			return "GNOME Terminal"
		case strings.Contains(lower, "konsole"):
			return "Konsole"
		case strings.Contains(lower, "alacritty"):
			return "Alacritty"
		case strings.Contains(lower, "kitty"):
			return "Kitty"
		case strings.Contains(lower, "wezterm"):
			return "WezTerm"
		case strings.Contains(lower, "foot"):
			return "Foot"
		case strings.Contains(lower, "code") && strings.Contains(lower, "helper"):
			return "VS Code"
		case strings.Contains(lower, "tilix"):
			return "Tilix"
		}

		// Read parent's parent from /proc
		ppidBytes, err := os.ReadFile(fmt.Sprintf("/proc/%s/stat", current))
		if err != nil {
			break
		}
		statFields := strings.Fields(string(ppidBytes))
		if len(statFields) < 4 {
			break
		}
		current = statFields[3]
	}

	return "unknown"
}

func (b *Backend) SendKeys(session pkg.Session, text string) error {
	ttyPath := session.Target
	if !strings.HasPrefix(ttyPath, "/dev/") {
		return fmt.Errorf("%w: invalid TTY target %q", pkg.ErrSendKeysFailed, ttyPath)
	}

	// Write directly to the TTY device — bypasses Wayland protocol entirely.
	// No ydotool needed, no window focus issues.
	f, err := os.OpenFile(ttyPath, os.O_WRONLY, 0)
	if err != nil {
		return fmt.Errorf("%w: cannot open TTY %s: %v", pkg.ErrSendKeysFailed, ttyPath, err)
	}
	defer f.Close()

	_, err = f.WriteString(text + "\n")
	if err != nil {
		return fmt.Errorf("%w: write to TTY %s: %v", pkg.ErrSendKeysFailed, ttyPath, err)
	}

	return nil
}

func (b *Backend) ReadOutput(session pkg.Session, lines int) (string, error) {
	// Wayland protocol does not expose window content to other applications.
	// Direct TTY reading is not supported from outside the session.
	// For reliable output reading, use the tmux backend.
	return "", fmt.Errorf("%w: Wayland backend does not support output capture — use tmux for output reading", pkg.ErrReadOutputFailed)
}

func (b *Backend) Ping(session pkg.Session) (pkg.PingResult, error) {
	start := time.Now()

	ttyPath := session.Target
	if !strings.HasPrefix(ttyPath, "/dev/") {
		return pkg.PingResult{}, nil
	}

	// Check if the TTY device still exists
	info, err := os.Stat(ttyPath)
	alive := err == nil && info.Mode()&os.ModeCharDevice != 0

	// Check if we can open the TTY for writing
	responsive := false
	if alive {
		f, err := os.OpenFile(ttyPath, os.O_WRONLY, 0)
		if err == nil {
			responsive = true
			f.Close()
		}
	}

	return pkg.PingResult{
		Alive:      alive,
		Responsive: responsive,
		Latency:    time.Since(start),
	}, nil
}
