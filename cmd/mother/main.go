package main

import (
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/kickthemoon0817/mother-terminal/internal/backend"
	"github.com/kickthemoon0817/mother-terminal/internal/config"
	"github.com/kickthemoon0817/mother-terminal/internal/discovery"
	"github.com/kickthemoon0817/mother-terminal/internal/scheduler"
	"github.com/kickthemoon0817/mother-terminal/internal/session"
	"github.com/kickthemoon0817/mother-terminal/internal/tui"
	"github.com/kickthemoon0817/mother-terminal/pkg"
)

func main() {
	configPath := flag.String("config", "", "path to config file")
	flag.Parse()

	// Resolve config path
	cfgPath := *configPath
	if cfgPath == "" {
		home, _ := os.UserHomeDir()
		cfgPath = filepath.Join(home, ".mother", "config.toml")
	}

	// Load config (optional — runs fine without one)
	var cfg *config.Config
	if _, err := os.Stat(cfgPath); err == nil {
		cfg, err = config.Load(cfgPath)
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error loading config: %v\n", err)
			os.Exit(1)
		}
	} else {
		cfg = &config.Config{
			Limits:   map[string]string{},
			Settings: config.SettingsConfig{StateDir: "~/.mother"},
		}
	}

	// Resolve state directory
	stateDir := cfg.Settings.StateDir
	if stateDir == "" {
		stateDir = "~/.mother"
	}
	if len(stateDir) >= 2 && stateDir[:2] == "~/" {
		home, _ := os.UserHomeDir()
		stateDir = filepath.Join(home, stateDir[2:])
	}
	os.MkdirAll(stateDir, 0755)

	// Initialize backend registry
	reg := backend.NewRegistry()
	_ = registerCommonBackends(reg)
	registerPlatformBackends(reg)

	// Initialize session manager
	mgr := session.NewManager()

	// Register manual sessions from config
	manualReg := discovery.NewRegistry()
	for _, sc := range cfg.Sessions {
		s := sc.ToSession()
		manualReg.Register(s)
	}

	// Run discovery
	discovered, _ := reg.DiscoverAll()

	// Merge with manual registrations
	allSessions := manualReg.Merge(discovered)

	// Add all sessions to manager
	for _, s := range allSessions {
		mgr.AddOrUpdate(s)
	}

	// Initialize usage window tracker
	limits := make(map[pkg.CLIType]time.Duration)
	for cli, durStr := range cfg.Limits {
		if durStr != "0" {
			d, err := time.ParseDuration(durStr)
			if err == nil {
				limits[pkg.CLIType(cli)] = d
			}
		}
	}
	windowTracker := scheduler.NewWindowTracker(limits, stateDir)

	// Initialize monitor
	defaultTimeout := 120 * time.Second
	if cfg.Settings.DefaultStallTimeout != "" {
		if d, err := time.ParseDuration(cfg.Settings.DefaultStallTimeout); err == nil {
			defaultTimeout = d
		}
	}
	mon := session.NewMonitor(mgr, reg, session.MonitorDefaults{
		StallTimeout:  defaultTimeout,
		ResumeMessage: "continue",
	})

	// Start monitoring all active sessions
	for _, s := range mgr.List() {
		if s.Status == pkg.StatusActive || s.Status == pkg.StatusDiscovered {
			mon.Watch(s.Name)
		}
	}

	// Initialize scheduler
	pingService := scheduler.NewPingService(mgr, reg, windowTracker)
	sched := scheduler.NewScheduler(pingService)
	for _, sc := range cfg.Schedules {
		sched.AddSchedule(scheduler.PingSchedule{
			SessionName: sc.Session,
			Time:        sc.Time,
			Repeat:      pkg.RepeatMode(sc.Repeat),
			Probe:       pkg.ProbeType(sc.Probe),
		})
	}
	sched.Start()
	defer sched.Stop()
	defer mon.UnwatchAll()

	// Start TUI
	model := tui.NewModel(mgr, reg, windowTracker, mon)
	if err := tui.Run(model); err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}
}
