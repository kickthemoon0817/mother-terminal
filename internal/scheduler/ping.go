package scheduler

import (
	"sync"
	"time"

	"github.com/kickthemoon0817/mother-terminal/internal/backend"
	"github.com/kickthemoon0817/mother-terminal/internal/session"
	"github.com/kickthemoon0817/mother-terminal/pkg"
)

// PingSchedule defines a scheduled ping for a session.
type PingSchedule struct {
	SessionName string
	Time        string     // "07:00" (24hr format)
	Repeat      pkg.RepeatMode
	Probe       pkg.ProbeType
}

// PingService handles liveness checks and active probes.
type PingService struct {
	manager  *session.Manager
	registry *backend.Registry
	windows  *WindowTracker
}

// NewPingService creates a new ping service.
func NewPingService(manager *session.Manager, registry *backend.Registry, windows *WindowTracker) *PingService {
	return &PingService{
		manager:  manager,
		registry: registry,
		windows:  windows,
	}
}

// LivenessCheck verifies a session process is running without sending input.
func (p *PingService) LivenessCheck(sessionName string) (pkg.PingResult, error) {
	sess, err := p.manager.Get(sessionName)
	if err != nil {
		return pkg.PingResult{}, err
	}

	inj, err := p.registry.Get(sess.Backend)
	if err != nil {
		return pkg.PingResult{}, err
	}

	return inj.Ping(*sess)
}

// ActiveProbe sends a lightweight test to the session and verifies response.
func (p *PingService) ActiveProbe(sessionName string) (pkg.PingResult, error) {
	sess, err := p.manager.Get(sessionName)
	if err != nil {
		return pkg.PingResult{}, err
	}

	inj, err := p.registry.Get(sess.Backend)
	if err != nil {
		return pkg.PingResult{}, err
	}

	// First do a liveness check
	result, err := inj.Ping(*sess)
	if err != nil || !result.Alive {
		return result, err
	}

	// Capture current output
	before, _ := inj.ReadOutput(*sess, 5)

	// Send a lightweight probe (empty line or simple test)
	start := time.Now()
	if err := inj.SendKeys(*sess, ""); err != nil {
		result.Responsive = false
		return result, nil
	}

	// Wait briefly and check if output changed
	time.Sleep(2 * time.Second)
	after, _ := inj.ReadOutput(*sess, 5)

	result.Responsive = after != before
	result.Latency = time.Since(start)

	// If probe succeeded, start a usage window
	if result.Responsive && p.windows != nil {
		p.windows.StartWindow(sessionName, sess.CLI)
	}

	return result, nil
}

// Scheduler runs pings on configured schedules.
type Scheduler struct {
	mu        sync.Mutex
	service   *PingService
	schedules []PingSchedule
	stops     map[string]chan struct{}
}

// NewScheduler creates a new ping scheduler.
func NewScheduler(service *PingService) *Scheduler {
	return &Scheduler{
		service: service,
		stops:   make(map[string]chan struct{}),
	}
}

// AddSchedule registers a new ping schedule.
func (s *Scheduler) AddSchedule(sched PingSchedule) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.schedules = append(s.schedules, sched)
}

// Start begins running all schedules.
func (s *Scheduler) Start() {
	s.mu.Lock()
	defer s.mu.Unlock()

	for _, sched := range s.schedules {
		stop := make(chan struct{})
		s.stops[sched.SessionName] = stop
		go s.runSchedule(sched, stop)
	}
}

// Stop halts all schedules.
func (s *Scheduler) Stop() {
	s.mu.Lock()
	defer s.mu.Unlock()
	for name, stop := range s.stops {
		close(stop)
		delete(s.stops, name)
	}
}

func (s *Scheduler) runSchedule(sched PingSchedule, stop chan struct{}) {
	for {
		now := time.Now()
		nextRun := s.nextRunTime(sched, now)
		waitDuration := time.Until(nextRun)

		select {
		case <-stop:
			return
		case <-time.After(waitDuration):
			s.executePing(sched)

			if sched.Repeat == pkg.RepeatOnce {
				return
			}
		}
	}
}

func (s *Scheduler) nextRunTime(sched PingSchedule, now time.Time) time.Time {
	hour, min := parseTime(sched.Time)
	target := time.Date(now.Year(), now.Month(), now.Day(), hour, min, 0, 0, now.Location())

	if target.Before(now) {
		target = target.Add(24 * time.Hour)
	}

	if sched.Repeat == pkg.RepeatWeekdays {
		for target.Weekday() == time.Saturday || target.Weekday() == time.Sunday {
			target = target.Add(24 * time.Hour)
		}
	}

	return target
}

func (s *Scheduler) executePing(sched PingSchedule) {
	switch sched.Probe {
	case pkg.ProbeLiveness:
		s.service.LivenessCheck(sched.SessionName)
	case pkg.ProbeActive:
		s.service.ActiveProbe(sched.SessionName)
	case pkg.ProbeBoth:
		s.service.LivenessCheck(sched.SessionName)
		s.service.ActiveProbe(sched.SessionName)
	}
}

func parseTime(t string) (int, int) {
	parsed, err := time.Parse("15:04", t)
	if err != nil {
		return 0, 0
	}
	return parsed.Hour(), parsed.Minute()
}
