package pkg

import "time"

// CLIType identifies a supported AI CLI tool.
type CLIType string

const (
	CLIClaude   CLIType = "claude"
	CLICodex    CLIType = "codex"
	CLIGemini   CLIType = "gemini"
	CLIOpenCode CLIType = "opencode"
)

// BackendType identifies the injection backend.
type BackendType string

const (
	BackendTmux    BackendType = "tmux"
	BackendMacOS   BackendType = "macos"
	BackendX11     BackendType = "x11"
	BackendWayland BackendType = "wayland"
	BackendPTY     BackendType = "pty"
)

// SessionStatus represents the state of a session.
type SessionStatus string

const (
	StatusDiscovered SessionStatus = "discovered"
	StatusActive     SessionStatus = "active"
	StatusStalled    SessionStatus = "stalled"
	StatusDead       SessionStatus = "dead"
)

// StallPolicy defines what happens when a session stalls.
type StallPolicy string

const (
	PolicyNotify     StallPolicy = "notify"
	PolicyAutoResume StallPolicy = "auto_resume"
	PolicyCustom     StallPolicy = "custom"
)

// Session represents a managed AI CLI session.
type Session struct {
	ID            string        `toml:"id"`
	Name          string        `toml:"name"`
	CLI           CLIType       `toml:"cli"`
	Backend       BackendType   `toml:"backend"`
	Target        string        `toml:"target"`
	Status        SessionStatus `toml:"-"`
	Policy        StallPolicy   `toml:"policy"`
	ResumeMessage string        `toml:"resume_message"`
	StallTimeout  time.Duration `toml:"stall_timeout"`
}

// PingResult holds the result of a ping operation.
type PingResult struct {
	Alive      bool
	Responsive bool
	Latency    time.Duration
}

// UsageWindow tracks a usage window for a session.
type UsageWindow struct {
	SessionName string        `json:"session_name"`
	CLI         CLIType       `json:"cli"`
	StartedAt   time.Time     `json:"started_at"`
	Duration    time.Duration `json:"duration"`
	ExpiresAt   time.Time     `json:"expires_at"`
	Active      bool          `json:"active"`
}

// Remaining returns the time left in the usage window, or zero if expired.
func (w UsageWindow) Remaining() time.Duration {
	if !w.Active {
		return 0
	}
	remaining := time.Until(w.ExpiresAt)
	if remaining < 0 {
		return 0
	}
	return remaining
}

// RepeatMode defines the repeat mode for scheduled pings.
type RepeatMode string

const (
	RepeatOnce     RepeatMode = "once"
	RepeatDaily    RepeatMode = "daily"
	RepeatWeekdays RepeatMode = "weekdays"
)

// ProbeType defines the type of probe to send.
type ProbeType string

const (
	ProbeLiveness ProbeType = "liveness"
	ProbeActive   ProbeType = "active"
	ProbeBoth     ProbeType = "both"
)
