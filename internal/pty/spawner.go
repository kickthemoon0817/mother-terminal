package pty

import (
	"fmt"
	"io"
	"os"
	"os/exec"
	"strings"
	"sync"
	"time"

	"github.com/creack/pty"
	"github.com/kickthemoon0817/mother-terminal/pkg"
)

// ManagedPTY represents a child process running in a pseudo-terminal.
type ManagedPTY struct {
	mu     sync.Mutex
	cmd    *exec.Cmd
	ptmx   *os.File
	output []byte
	closed bool
}

// Backend implements the Injector interface for Mother-spawned PTY sessions.
type Backend struct {
	mu       sync.RWMutex
	sessions map[string]*ManagedPTY
}

// NewBackend creates a new PTY backend.
func NewBackend() *Backend {
	return &Backend{
		sessions: make(map[string]*ManagedPTY),
	}
}

func (b *Backend) Name() pkg.BackendType {
	return pkg.BackendPTY
}

func (b *Backend) IsAvailable() bool {
	return true // PTY is always available on Unix systems
}

// Spawn creates a new child process in a pseudo-terminal.
func (b *Backend) Spawn(name string, command string, args ...string) (*pkg.Session, error) {
	cmd := exec.Command(command, args...)
	ptmx, err := pty.Start(cmd)
	if err != nil {
		return nil, fmt.Errorf("failed to spawn PTY for %s: %w", command, err)
	}

	managed := &ManagedPTY{
		cmd:  cmd,
		ptmx: ptmx,
	}

	// Read output in background
	go func() {
		buf := make([]byte, 4096)
		for {
			n, err := ptmx.Read(buf)
			if n > 0 {
				managed.mu.Lock()
				managed.output = append(managed.output, buf[:n]...)
				// Keep only last 64KB of output
				if len(managed.output) > 65536 {
					managed.output = managed.output[len(managed.output)-65536:]
				}
				managed.mu.Unlock()
			}
			if err != nil {
				if err != io.EOF {
					// PTY closed
				}
				return
			}
		}
	}()

	// Determine CLI type from command
	cliType := pkg.CLIType("unknown")
	cmdLower := strings.ToLower(command)
	switch {
	case strings.Contains(cmdLower, "claude"):
		cliType = pkg.CLIClaude
	case strings.Contains(cmdLower, "codex"):
		cliType = pkg.CLICodex
	case strings.Contains(cmdLower, "gemini"):
		cliType = pkg.CLIGemini
	case strings.Contains(cmdLower, "opencode"):
		cliType = pkg.CLIOpenCode
	}

	session := &pkg.Session{
		ID:      fmt.Sprintf("pty-%s-%d", name, cmd.Process.Pid),
		Name:    name,
		CLI:     cliType,
		Backend: pkg.BackendPTY,
		Target:  fmt.Sprintf("%d", cmd.Process.Pid),
		Status:  pkg.StatusActive,
		Policy:  pkg.PolicyNotify,
	}

	b.mu.Lock()
	b.sessions[session.ID] = managed
	b.mu.Unlock()

	return session, nil
}

func (b *Backend) Discover() ([]pkg.Session, error) {
	b.mu.RLock()
	defer b.mu.RUnlock()

	var sessions []pkg.Session
	for id, m := range b.sessions {
		m.mu.Lock()
		closed := m.closed
		m.mu.Unlock()

		if !closed {
			sessions = append(sessions, pkg.Session{
				ID:      id,
				Backend: pkg.BackendPTY,
				Status:  pkg.StatusActive,
			})
		}
	}
	return sessions, nil
}

func (b *Backend) SendKeys(session pkg.Session, text string) error {
	b.mu.RLock()
	managed, ok := b.sessions[session.ID]
	b.mu.RUnlock()

	if !ok {
		return fmt.Errorf("%w: PTY session %s", pkg.ErrSessionNotFound, session.ID)
	}

	managed.mu.Lock()
	defer managed.mu.Unlock()

	if managed.closed {
		return fmt.Errorf("%w: PTY session %s is closed", pkg.ErrSendKeysFailed, session.ID)
	}

	// Write text + newline to the PTY
	_, err := managed.ptmx.WriteString(text + "\n")
	if err != nil {
		return fmt.Errorf("%w: write to PTY %s: %v", pkg.ErrSendKeysFailed, session.ID, err)
	}

	return nil
}

func (b *Backend) ReadOutput(session pkg.Session, lines int) (string, error) {
	b.mu.RLock()
	managed, ok := b.sessions[session.ID]
	b.mu.RUnlock()

	if !ok {
		return "", fmt.Errorf("%w: PTY session %s", pkg.ErrSessionNotFound, session.ID)
	}

	managed.mu.Lock()
	output := string(managed.output)
	managed.mu.Unlock()

	if lines > 0 {
		allLines := strings.Split(output, "\n")
		if len(allLines) > lines {
			allLines = allLines[len(allLines)-lines:]
		}
		output = strings.Join(allLines, "\n")
	}

	return output, nil
}

func (b *Backend) Ping(session pkg.Session) (pkg.PingResult, error) {
	start := time.Now()

	b.mu.RLock()
	managed, ok := b.sessions[session.ID]
	b.mu.RUnlock()

	if !ok {
		return pkg.PingResult{Alive: false}, nil
	}

	managed.mu.Lock()
	closed := managed.closed
	managed.mu.Unlock()

	alive := !closed && managed.cmd.ProcessState == nil

	return pkg.PingResult{
		Alive:      alive,
		Responsive: alive,
		Latency:    time.Since(start),
	}, nil
}

// Close terminates a PTY session.
func (b *Backend) Close(sessionID string) error {
	b.mu.Lock()
	managed, ok := b.sessions[sessionID]
	if !ok {
		b.mu.Unlock()
		return fmt.Errorf("%w: PTY session %s", pkg.ErrSessionNotFound, sessionID)
	}
	b.mu.Unlock()

	managed.mu.Lock()
	defer managed.mu.Unlock()

	if managed.closed {
		return nil
	}

	managed.closed = true
	managed.ptmx.Close()

	if managed.cmd.Process != nil {
		managed.cmd.Process.Kill()
		managed.cmd.Wait()
	}

	return nil
}
