package config

import (
	"fmt"
	"os"
	"time"

	"github.com/BurntSushi/toml"
	"github.com/kickthemoon0817/mother-terminal/pkg"
)

// Config represents the full Mother Terminal configuration.
type Config struct {
	Limits    map[string]string  `toml:"limits"`
	Sessions  []SessionConfig    `toml:"sessions"`
	Schedules []ScheduleConfig   `toml:"schedules"`
	Settings  SettingsConfig     `toml:"settings"`
}

// SessionConfig represents a manually registered session.
type SessionConfig struct {
	Name          string `toml:"name"`
	CLI           string `toml:"cli"`
	Backend       string `toml:"backend"`
	Target        string `toml:"target"`
	Policy        string `toml:"policy"`
	ResumeMessage string `toml:"resume_message"`
	StallTimeout  string `toml:"stall_timeout"`
}

// ScheduleConfig represents a scheduled ping.
type ScheduleConfig struct {
	Session string `toml:"session"`
	Time    string `toml:"time"`
	Repeat  string `toml:"repeat"`
	Probe   string `toml:"probe"`
}

// SettingsConfig holds general settings.
type SettingsConfig struct {
	StateDir            string `toml:"state_dir"`
	DiscoveryInterval   string `toml:"discovery_interval"`
	DefaultStallTimeout string `toml:"default_stall_timeout"`
	DefaultPolicy       string `toml:"default_policy"`
}

// Load reads and parses a TOML config file.
func Load(path string) (*Config, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("reading config file: %w", err)
	}

	var cfg Config
	if err := toml.Unmarshal(data, &cfg); err != nil {
		return nil, fmt.Errorf("parsing config file: %w", err)
	}

	if err := cfg.Validate(); err != nil {
		return nil, err
	}

	return &cfg, nil
}

// Validate checks the config for required fields and valid values.
func (c *Config) Validate() error {
	for i, s := range c.Sessions {
		if s.Name == "" {
			return fmt.Errorf("%w: session[%d] missing name", pkg.ErrConfigInvalid, i)
		}
		if s.CLI == "" {
			return fmt.Errorf("%w: session[%d] %q missing cli", pkg.ErrConfigInvalid, i, s.Name)
		}
		if s.Backend == "" {
			return fmt.Errorf("%w: session[%d] %q missing backend", pkg.ErrConfigInvalid, i, s.Name)
		}
		if s.Target == "" {
			return fmt.Errorf("%w: session[%d] %q missing target", pkg.ErrConfigInvalid, i, s.Name)
		}
		if s.StallTimeout != "" {
			if _, err := time.ParseDuration(s.StallTimeout); err != nil {
				return fmt.Errorf("%w: session[%d] %q invalid stall_timeout %q: %v", pkg.ErrConfigInvalid, i, s.Name, s.StallTimeout, err)
			}
		}
	}

	for i, sched := range c.Schedules {
		if sched.Session == "" {
			return fmt.Errorf("%w: schedule[%d] missing session", pkg.ErrConfigInvalid, i)
		}
		if sched.Time == "" {
			return fmt.Errorf("%w: schedule[%d] missing time", pkg.ErrConfigInvalid, i)
		}
	}

	for name, dur := range c.Limits {
		if dur != "0" {
			if _, err := time.ParseDuration(dur); err != nil {
				return fmt.Errorf("%w: limits.%s invalid duration %q: %v", pkg.ErrConfigInvalid, name, dur, err)
			}
		}
	}

	return nil
}

// ToSession converts a SessionConfig to a pkg.Session.
func (s SessionConfig) ToSession() pkg.Session {
	sess := pkg.Session{
		Name:          s.Name,
		CLI:           pkg.CLIType(s.CLI),
		Backend:       pkg.BackendType(s.Backend),
		Target:        s.Target,
		Status:        pkg.StatusDiscovered,
		Policy:        pkg.StallPolicy(s.Policy),
		ResumeMessage: s.ResumeMessage,
	}

	if s.StallTimeout != "" {
		sess.StallTimeout, _ = time.ParseDuration(s.StallTimeout)
	}

	return sess
}

// GetLimitDuration returns the usage limit duration for a CLI type.
func (c *Config) GetLimitDuration(cli pkg.CLIType) time.Duration {
	durStr, ok := c.Limits[string(cli)]
	if !ok || durStr == "0" {
		return 0
	}
	d, err := time.ParseDuration(durStr)
	if err != nil {
		return 0
	}
	return d
}
