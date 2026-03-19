package discovery

import (
	"testing"

	"github.com/kickthemoon0817/mother-terminal/pkg"
)

func TestScannerParsePS(t *testing.T) {
	s := NewScanner()

	output := `  PID COMM
  123 /usr/bin/zsh
  456 claude
  789 /usr/local/bin/codex
  012 node
`
	entries := s.parsePS(output)

	if len(entries) != 2 {
		t.Fatalf("expected 2 entries, got %d", len(entries))
	}

	found := map[pkg.CLIType]bool{}
	for _, e := range entries {
		found[e.CLI] = true
	}

	if !found[pkg.CLIClaude] {
		t.Error("expected to find claude")
	}
	if !found[pkg.CLICodex] {
		t.Error("expected to find codex")
	}
}

func TestRegistryRegisterAndList(t *testing.T) {
	r := NewRegistry()

	s := pkg.Session{
		Name:    "test-claude",
		CLI:     pkg.CLIClaude,
		Backend: pkg.BackendTmux,
		Target:  "dev:0.1",
	}
	r.Register(s)

	sessions := r.List()
	if len(sessions) != 1 {
		t.Fatalf("expected 1 session, got %d", len(sessions))
	}
	if sessions[0].Name != "test-claude" {
		t.Errorf("expected name 'test-claude', got %q", sessions[0].Name)
	}
}

func TestRegistryMergeManualPriority(t *testing.T) {
	r := NewRegistry()

	manual := pkg.Session{
		Name:    "claude-1",
		CLI:     pkg.CLIClaude,
		Backend: pkg.BackendTmux,
		Target:  "manual-target",
		Policy:  pkg.PolicyAutoResume,
	}
	r.Register(manual)

	discovered := []pkg.Session{
		{
			Name:    "claude-1",
			CLI:     pkg.CLIClaude,
			Backend: pkg.BackendTmux,
			Target:  "auto-target",
			Policy:  pkg.PolicyNotify,
		},
		{
			Name:    "codex-1",
			CLI:     pkg.CLICodex,
			Backend: pkg.BackendTmux,
			Target:  "codex-target",
		},
	}

	merged := r.Merge(discovered)

	if len(merged) != 2 {
		t.Fatalf("expected 2 sessions, got %d", len(merged))
	}

	// Find claude-1 and verify manual override
	for _, s := range merged {
		if s.Name == "claude-1" {
			if s.Target != "manual-target" {
				t.Errorf("expected manual target override, got %q", s.Target)
			}
			if s.Policy != pkg.PolicyAutoResume {
				t.Errorf("expected manual policy override, got %q", s.Policy)
			}
		}
	}
}

func TestRegistryUnregister(t *testing.T) {
	r := NewRegistry()
	r.Register(pkg.Session{Name: "test"})
	r.Unregister("test")

	if _, ok := r.Get("test"); ok {
		t.Error("expected session to be unregistered")
	}
}
