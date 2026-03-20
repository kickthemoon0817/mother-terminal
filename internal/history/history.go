package history

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/kickthemoon0817/mother-terminal/internal/backend"
	"github.com/kickthemoon0817/mother-terminal/pkg"
)

// Recorder periodically captures tmux pane output and appends to log files.
type Recorder struct {
	mu       sync.Mutex
	registry *backend.Registry
	baseDir  string
	interval time.Duration
	stops    map[string]chan struct{}
	lastSnap map[string]string // last captured output per session
}

// NewRecorder creates a new history recorder.
func NewRecorder(registry *backend.Registry, baseDir string, interval time.Duration) *Recorder {
	if interval == 0 {
		interval = 5 * time.Second
	}
	os.MkdirAll(baseDir, 0700)
	return &Recorder{
		registry: registry,
		baseDir:  baseDir,
		interval: interval,
		stops:    make(map[string]chan struct{}),
		lastSnap: make(map[string]string),
	}
}

// Record starts capturing output for a session.
func (r *Recorder) Record(sess pkg.Session) {
	r.mu.Lock()
	if _, exists := r.stops[sess.Name]; exists {
		r.mu.Unlock()
		return
	}
	stop := make(chan struct{})
	r.stops[sess.Name] = stop
	r.mu.Unlock()

	go r.captureLoop(sess, stop)
}

// StopRecording stops capturing for a session.
func (r *Recorder) StopRecording(name string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if stop, ok := r.stops[name]; ok {
		close(stop)
		delete(r.stops, name)
	}
}

// StopAll stops all recording.
func (r *Recorder) StopAll() {
	r.mu.Lock()
	defer r.mu.Unlock()
	for name, stop := range r.stops {
		close(stop)
		delete(r.stops, name)
	}
}

// GetHistory returns the stored history for a session.
func (r *Recorder) GetHistory(sessionName string, lines int) (string, error) {
	logPath := r.logPath(sessionName)
	data, err := os.ReadFile(logPath)
	if err != nil {
		return "", fmt.Errorf("no history for session %q", sessionName)
	}

	content := string(data)
	if lines > 0 {
		allLines := strings.Split(content, "\n")
		if len(allLines) > lines {
			allLines = allLines[len(allLines)-lines:]
		}
		content = strings.Join(allLines, "\n")
	}
	return content, nil
}

// Search searches history across all sessions for a query string.
func (r *Recorder) Search(query string) ([]SearchResult, error) {
	var results []SearchResult

	entries, err := os.ReadDir(r.baseDir)
	if err != nil {
		return nil, err
	}

	queryLower := strings.ToLower(query)
	for _, entry := range entries {
		if !entry.IsDir() || entry.Name() == "." {
			continue
		}
		sessionName := entry.Name()
		logPath := filepath.Join(r.baseDir, sessionName, "output.log")
		data, err := os.ReadFile(logPath)
		if err != nil {
			continue
		}

		lines := strings.Split(string(data), "\n")
		for i, line := range lines {
			if strings.Contains(strings.ToLower(line), queryLower) {
				results = append(results, SearchResult{
					Session: sessionName,
					Line:    i + 1,
					Content: line,
				})
			}
		}
	}

	return results, nil
}

// SearchResult holds a single search match.
type SearchResult struct {
	Session string
	Line    int
	Content string
}

// ListSessions returns all sessions that have history.
func (r *Recorder) ListSessions() []string {
	var sessions []string
	entries, err := os.ReadDir(r.baseDir)
	if err != nil {
		return nil
	}
	for _, entry := range entries {
		if entry.IsDir() && entry.Name() != "." {
			sessions = append(sessions, entry.Name())
		}
	}
	return sessions
}

func (r *Recorder) captureLoop(sess pkg.Session, stop chan struct{}) {
	ticker := time.NewTicker(r.interval)
	defer ticker.Stop()

	for {
		select {
		case <-stop:
			return
		case <-ticker.C:
			r.captureSnapshot(sess)
		}
	}
}

func (r *Recorder) captureSnapshot(sess pkg.Session) {
	inj, err := r.registry.Get(sess.Backend)
	if err != nil {
		return
	}

	output, err := inj.ReadOutput(sess, 200)
	if err != nil || output == "" {
		return
	}

	r.mu.Lock()
	last := r.lastSnap[sess.Name]
	r.mu.Unlock()

	if output == last {
		return // No new output
	}

	// Find new lines (diff from last snapshot)
	newContent := output
	if last != "" {
		lastLines := strings.Split(last, "\n")
		currentLines := strings.Split(output, "\n")
		// Find where new content starts
		overlap := 0
		for i := len(lastLines) - 1; i >= 0; i-- {
			for j := 0; j < len(currentLines); j++ {
				if lastLines[i] == currentLines[j] {
					overlap = j + 1
					break
				}
			}
			if overlap > 0 {
				break
			}
		}
		if overlap > 0 && overlap < len(currentLines) {
			newContent = strings.Join(currentLines[overlap:], "\n")
		}
	}

	r.mu.Lock()
	r.lastSnap[sess.Name] = output
	r.mu.Unlock()

	// Append to log file
	r.appendLog(sess.Name, newContent)
}

func (r *Recorder) appendLog(sessionName, content string) {
	logPath := r.logPath(sessionName)
	dir := filepath.Dir(logPath)
	os.MkdirAll(dir, 0700)

	f, err := os.OpenFile(logPath, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0600)
	if err != nil {
		return
	}
	defer f.Close()

	f.WriteString(content)
	if !strings.HasSuffix(content, "\n") {
		f.WriteString("\n")
	}
}

func (r *Recorder) logPath(sessionName string) string {
	// Sanitize session name for filesystem
	safe := strings.ReplaceAll(sessionName, "/", "_")
	safe = strings.ReplaceAll(safe, " ", "_")
	return filepath.Join(r.baseDir, safe, "output.log")
}
