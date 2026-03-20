package discovery

import (
	"fmt"
	"os/exec"
	"runtime"
	"strings"

	"github.com/kickthemoon0817/mother-terminal/pkg"
)

// ProcessEntry represents a discovered process.
type ProcessEntry struct {
	PID     string
	Command string
	CLI     pkg.CLIType
}


// Scanner scans running processes for known AI CLI binaries.
type Scanner struct{}

// NewScanner creates a new process scanner.
func NewScanner() *Scanner {
	return &Scanner{}
}

// Scan discovers running AI CLI processes.
func (s *Scanner) Scan() ([]ProcessEntry, error) {
	switch runtime.GOOS {
	case "darwin":
		return s.scanDarwin()
	case "linux":
		return s.scanLinux()
	default:
		return nil, fmt.Errorf("unsupported OS: %s", runtime.GOOS)
	}
}

func (s *Scanner) scanDarwin() ([]ProcessEntry, error) {
	out, err := exec.Command("ps", "-eo", "pid,comm").Output()
	if err != nil {
		return nil, fmt.Errorf("ps command failed: %w", err)
	}
	return s.parsePS(string(out)), nil
}

func (s *Scanner) scanLinux() ([]ProcessEntry, error) {
	out, err := exec.Command("ps", "-eo", "pid,comm").Output()
	if err != nil {
		return nil, fmt.Errorf("ps command failed: %w", err)
	}
	return s.parsePS(string(out)), nil
}

func (s *Scanner) parsePS(output string) []ProcessEntry {
	var entries []ProcessEntry
	for _, line := range strings.Split(output, "\n") {
		fields := strings.Fields(line)
		if len(fields) < 2 {
			continue
		}
		pid := fields[0]
		comm := fields[1]

		// Get the base command name
		parts := strings.Split(comm, "/")
		baseName := strings.ToLower(parts[len(parts)-1])

		for name, cliType := range pkg.KnownCLIs {
			if strings.Contains(baseName, name) {
				entries = append(entries, ProcessEntry{
					PID:     pid,
					Command: comm,
					CLI:     cliType,
				})
				break
			}
		}
	}
	return entries
}
