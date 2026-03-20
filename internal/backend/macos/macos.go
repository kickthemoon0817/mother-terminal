//go:build darwin

package macos

import (
	"fmt"
	"os"
	"os/exec"
	"strings"
	"time"

	"github.com/kickthemoon0817/mother-terminal/pkg"
)

// Backend implements the Injector interface for macOS.
// Uses process scanning + TTY identification for universal terminal support.
type Backend struct{}

func (b *Backend) Name() pkg.BackendType {
	return pkg.BackendMacOS
}

func (b *Backend) IsAvailable() bool {
	// Available on any macOS — uses ps + direct TTY write
	return true
}

// discoveredProcess holds a process with its TTY and parent app info.
type discoveredProcess struct {
	PID       string
	TTY       string
	CLI       pkg.CLIType
	Command   string
	ParentApp string
}

func (b *Backend) Discover() ([]pkg.Session, error) {
	// Scan for AI CLI processes with their TTYs
	procs, err := b.scanProcesses()
	if err != nil || len(procs) == 0 {
		return nil, nil
	}

	var sessions []pkg.Session
	seen := make(map[string]bool) // dedupe by TTY

	for _, p := range procs {
		if p.TTY == "" || p.TTY == "??" || seen[p.TTY] {
			continue
		}
		seen[p.TTY] = true

		// Verify the TTY device exists and is writable
		ttyPath := "/dev/" + p.TTY
		if _, err := os.Stat(ttyPath); err != nil {
			continue
		}

		// Gather identifying metadata
		cwd, args, startTime := b.getProcessMeta(p.PID)

		// Use CWD basename for a friendlier name
		displayName := fmt.Sprintf("%s-%s", p.CLI, p.TTY)
		if cwd != "" {
			parts := strings.Split(cwd, "/")
			displayName = fmt.Sprintf("%s [%s]", p.CLI, parts[len(parts)-1])
		}

		sessions = append(sessions, pkg.Session{
			ID:        fmt.Sprintf("macos-%s-%s", p.CLI, p.PID),
			Name:      displayName,
			CLI:       p.CLI,
			Backend:   pkg.BackendMacOS,
			Target:    ttyPath,
			Status:    pkg.StatusDiscovered,
			Policy:    pkg.PolicyNotify,
			PID:       p.PID,
			CWD:       cwd,
			Args:      args,
			StartTime: startTime,
			ParentApp: p.ParentApp,
		})
	}

	return sessions, nil
}

// scanProcesses finds AI CLI processes with their TTY and parent app.
func (b *Backend) scanProcesses() ([]discoveredProcess, error) {
	out, err := exec.Command("ps", "-eo", "pid,tty,ppid,comm").Output()
	if err != nil {
		return nil, err
	}

	// Build PID -> comm map for parent lookup
	// comm field can contain spaces (e.g., "Code Helper"), so join fields[3:]
	pidComm := make(map[string]string)
	for _, line := range strings.Split(string(out), "\n") {
		fields := strings.Fields(line)
		if len(fields) < 4 {
			continue
		}
		pidComm[fields[0]] = strings.Join(fields[3:], " ")
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
		comm := strings.Join(fields[3:], " ")

		// Get base command name
		parts := strings.Split(comm, "/")
		baseName := strings.ToLower(parts[len(parts)-1])

		for name, cliType := range pkg.KnownCLIs {
			if baseName == name {
				// Trace parent to find terminal app
				parentApp := b.traceParentApp(ppid, pidComm)

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

// getProcessMeta retrieves CWD, full args, and start time for a process.
func (b *Backend) getProcessMeta(pid string) (cwd, args, startTime string) {
	// Working directory via lsof
	out, err := exec.Command("lsof", "-p", pid).Output()
	if err == nil {
		for _, line := range strings.Split(string(out), "\n") {
			if strings.Contains(line, "cwd") {
				fields := strings.Fields(line)
				if len(fields) > 0 {
					cwd = fields[len(fields)-1]
				}
				break
			}
		}
	}

	// Full command with arguments
	out, err = exec.Command("ps", "-o", "args=", "-p", pid).Output()
	if err == nil {
		args = strings.TrimSpace(string(out))
	}

	// Start time
	out, err = exec.Command("ps", "-o", "lstart=", "-p", pid).Output()
	if err == nil {
		startTime = strings.TrimSpace(string(out))
	}

	return
}

// traceParentApp walks up the process tree to find the terminal app.
func (b *Backend) traceParentApp(ppid string, pidComm map[string]string) string {
	visited := make(map[string]bool)
	current := ppid

	for i := 0; i < 10; i++ { // max depth to prevent infinite loops
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
		case strings.Contains(lower, "visual studio code") || strings.Contains(lower, "code helper"):
			return "VS Code"
		case strings.Contains(lower, "iterm2"):
			return "iTerm2"
		case strings.Contains(lower, "terminal"):
			return "Terminal.app"
		case strings.Contains(lower, "warp"):
			return "Warp"
		case strings.Contains(lower, "kitty"):
			return "Kitty"
		case strings.Contains(lower, "alacritty"):
			return "Alacritty"
		case strings.Contains(lower, "wezterm"):
			return "WezTerm"
		}

		// Get parent's parent
		ppOut, err := exec.Command("ps", "-o", "ppid=", "-p", current).Output()
		if err != nil {
			break
		}
		current = strings.TrimSpace(string(ppOut))
	}

	return "unknown"
}

func (b *Backend) SendKeys(session pkg.Session, text string) error {
	// Strategy 1: If tmux is available and session is in a tmux pane, use send-keys
	if tmuxTarget := b.findTmuxPane(session.PID); tmuxTarget != "" {
		cmd := exec.Command("tmux", "send-keys", "-t", tmuxTarget, text, "Enter")
		if err := cmd.Run(); err == nil {
			return nil
		}
	}

	// Strategy 2: Non-tmux sessions cannot receive input on macOS
	// without Accessibility permissions (macOS blocks osascript keystroke).
	// Suggest the user spawn sessions via /spawn for full control.
	return fmt.Errorf("%w: session %q is not in a tmux pane — use /spawn to start sessions with full input control", pkg.ErrSendKeysFailed, session.Name)
}

// findTmuxPane checks if a process is running inside a tmux pane.
func (b *Backend) findTmuxPane(pid string) string {
	if pid == "" {
		return ""
	}
	if _, err := exec.LookPath("tmux"); err != nil {
		return ""
	}
	// List all tmux panes and match by PID
	out, err := exec.Command("tmux", "list-panes", "-a", "-F", "#{pane_pid} #{session_name}:#{window_index}.#{pane_index}").Output()
	if err != nil {
		return ""
	}
	for _, line := range strings.Split(string(out), "\n") {
		fields := strings.Fields(line)
		if len(fields) >= 2 && fields[0] == pid {
			return fields[1]
		}
	}
	return ""
}

// resolveAppName maps parent app names to AppleScript-compatible names.
func (b *Backend) resolveAppName(app string) string {
	switch app {
	case "VS Code":
		return "Visual Studio Code"
	case "Terminal.app":
		return "Terminal"
	default:
		return app
	}
}

func (b *Backend) ReadOutput(session pkg.Session, lines int) (string, error) {
	// Try to read recent output via the log file if available,
	// or fall back to reading the TTY scrollback where possible.
	// Direct TTY reading is not reliably supported — the TTY device
	// is write-only from our perspective.

	// Attempt AppleScript-based reading for Terminal.app and iTerm2
	// by checking which app owns the session
	ttyPath := session.Target
	app := b.detectAppForTTY(ttyPath)

	var script string
	switch app {
	case "Terminal.app":
		script = `tell application "Terminal" to return contents of front window`
	case "iTerm2":
		script = `tell application "iTerm2" to tell current session of current tab of current window to return contents`
	default:
		return "", fmt.Errorf("%w: output capture not supported for %s terminals — use tmux for output reading", pkg.ErrReadOutputFailed, app)
	}

	out, err := exec.Command("osascript", "-e", script).Output()
	if err != nil {
		return "", fmt.Errorf("%w: osascript read: %v", pkg.ErrReadOutputFailed, err)
	}

	content := string(out)
	if lines > 0 {
		allLines := strings.Split(content, "\n")
		if len(allLines) > lines {
			allLines = allLines[len(allLines)-lines:]
		}
		content = strings.Join(allLines, "\n")
	}

	return content, nil
}

// detectAppForTTY identifies which terminal app owns a TTY.
func (b *Backend) detectAppForTTY(ttyPath string) string {
	ttyName := strings.TrimPrefix(ttyPath, "/dev/")

	// Find the shell process on this TTY
	out, err := exec.Command("ps", "-eo", "pid,tty,ppid").Output()
	if err != nil {
		return "unknown"
	}

	pidComm := make(map[string]string)
	commOut, _ := exec.Command("ps", "-eo", "pid,comm").Output()
	for _, line := range strings.Split(string(commOut), "\n") {
		fields := strings.Fields(line)
		if len(fields) >= 2 {
			pidComm[fields[0]] = strings.Join(fields[1:], " ")
		}
	}

	for _, line := range strings.Split(string(out), "\n") {
		fields := strings.Fields(line)
		if len(fields) < 3 {
			continue
		}
		if fields[1] == ttyName {
			return b.traceParentApp(fields[2], pidComm)
		}
	}

	return "unknown"
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

	// Check if we can open the TTY for writing (responsive)
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
