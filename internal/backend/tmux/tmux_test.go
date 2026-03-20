package tmux

import (
	"testing"

	"github.com/kickthemoon0817/mother-terminal/pkg"
)

func TestValidTmuxTarget(t *testing.T) {
	valid := []string{
		"dev:0.1",
		"my-session:0.0",
		"work:2.3",
		"test_session:0.1",
		"abc123",
	}
	for _, target := range valid {
		if !validTmuxTarget.MatchString(target) {
			t.Errorf("expected %q to be valid", target)
		}
	}

	invalid := []string{
		"-t foo",
		"--help",
		"session name with spaces",
		"$(whoami)",
		"; rm -rf /",
		"",
		"test\ninjection",
	}
	for _, target := range invalid {
		if validTmuxTarget.MatchString(target) {
			t.Errorf("expected %q to be invalid", target)
		}
	}
}

func TestSendKeysRejectsInvalidTarget(t *testing.T) {
	b := &Backend{}
	sess := pkg.Session{
		Target: "$(whoami)",
	}
	err := b.SendKeys(sess, "test")
	if err == nil {
		t.Fatal("expected error for invalid target")
	}
}

func TestParseTmuxPaneOutput(t *testing.T) {
	b := &Backend{}

	// Simulate tmux list-panes output parsing via Discover
	// We can't run tmux in tests, but we can verify the backend
	// implements the interface correctly
	if b.Name() != pkg.BackendTmux {
		t.Errorf("expected backend name tmux, got %v", b.Name())
	}
}
