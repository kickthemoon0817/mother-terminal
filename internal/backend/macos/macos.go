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

// Backend implements the Injector interface for macOS native terminals.
type Backend struct{}

func (b *Backend) Name() pkg.BackendType {
	return pkg.BackendMacOS
}

func (b *Backend) IsAvailable() bool {
	_, err := exec.LookPath("osascript")
	return err == nil
}

func (b *Backend) Discover() ([]pkg.Session, error) {
	// Query Terminal.app and iTerm2 for open windows/tabs
	var sessions []pkg.Session

	termSessions, _ := b.discoverTerminalApp()
	sessions = append(sessions, termSessions...)

	itermSessions, _ := b.discoverITerm2()
	sessions = append(sessions, itermSessions...)

	return sessions, nil
}

func (b *Backend) discoverTerminalApp() ([]pkg.Session, error) {
	script := `tell application "System Events" to return (name of every process whose name is "Terminal")`
	out, err := exec.Command("osascript", "-e", script).Output()
	if err != nil || strings.TrimSpace(string(out)) == "" {
		return nil, nil
	}

	// Get Terminal.app window/tab info
	script = `tell application "Terminal"
		set result to ""
		repeat with w from 1 to count of windows
			repeat with t from 1 to count of tabs of window w
				set proc to processes of tab t of window w
				set result to result & w & ":" & t & " " & (item 1 of proc) & linefeed
			end repeat
		end repeat
		return result
	end tell`

	out, err = exec.Command("osascript", "-e", script).Output()
	if err != nil {
		return nil, nil
	}

	return b.parseDiscoveredSessions(string(out), "Terminal"), nil
}

func (b *Backend) discoverITerm2() ([]pkg.Session, error) {
	script := `tell application "System Events" to return (name of every process whose name is "iTerm2")`
	out, err := exec.Command("osascript", "-e", script).Output()
	if err != nil || strings.TrimSpace(string(out)) == "" {
		return nil, nil
	}

	script = `tell application "iTerm2"
		set result to ""
		repeat with w from 1 to count of windows
			repeat with t from 1 to count of tabs of window w
				repeat with s from 1 to count of sessions of tab t of window w
					set proc to name of current session of tab t of window w
					set result to result & w & ":" & t & ":" & s & " " & proc & linefeed
				end repeat
			end repeat
		end repeat
		return result
	end tell`

	out, err = exec.Command("osascript", "-e", script).Output()
	if err != nil {
		return nil, nil
	}

	return b.parseDiscoveredSessions(string(out), "iTerm2"), nil
}

func (b *Backend) parseDiscoveredSessions(output, app string) []pkg.Session {
	var sessions []pkg.Session
	for _, line := range strings.Split(strings.TrimSpace(output), "\n") {
		if line == "" {
			continue
		}
		parts := strings.SplitN(line, " ", 2)
		if len(parts) < 2 {
			continue
		}
		tabID := parts[0]
		proc := parts[1]

		for name, cliType := range pkg.KnownCLIs {
			if strings.Contains(strings.ToLower(proc), name) {
				target := fmt.Sprintf("%s:%s", app, tabID)
				sessions = append(sessions, pkg.Session{
					ID:      fmt.Sprintf("macos-%s-%s", app, tabID),
					Name:    fmt.Sprintf("%s-%s-%s", name, app, tabID),
					CLI:     cliType,
					Backend: pkg.BackendMacOS,
					Target:  target,
					Status:  pkg.StatusDiscovered,
					Policy:  pkg.PolicyNotify,
				})
				break
			}
		}
	}
	return sessions
}

func (b *Backend) SendKeys(session pkg.Session, text string) error {
	parts := strings.SplitN(session.Target, ":", 2)
	if len(parts) < 2 {
		return fmt.Errorf("%w: invalid macos target format %q", pkg.ErrSendKeysFailed, session.Target)
	}
	app := parts[0]

	// Write text to a temp file to avoid AppleScript injection.
	// Never interpolate user text into AppleScript string literals.
	tmpFile, err := os.CreateTemp("", "mother-input-*")
	if err != nil {
		return fmt.Errorf("%w: failed to create temp file: %v", pkg.ErrSendKeysFailed, err)
	}
	defer os.Remove(tmpFile.Name())

	if _, err := tmpFile.WriteString(text + "\n"); err != nil {
		tmpFile.Close()
		return fmt.Errorf("%w: failed to write temp file: %v", pkg.ErrSendKeysFailed, err)
	}
	tmpFile.Close()

	// Use keystroke-based injection via System Events to type into the
	// frontmost window of the target app. This avoids `do script` which
	// opens a new shell, and avoids interpolating text into AppleScript.
	script := fmt.Sprintf(`
		set inputText to (read POSIX file %q)
		-- Remove trailing newline from file read
		if inputText ends with linefeed then
			set inputText to text 1 thru -2 of inputText
		end if
		tell application %q to activate
		delay 0.3
		tell application "System Events"
			keystroke inputText
			keystroke return
		end tell
	`, tmpFile.Name(), app)

	if err := exec.Command("osascript", "-e", script).Run(); err != nil {
		return fmt.Errorf("%w: osascript send to %s: %v", pkg.ErrSendKeysFailed, session.Target, err)
	}
	return nil
}

func (b *Backend) ReadOutput(session pkg.Session, lines int) (string, error) {
	parts := strings.SplitN(session.Target, ":", 2)
	if len(parts) < 2 {
		return "", fmt.Errorf("%w: invalid macos target format", pkg.ErrReadOutputFailed)
	}
	app := parts[0]

	var script string
	switch app {
	case "Terminal":
		script = `tell application "Terminal" to return contents of front window`
	case "iTerm2":
		script = `tell application "iTerm2" to tell current session of current tab of current window to return contents`
	default:
		return "", fmt.Errorf("%w: unsupported app %q", pkg.ErrReadOutputFailed, app)
	}

	out, err := exec.Command("osascript", "-e", script).Output()
	if err != nil {
		return "", fmt.Errorf("%w: osascript read from %s: %v", pkg.ErrReadOutputFailed, app, err)
	}

	// Trim to requested line count
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

func (b *Backend) Ping(session pkg.Session) (pkg.PingResult, error) {
	start := time.Now()

	// Check if the app process is running
	parts := strings.SplitN(session.Target, ":", 2)
	if len(parts) < 2 {
		return pkg.PingResult{}, nil
	}
	app := parts[0]

	script := fmt.Sprintf(`tell application "System Events" to return (name of every process whose name is %q)`, app)
	out, err := exec.Command("osascript", "-e", script).Output()
	alive := err == nil && strings.TrimSpace(string(out)) != ""

	responsive := false
	if alive {
		_, err := b.ReadOutput(session, 1)
		responsive = err == nil
	}

	return pkg.PingResult{
		Alive:      alive,
		Responsive: responsive,
		Latency:    time.Since(start),
	}, nil
}
