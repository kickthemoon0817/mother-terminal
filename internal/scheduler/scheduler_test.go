package scheduler

import (
	"testing"
	"time"

	"github.com/kickthemoon0817/mother-terminal/pkg"
)

func TestParseTime(t *testing.T) {
	h, m := parseTime("07:00")
	if h != 7 || m != 0 {
		t.Errorf("expected 7:0, got %d:%d", h, m)
	}

	h, m = parseTime("14:30")
	if h != 14 || m != 30 {
		t.Errorf("expected 14:30, got %d:%d", h, m)
	}

	h, m = parseTime("invalid")
	if h != 0 || m != 0 {
		t.Errorf("expected 0:0 for invalid, got %d:%d", h, m)
	}
}

func TestWindowTrackerStartAndRemaining(t *testing.T) {
	dir := t.TempDir()
	limits := map[pkg.CLIType]time.Duration{
		pkg.CLIClaude: 5 * time.Hour,
	}

	wt := NewWindowTracker(limits, dir)
	wt.StartWindow("claude-main", pkg.CLIClaude)

	w := wt.GetWindow("claude-main")
	if w == nil {
		t.Fatal("expected window to exist")
	}
	if !w.Active {
		t.Error("expected window to be active")
	}

	remaining := wt.Remaining("claude-main")
	if remaining < 4*time.Hour+59*time.Minute {
		t.Errorf("expected ~5h remaining, got %v", remaining)
	}
}

func TestWindowTrackerNoLimit(t *testing.T) {
	dir := t.TempDir()
	limits := map[pkg.CLIType]time.Duration{
		pkg.CLIOpenCode: 0, // no tracking
	}

	wt := NewWindowTracker(limits, dir)
	wt.StartWindow("opencode-1", pkg.CLIOpenCode)

	w := wt.GetWindow("opencode-1")
	if w != nil {
		t.Error("expected no window for CLI with 0 limit")
	}
}

func TestWindowTrackerExpiry(t *testing.T) {
	dir := t.TempDir()
	limits := map[pkg.CLIType]time.Duration{
		pkg.CLIClaude: 1 * time.Millisecond, // tiny for testing
	}

	wt := NewWindowTracker(limits, dir)
	wt.StartWindow("claude-test", pkg.CLIClaude)

	time.Sleep(5 * time.Millisecond)

	w := wt.GetWindow("claude-test")
	if w == nil {
		t.Fatal("expected window to exist")
	}
	if w.Active {
		t.Error("expected window to be expired")
	}
	if wt.Remaining("claude-test") != 0 {
		t.Error("expected 0 remaining after expiry")
	}
}

func TestWindowTrackerGetAllWindows(t *testing.T) {
	dir := t.TempDir()
	limits := map[pkg.CLIType]time.Duration{
		pkg.CLIClaude: 5 * time.Hour,
		pkg.CLICodex:  5 * time.Hour,
	}

	wt := NewWindowTracker(limits, dir)
	wt.StartWindow("claude-1", pkg.CLIClaude)
	wt.StartWindow("codex-1", pkg.CLICodex)

	windows := wt.GetAllWindows()
	if len(windows) != 2 {
		t.Fatalf("expected 2 windows, got %d", len(windows))
	}
}

func TestWindowTrackerUnknownSession(t *testing.T) {
	dir := t.TempDir()
	wt := NewWindowTracker(nil, dir)

	w := wt.GetWindow("nonexistent")
	if w != nil {
		t.Error("expected nil for unknown session")
	}

	remaining := wt.Remaining("nonexistent")
	if remaining != 0 {
		t.Error("expected 0 remaining for unknown session")
	}
}
