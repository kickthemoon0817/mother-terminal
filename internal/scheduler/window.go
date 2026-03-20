package scheduler

import (
	"encoding/json"
	"os"
	"path/filepath"
	"sync"
	"time"

	"github.com/kickthemoon0817/mother-terminal/pkg"
)

// WindowTracker tracks usage windows per session.
type WindowTracker struct {
	mu        sync.RWMutex
	windows   map[string]*pkg.UsageWindow
	limits    map[pkg.CLIType]time.Duration
	statePath string
}

// NewWindowTracker creates a new usage window tracker.
func NewWindowTracker(limits map[pkg.CLIType]time.Duration, stateDir string) *WindowTracker {
	wt := &WindowTracker{
		windows:   make(map[string]*pkg.UsageWindow),
		limits:    limits,
		statePath: filepath.Join(stateDir, "state.json"),
	}
	wt.loadState()
	return wt
}

// StartWindow creates and activates a usage window for a session.
func (wt *WindowTracker) StartWindow(sessionName string, cli pkg.CLIType) {
	wt.mu.Lock()
	defer wt.mu.Unlock()

	duration, ok := wt.limits[cli]
	if !ok || duration == 0 {
		return // No limit tracking for this CLI
	}

	now := time.Now()
	wt.windows[sessionName] = &pkg.UsageWindow{
		SessionName: sessionName,
		CLI:         cli,
		StartedAt:   now,
		Duration:    duration,
		ExpiresAt:   now.Add(duration),
		Active:      true,
	}

	wt.saveState()
}

// GetWindow returns a copy of the usage window for a session.
// Expiry is computed on read without mutating stored state.
func (wt *WindowTracker) GetWindow(sessionName string) *pkg.UsageWindow {
	wt.mu.RLock()
	defer wt.mu.RUnlock()

	w, ok := wt.windows[sessionName]
	if !ok {
		return nil
	}

	// Return a copy with computed expiry — no mutation under RLock
	copy := *w
	if copy.Active && time.Now().After(copy.ExpiresAt) {
		copy.Active = false
	}
	return &copy
}

// GetAllWindows returns copies of all tracked windows.
// Expiry is computed on read without mutating stored state.
func (wt *WindowTracker) GetAllWindows() []pkg.UsageWindow {
	wt.mu.RLock()
	defer wt.mu.RUnlock()

	now := time.Now()
	result := make([]pkg.UsageWindow, 0, len(wt.windows))
	for _, w := range wt.windows {
		copy := *w
		if copy.Active && now.After(copy.ExpiresAt) {
			copy.Active = false
		}
		result = append(result, copy)
	}
	return result
}

// Remaining returns remaining time for a session's window, or 0.
func (wt *WindowTracker) Remaining(sessionName string) time.Duration {
	w := wt.GetWindow(sessionName)
	if w == nil {
		return 0
	}
	return w.Remaining()
}

func (wt *WindowTracker) loadState() {
	data, err := os.ReadFile(wt.statePath)
	if err != nil {
		return
	}

	var windows map[string]*pkg.UsageWindow
	if err := json.Unmarshal(data, &windows); err != nil {
		return
	}

	wt.windows = windows
}

func (wt *WindowTracker) saveState() {
	dir := filepath.Dir(wt.statePath)
	os.MkdirAll(dir, 0700)

	data, err := json.MarshalIndent(wt.windows, "", "  ")
	if err != nil {
		return
	}

	os.WriteFile(wt.statePath, data, 0600)
}
