package backend

import (
	"sync"

	"github.com/kickthemoon0817/mother-terminal/pkg"
)

// Registry manages available injection backends.
type Registry struct {
	mu       sync.RWMutex
	backends map[pkg.BackendType]Injector
}

// NewRegistry creates a new empty backend registry.
func NewRegistry() *Registry {
	return &Registry{
		backends: make(map[pkg.BackendType]Injector),
	}
}

// Register adds a backend to the registry if it is available on this platform.
func (r *Registry) Register(b Injector) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if b.IsAvailable() {
		r.backends[b.Name()] = b
	}
}

// Get returns a specific backend by type, or ErrBackendUnavailable.
func (r *Registry) Get(bt pkg.BackendType) (Injector, error) {
	r.mu.RLock()
	defer r.mu.RUnlock()
	b, ok := r.backends[bt]
	if !ok {
		return nil, pkg.ErrBackendUnavailable
	}
	return b, nil
}

// Available returns all backends that are available on this platform.
func (r *Registry) Available() []Injector {
	r.mu.RLock()
	defer r.mu.RUnlock()
	result := make([]Injector, 0, len(r.backends))
	for _, b := range r.backends {
		result = append(result, b)
	}
	return result
}

// DiscoverAll runs Discover on all available backends and merges results.
func (r *Registry) DiscoverAll() ([]pkg.Session, error) {
	r.mu.RLock()
	backends := make([]Injector, 0, len(r.backends))
	for _, b := range r.backends {
		backends = append(backends, b)
	}
	r.mu.RUnlock()

	var (
		allSessions []pkg.Session
		mu          sync.Mutex
		wg          sync.WaitGroup
		firstErr    error
	)

	for _, b := range backends {
		wg.Add(1)
		go func(inj Injector) {
			defer wg.Done()
			sessions, err := inj.Discover()
			mu.Lock()
			defer mu.Unlock()
			if err != nil && firstErr == nil {
				firstErr = err
			}
			allSessions = append(allSessions, sessions...)
		}(b)
	}

	wg.Wait()
	return allSessions, firstErr
}
