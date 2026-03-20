package session

import (
	"testing"
	"time"

	"github.com/kickthemoon0817/mother-terminal/internal/backend"
	"github.com/kickthemoon0817/mother-terminal/pkg"
)

// mockInjector for monitor tests.
type mockMonitorInjector struct {
	available bool
	output    string
	sendCalls int
}

func (m *mockMonitorInjector) Discover() ([]pkg.Session, error) { return nil, nil }
func (m *mockMonitorInjector) SendKeys(_ pkg.Session, _ string) error {
	m.sendCalls++
	return nil
}
func (m *mockMonitorInjector) ReadOutput(_ pkg.Session, _ int) (string, error) {
	return m.output, nil
}
func (m *mockMonitorInjector) Ping(_ pkg.Session) (pkg.PingResult, error) {
	return pkg.PingResult{Alive: true}, nil
}
func (m *mockMonitorInjector) IsAvailable() bool    { return m.available }
func (m *mockMonitorInjector) Name() pkg.BackendType { return pkg.BackendTmux }

func TestMonitorDetectsStall(t *testing.T) {
	mgr := NewManager()
	reg := backend.NewRegistry()

	mock := &mockMonitorInjector{available: true, output: "static output"}
	reg.Register(mock)

	mgr.Add(pkg.Session{
		Name:    "test-session",
		CLI:     pkg.CLIClaude,
		Backend: pkg.BackendTmux,
		Status:  pkg.StatusActive,
	})

	mon := NewMonitor(mgr, reg, MonitorDefaults{
		StallTimeout: 50 * time.Millisecond,
		PollInterval: 10 * time.Millisecond,
	})

	mon.Watch("test-session")
	defer mon.UnwatchAll()

	// Wait for stall detection
	select {
	case event := <-mon.Events():
		if event.Type != EventStalled {
			t.Errorf("expected EventStalled, got %v", event.Type)
		}
		if event.SessionName != "test-session" {
			t.Errorf("expected session name 'test-session', got %q", event.SessionName)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("timed out waiting for stall event")
	}

	// Verify session status changed
	s, _ := mgr.Get("test-session")
	if s.Status != pkg.StatusStalled {
		t.Errorf("expected status stalled, got %v", s.Status)
	}
}

func TestMonitorDetectsResume(t *testing.T) {
	mgr := NewManager()
	reg := backend.NewRegistry()

	mock := &mockMonitorInjector{available: true, output: "initial"}
	reg.Register(mock)

	mgr.Add(pkg.Session{
		Name:    "test-resume",
		CLI:     pkg.CLIClaude,
		Backend: pkg.BackendTmux,
		Status:  pkg.StatusActive,
	})

	mon := NewMonitor(mgr, reg, MonitorDefaults{
		StallTimeout: 30 * time.Millisecond,
		PollInterval: 10 * time.Millisecond,
	})

	mon.Watch("test-resume")
	defer mon.UnwatchAll()

	// Wait for stall
	select {
	case <-mon.Events():
	case <-time.After(2 * time.Second):
		t.Fatal("timed out waiting for stall")
	}

	// Change output to trigger resume
	mock.output = "new output"

	select {
	case event := <-mon.Events():
		if event.Type != EventResumed {
			t.Errorf("expected EventResumed, got %v", event.Type)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("timed out waiting for resume event")
	}
}

func TestMonitorAutoResumePolicy(t *testing.T) {
	mgr := NewManager()
	reg := backend.NewRegistry()

	mock := &mockMonitorInjector{available: true, output: "static"}
	reg.Register(mock)

	mgr.Add(pkg.Session{
		Name:    "test-autoresume",
		CLI:     pkg.CLIClaude,
		Backend: pkg.BackendTmux,
		Status:  pkg.StatusActive,
		Policy:  pkg.PolicyAutoResume,
	})

	mon := NewMonitor(mgr, reg, MonitorDefaults{
		StallTimeout:  30 * time.Millisecond,
		PollInterval:  10 * time.Millisecond,
		ResumeMessage: "continue",
	})

	mon.Watch("test-autoresume")
	defer mon.UnwatchAll()

	// Wait for stall + recovery events
	events := make([]Event, 0)
	timeout := time.After(2 * time.Second)
	for len(events) < 2 {
		select {
		case e := <-mon.Events():
			events = append(events, e)
		case <-timeout:
			t.Fatalf("timed out, got %d events", len(events))
		}
	}

	if events[0].Type != EventStalled {
		t.Errorf("expected first event EventStalled, got %v", events[0].Type)
	}
	if events[1].Type != EventRecovery {
		t.Errorf("expected second event EventRecovery, got %v", events[1].Type)
	}
	if mock.sendCalls == 0 {
		t.Error("expected SendKeys to be called for auto-resume")
	}
}

func TestMonitorNonBlockingEmit(t *testing.T) {
	mgr := NewManager()
	reg := backend.NewRegistry()

	mock := &mockMonitorInjector{available: true, output: "static"}
	reg.Register(mock)

	mon := NewMonitor(mgr, reg, MonitorDefaults{
		StallTimeout: 10 * time.Millisecond,
		PollInterval: 5 * time.Millisecond,
	})

	// Don't consume events — verify emit doesn't block
	for i := 0; i < 150; i++ {
		mon.emit(Event{Type: EventStalled})
	}
	// If we get here without deadlock, the non-blocking emit works
}

func TestMonitorDoneChannel(t *testing.T) {
	mgr := NewManager()
	reg := backend.NewRegistry()

	mon := NewMonitor(mgr, reg, MonitorDefaults{})

	mon.UnwatchAll()

	select {
	case <-mon.Done():
		// Success — done channel closed
	case <-time.After(100 * time.Millisecond):
		t.Fatal("done channel not closed after UnwatchAll")
	}
}

func TestMonitorUnwatch(t *testing.T) {
	mgr := NewManager()
	reg := backend.NewRegistry()

	mock := &mockMonitorInjector{available: true, output: "test"}
	reg.Register(mock)

	mgr.Add(pkg.Session{
		Name:    "test-unwatch",
		CLI:     pkg.CLIClaude,
		Backend: pkg.BackendTmux,
		Status:  pkg.StatusActive,
	})

	mon := NewMonitor(mgr, reg, MonitorDefaults{
		PollInterval: 10 * time.Millisecond,
	})

	mon.Watch("test-unwatch")
	mon.Unwatch("test-unwatch")

	// Verify watcher removed
	mon.mu.Lock()
	_, exists := mon.watchers["test-unwatch"]
	mon.mu.Unlock()

	if exists {
		t.Error("expected watcher to be removed after Unwatch")
	}
}
