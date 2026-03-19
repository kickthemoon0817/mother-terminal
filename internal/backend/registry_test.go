package backend

import (
	"testing"

	"github.com/kickthemoon0817/mother-terminal/pkg"
)

// mockInjector is a test double for the Injector interface.
type mockInjector struct {
	available bool
	name      pkg.BackendType
	sessions  []pkg.Session
}

func (m *mockInjector) Discover() ([]pkg.Session, error) { return m.sessions, nil }
func (m *mockInjector) SendKeys(_ pkg.Session, _ string) error { return nil }
func (m *mockInjector) ReadOutput(_ pkg.Session, _ int) (string, error) { return "", nil }
func (m *mockInjector) Ping(_ pkg.Session) (pkg.PingResult, error) {
	return pkg.PingResult{Alive: true}, nil
}
func (m *mockInjector) IsAvailable() bool    { return m.available }
func (m *mockInjector) Name() pkg.BackendType { return m.name }

func TestRegisterAvailableBackend(t *testing.T) {
	r := NewRegistry()
	r.Register(&mockInjector{available: true, name: pkg.BackendTmux})

	backends := r.Available()
	if len(backends) != 1 {
		t.Fatalf("expected 1 available backend, got %d", len(backends))
	}
	if backends[0].Name() != pkg.BackendTmux {
		t.Errorf("expected tmux backend, got %v", backends[0].Name())
	}
}

func TestRegisterUnavailableBackend(t *testing.T) {
	r := NewRegistry()
	r.Register(&mockInjector{available: false, name: pkg.BackendX11})

	backends := r.Available()
	if len(backends) != 0 {
		t.Fatalf("expected 0 available backends, got %d", len(backends))
	}
}

func TestGetBackend(t *testing.T) {
	r := NewRegistry()
	r.Register(&mockInjector{available: true, name: pkg.BackendTmux})

	b, err := r.Get(pkg.BackendTmux)
	if err != nil {
		t.Fatalf("expected no error, got: %v", err)
	}
	if b.Name() != pkg.BackendTmux {
		t.Errorf("expected tmux, got %v", b.Name())
	}
}

func TestGetUnavailableBackend(t *testing.T) {
	r := NewRegistry()

	_, err := r.Get(pkg.BackendX11)
	if err != pkg.ErrBackendUnavailable {
		t.Fatalf("expected ErrBackendUnavailable, got: %v", err)
	}
}

func TestDiscoverAll(t *testing.T) {
	r := NewRegistry()
	r.Register(&mockInjector{
		available: true,
		name:      pkg.BackendTmux,
		sessions: []pkg.Session{
			{Name: "claude-tmux", CLI: pkg.CLIClaude, Backend: pkg.BackendTmux},
		},
	})
	r.Register(&mockInjector{
		available: true,
		name:      pkg.BackendPTY,
		sessions: []pkg.Session{
			{Name: "codex-pty", CLI: pkg.CLICodex, Backend: pkg.BackendPTY},
		},
	})

	sessions, err := r.DiscoverAll()
	if err != nil {
		t.Fatalf("expected no error, got: %v", err)
	}
	if len(sessions) != 2 {
		t.Fatalf("expected 2 sessions, got %d", len(sessions))
	}
}
