package session

import (
	"fmt"
	"sort"
	"sync"

	"github.com/kickthemoon0817/mother-terminal/pkg"
)

// Manager manages session lifecycle.
type Manager struct {
	mu       sync.RWMutex
	sessions map[string]*pkg.Session
}

// NewManager creates a new session manager.
func NewManager() *Manager {
	return &Manager{
		sessions: make(map[string]*pkg.Session),
	}
}

// Add registers a new session.
func (m *Manager) Add(s pkg.Session) error {
	m.mu.Lock()
	defer m.mu.Unlock()

	key := s.Name
	if key == "" {
		key = s.ID
	}

	if _, exists := m.sessions[key]; exists {
		return fmt.Errorf("%w: %s", pkg.ErrSessionAlreadyExists, key)
	}

	m.sessions[key] = &s
	return nil
}

// Remove deletes a session.
func (m *Manager) Remove(name string) {
	m.mu.Lock()
	defer m.mu.Unlock()
	delete(m.sessions, name)
}

// Get returns a session by name.
func (m *Manager) Get(name string) (*pkg.Session, error) {
	m.mu.RLock()
	defer m.mu.RUnlock()

	s, ok := m.sessions[name]
	if !ok {
		return nil, fmt.Errorf("%w: %s", pkg.ErrSessionNotFound, name)
	}
	return s, nil
}

// List returns all sessions sorted by StartTime (newest first), then by Name.
func (m *Manager) List() []pkg.Session {
	m.mu.RLock()
	defer m.mu.RUnlock()

	result := make([]pkg.Session, 0, len(m.sessions))
	for _, s := range m.sessions {
		result = append(result, *s)
	}
	sort.Slice(result, func(i, j int) bool {
		if result[i].StartTime != result[j].StartTime {
			return result[i].StartTime > result[j].StartTime
		}
		return result[i].Name < result[j].Name
	})
	return result
}

// UpdateStatus transitions a session to a new state.
func (m *Manager) UpdateStatus(name string, status pkg.SessionStatus) error {
	m.mu.Lock()
	defer m.mu.Unlock()

	s, ok := m.sessions[name]
	if !ok {
		return fmt.Errorf("%w: %s", pkg.ErrSessionNotFound, name)
	}

	if err := ValidateTransition(s.Status, status); err != nil {
		return err
	}

	s.Status = status
	return nil
}

// AddOrUpdate adds a session or updates it if it already exists.
func (m *Manager) AddOrUpdate(s pkg.Session) {
	m.mu.Lock()
	defer m.mu.Unlock()

	key := s.Name
	if key == "" {
		key = s.ID
	}

	if existing, ok := m.sessions[key]; ok {
		// Preserve status from existing session
		s.Status = existing.Status
	}

	m.sessions[key] = &s
}
