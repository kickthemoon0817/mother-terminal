package session

import (
	"testing"

	"github.com/kickthemoon0817/mother-terminal/pkg"
)

func TestValidTransitions(t *testing.T) {
	cases := []struct {
		from    pkg.SessionStatus
		to      pkg.SessionStatus
		wantErr bool
	}{
		{pkg.StatusDiscovered, pkg.StatusActive, false},
		{pkg.StatusDiscovered, pkg.StatusDead, false},
		{pkg.StatusActive, pkg.StatusStalled, false},
		{pkg.StatusActive, pkg.StatusDead, false},
		{pkg.StatusStalled, pkg.StatusActive, false},
		{pkg.StatusStalled, pkg.StatusDead, false},
		// Invalid transitions
		{pkg.StatusDead, pkg.StatusActive, true},
		{pkg.StatusDead, pkg.StatusStalled, true},
		{pkg.StatusDiscovered, pkg.StatusStalled, true},
	}

	for _, tc := range cases {
		err := ValidateTransition(tc.from, tc.to)
		if tc.wantErr && err == nil {
			t.Errorf("expected error for %s -> %s", tc.from, tc.to)
		}
		if !tc.wantErr && err != nil {
			t.Errorf("unexpected error for %s -> %s: %v", tc.from, tc.to, err)
		}
	}
}

func TestManagerAddAndGet(t *testing.T) {
	m := NewManager()

	s := pkg.Session{
		Name:   "test-claude",
		CLI:    pkg.CLIClaude,
		Status: pkg.StatusDiscovered,
	}

	if err := m.Add(s); err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	got, err := m.Get("test-claude")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if got.Name != "test-claude" {
		t.Errorf("expected name 'test-claude', got %q", got.Name)
	}
}

func TestManagerAddDuplicate(t *testing.T) {
	m := NewManager()

	s := pkg.Session{Name: "test"}
	m.Add(s)

	err := m.Add(s)
	if err == nil {
		t.Fatal("expected error for duplicate session")
	}
}

func TestManagerUpdateStatus(t *testing.T) {
	m := NewManager()
	m.Add(pkg.Session{Name: "test", Status: pkg.StatusDiscovered})

	if err := m.UpdateStatus("test", pkg.StatusActive); err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	s, _ := m.Get("test")
	if s.Status != pkg.StatusActive {
		t.Errorf("expected status active, got %q", s.Status)
	}
}

func TestManagerUpdateStatusInvalidTransition(t *testing.T) {
	m := NewManager()
	m.Add(pkg.Session{Name: "test", Status: pkg.StatusDead})

	err := m.UpdateStatus("test", pkg.StatusActive)
	if err == nil {
		t.Fatal("expected error for invalid transition dead -> active")
	}
}

func TestManagerList(t *testing.T) {
	m := NewManager()
	m.Add(pkg.Session{Name: "a"})
	m.Add(pkg.Session{Name: "b"})

	sessions := m.List()
	if len(sessions) != 2 {
		t.Fatalf("expected 2 sessions, got %d", len(sessions))
	}
}

func TestManagerRemove(t *testing.T) {
	m := NewManager()
	m.Add(pkg.Session{Name: "test"})
	m.Remove("test")

	_, err := m.Get("test")
	if err == nil {
		t.Fatal("expected error after removal")
	}
}
