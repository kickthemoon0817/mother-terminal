package config

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/kickthemoon0817/mother-terminal/pkg"
)

func TestLoadValidConfig(t *testing.T) {
	content := `
[limits]
claude = "5h"
codex = "5h"

[[sessions]]
name = "test-claude"
cli = "claude"
backend = "tmux"
target = "dev:0.1"
policy = "notify"
stall_timeout = "120s"

[[schedules]]
session = "test-claude"
time = "07:00"
repeat = "daily"
probe = "active"

[settings]
state_dir = "~/.mother"
`
	path := writeTempConfig(t, content)
	cfg, err := Load(path)
	if err != nil {
		t.Fatalf("expected no error, got: %v", err)
	}

	if len(cfg.Sessions) != 1 {
		t.Fatalf("expected 1 session, got %d", len(cfg.Sessions))
	}
	if cfg.Sessions[0].Name != "test-claude" {
		t.Errorf("expected session name 'test-claude', got %q", cfg.Sessions[0].Name)
	}
	if len(cfg.Schedules) != 1 {
		t.Fatalf("expected 1 schedule, got %d", len(cfg.Schedules))
	}
	if cfg.Limits["claude"] != "5h" {
		t.Errorf("expected claude limit '5h', got %q", cfg.Limits["claude"])
	}
}

func TestLoadMissingFile(t *testing.T) {
	_, err := Load("/nonexistent/path/config.toml")
	if err == nil {
		t.Fatal("expected error for missing file")
	}
}

func TestLoadInvalidTOML(t *testing.T) {
	path := writeTempConfig(t, "this is not valid toml [[[")
	_, err := Load(path)
	if err == nil {
		t.Fatal("expected error for invalid TOML")
	}
}

func TestValidateMissingSessionName(t *testing.T) {
	content := `
[[sessions]]
cli = "claude"
backend = "tmux"
target = "dev:0.1"
`
	path := writeTempConfig(t, content)
	_, err := Load(path)
	if err == nil {
		t.Fatal("expected validation error for missing session name")
	}
}

func TestValidateInvalidStallTimeout(t *testing.T) {
	content := `
[[sessions]]
name = "test"
cli = "claude"
backend = "tmux"
target = "dev:0.1"
stall_timeout = "not-a-duration"
`
	path := writeTempConfig(t, content)
	_, err := Load(path)
	if err == nil {
		t.Fatal("expected validation error for invalid stall_timeout")
	}
}

func TestValidateInvalidLimitDuration(t *testing.T) {
	content := `
[limits]
claude = "bad"
`
	path := writeTempConfig(t, content)
	_, err := Load(path)
	if err == nil {
		t.Fatal("expected validation error for invalid limit duration")
	}
}

func TestGetLimitDuration(t *testing.T) {
	cfg := &Config{
		Limits: map[string]string{
			"claude":   "5h",
			"opencode": "0",
		},
	}

	d := cfg.GetLimitDuration(pkg.CLIClaude)
	if d.Hours() != 5 {
		t.Errorf("expected 5h, got %v", d)
	}

	d = cfg.GetLimitDuration(pkg.CLIOpenCode)
	if d != 0 {
		t.Errorf("expected 0 for opencode, got %v", d)
	}

	d = cfg.GetLimitDuration(pkg.CLIGemini)
	if d != 0 {
		t.Errorf("expected 0 for unknown CLI, got %v", d)
	}
}

func TestToSession(t *testing.T) {
	sc := SessionConfig{
		Name:          "test",
		CLI:           "claude",
		Backend:       "tmux",
		Target:        "dev:0.1",
		Policy:        "auto_resume",
		ResumeMessage: "continue",
		StallTimeout:  "60s",
	}

	sess := sc.ToSession()
	if sess.Name != "test" {
		t.Errorf("expected name 'test', got %q", sess.Name)
	}
	if sess.CLI != pkg.CLIClaude {
		t.Errorf("expected CLI claude, got %q", sess.CLI)
	}
	if sess.StallTimeout.Seconds() != 60 {
		t.Errorf("expected 60s timeout, got %v", sess.StallTimeout)
	}
}

func writeTempConfig(t *testing.T, content string) string {
	t.Helper()
	dir := t.TempDir()
	path := filepath.Join(dir, "config.toml")
	if err := os.WriteFile(path, []byte(content), 0644); err != nil {
		t.Fatalf("failed to write temp config: %v", err)
	}
	return path
}
