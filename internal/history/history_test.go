package history

import (
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/kickthemoon0817/mother-terminal/internal/backend"
	"github.com/kickthemoon0817/mother-terminal/pkg"
)

// ── test doubles ─────────────────────────────────────────────────────────────

type mockInjector struct {
	mu      sync.Mutex
	outputs []string // each call to ReadOutput returns the next value
	callIdx int
	name    pkg.BackendType
}

func (m *mockInjector) ReadOutput(_ pkg.Session, _ int) (string, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.callIdx >= len(m.outputs) {
		return "", nil
	}
	out := m.outputs[m.callIdx]
	m.callIdx++
	return out, nil
}
func (m *mockInjector) Discover() ([]pkg.Session, error)          { return nil, nil }
func (m *mockInjector) SendKeys(_ pkg.Session, _ string) error    { return nil }
func (m *mockInjector) Ping(_ pkg.Session) (pkg.PingResult, error) {
	return pkg.PingResult{Alive: true}, nil
}
func (m *mockInjector) IsAvailable() bool     { return true }
func (m *mockInjector) Name() pkg.BackendType { return m.name }

func newTestRegistry(inj *mockInjector) *backend.Registry {
	r := backend.NewRegistry()
	r.Register(inj)
	return r
}

func newTestRecorder(t *testing.T, inj *mockInjector) (*Recorder, string) {
	t.Helper()
	dir := t.TempDir()
	reg := newTestRegistry(inj)
	rec := NewRecorder(reg, dir, 10*time.Millisecond)
	return rec, dir
}

func testSession(name string) pkg.Session {
	return pkg.Session{
		Name:    name,
		CLI:     pkg.CLIClaude,
		Backend: pkg.BackendTmux,
		Target:  name + ":0.0",
		Status:  pkg.StatusActive,
	}
}

// ── NewRecorder ───────────────────────────────────────────────────────────────

func TestNewRecorder_usesDefaultIntervalWhenZero(t *testing.T) {
	dir := t.TempDir()
	reg := backend.NewRegistry()
	rec := NewRecorder(reg, dir, 0)
	if rec.interval != 5*time.Second {
		t.Errorf("expected default interval 5s, got %v", rec.interval)
	}
}

func TestNewRecorder_createsBaseDirectory(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "deep", "nested", "dir")
	reg := backend.NewRegistry()
	NewRecorder(reg, dir, time.Second)
	if _, err := os.Stat(dir); err != nil {
		t.Fatalf("expected base directory to exist: %v", err)
	}
}

// ── Record / StopRecording ────────────────────────────────────────────────────

func TestRecord_startsCapturingOutput(t *testing.T) {
	inj := &mockInjector{
		name:    pkg.BackendTmux,
		outputs: []string{"line one\nline two\n"},
	}
	rec, dir := newTestRecorder(t, inj)
	sess := testSession("myapp")

	rec.Record(sess)
	defer rec.StopRecording(sess.Name)

	// Wait for at least one capture tick
	deadline := time.Now().Add(500 * time.Millisecond)
	logPath := filepath.Join(dir, "myapp", "output.log")
	for time.Now().Before(deadline) {
		if _, err := os.Stat(logPath); err == nil {
			break
		}
		time.Sleep(20 * time.Millisecond)
	}

	data, err := os.ReadFile(logPath)
	if err != nil {
		t.Fatalf("log file not created: %v", err)
	}
	if !strings.Contains(string(data), "line one") {
		t.Errorf("expected captured output in log, got: %q", string(data))
	}
}

func TestRecord_doesNotStartDuplicateGoroutineForSameSession(t *testing.T) {
	inj := &mockInjector{name: pkg.BackendTmux, outputs: []string{"x"}}
	rec, _ := newTestRecorder(t, inj)
	sess := testSession("dup-session")

	rec.Record(sess)
	rec.Record(sess) // second call must be a no-op

	rec.mu.Lock()
	count := len(rec.stops)
	rec.mu.Unlock()

	if count != 1 {
		t.Errorf("expected exactly 1 stop channel, got %d", count)
	}
	rec.StopAll()
}

func TestStopRecording_removesSessionFromStopsMap(t *testing.T) {
	inj := &mockInjector{name: pkg.BackendTmux}
	rec, _ := newTestRecorder(t, inj)
	sess := testSession("stop-me")

	rec.Record(sess)
	rec.StopRecording(sess.Name)

	rec.mu.Lock()
	_, stillPresent := rec.stops[sess.Name]
	rec.mu.Unlock()

	if stillPresent {
		t.Error("expected session to be removed from stops map after StopRecording")
	}
}

func TestStopRecording_onUnknownSessionIsNoop(t *testing.T) {
	inj := &mockInjector{name: pkg.BackendTmux}
	rec, _ := newTestRecorder(t, inj)
	// Must not panic or block
	rec.StopRecording("nonexistent-session")
}

func TestStopAll_stopsAllActiveSessions(t *testing.T) {
	inj := &mockInjector{name: pkg.BackendTmux}
	rec, _ := newTestRecorder(t, inj)

	for _, name := range []string{"sess-a", "sess-b", "sess-c"} {
		rec.Record(testSession(name))
	}

	rec.StopAll()

	rec.mu.Lock()
	remaining := len(rec.stops)
	rec.mu.Unlock()

	if remaining != 0 {
		t.Errorf("expected 0 remaining sessions after StopAll, got %d", remaining)
	}
}

// ── GetHistory ────────────────────────────────────────────────────────────────

func TestGetHistory_returnsErrorForSessionWithNoLog(t *testing.T) {
	inj := &mockInjector{name: pkg.BackendTmux}
	rec, _ := newTestRecorder(t, inj)

	_, err := rec.GetHistory("no-such-session", 10)
	if err == nil {
		t.Fatal("expected error for session with no history, got nil")
	}
}

func TestGetHistory_returnsAllLinesWhenLimitIsZero(t *testing.T) {
	inj := &mockInjector{name: pkg.BackendTmux}
	rec, dir := newTestRecorder(t, inj)

	// Write log manually
	logDir := filepath.Join(dir, "mysession")
	os.MkdirAll(logDir, 0700)
	content := "alpha\nbeta\ngamma\ndelta\n"
	os.WriteFile(filepath.Join(logDir, "output.log"), []byte(content), 0600)

	got, err := rec.GetHistory("mysession", 0)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !strings.Contains(got, "alpha") || !strings.Contains(got, "delta") {
		t.Errorf("expected all lines returned, got: %q", got)
	}
}

func TestGetHistory_truncatesToLastNLines(t *testing.T) {
	inj := &mockInjector{name: pkg.BackendTmux}
	rec, dir := newTestRecorder(t, inj)

	logDir := filepath.Join(dir, "trunc-session")
	os.MkdirAll(logDir, 0700)
	var sb strings.Builder
	for i := 1; i <= 100; i++ {
		sb.WriteString(strings.Repeat("x", 10) + "\n")
	}
	// Last two lines are identifiable
	content := sb.String() + "second-to-last\nlast-line\n"
	os.WriteFile(filepath.Join(logDir, "output.log"), []byte(content), 0600)

	got, err := rec.GetHistory("trunc-session", 3)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if strings.Contains(got, "xxxxxxxxxx\nxxxxxxxxxx") {
		t.Error("expected truncation, but got many early lines")
	}
	if !strings.Contains(got, "last-line") {
		t.Errorf("expected last-line in truncated result, got: %q", got)
	}
}

// ── Search ────────────────────────────────────────────────────────────────────

func TestSearch_returnsEmptySliceWhenNothingMatches(t *testing.T) {
	inj := &mockInjector{name: pkg.BackendTmux}
	rec, dir := newTestRecorder(t, inj)

	logDir := filepath.Join(dir, "sess1")
	os.MkdirAll(logDir, 0700)
	os.WriteFile(filepath.Join(logDir, "output.log"), []byte("hello world\n"), 0600)

	results, err := rec.Search("zzznomatch")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(results) != 0 {
		t.Errorf("expected 0 results, got %d", len(results))
	}
}

func TestSearch_isCaseInsensitive(t *testing.T) {
	inj := &mockInjector{name: pkg.BackendTmux}
	rec, dir := newTestRecorder(t, inj)

	logDir := filepath.Join(dir, "sess-ci")
	os.MkdirAll(logDir, 0700)
	os.WriteFile(filepath.Join(logDir, "output.log"), []byte("Claude is running\n"), 0600)

	results, err := rec.Search("claude")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(results) != 1 {
		t.Fatalf("expected 1 result for case-insensitive match, got %d", len(results))
	}
	if results[0].Session != "sess-ci" {
		t.Errorf("unexpected session name: %q", results[0].Session)
	}
}

func TestSearch_reportsCorrectLineNumbers(t *testing.T) {
	inj := &mockInjector{name: pkg.BackendTmux}
	rec, dir := newTestRecorder(t, inj)

	logDir := filepath.Join(dir, "sess-ln")
	os.MkdirAll(logDir, 0700)
	os.WriteFile(filepath.Join(logDir, "output.log"), []byte("first\nsecond\nTARGET\nfourth\n"), 0600)

	results, err := rec.Search("TARGET")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(results) != 1 {
		t.Fatalf("expected 1 result, got %d", len(results))
	}
	if results[0].Line != 3 {
		t.Errorf("expected line 3, got %d", results[0].Line)
	}
}

func TestSearch_searchesAcrossMultipleSessions(t *testing.T) {
	inj := &mockInjector{name: pkg.BackendTmux}
	rec, dir := newTestRecorder(t, inj)

	for _, sessName := range []string{"sess-x", "sess-y"} {
		logDir := filepath.Join(dir, sessName)
		os.MkdirAll(logDir, 0700)
		os.WriteFile(filepath.Join(logDir, "output.log"), []byte("found it\n"), 0600)
	}

	results, err := rec.Search("found it")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(results) != 2 {
		t.Errorf("expected 2 results across sessions, got %d", len(results))
	}
}

// ── logPath sanitization ──────────────────────────────────────────────────────

func TestLogPath_sanitizesSlashesAndSpaces(t *testing.T) {
	inj := &mockInjector{name: pkg.BackendTmux}
	rec, _ := newTestRecorder(t, inj)

	path := rec.logPath("my/session name")
	base := filepath.Base(filepath.Dir(path))
	if strings.Contains(base, "/") || strings.Contains(base, " ") {
		t.Errorf("logPath not sanitized: %q", base)
	}
}

// ── Concurrent capture + read ─────────────────────────────────────────────────

func TestConcurrentRecordAndGetHistory_noDataRace(t *testing.T) {
	// This test is valuable when run with -race flag.
	inj := &mockInjector{
		name:    pkg.BackendTmux,
		outputs: []string{"output-1\n", "output-2\n", "output-3\n"},
	}
	rec, dir := newTestRecorder(t, inj)
	sess := testSession("race-session")

	// Seed a log so GetHistory has something to read
	logDir := filepath.Join(dir, "race-session")
	os.MkdirAll(logDir, 0700)
	os.WriteFile(filepath.Join(logDir, "output.log"), []byte("seed\n"), 0600)

	rec.Record(sess)
	defer rec.StopAll()

	var wg sync.WaitGroup
	for i := 0; i < 5; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			rec.GetHistory(sess.Name, 10) //nolint:errcheck
		}()
	}
	wg.Wait()
}

// ── appendLog ─────────────────────────────────────────────────────────────────

func TestAppendLog_addsNewlineWhenMissing(t *testing.T) {
	inj := &mockInjector{name: pkg.BackendTmux}
	rec, dir := newTestRecorder(t, inj)

	rec.appendLog("no-newline-session", "content without newline")

	logPath := filepath.Join(dir, "no-newline-session", "output.log")
	data, err := os.ReadFile(logPath)
	if err != nil {
		t.Fatalf("log not created: %v", err)
	}
	if !strings.HasSuffix(string(data), "\n") {
		t.Errorf("expected trailing newline, got: %q", string(data))
	}
}

func TestAppendLog_appendsToExistingFile(t *testing.T) {
	inj := &mockInjector{name: pkg.BackendTmux}
	rec, _ := newTestRecorder(t, inj)

	rec.appendLog("append-session", "first\n")
	rec.appendLog("append-session", "second\n")

	got, err := rec.GetHistory("append-session", 0)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !strings.Contains(got, "first") || !strings.Contains(got, "second") {
		t.Errorf("expected both appended lines, got: %q", got)
	}
}
