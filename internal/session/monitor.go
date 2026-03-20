package session

import (
	"sync"
	"time"

	"github.com/kickthemoon0817/mother-terminal/internal/backend"
	"github.com/kickthemoon0817/mother-terminal/pkg"
)

// Event represents a monitor event.
type Event struct {
	SessionName string
	Type        EventType
	Message     string
	Timestamp   time.Time
}

// EventType identifies the type of monitor event.
type EventType string

const (
	EventStalled  EventType = "stalled"
	EventResumed  EventType = "resumed"
	EventDied     EventType = "died"
	EventRecovery EventType = "recovery"
)

// Monitor watches sessions for stall conditions and applies recovery policies.
type Monitor struct {
	mu       sync.Mutex
	manager  *Manager
	registry *backend.Registry
	events   chan Event
	done     chan struct{}
	watchers map[string]chan struct{}
	defaults MonitorDefaults
}

// MonitorDefaults holds default monitoring settings.
type MonitorDefaults struct {
	StallTimeout  time.Duration
	PollInterval  time.Duration
	ResumeMessage string
}

// NewMonitor creates a new session monitor.
func NewMonitor(manager *Manager, registry *backend.Registry, defaults MonitorDefaults) *Monitor {
	if defaults.PollInterval == 0 {
		defaults.PollInterval = 5 * time.Second
	}
	if defaults.StallTimeout == 0 {
		defaults.StallTimeout = 120 * time.Second
	}
	if defaults.ResumeMessage == "" {
		defaults.ResumeMessage = "continue"
	}

	return &Monitor{
		manager:  manager,
		registry: registry,
		events:   make(chan Event, 100),
		done:     make(chan struct{}),
		watchers: make(map[string]chan struct{}),
		defaults: defaults,
	}
}

// Events returns the event channel.
func (m *Monitor) Events() <-chan Event {
	return m.events
}

// Done returns a channel that is closed when the monitor is stopped.
func (m *Monitor) Done() <-chan struct{} {
	return m.done
}

// Watch starts monitoring a session for stalls.
func (m *Monitor) Watch(sessionName string) {
	m.mu.Lock()
	if _, exists := m.watchers[sessionName]; exists {
		m.mu.Unlock()
		return
	}
	stop := make(chan struct{})
	m.watchers[sessionName] = stop
	m.mu.Unlock()

	go m.watchLoop(sessionName, stop)
}

// Unwatch stops monitoring a session.
func (m *Monitor) Unwatch(sessionName string) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if stop, ok := m.watchers[sessionName]; ok {
		close(stop)
		delete(m.watchers, sessionName)
	}
}

// UnwatchAll stops monitoring all sessions and signals done.
func (m *Monitor) UnwatchAll() {
	m.mu.Lock()
	defer m.mu.Unlock()
	for name, stop := range m.watchers {
		close(stop)
		delete(m.watchers, name)
	}
	select {
	case <-m.done:
		// already closed
	default:
		close(m.done)
	}
}

// emit sends an event without blocking. Drops the event if the channel is full.
func (m *Monitor) emit(event Event) {
	select {
	case m.events <- event:
	default:
	}
}

func (m *Monitor) watchLoop(sessionName string, stop chan struct{}) {
	ticker := time.NewTicker(m.defaults.PollInterval)
	defer ticker.Stop()

	var lastOutput string
	var unchangedSince time.Time
	stalled := false

	for {
		select {
		case <-stop:
			return
		case <-ticker.C:
			sess, err := m.manager.Get(sessionName)
			if err != nil {
				continue
			}

			if sess.Status == pkg.StatusDead {
				return
			}

			// Read current output
			inj, err := m.registry.Get(sess.Backend)
			if err != nil {
				continue
			}

			output, err := inj.ReadOutput(*sess, 20)
			if err != nil {
				continue
			}

			if output == lastOutput {
				if unchangedSince.IsZero() {
					unchangedSince = time.Now()
				}

				stallTimeout := sess.StallTimeout
				if stallTimeout == 0 {
					stallTimeout = m.defaults.StallTimeout
				}

				if !stalled && time.Since(unchangedSince) > stallTimeout {
					stalled = true
					m.manager.UpdateStatus(sessionName, pkg.StatusStalled)

					m.emit(Event{
						SessionName: sessionName,
						Type:        EventStalled,
						Message:     "session output unchanged, marking as stalled",
						Timestamp:   time.Now(),
					})

					// Apply stall policy
					m.applyPolicy(sess, inj)
				}
			} else {
				lastOutput = output
				unchangedSince = time.Time{}

				if stalled {
					stalled = false
					m.manager.UpdateStatus(sessionName, pkg.StatusActive)
					m.emit(Event{
						SessionName: sessionName,
						Type:        EventResumed,
						Message:     "session output changed, marking as active",
						Timestamp:   time.Now(),
					})
				}
			}
		}
	}
}

func (m *Monitor) applyPolicy(sess *pkg.Session, inj backend.Injector) {
	policy := sess.Policy
	if policy == "" {
		policy = pkg.PolicyNotify
	}

	switch policy {
	case pkg.PolicyAutoResume:
		msg := sess.ResumeMessage
		if msg == "" {
			msg = m.defaults.ResumeMessage
		}
		if err := inj.SendKeys(*sess, msg); err == nil {
			m.emit(Event{
				SessionName: sess.Name,
				Type:        EventRecovery,
				Message:     "auto-resume sent: " + msg,
				Timestamp:   time.Now(),
			})
		}
	case pkg.PolicyNotify:
		// Event already emitted above
	case pkg.PolicyCustom:
		// Custom policies can be extended later
	}
}
