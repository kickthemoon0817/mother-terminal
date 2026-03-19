package discovery

import (
	"sync"

	"github.com/kickthemoon0817/mother-terminal/pkg"
)

// Registry manages manually registered sessions.
type Registry struct {
	mu       sync.RWMutex
	sessions map[string]pkg.Session
}

// NewRegistry creates a new manual session registry.
func NewRegistry() *Registry {
	return &Registry{
		sessions: make(map[string]pkg.Session),
	}
}

// Register adds a manually configured session.
func (r *Registry) Register(s pkg.Session) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.sessions[s.Name] = s
}

// Unregister removes a session by name.
func (r *Registry) Unregister(name string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	delete(r.sessions, name)
}

// Get returns a session by name.
func (r *Registry) Get(name string) (pkg.Session, bool) {
	r.mu.RLock()
	defer r.mu.RUnlock()
	s, ok := r.sessions[name]
	return s, ok
}

// List returns all registered sessions.
func (r *Registry) List() []pkg.Session {
	r.mu.RLock()
	defer r.mu.RUnlock()
	result := make([]pkg.Session, 0, len(r.sessions))
	for _, s := range r.sessions {
		result = append(result, s)
	}
	return result
}

// Merge combines auto-discovered sessions with manual ones.
// Manual registrations take priority when names conflict.
func (r *Registry) Merge(discovered []pkg.Session) []pkg.Session {
	r.mu.RLock()
	defer r.mu.RUnlock()

	merged := make(map[string]pkg.Session)

	// Add discovered sessions first
	for _, s := range discovered {
		merged[s.Name] = s
	}

	// Override with manual registrations
	for name, s := range r.sessions {
		merged[name] = s
	}

	result := make([]pkg.Session, 0, len(merged))
	for _, s := range merged {
		result = append(result, s)
	}
	return result
}
